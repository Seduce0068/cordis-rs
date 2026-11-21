//! Context, service registry, effects and lifecycle.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::Notify;

use crate::error::{Error, Result};
use crate::events::{BoxFuture, EventRegistry};

/// Seam for fibers that react to service changes. Defined here so `Context`
/// does not depend on the `plugin` module's concrete `FiberInner` type
/// (ADR-009). Implementations must not re-enter the notifying `Context`.
pub(crate) trait FiberObserver: Send + Sync {
    fn injects(&self, name: &str) -> bool;
    fn is_retiring(&self) -> bool;
    fn refresh(&self) -> BoxFuture<'_, crate::Result<()>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Pending = 0,
    Loading = 1,
    Active = 2,
    Unloading = 3,
    Unloaded = 4,
    Failed = 5,
}

impl From<u8> for Lifecycle {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Pending,
            1 => Self::Loading,
            2 => Self::Active,
            3 => Self::Unloading,
            4 => Self::Unloaded,
            _ => Self::Failed,
        }
    }
}

type Cleanup = Box<dyn FnOnce() -> BoxFuture<'static, Result<()>> + Send>;

/// One-shot reversible registration. Calling `dispose` twice is a no-op.
#[derive(Clone)]
pub struct Disposer {
    inner: Arc<Mutex<Option<Cleanup>>>,
}

impl Disposer {
    pub fn new<F, Fut>(setup: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            inner: Arc::new(Mutex::new(Some(Box::new(move || Box::pin(setup()))))),
        }
    }

    fn from_children(children: Arc<Mutex<Vec<Disposer>>>) -> Self {
        Self::new(move || {
            let children = children.clone();
            async move {
                let owned: Vec<Disposer> = {
                    let mut guard = children.lock().expect("effect scope poisoned");
                    guard.drain(..).collect()
                };
                let mut first_error = None;
                for disposer in owned.into_iter().rev() {
                    if let Err(error) = disposer.dispose().await {
                        first_error.get_or_insert(error);
                    }
                }
                match first_error {
                    Some(error) => Err(error),
                    None => Ok(()),
                }
            }
        })
    }

    /// Dispose and report whether this call claimed the cleanup (upstream's
    /// boolean disposer semantics). The cleanup is one-shot: even on error a
    /// later call returns `false`.
    pub async fn dispose_checked(&self) -> Result<bool> {
        let maybe_setup = self.inner.lock().expect("disposer poisoned").take();
        match maybe_setup {
            Some(setup) => {
                setup().await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub async fn dispose(&self) -> Result<()> {
        self.dispose_checked().await.map(|_| ())
    }

    /// Whether the cleanup has already been claimed, without awaiting it.
    pub fn is_spent(&self) -> bool {
        self.inner.lock().expect("disposer poisoned").is_none()
    }

    fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Remove a disposer from a list without taking ownership of it.
fn drop_disposer(list: &Mutex<Vec<Disposer>>, target: &Disposer) {
    list.lock()
        .expect("disposer list poisoned")
        .retain(|current| !current.same_as(target));
}

/// Scope passed to an effect setup closure. Registrations made through the
/// scope are unwound together, in reverse order, by the parent disposer.
pub struct EffectScope {
    ctx: Context,
    children: Arc<Mutex<Vec<Disposer>>>,
}

impl EffectScope {
    pub fn ctx(&self) -> &Context {
        &self.ctx
    }

    pub fn on_dispose<F, Fut>(&self, cleanup: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.children
            .lock()
            .expect("effect scope poisoned")
            .push(Disposer::new(cleanup));
    }

    pub async fn effect<F, Fut>(&self, setup: F) -> Result<()>
    where
        F: FnOnce(EffectScope) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let children = self.children.clone();
        self.ctx.effect_with_parent(setup, Some(children)).await
    }

    /// Register a service that is removed when this effect is disposed.
    pub async fn provide<T>(&self, name: impl Into<String>, service: T) -> Result<Disposer>
    where
        T: Any + Send + Sync,
    {
        let ctx = self.ctx.clone();
        let name = name.into();
        let disposer = ctx.provide(name, service).await?;
        let cleanup = disposer.clone();
        self.on_dispose(move || {
            let cleanup = cleanup.clone();
            async move {
                cleanup.dispose().await?;
                Ok(())
            }
        });
        Ok(disposer)
    }
}

#[derive(Clone)]
pub struct Context {
    inner: Arc<ContextInner>,
}

type EventFilter = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// A dispatch view that separates the registry owner from the scope whose
/// filter decides which listeners are visible (ADR-011). Concrete borrow type:
/// a trait object would require `'static` and cannot borrow these references.
pub(crate) struct EventScopeView<'a> {
    registry: &'a EventRegistry,
    filter: Option<&'a EventFilter>,
}

impl<'a> EventScopeView<'a> {
    pub fn new(registry: &'a EventRegistry, filter: Option<&'a EventFilter>) -> Self {
        Self { registry, filter }
    }

    pub fn registry(&self) -> &'a EventRegistry {
        self.registry
    }

    /// A listener passes when global or the scope filter allows its name.
    pub fn allows(&self, id: u64, name: &str) -> bool {
        self.registry.is_global(id) || self.filter.is_none_or(|f| f(name))
    }
}

