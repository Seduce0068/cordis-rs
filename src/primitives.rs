//! Shared primitives that neither depend on `context` nor `events`.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::error::Result;

/// Local future alias: avoids depending on events::BoxFuture and forming a
/// primitives -> events -> primitives cycle (R1 review).
type CleanupFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
type Cleanup = Box<dyn FnOnce() -> CleanupFuture<'static> + Send>;

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

    pub(crate) fn from_children(children: Arc<Mutex<Vec<Disposer>>>) -> Self {
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

    pub(crate) fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Remove a disposer from a list without taking ownership of it.
pub(crate) fn drop_disposer(list: &Mutex<Vec<Disposer>>, target: &Disposer) {
    list.lock()
        .expect("disposer list poisoned")
        .retain(|current| !current.same_as(target));
}
