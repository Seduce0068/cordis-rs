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
//! `Some(false)` is detected via `Any` downcast and treated as non-bail.

use crate::error::{Error, Result};

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::primitives::Disposer;

/// A decode failure observed by a `typed_event!` listener.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EventDecodeError {
    pub event: &'static str,
    pub reason: String,
}

/// Declares a typed event dispatched through the named `Value` slot, so
/// `ctx.on(name, ...)` listeners and typed listeners share one stream.
/// Payload must implement `Serialize` + `DeserializeOwned`. Decode failures
/// are recorded (never silently dropped) via `take_decode_errors()`.
#[macro_export]
macro_rules! typed_event {
    ($name:ident, $event_name:expr, $payload:ty, $store:ident) => {
        static $store: std::sync::OnceLock<std::sync::Mutex<Vec<$crate::EventDecodeError>>> =
            std::sync::OnceLock::new();

        pub struct $name;

        impl $name {
            pub const NAME: &'static str = $event_name;

            pub fn take_decode_errors() -> Vec<$crate::EventDecodeError> {
                let store = $store.get_or_init(|| std::sync::Mutex::new(Vec::new()));
                std::mem::take(&mut *store.lock().expect("decode error store poisoned"))
            }

            pub fn on<F>(ctx: &$crate::Context, listener: F) -> $crate::Disposer
            where
                F: Fn($payload) -> $crate::events::BoxFuture<'static, ()> + Send + Sync + 'static,
            {
                let listener = std::sync::Arc::new(listener);
                ctx.on(Self::NAME, move |value: serde_json::Value| {
                    let listener = listener.clone();
                    Box::pin(async move {
                        match serde_json::from_value::<$payload>(value) {
                            Ok(typed) => listener(typed).await,
                            Err(error) => {
                                let store =
                                    $store.get_or_init(|| std::sync::Mutex::new(Vec::new()));
                                store.lock().expect("decode error store poisoned").push(
                                    $crate::EventDecodeError {
                                        event: Self::NAME,
                                        reason: error.to_string(),
                                    },
                                );
                            }
                        }
                    })
                })
            }

            pub async fn emit(ctx: &$crate::Context, payload: &$payload) -> $crate::Result<()> {
                let value = serde_json::to_value(payload)
                    .map_err(|error| $crate::Error::Other(error.to_string()))?;
                ctx.emit(Self::NAME, value).await;
                Ok(())
            }
        }
    };
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Build a disposer that removes one listener from the registry by id.
/// Shared by all `on_*_with_registry` helpers (R1 review).
fn disposer_for(registry: &EventRegistry, key: SlotKey, id: u64) -> Disposer {
    let registry = registry.clone();
    Disposer::new(move || {
        let registry = registry.clone();
        let key = key.clone();
        async move { registry.remove(key, id) }
    })
}
pub type Shared<A> = Arc<Mutex<A>>;

pub type Next<A, R> = Arc<dyn Fn(Shared<A>) -> BoxFuture<'static, Result<R>> + Send + Sync>;
pub type WaterfallListener<A, R> =
    Arc<dyn Fn(Shared<A>, Next<A, R>) -> BoxFuture<'static, Result<R>> + Send + Sync>;
pub type EmitListener<A> = Arc<dyn Fn(A) -> BoxFuture<'static, ()> + Send + Sync>;
/// Separate listener type for fallible parallel dispatch. Because `SlotKey`
/// includes `TypeId<L>`, these occupy a distinct slot from `EmitListener`.
pub type FallibleEmitListener<A> = Arc<dyn Fn(A) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Aggregate of fallible emit listener failures, in registration order.
#[derive(Debug)]
pub struct EmitErrors(pub Vec<Error>);

impl EmitErrors {
    /// Numbered breakdown of listener failures for diagnostics. The public
    /// tuple field already exposes the errors; this is a convenience for
    /// logging (swarm review R1-AP).
    pub fn details(&self) -> String {
        self.0
            .iter()
            .enumerate()
            .map(|(index, error)| format!("{}: {error}", index + 1))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

impl std::fmt::Display for EmitErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keep the established message: callers and tests depend on it.
        write!(f, "{} listener error(s)", self.0.len())
    }
}

impl std::error::Error for EmitErrors {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0
            .first()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}
pub type SerialListener<A, R> = Arc<dyn Fn(A) -> BoxFuture<'static, Option<R>> + Send + Sync>;
pub type BailListener<A, R> = Arc<dyn Fn(A) -> Option<R> + Send + Sync>;

pub type SlotKey = (String, TypeId, TypeId, TypeId);

/// Build a slot key for a named emit listener from `ctx.on(name, ...)`.
pub fn named_emit_slot(name: &str) -> SlotKey {
    (
        name.to_string(),
        TypeId::of::<serde_json::Value>(),
        TypeId::of::<()>(),
        TypeId::of::<EmitListener<serde_json::Value>>(),
    )
}

type ErasedListener = Arc<dyn Any + Send + Sync>;
type ErasedListeners = Arc<RwLock<Vec<(u64, ErasedListener)>>>;

/// Options for a named listener registration (upstream `EventOptions`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventOptions {
    /// Insert at the head of the listener list instead of the tail.
    pub prepend: bool,
    /// Ignore caller-scope filtering during dispatch.
    pub global: bool,
}