struct ContextInner {
    name: String,
    parent: Option<Context>,
    isolated: bool,
    /// Optional listener filter; `false` excludes a named event for this scope.
    event_filter: Option<EventFilter>,
    lifecycle: AtomicU8,
    services: RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>,
    /// Generation per service name. A disposer only removes an entry whose
    /// generation still matches the one it was created for (ghost-disposer
    /// guard, same fencing pattern as ToolRegistry::register).
    service_generations: RwLock<HashMap<String, u64>>,
    next_service_generation: AtomicU64,
    events: EventRegistry,
    effects: Arc<Mutex<Vec<Disposer>>>,
    service_added: Arc<Notify>,
    fibers: RwLock<Vec<std::sync::Weak<dyn FiberObserver>>>,
}

impl Context {
    pub fn root(name: impl Into<String>) -> Self {
        Self::make(name, None, false, None)
    }

    pub fn child(&self, name: impl Into<String>) -> Self {
        Self::make(name, Some(self.clone()), false, None)
    }

    /// An isolated scope never falls back to its parent for services.
    pub fn isolate(&self, name: impl Into<String>) -> Self {
        Self::make(name, Some(self.clone()), true, None)
    }

    fn make(
        name: impl Into<String>,
        parent: Option<Context>,
        isolated: bool,
        event_filter: Option<EventFilter>,
    ) -> Self {
        let service_added = parent
            .as_ref()
            .map(|p| p.inner.service_added.clone())
            .unwrap_or_else(|| Arc::new(Notify::new()));
        Self {
            inner: Arc::new(ContextInner {
                name: name.into(),
                parent,
                isolated,
                event_filter,
                lifecycle: AtomicU8::new(Lifecycle::Pending as u8),
                services: RwLock::new(HashMap::new()),
                service_generations: RwLock::new(HashMap::new()),
                next_service_generation: AtomicU64::new(1),
                events: EventRegistry::default(),
                effects: Arc::new(Mutex::new(Vec::new())),
                service_added,
                fibers: RwLock::new(Vec::new()),
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn parent(&self) -> Option<&Context> {
        self.inner.parent.as_ref()
    }

    pub fn lifecycle(&self) -> Lifecycle {
        Lifecycle::from(self.inner.lifecycle.load(Ordering::Acquire))
    }

    pub fn set_lifecycle(&self, lifecycle: Lifecycle) {
        self.inner
            .lifecycle
            .store(lifecycle as u8, Ordering::Release);
        // A state change can hide services from get_untyped; wake waiters so
        // require_untyped re-checks instead of blocking forever.
        self.inner.service_added.notify_waiters();
    }

    pub fn event_registry(&self) -> &EventRegistry {
        &self.inner.events
    }

    pub(crate) fn event_filter(&self) -> Option<&EventFilter> {
        self.inner.event_filter.as_ref()
    }

    /// Register a named emit listener (upstream-style `ctx.on(name, listener)`).
    /// Payloads are `serde_json::Value`; this occupies the same slot as
    /// `Event::<serde_json::Value, ()>::new(name).on_emit(...)`, so both forms
    /// are dispatched together.
    pub fn on<F>(&self, name: &str, listener: F) -> Disposer
    where
        F: Fn(serde_json::Value) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.on_with(name, listener, crate::events::EventOptions::default())
    }

    /// Register a named listener with upstream-style options (`prepend` /
    /// `global`). `prepend` inserts at the head of the listener list.
    pub fn on_with<F>(
        &self,
        name: &str,
        listener: F,
        options: crate::events::EventOptions,
    ) -> Disposer
    where
        F: Fn(serde_json::Value) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        use crate::events::named_emit_slot;
        let listener: crate::events::EmitListener<serde_json::Value> = Arc::new(listener);
        let registry = self.event_registry().clone();
        let key = named_emit_slot(name);
        let id = registry.push_with(key.clone(), listener, options);
        Disposer::new(move || {
            let registry = registry.clone();
            let key = key.clone();
            async move { registry.remove(key, id) }
        })
    }

    /// Like `on` but the listener is ignored after its first invocation.
    pub fn once<F>(&self, name: &str, listener: F) -> Disposer
    where
        F: Fn(serde_json::Value) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        // A standalone remover shared by the listener and the disposer avoids
        // a Disposer <-> closure cycle while still unregistering after first
        // invocation (upstream `once` semantics).
        struct OnceRemover {
            registry: crate::events::EventRegistry,
            key: crate::events::SlotKey,
            id: u64,
            fired: std::sync::atomic::AtomicBool,
        }
        let listener: crate::events::EmitListener<serde_json::Value> = Arc::new(listener);
        let registry = self.event_registry().clone();
        let key = crate::events::named_emit_slot(name);
        let id = registry.push(key.clone(), listener.clone());
        let remover = Arc::new(OnceRemover {
            registry,
            key: key.clone(),
            id,
            fired: std::sync::atomic::AtomicBool::new(false),
        });

        let listener_remover = remover.clone();
        let once_listener: crate::events::EmitListener<serde_json::Value> =
            Arc::new(move |args: serde_json::Value| {
                let listener = listener.clone();
                let remover = listener_remover.clone();
                Box::pin(async move {
                    if !remover.fired.swap(true, Ordering::SeqCst) {
                        listener(args).await;
                        let _ = remover.registry.remove(remover.key.clone(), remover.id);
                    }
                })
            });
        // Replace the listener we just pushed with the once-wrapping one.
        let _ = self
            .event_registry()
            .replace(key.clone(), id, once_listener);

        let disposer_remover = remover.clone();
        Disposer::new(move || {
            let remover = disposer_remover.clone();
            async move {
                let _ = remover.registry.remove(remover.key.clone(), remover.id);
                Ok(())
            }
        })
    }

    /// Emit a named event to all `on`/`once` listeners in registration order.
    ///
    /// This deliberately does not apply scope filters: upstream's `emit`
    /// without a thisArg dispatches unfiltered. Use `emit_with(this, ...)` to
    /// apply a scope filter (ADR-011).
    pub async fn emit(&self, name: &str, args: serde_json::Value) {
        use crate::events::named_emit_slot;
        let listeners: Vec<crate::events::EmitListener<serde_json::Value>> =
            self.event_registry().snapshot(named_emit_slot(name));
        for listener in listeners {
            listener(args.clone()).await;
        }
    }

    /// Derive a child scope that filters which named events are visible.
    /// `global` listeners bypass the filter during `emit_with`.
    pub fn filtered<F>(&self, filter: F) -> Self
    where
        F: Fn(&str) -> bool + Send + Sync + 'static,
    {
        Self::make(
            format!("{}-filtered", self.inner.name),
            Some(self.clone()),
            false,
            Some(Arc::new(filter)),
        )
    }

    /// Emit with an explicit `this` scope: non-global listeners whose event
    /// name is rejected by `this`'s filter are skipped.
    pub async fn emit_with(&self, this: &Context, name: &str, args: serde_json::Value) {
        use crate::events::named_emit_slot;
        // Registry owner (`self`) and filter scope (`this`) are separated in
        // the view, making the two-scope contract explicit (ADR-011).
        let view = EventScopeView::new(self.event_registry(), this.event_filter());
        let listeners: Vec<(u64, crate::events::EmitListener<serde_json::Value>)> =
            view.registry().snapshot_with_ids(named_emit_slot(name));
        for (id, listener) in listeners {
            if view.allows(id, name) {
                listener(args.clone()).await;
            }
        }
    }

    fn effect_list(&self) -> Arc<Mutex<Vec<Disposer>>> {
        // The list is owned by the inner context; expose it as an Arc so an
        // in-flight effect can be replaced atomically after setup completes.
        self.inner.effects.clone()
    }

    /// Register a service and return a disposer. The disposer is also pushed
    /// onto this context's effect list, so a fiber unload removes the service
    /// even when the caller discards the returned value (upstream semantics).
    pub async fn provide<T>(&self, name: impl Into<String>, service: T) -> Result<Disposer>
    where
        T: Any + Send + Sync,
    {
        let name = name.into();
        let mut services = self
            .inner
            .services
            .write()
            .expect("service registry poisoned");
        if services.contains_key(&name) {
            return Err(Error::DuplicateService(name));
        }
        services.insert(name.clone(), Arc::new(service));
        drop(services);
        let generation = self
            .inner
            .next_service_generation
            .fetch_add(1, Ordering::SeqCst);
        self.inner
            .service_generations
            .write()
            .expect("service generation registry poisoned")
            .insert(name.clone(), generation);
        self.inner.service_added.notify_waiters();
        self.notify_fibers();

        let disposer_ctx = self.clone();
        let disposer_name = name.clone();
        let disposer = Disposer::new(move || {
            let ctx = disposer_ctx.clone();
            let name = disposer_name.clone();
            async move { ctx.remove_service_generation(&name, generation).await }
        });
        self.inner
            .effects
            .lock()
            .expect("effect list poisoned")
            .push(disposer.clone());
        Ok(disposer)
    }

    /// Remove a service only when its generation still matches. Returns
    /// whether an entry was actually removed; stale disposers are no-ops.
    async fn remove_service_generation(&self, name: &str, generation: u64) -> Result<()> {
        let current = self
            .inner
            .service_generations
            .read()
            .expect("service generation registry poisoned")
            .get(name)
            .copied();
        if current != Some(generation) {
            return Ok(());
        }
        self.inner
            .services
            .write()
            .expect("service registry poisoned")
            .remove(name);
        self.inner
            .service_generations
            .write()
            .expect("service generation registry poisoned")
            .remove(name);
        self.inner.service_added.notify_waiters();
        self.drain_dependent_fibers(name).await;
        Ok(())
    }

    pub fn remove_service(&self, name: &str) -> Result<()> {
        self.inner
            .services
            .write()
            .expect("service registry poisoned")
            .remove(name);
        self.inner
            .service_generations
            .write()
            .expect("service generation registry poisoned")
            .remove(name);
        self.inner.service_added.notify_waiters();
        self.notify_fibers();
        Ok(())
    }

    /// Two-phase removal: drop the service from lookup, drain dependent
    /// fibers, then drop the remaining value. Prevents teardown races where a
    /// provider service disappears while its dependents are still running.
    pub async fn remove_service_async(&self, name: &str) -> Result<()> {
        self.inner
            .services
            .write()
            .expect("service registry poisoned")
            .remove(name);
        self.inner
            .service_generations
            .write()
            .expect("service generation registry poisoned")
            .remove(name);
        self.inner.service_added.notify_waiters();
        self.drain_dependent_fibers(name).await;
        Ok(())
    }

    fn root_context(&self) -> &Context {
        match &self.inner.parent {
            Some(parent) => parent.root_context(),
            None => self,
        }
    }

    /// Register a weak observer. The observer must not strongly reference the
    /// context or re-enter it from `refresh` (ADR-009).
    pub(crate) fn register_fiber(&self, observer: std::sync::Weak<dyn FiberObserver>) {
        self.root_context()
            .inner
            .fibers
            .write()
            .expect("fiber registry poisoned")
            .push(observer);
    }

    /// Weak snapshots of live observers; dead entries are self-pruned.
    fn dependent_fibers(&self) -> Vec<std::sync::Weak<dyn FiberObserver>> {
        let mut guard = self
            .root_context()
            .inner
            .fibers
            .write()
            .expect("fiber registry poisoned");
        guard.retain(|weak| weak.strong_count() > 0);
        guard.clone()
    }

    /// Re-evaluate every mounted observer. Upstream's `reflect.notify()` is
    /// notification-driven rather than polling; spawning the refresh keeps
    /// service registration non-blocking.
    pub(crate) fn notify_fibers(&self) {
        for weak in self.dependent_fibers() {
            if let Some(observer) = weak.upgrade() {
                tokio::spawn(async move {
                    let _ = observer.refresh().await;
                });
            }
        }
    }

    /// Notify observers that inject `name` and await their refresh, so callers
    /// can drain dependents before removing a service they depend on.
    /// Retiring observers are skipped to avoid a self-deadlock.
    pub(crate) async fn drain_dependent_fibers(&self, name: &str) {
        use futures::future::join_all;
        let tasks: Vec<_> = self
            .dependent_fibers()
            .into_iter()
            .filter_map(|weak| weak.upgrade())
            .filter(|observer| observer.injects(name) && !observer.is_retiring())
            .map(|observer| async move { observer.refresh().await })
            .collect();
        let _ = join_all(tasks).await;
    }

    pub fn get_untyped(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        // A provider that is unloading/unloaded/failed must not expose its
        // services; fall through to the parent like a missing entry.
        let closed = matches!(
            self.lifecycle(),
            Lifecycle::Unloading | Lifecycle::Unloaded | Lifecycle::Failed
        );
        if !closed
            && let Some(service) = self
                .inner
                .services
                .read()
                .expect("service registry poisoned")
                .get(name)
        {
            return Some(service.clone());
        }
        if self.inner.isolated {
            return None;
        }
        self.inner
            .parent
            .as_ref()
            .and_then(|parent| parent.get_untyped(name))
    }

    pub fn get<T>(&self, name: &str) -> Option<Arc<T>>
    where
        T: Any + Send + Sync,
    {
        self.get_untyped(name)?.downcast::<T>().ok()
    }

    /// Resolve a service, waiting until it is registered. A lost-wakeup is
    /// avoided by arming the notification before re-checking the registry.
    pub async fn require_untyped(&self, name: &str) -> Result<Arc<dyn Any + Send + Sync>> {
        loop {
            if matches!(
                self.lifecycle(),
                Lifecycle::Unloading | Lifecycle::Unloaded | Lifecycle::Failed
            ) {
                return Err(Error::Unloading);
            }
            let notified = self.inner.service_added.notified();
            if let Some(service) = self.get_untyped(name) {
                return Ok(service);
            }
            notified.await;
        }
    }

    pub async fn require<T>(&self, name: &str) -> Result<Arc<T>>
    where
        T: Any + Send + Sync,
    {
        let untyped = self.require_untyped(name).await?;
        untyped
            .downcast::<T>()
            .map_err(|_| Error::ServiceTypeMismatch(name.to_string()))
    }

    /// Register an effect. Nested effects registered through the scope are
    /// disposed together in reverse order when the returned disposer runs.
    pub async fn effect<F, Fut>(&self, setup: F) -> Result<()>
    where
        F: FnOnce(EffectScope) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.effect_with_parent(setup, None).await
    }

