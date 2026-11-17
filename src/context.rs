//! Context, service registry, effects and lifecycle.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tokio::sync::Notify;

use crate::error::{Error, Result};
use crate::events::{BoxFuture, EventRegistry};
use crate::plugin::FiberInner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Pending = 0,
    Loading = 1,
    Active = 2,
    Unloading = 3,
    Unloaded = 4,
}

impl From<u8> for Lifecycle {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Pending,
            1 => Self::Loading,
            2 => Self::Active,
            3 => Self::Unloading,
            _ => Self::Unloaded,
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

    pub async fn dispose(&self) -> Result<()> {
        let maybe_setup = self.inner.lock().expect("disposer poisoned").take();
        match maybe_setup {
            Some(setup) => setup().await,
            None => Ok(()),
        }
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

struct ContextInner {
    name: String,
    parent: Option<Context>,
    isolated: bool,
    lifecycle: AtomicU8,
    services: RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>,
    events: EventRegistry,
    effects: Arc<Mutex<Vec<Disposer>>>,
    service_added: Notify,
    fibers: RwLock<Vec<std::sync::Weak<FiberInner>>>,
}

impl Context {
    pub fn root(name: impl Into<String>) -> Self {
        Self::make(name, None, false)
    }

    pub fn child(&self, name: impl Into<String>) -> Self {
        Self::make(name, Some(self.clone()), false)
    }

    /// An isolated scope never falls back to its parent for services.
    pub fn isolate(&self, name: impl Into<String>) -> Self {
        Self::make(name, Some(self.clone()), true)
    }

    fn make(name: impl Into<String>, parent: Option<Context>, isolated: bool) -> Self {
        Self {
            inner: Arc::new(ContextInner {
                name: name.into(),
                parent,
                isolated,
                lifecycle: AtomicU8::new(Lifecycle::Pending as u8),
                services: RwLock::new(HashMap::new()),
                events: EventRegistry::default(),
                effects: Arc::new(Mutex::new(Vec::new())),
                service_added: Notify::new(),
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
    }

    pub fn event_registry(&self) -> &EventRegistry {
        &self.inner.events
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
        self.inner.service_added.notify_waiters();
        self.notify_fibers();

        let disposer_ctx = self.clone();
        let disposer_name = name.clone();
        let disposer = Disposer::new(move || {
            let ctx = disposer_ctx.clone();
            let name = disposer_name.clone();
            async move { ctx.remove_service_async(&name).await }
        });
        self.inner
            .effects
            .lock()
            .expect("effect list poisoned")
            .push(disposer.clone());
        Ok(disposer)
    }

    pub fn remove_service(&self, name: &str) -> Result<()> {
        self.inner
            .services
            .write()
            .expect("service registry poisoned")
            .remove(name);
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
        self.drain_dependent_fibers(name).await;
        Ok(())
    }

    fn root_context(&self) -> &Context {
        match &self.inner.parent {
            Some(parent) => parent.root_context(),
            None => self,
        }
    }

    pub(crate) fn register_fiber(&self, fiber: &std::sync::Arc<FiberInner>) {
        self.root_context()
            .inner
            .fibers
            .write()
            .expect("fiber registry poisoned")
            .push(std::sync::Arc::downgrade(fiber));
    }

    /// Re-evaluate every mounted fiber. Upstream's `reflect.notify()` is
    /// notification-driven rather than polling; spawning the refresh keeps
    /// service registration non-blocking.
    fn dependent_fibers(&self) -> Vec<std::sync::Weak<FiberInner>> {
        let mut guard = self
            .root_context()
            .inner
            .fibers
            .write()
            .expect("fiber registry poisoned");
        guard.retain(|weak| weak.strong_count() > 0);
        guard.clone()
    }

    pub(crate) fn notify_fibers(&self) {
        for weak in self.dependent_fibers() {
            if let Some(fiber) = weak.upgrade() {
                let fiber = crate::plugin::Fiber::from_inner(fiber);
                tokio::spawn(async move {
                    let _ = fiber.refresh().await;
                });
            }
        }
    }

    /// Notify fibers that inject `name` and await their refresh, so callers
    /// can drain dependents before removing a service they depend on.
    /// Fibers already retiring are skipped to avoid a self-deadlock.
    pub(crate) async fn drain_dependent_fibers(&self, name: &str) {
        use futures::future::join_all;
        let tasks: Vec<_> = self
            .dependent_fibers()
            .into_iter()
            .filter_map(|weak| weak.upgrade())
            .filter(|inner| inner.injects(name) && !inner.is_retiring())
            .map(crate::plugin::Fiber::from_inner)
            .map(|fiber| async move { fiber.refresh().await })
            .collect();
        let _ = join_all(tasks).await;
    }

    pub fn get_untyped(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        if let Some(service) = self
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
            if self.lifecycle() == Lifecycle::Unloading || self.lifecycle() == Lifecycle::Unloaded {
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
        if self.lifecycle() == Lifecycle::Unloading || self.lifecycle() == Lifecycle::Unloaded {
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
        {
            let mut list = target_list.lock().expect("disposer list poisoned");
            let mut replaced = false;
            for slot in list.iter_mut() {
                if slot.same_as(&placeholder) {
                    *slot = disposer.clone();
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                list.push(disposer);
            }
        }
        Ok(())
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