impl From<bool> for EventOptions {
    fn from(prepend: bool) -> Self {
        Self {
            prepend,
            global: false,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct HookMeta {
    options: EventOptions,
}

#[derive(Clone, Default)]
pub struct EventRegistry {
    slots: Arc<RwLock<HashMap<SlotKey, ErasedListeners>>>,
    next_id: Arc<AtomicU64>,
    meta: Arc<RwLock<HashMap<u64, HookMeta>>>,
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
    pub fn push<T>(&self, key: SlotKey, listener: T) -> u64
    where
        T: Send + Sync + 'static,
    {
        self.push_with(key, listener, EventOptions::default())
    }

    /// Push with options, honouring `prepend` ordering.
    pub fn push_with<T>(&self, key: SlotKey, listener: T, options: EventOptions) -> u64
    where
        T: Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // Publish metadata before the listener becomes visible in the slot:
        // a concurrent dispatch may call is_global(id) as soon as the slot
        // contains the listener, and must not observe missing metadata
        // (swarm review R1-AP).
        self.meta
            .write()
            .expect("event meta poisoned")
            .insert(id, HookMeta { options });
        let binding = self.slot(key);
        let mut slot = binding.write().expect("event slot poisoned");
        if options.prepend {
            slot.insert(0, (id, Arc::new(listener)));
        } else {
            slot.push((id, Arc::new(listener)));
        }
        drop(slot);
        id
    }

    /// Snapshot listeners together with their registration ids.
    pub fn snapshot_with_ids<T>(&self, key: SlotKey) -> Vec<(u64, T)>
    where
        T: Clone + Send + Sync + 'static,
    {
        let erased = self.slot(key).read().expect("event slot poisoned").clone();
        erased
            .into_iter()
            .filter_map(|(id, listener)| {
                listener
                    .downcast::<T>()
                    .ok()
                    .map(|arc| (id, (*arc).clone()))
            })
            .collect()
    }

    /// Whether a listener id is global (ignores scope filtering).
    pub fn is_global(&self, id: u64) -> bool {
        self.meta
            .read()
            .expect("event meta poisoned")
            .get(&id)
            .is_some_and(|meta| meta.options.global)
    }

    /// Replace a listener in place by id (used by `once` wrapping).
    pub fn replace<T: Send + Sync + 'static>(&self, key: SlotKey, id: u64, listener: T) -> bool {
        let binding = self.slot(key);
        let mut slot = binding.write().expect("event slot poisoned");
        if let Some(entry) = slot.iter_mut().find(|(current, _)| *current == id) {
            entry.1 = Arc::new(listener);
            true
        } else {
            false
        }
    }

    pub fn remove(&self, key: SlotKey, id: u64) -> Result<()> {
        let binding = self.slot(key);
        binding
            .write()
            .expect("event slot poisoned")
            .retain(|(current, _)| *current != id);
        // Drop the option metadata too; otherwise HookMeta accumulates in
        // long-running hosts even after every listener is unloaded.
        self.meta.write().expect("event meta poisoned").remove(&id);
        Ok(())
    }

    /// Number of live hook metadata entries (test/observability accessor).
    pub fn meta_len(&self) -> usize {
        self.meta.read().expect("event meta poisoned").len()
    }

    pub fn snapshot<T>(&self, key: SlotKey) -> Vec<T>
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
    pub fn on_waterfall<F>(&self, ctx: &crate::Context, listener: F) -> Disposer
    where
        F: Fn(Shared<A>, Next<A, R>) -> BoxFuture<'static, Result<R>> + Send + Sync + 'static,
    {
        self.on_waterfall_with_registry(ctx.event_registry(), listener)
    }

    pub fn on_waterfall_with_registry<F>(&self, registry: &EventRegistry, listener: F) -> Disposer
    where
        F: Fn(Shared<A>, Next<A, R>) -> BoxFuture<'static, Result<R>> + Send + Sync + 'static,
    {
        let listener: WaterfallListener<A, R> = Arc::new(listener);
        let registry = registry.clone();
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
    /// Calling `next` twice returns `Err`, as upstream throws
    /// `next() called multiple times` (catchable rather than a panic).
    pub async fn waterfall<F>(&self, ctx: &crate::Context, args: A, terminal: F) -> Result<R>
    where
        F: Fn(&A) -> R + Send + Sync + 'static,
    {
        self.waterfall_with_registry(ctx.event_registry(), args, terminal)
            .await
    }

    /// Dispatch with an explicit registry, so a service can hold only the
    /// registry (Arc-backed, no Context back-reference) instead of a full
    /// Context and avoid an ownership cycle.
    pub async fn waterfall_with_registry<F>(
        &self,
        registry: &EventRegistry,
        args: A,
        terminal: F,
    ) -> Result<R>
    where
        F: Fn(&A) -> R + Send + Sync + 'static,
    {
        let terminal: Arc<dyn Fn(&A) -> R + Send + Sync> = Arc::new(terminal);
        let mut next: Next<A, R> = Arc::new(move |shared: Shared<A>| {
            let terminal = terminal.clone();
            Box::pin(async move {
                let guard = shared.lock().expect("event args poisoned");
                Ok(terminal(&guard))
            }) as BoxFuture<'static, Result<R>>
        });

        // One flag per chain so a discarded second future is still detected
        // after the chain completes (reviewer hardening).
        let double_called = Arc::new(AtomicBool::new(false));
        let listeners: Vec<WaterfallListener<A, R>> =
            registry.snapshot(self.key::<WaterfallListener<A, R>>());
        for listener in listeners.into_iter().rev() {
            let listener = listener.clone();
            let inner = next.clone();
            let double_called = double_called.clone();
            next = Arc::new(move |shared: Shared<A>| {
                let listener = listener.clone();
                let inner = inner.clone();
                let shared = shared.clone();
                let double_called = double_called.clone();
                Box::pin(async move {
                    let called = Arc::new(AtomicBool::new(false));
                    let inner = inner.clone();
                    let guarded: Next<A, R> = Arc::new(move |next_shared| {
                        if called.swap(true, Ordering::SeqCst) {
                            double_called.store(true, Ordering::SeqCst);
                            return Box::pin(async {
                                Err(Error::Other("next() called multiple times".to_string()))
                            }) as BoxFuture<'static, Result<R>>;
                        }
                        inner(next_shared)
                    });
                    listener(shared, guarded).await
                }) as BoxFuture<'static, Result<R>>
            });
        }

        let result = next(Arc::new(Mutex::new(args))).await;
        if double_called.load(Ordering::SeqCst) {
            return Err(Error::Other("next() called multiple times".to_string()));
        }
        result
    }

    /// Register a serial listener. Returning `Some` bails the chain.
    pub fn on_serial<F>(&self, ctx: &crate::Context, listener: F) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, Option<R>> + Send + Sync + 'static,
    {
        self.on_serial_with_registry(ctx.event_registry(), listener)
    }

    /// Register with an explicit registry (ADR-010 stage 4).
    pub fn on_serial_with_registry<F>(&self, registry: &EventRegistry, listener: F) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, Option<R>> + Send + Sync + 'static,
    {
        let listener: SerialListener<A, R> = Arc::new(listener);
        let key = self.key::<SerialListener<A, R>>();
        let id = registry.push::<SerialListener<A, R>>(key.clone(), listener);
        disposer_for(registry, key, id)
    }

    /// Upstream's `false` is a non-bail value; detect it generically via Any.
    fn is_bail_value(value: &R) -> bool {
        let any = value as &dyn std::any::Any;
        !matches!(any.downcast_ref::<bool>(), Some(false))
    }

    /// Await serial listeners in registration order, stopping at the first bail.
    pub async fn serial(&self, ctx: &crate::Context, args: A) -> Option<R> {
        self.serial_with_registry(ctx.event_registry(), args).await
    }

    /// Dispatch with an explicit registry (ADR-010 stage 4).
    pub async fn serial_with_registry(&self, registry: &EventRegistry, args: A) -> Option<R> {
        let listeners: Vec<SerialListener<A, R>> =
            registry.snapshot(self.key::<SerialListener<A, R>>());
        for listener in listeners {
            if let Some(value) = listener(args.clone()).await
                && Self::is_bail_value(&value)
            {
                return Some(value);
            }
        }
        None
    }

    /// Register a synchronous bail listener. Returning `Some` bails the chain.
    pub fn on_bail<F>(&self, ctx: &crate::Context, listener: F) -> Disposer
    where
        F: Fn(A) -> Option<R> + Send + Sync + 'static,
    {
        self.on_bail_with_registry(ctx.event_registry(), listener)
    }

    /// Register with an explicit registry (ADR-010 stage 4).
    pub fn on_bail_with_registry<F>(&self, registry: &EventRegistry, listener: F) -> Disposer
    where
        F: Fn(A) -> Option<R> + Send + Sync + 'static,
    {
        let listener: BailListener<A, R> = Arc::new(listener);
        let key = self.key::<BailListener<A, R>>();
        let id = registry.push::<BailListener<A, R>>(key.clone(), listener);
        disposer_for(registry, key, id)
    }

    /// Run synchronous bail listeners in registration order.
    pub fn bail(&self, ctx: &crate::Context, args: A) -> Option<R> {
        self.bail_with_registry(ctx.event_registry(), args)
    }

    /// Dispatch with an explicit registry (ADR-010 stage 4).
    pub fn bail_with_registry(&self, registry: &EventRegistry, args: A) -> Option<R> {
        let listeners: Vec<BailListener<A, R>> =
            registry.snapshot(self.key::<BailListener<A, R>>());
        for listener in listeners {
            if let Some(value) = listener(args.clone())
                && Self::is_bail_value(&value)
            {
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
    pub fn on_emit<F>(&self, ctx: &crate::Context, listener: F) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.on_emit_with_registry(ctx.event_registry(), listener)
    }

    /// Register with an explicit registry (ADR-010 stage 4).
    pub fn on_emit_with_registry<F>(&self, registry: &EventRegistry, listener: F) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        let listener: EmitListener<A> = Arc::new(listener);
        let key = self.key::<EmitListener<A>>();
        let id = registry.push::<EmitListener<A>>(key.clone(), listener);
        disposer_for(registry, key, id)
    }

    /// Await observers in registration order (deterministic test helper).
    /// A panicking observer stops dispatch and propagates, matching upstream.
    pub async fn emit(&self, ctx: &crate::Context, args: A)
    where
        A: Clone,
    {
        self.emit_with_registry(ctx.event_registry(), args).await
    }

    /// Dispatch with an explicit registry (ADR-010 stage 4).
    pub async fn emit_with_registry(&self, registry: &EventRegistry, args: A)
    where
        A: Clone,
    {
        let listeners: Vec<EmitListener<A>> =
            registry.snapshot::<EmitListener<A>>(self.key::<EmitListener<A>>());
        for listener in listeners {
            listener(args.clone()).await;
        }
    }

    /// Fire-and-forget dispatch on the current Tokio runtime.
    /// Spawn each listener on the current Tokio runtime (fire-and-forget).
    pub fn emit_spawn(&self, ctx: &crate::Context, args: A)
    where
        A: Clone + 'static,
    {
        self.emit_spawn_with_registry(ctx.event_registry(), args)
    }

    /// Fire-and-forget dispatch with an explicit registry (ADR-010 stage 4).
    /// The registry borrow ends before spawn so the moved listener stays 'static.
    pub fn emit_spawn_with_registry(&self, registry: &EventRegistry, args: A)
    where
        A: Clone + 'static,
    {
        let listeners: Vec<EmitListener<A>> =
            registry.snapshot::<EmitListener<A>>(self.key::<EmitListener<A>>());
        for listener in listeners {
            let args = args.clone();
            tokio::spawn(async move { listener(args).await });
        }
    }

    /// Deprecated alias for [`Event::emit_spawn`].
    #[deprecated(note = "use `emit_spawn` instead")]
    pub fn emit_detached(&self, ctx: &crate::Context, args: A)
    where
        A: Clone + 'static,
    {
        self.emit_spawn(ctx, args)
    }
}

impl<A> Event<A, ()>
where
    A: Clone + Send + Sync + 'static,
{
    /// Run all observers concurrently on the current executor.
    pub async fn parallel(&self, ctx: &crate::Context, args: A) {
        self.parallel_with_registry(ctx.event_registry(), args)
            .await
    }

    /// Dispatch with an explicit registry (ADR-010 stage 4).
    pub async fn parallel_with_registry(&self, registry: &EventRegistry, args: A) {
        let listeners: Vec<EmitListener<A>> =
            registry.snapshot::<EmitListener<A>>(self.key::<EmitListener<A>>());
        let futures = listeners.into_iter().map(|listener| {
            let args = args.clone();
            async move { listener(args).await }
        });
        futures::future::join_all(futures).await;
    }

    /// Register a fallible observer. These live in a separate slot, so they
    /// are dispatched only by `parallel_errors`, never by `emit`/`parallel`.
    pub fn on_emit_fallible<F>(&self, ctx: &crate::Context, listener: F) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
    {
        self.on_emit_fallible_with_registry(ctx.event_registry(), listener)
    }

    /// Register with an explicit registry (ADR-010 stage 4).
    pub fn on_emit_fallible_with_registry<F>(
        &self,
        registry: &EventRegistry,
        listener: F,
    ) -> Disposer
    where
        F: Fn(A) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
    {
        let listener: FallibleEmitListener<A> = Arc::new(listener);
        let key = self.key::<FallibleEmitListener<A>>();
        let id = registry.push::<FallibleEmitListener<A>>(key.clone(), listener);
        disposer_for(registry, key, id)
    }

    /// Run fallible observers concurrently and aggregate every failure in
    /// registration order (upstream `AggregateError` semantics). Panics still
    /// propagate; only returned errors are collected.
    pub async fn parallel_errors(
        &self,
        ctx: &crate::Context,
        args: A,
    ) -> std::result::Result<(), EmitErrors> {
        self.parallel_errors_with_registry(ctx.event_registry(), args)
            .await
    }

    /// Dispatch with an explicit registry (ADR-010 stage 4).
    pub async fn parallel_errors_with_registry(
        &self,
        registry: &EventRegistry,
        args: A,
    ) -> std::result::Result<(), EmitErrors> {
        let listeners: Vec<FallibleEmitListener<A>> =
            registry.snapshot::<FallibleEmitListener<A>>(self.key::<FallibleEmitListener<A>>());
        let futures = listeners.into_iter().map(|listener| {
            let args = args.clone();
            async move { listener(args).await }
        });
        let results = futures::future::join_all(futures).await;
        let errors: Vec<Error> = results
            .into_iter()
            .filter_map(std::result::Result::err)
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(EmitErrors(errors))
        }
    }
}