    async fn effect_with_parent<F, Fut>(
        &self,
        setup: F,
        parent: Option<Arc<Mutex<Vec<Disposer>>>>,
    ) -> Result<()>
    where
        F: FnOnce(EffectScope) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        if matches!(
            self.lifecycle(),
            Lifecycle::Unloading | Lifecycle::Unloaded | Lifecycle::Failed
        ) {
            return Err(Error::Unloading);
        }
        let children: Arc<Mutex<Vec<Disposer>>> = Arc::new(Mutex::new(Vec::new()));
        let scope = EffectScope {
            ctx: self.clone(),
            children: children.clone(),
        };
        // Upstream registers the wrapper before running setup so a reentrant
        // unload can still observe and await this effect's cleanup.
        let placeholder = Disposer::new(|| async { Ok(()) });
        let target_list: Arc<Mutex<Vec<Disposer>>> =
            parent.clone().unwrap_or_else(|| self.effect_list());
        target_list
            .lock()
            .expect("disposer list poisoned")
            .push(placeholder.clone());
        let result = setup(scope).await;
        if result.is_err() {
            drop_disposer(&target_list, &placeholder);
            // Roll back every child registered before the failure.
            if let Some(disposer) = Disposer::from_children(children).dispose().await.err() {
                return Err(disposer);
            }
            return result;
        }
        let disposer = Disposer::from_children(children);
        // A reentrant teardown during `setup().await` consumes the placeholder.
        // `is_spent()` detects that directly (Lifecycle may be untouched for an
        // EffectScope-only dispose): unwind the children and do NOT re-register
        // the effect into a list that has already been drained (swarm R2-BV).
        if placeholder.is_spent() {
            return disposer.dispose().await;
        }
        {
            let mut list = target_list.lock().expect("disposer list poisoned");
            for slot in list.iter_mut() {
                if slot.same_as(&placeholder) {
                    *slot = disposer;
                    return Ok(());
                }
            }
            // The placeholder vanished without being spent (e.g. removed by
            // id). Do not re-push into a torn-down list; unwind instead.
        }
        disposer.dispose().await
    }

    pub async fn dispose_effects(&self) -> Result<()> {
        let owned: Vec<Disposer> = {
            let mut guard = self.inner.effects.lock().expect("effect list poisoned");
            guard.drain(..).collect()
        };
        // Disposal runs in reverse registration order. Each disposer's own
        // children are unwound serially by `Disposer::from_children`; running
        // top-level disposers inline also avoids nested runtime tasks, which
        // deadlocked under a single-threaded Tokio test runtime.
        let mut first_error = None;
        for disposer in owned.into_iter().rev() {
            if let Err(error) = disposer.dispose().await {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
