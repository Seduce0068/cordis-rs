//! `ctx.subagents`: named provider registry for delegating to child agents.
//!
//! Upstream: `packages/subagent/subagent/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Providers prepare a child run and return a result; this seam stays
//! provider-neutral.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::primitives::Disposer;

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentRequest {
    pub prompt: String,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentResult {
    pub text: String,
    pub provider: String,
}

pub type SubagentHandler =
    Arc<dyn Fn(SubagentRequest) -> crate::events::BoxFuture<'static, String> + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub enum SubagentError {
    UnknownProvider(String),
    DepthExceeded {
        max: usize,
        requested: usize,
    },
    /// A provider with this name is already registered.
    DuplicateProvider(String),
}

impl std::fmt::Display for SubagentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownProvider(name) => write!(f, "unknown subagent provider: {name}"),
            Self::DuplicateProvider(name) => write!(f, "duplicate subagent provider: {name}"),
            Self::DepthExceeded { max, requested } => {
                write!(f, "subagent depth {requested} exceeds max {max}")
            }
        }
    }
}

impl std::error::Error for SubagentError {}

#[derive(Default)]
struct RegistryState {
    next_id: u64,
    providers: BTreeMap<String, (u64, SubagentHandler)>,
}

#[derive(Default, Clone)]
pub struct SubagentRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl SubagentRegistry {
    /// Register a provider, rejecting an existing name. The legacy
    /// [`Self::register`] keeps its last-write-wins behaviour (swarm review
    /// R20-ID08).
    pub fn try_register(
        &self,
        name: impl Into<String>,
        handler: SubagentHandler,
    ) -> std::result::Result<Disposer, SubagentError> {
        let name = name.into();
        if self
            .state
            .read()
            .expect("subagent registry poisoned")
            .providers
            .contains_key(&name)
        {
            return Err(SubagentError::DuplicateProvider(name));
        }
        Ok(self.register(name, handler))
    }

    pub fn register(&self, name: impl Into<String>, handler: SubagentHandler) -> Disposer {
        let name = name.into();
        let state = self.state.clone();
        let id = {
            let mut guard = state.write().expect("subagent registry poisoned");
            let id = guard.next_id;
            guard.next_id += 1;
            guard.providers.insert(name.clone(), (id, handler));
            id
        };
        Disposer::new(move || {
            let state = state.clone();
            let name = name.clone();
            async move {
                let mut guard = state.write().expect("subagent registry poisoned");
                // Stale disposers must not remove a newer registration
                // (swarm review R10-IV03).
                if guard.providers.get(&name).map(|(current, _)| *current) == Some(id) {
                    guard.providers.remove(&name);
                }
                Ok(())
            }
        })
    }

    pub fn names(&self) -> Vec<String> {
        self.state
            .read()
            .expect("subagent registry poisoned")
            .providers
            .keys()
            .cloned()
            .collect()
    }

    pub async fn delegate(
        &self,
        provider: &str,
        request: SubagentRequest,
        max_depth: usize,
    ) -> std::result::Result<SubagentResult, SubagentError> {
        if request.depth > max_depth {
            return Err(SubagentError::DepthExceeded {
                max: max_depth,
                requested: request.depth,
            });
        }
        let handler = self
            .state
            .read()
            .expect("subagent registry poisoned")
            .providers
            .get(provider)
            .map(|(_, handler)| handler.clone())
            .ok_or_else(|| SubagentError::UnknownProvider(provider.to_string()))?;
        let text = handler(request).await;
        Ok(SubagentResult {
            text,
            provider: provider.to_string(),
        })
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<SubagentRegistry> {
    let registry = SubagentRegistry::default();
    ctx.provide("subagents", registry.clone()).await?;
    Ok(registry)
}
