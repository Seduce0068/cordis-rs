//! Typed event bus reproducing Cordis dispatch modes.
//!
//! Source: `vendor/cordis/src/events.ts` in deepseek-ai/deepseek-harness
//! (upstream cordiverse/cordis v4.0.0-rc.12).
//!
//! | mode | awaited | order | result |
//! |---|---|---|---|
//! | `emit` | no (awaited here for determinism) | registration | none |
//! | `parallel` | yes | concurrent | none, failures aggregated |
//! | `serial` | yes | registration, stop on first bail | first bail value |
//! | `bail` | no (synchronous) | registration, stop on first bail | first bail value |
//! | `waterfall` | yes | registration, around | outermost return value |
//!
//! In upstream, `isBailed(v) = v !== null && v !== false && v !== undefined`.
//! This crate uses `Option<R>`: `None` is a non-bail, `Some(value)` bails.
//! `Some(0)` / `Some(String::new())` therefore bail, matching upstream.
//! `Some(false)` is the one divergence (upstream `false` does not bail).

use crate::error::Result;

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::context::{Context, Disposer};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type Shared<A> = Arc<Mutex<A>>;

pub type Next<A, R> = Arc<dyn Fn(Shared<A>) -> BoxFuture<'static, R> + Send + Sync>;
pub type WaterfallListener<A, R> =
    Arc<dyn Fn(Shared<A>, Next<A, R>) -> BoxFuture<'static, R> + Send + Sync>;
pub type EmitListener<A> = Arc<dyn Fn(A) -> BoxFuture<'static, ()> + Send + Sync>;
pub type SerialListener<A, R> = Arc<dyn Fn(A) -> BoxFuture<'static, Option<R>> + Send + Sync>;
pub type BailListener<A, R> = Arc<dyn Fn(A) -> Option<R> + Send + Sync>;

type SlotKey = (String, TypeId, TypeId, TypeId);

type ErasedListener = Arc<dyn Any + Send + Sync>;
type ErasedListeners = Arc<RwLock<Vec<(u64, ErasedListener)>>>;

#[derive(Clone, Default)]
pub struct EventRegistry {
    slots: Arc<RwLock<HashMap<SlotKey, ErasedListeners>>>,
    next_id: Arc<AtomicU64>,
}

impl EventRegistry {
    fn slot(&self, key: SlotKey) -> ErasedListeners {
        self.slots
            .write()
            .expect("event slots poisoned")
            .entry(key)
            .or_default()
            .clone()
    }

    /// `T` is an owned listener handle such as `Arc<dyn Fn...>`.
    fn push<T>(&self, key: SlotKey, listener: T) -> u64
    where
        T: Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.slot(key)
            .write()
            .expect("event slot poisoned")
            .push((id, Arc::new(listener)));
        id
    }

    fn remove(&self, key: SlotKey, id: u64) -> Result<()> {
        self.slot(key)
            .write()
            .expect("event slot poisoned")
            .retain(|(current, _)| *current != id);
        Ok(())
    }

    fn snapshot<T>(&self, key: SlotKey) -> Vec<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        let erased = self.slot(key).read().expect("event slot poisoned").clone();
        erased
            .into_iter()
            .filter_map(|(_, listener)| listener.downcast::<T>().ok().map(|arc| (*arc).clone()))
            .collect()
    }
}

/// A typed event descriptor. Cloneable and shareable across plugins.
pub struct Event<A, R = ()> {
    name: String,
    _marker: PhantomData<fn(A) -> R>,
}

impl<A, R> Clone for Event<A, R> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            _marker: PhantomData,
        }
    }
}

