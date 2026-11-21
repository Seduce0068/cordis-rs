//! Plugins and fibers: mount, dependency-driven reload, unload.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;

use crate::context::{Context, FiberObserver, Lifecycle};
use crate::error::{Error, Result};

/// A Cordis plugin. Upstream accepts a function, a constructor, or an object
/// with `apply`; Rust uses one trait with a function adapter below.
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn name(&self) -> &str;

    /// Services that must be present before `apply` runs. When one disappears,
    /// the fiber unloads; when it returns, the fiber reloads.
    fn inject(&self) -> &[&str] {
        &[]
    }

    async fn apply(&self, ctx: &Context) -> Result<()>;
}

type ApplyFn = dyn Fn(&Context) -> crate::events::BoxFuture<'static, Result<()>> + Send + Sync;

pub struct FunctionPlugin {
    name: String,
    inject: Vec<&'static str>,
    apply: Arc<ApplyFn>,
}

impl FunctionPlugin {
    pub fn new<F, Fut>(name: impl Into<String>, apply: F) -> Self
    where
        F: Fn(&Context) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            name: name.into(),
            inject: Vec::new(),
            apply: Arc::new(move |ctx| Box::pin(apply(ctx))),
        }
    }

    pub fn with_inject(mut self, inject: Vec<&'static str>) -> Self {
        self.inject = inject;
        self
    }
}

#[async_trait]
impl Plugin for FunctionPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn inject(&self) -> &[&str] {
        &self.inject
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        (self.apply)(ctx).await
    }
}

pub struct FiberInner {
    name: String,
    ctx: Context,
    plugin: Box<dyn Plugin>,
    inject: Vec<String>,
    state: Mutex<Lifecycle>,
    guard: AsyncMutex<()>,
    /// Set before disposing effects so a dependent-drain refresh skips this
    /// fiber instead of deadlocking on its own guard (team review finding).
    retiring: std::sync::atomic::AtomicBool,
}

impl FiberInner {
    pub(crate) fn is_retiring(&self) -> bool {
        self.retiring.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn injects(&self, name: &str) -> bool {
        self.inject.iter().any(|item| item == name)
    }

    async fn refresh_inner(&self) -> Result<()> {
        let _guard = self.guard.lock().await;
        let resolved = self
            .inject
            .iter()
            .all(|name| self.ctx.get_untyped(name).is_some());
        let state = *self.state.lock().expect("fiber state poisoned");

        if state == Lifecycle::Active && !resolved {
            self.retiring
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.ctx.set_lifecycle(Lifecycle::Unloading);
            self.ctx.dispose_effects().await?;
            *self.state.lock().expect("fiber state poisoned") = Lifecycle::Pending;
            // Teardown is complete: the fiber is now waiting to reload, not
            // unloading itself. Clear the re-entry guard so drain_dependent_fibers
            // no longer skips it (R2-BV01). Explicit unload() keeps it set.
            self.retiring
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.ctx.set_lifecycle(Lifecycle::Pending);
        } else if matches!(state, Lifecycle::Pending | Lifecycle::Failed) && resolved {
            *self.state.lock().expect("fiber state poisoned") = Lifecycle::Loading;
            self.ctx.set_lifecycle(Lifecycle::Loading);
            match self.plugin.apply(&self.ctx).await {
                Ok(()) => {
                    *self.state.lock().expect("fiber state poisoned") = Lifecycle::Active;
                    self.ctx.set_lifecycle(Lifecycle::Active);
                }
                Err(error) => {
                    // Clean up any effects the failed apply already registered.
                    let _ = self.ctx.dispose_effects().await;
                    *self.state.lock().expect("fiber state poisoned") = Lifecycle::Failed;
                    // Keep the context lifecycle Unloaded: require()/effect()
                    // guards only treat Unloading/Unloaded as closed.
                    self.ctx.set_lifecycle(Lifecycle::Unloaded);
                    return Err(Error::Plugin(self.name.clone(), error.to_string()));
                }
            }
        }
        Ok(())
    }
}

impl FiberObserver for FiberInner {
    fn injects(&self, name: &str) -> bool {
        FiberInner::injects(self, name)
    }

    fn is_retiring(&self) -> bool {
        FiberInner::is_retiring(self)
    }

    fn refresh(&self) -> crate::events::BoxFuture<'_, crate::Result<()>> {
        Box::pin(self.refresh_inner())
    }
}

#[derive(Clone)]
pub struct Fiber(Arc<FiberInner>);

impl Fiber {
    /// Mount a plugin. The plugin body does not run until its `inject`
    /// dependencies are available. `await`ing this function waits for load.
    pub async fn mount(ctx: &Context, plugin: Box<dyn Plugin>) -> Result<Self> {
        let name = plugin.name().to_string();
        let inject: Vec<String> = plugin
            .inject()
            .iter()
            .map(|name| name.to_string())
            .collect();
        let fiber_ctx = ctx.child(&name);
        let fiber = Self(Arc::new(FiberInner {
            name,
            ctx: fiber_ctx,
            plugin,
            inject,
            state: Mutex::new(Lifecycle::Pending),
            guard: AsyncMutex::new(()),
            retiring: std::sync::atomic::AtomicBool::new(false),
        }));
        let arc: Arc<dyn FiberObserver> = fiber.0.clone();
        ctx.register_fiber(Arc::downgrade(&arc));
        fiber.refresh().await?;
        Ok(fiber)
    }

    /// Mount a plugin and retain its Fiber in the context effect list so the
    /// fiber (and its effects) are unloaded when the context disposes them.
    /// All port mount helpers should use this instead of `mount` + discarding
    /// the handle (the ghost-fiber pattern).
    pub async fn mount_retained(ctx: &Context, plugin: Box<dyn Plugin>) -> Result<Self> {
        let fiber = Self::mount(ctx, plugin).await?;
        let retained = fiber.clone();
        ctx.effect(move |scope| {
            let fiber = retained;
            async move {
                scope.on_dispose(move || async move {
                    fiber.unload().await?;
                    Ok(())
                });
                Ok(())
            }
        })
        .await?;
        Ok(fiber)
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn state(&self) -> Lifecycle {
        *self.0.state.lock().expect("fiber state poisoned")
    }

    pub fn context(&self) -> &Context {
        &self.0.ctx
    }

    pub async fn refresh(&self) -> Result<()> {
        self.0.refresh_inner().await
    }

    /// Unload the fiber and dispose every effect it registered.
    pub async fn unload(&self) -> Result<()> {
        self.0
            .retiring
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _guard = self.0.guard.lock().await;
        if self.state() == Lifecycle::Unloaded {
            return Ok(());
        }
        self.0.ctx.set_lifecycle(Lifecycle::Unloading);
        *self.0.state.lock().expect("fiber state poisoned") = Lifecycle::Unloading;
        self.0.ctx.dispose_effects().await?;
        *self.0.state.lock().expect("fiber state poisoned") = Lifecycle::Unloaded;
        self.0.ctx.set_lifecycle(Lifecycle::Unloaded);
        Ok(())
    }
}