impl<A, R> Event<A, R>
where
    A: Clone + Send + Sync + 'static,
    R: Send + Sync + 'static,
{
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            _marker: PhantomData,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn key<L: ?Sized + 'static>(&self) -> SlotKey {
        (
            self.name.clone(),
            TypeId::of::<A>(),
            TypeId::of::<R>(),
            TypeId::of::<L>(),
        )
    }

    /// Register an around-middleware listener. Returns a one-shot disposer.
    pub fn on_waterfall<F>(&self, ctx: &Context, listener: F) -> Disposer
    where
        F: Fn(Shared<A>, Next<A, R>) -> BoxFuture<'static, R> + Send + Sync + 'static,
    {
        let listener: WaterfallListener<A, R> = Arc::new(listener);
        let registry = ctx.event_registry().clone();
        let key = self.key::<WaterfallListener<A, R>>();
        let id = registry.push::<WaterfallListener<A, R>>(key.clone(), listener.clone());
        Disposer::new(move || {
            let registry = registry.clone();
            let key = key.clone();
            async move { registry.remove(key, id) }
        })
    }

    /// Run the around-middleware chain, outermost listener first.
    ///
    /// Each listener may mutate the shared argument before and/or after
    /// calling `next`. Not calling `next` vetoes the rest of the chain.
    /// Calling `next` twice panics, as upstream throws `next() called multiple times`.
    pub async fn waterfall<F>(&self, ctx: &Context, args: A, terminal: F) -> R
    where
        F: Fn(&A) -> R + Send + Sync + 'static,
    {
        let terminal: Arc<dyn Fn(&A) -> R + Send + Sync> = Arc::new(terminal);
        let mut next: Next<A, R> = Arc::new(move |shared: Shared<A>| {
            let terminal = terminal.clone();
            Box::pin(async move {
                let guard = shared.lock().expect("event args poisoned");
                terminal(&guard)
            }) as BoxFuture<'static, R>
        });

        let listeners: Vec<WaterfallListener<A, R>> = ctx
            .event_registry()
            .snapshot(self.key::<WaterfallListener<A, R>>());
        for listener in listeners.into_iter().rev() {
            let listener = listener.clone();
            let inner = next.clone();
            next = Arc::new(move |shared: Shared<A>| {
                let listener = listener.clone();
                let inner = inner.clone();
                let shared = shared.clone();
                Box::pin(async move {
                    let called = Arc::new(AtomicBool::new(false));
                    let inner = inner.clone();
                    let guarded: Next<A, R> = Arc::new(move |next_shared| {
                        assert!(
                            !called.swap(true, Ordering::SeqCst),
                            "next() called multiple times"
                        );
                        inner(next_shared)
                    });
                    listener(shared, guarded).await
                }) as BoxFuture<'static, R>
            });
        }

        next(Arc::new(Mutex::new(args))).await
    }

    /// Register a serial listener. Returning `Some` bails the chain.
    pub fn on_serial<F>(&self, ctx: &Context, listener: F) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, Option<R>> + Send + Sync + 'static,
    {
        let listener: SerialListener<A, R> = Arc::new(listener);
        let registry = ctx.event_registry().clone();
        let key = self.key::<SerialListener<A, R>>();
        let id = registry.push::<SerialListener<A, R>>(key.clone(), listener.clone());
        Disposer::new(move || {
            let registry = registry.clone();
            let key = key.clone();
            async move { registry.remove(key, id) }
        })
    }

    /// Await serial listeners in registration order, stopping at the first bail.
    pub async fn serial(&self, ctx: &Context, args: A) -> Option<R> {
        let listeners: Vec<SerialListener<A, R>> = ctx
            .event_registry()
            .snapshot(self.key::<SerialListener<A, R>>());
        for listener in listeners {
            if let Some(value) = listener(args.clone()).await {
                return Some(value);
            }
        }
        None
    }

    /// Register a synchronous bail listener. Returning `Some` bails the chain.
    pub fn on_bail<F>(&self, ctx: &Context, listener: F) -> Disposer
    where
        F: Fn(A) -> Option<R> + Send + Sync + 'static,
    {
        let listener: BailListener<A, R> = Arc::new(listener);
        let registry = ctx.event_registry().clone();
        let key = self.key::<BailListener<A, R>>();
        let id = registry.push::<BailListener<A, R>>(key.clone(), listener.clone());
        Disposer::new(move || {
            let registry = registry.clone();
            let key = key.clone();
            async move { registry.remove(key, id) }
        })
    }

    /// Run synchronous bail listeners in registration order.
    pub fn bail(&self, ctx: &Context, args: A) -> Option<R> {
        let listeners: Vec<BailListener<A, R>> = ctx
            .event_registry()
            .snapshot(self.key::<BailListener<A, R>>());
        for listener in listeners {
            if let Some(value) = listener(args.clone()) {
                return Some(value);
            }
        }
        None
    }
}

impl<A> Event<A, ()>
where
    A: Clone + Send + Sync + 'static,
{
    pub fn on_emit<F>(&self, ctx: &Context, listener: F) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        let listener: EmitListener<A> = Arc::new(listener);
        let registry = ctx.event_registry().clone();
        let key = self.key::<EmitListener<A>>();
        let id = registry.push::<EmitListener<A>>(key.clone(), listener.clone());
        Disposer::new(move || {
            let registry = registry.clone();
            let key = key.clone();
            async move { registry.remove(key, id) }
        })
    }

    /// Await observers in registration order (deterministic test helper).
    /// A panicking observer stops dispatch and propagates, matching upstream.
    pub async fn emit(&self, ctx: &Context, args: A)
    where
        A: Clone,
    {
        let listeners: Vec<EmitListener<A>> = ctx
            .event_registry()
            .snapshot::<EmitListener<A>>(self.key::<EmitListener<A>>());
        for listener in listeners {
            listener(args.clone()).await;
        }
    }

    /// Fire-and-forget dispatch on the current Tokio runtime.
    pub fn emit_detached(&self, ctx: &Context, args: A)
    where
        A: Clone + 'static,
    {
        let listeners: Vec<EmitListener<A>> = ctx
            .event_registry()
            .snapshot::<EmitListener<A>>(self.key::<EmitListener<A>>());
        for listener in listeners {
            let args = args.clone();
            tokio::spawn(async move { listener(args).await });
        }
    }
}

impl<A> Event<A, ()>
where
    A: Clone + Send + Sync + 'static,
{
    /// Run all observers concurrently on the current executor.
    pub async fn parallel(&self, ctx: &Context, args: A) {
        let listeners: Vec<EmitListener<A>> = ctx
            .event_registry()
            .snapshot::<EmitListener<A>>(self.key::<EmitListener<A>>());
        let futures = listeners.into_iter().map(|listener| {
            let args = args.clone();
            async move { listener(args).await }
        });
        futures::future::join_all(futures).await;
    }
}
