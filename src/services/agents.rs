//! `ctx.agents`: registry of agent instances (1:1 with sessions).
//!
//! Upstream: `packages/core/agent/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Agents share an id with their session; this port keeps a lightweight
//! record (id, session id, cwd) and a named factory registry.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::primitives::Disposer;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRecord {
    pub id: String,
    pub session_id: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentError {
    NotFound(String),
    Duplicate(String),
    UnknownFactory(String),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "agent not found: {id}"),
            Self::Duplicate(id) => write!(f, "agent already registered: {id}"),
            Self::UnknownFactory(name) => write!(f, "unknown agent factory: {name}"),
        }
    }
}

pub type AgentFactory = Arc<dyn Fn(&str) -> std::result::Result<AgentRecord, Error> + Send + Sync>;

#[derive(Default)]
struct RegistryState {
    agents: BTreeMap<String, AgentRecord>,
    factories: BTreeMap<String, AgentFactory>,
}

#[derive(Default, Clone)]
pub struct AgentRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a named factory that builds an agent record from an id.
    pub fn register_factory(&self, name: impl Into<String>, factory: AgentFactory) -> Disposer {
        let name = name.into();
        let state = self.state.clone();
        state
            .write()
            .expect("agent registry poisoned")
            .factories
            .insert(name.clone(), factory);
        Disposer::new(move || {
            let state = state.clone();
            let name = name.clone();
            async move {
                state
                    .write()
                    .expect("agent registry poisoned")
                    .factories
                    .remove(&name);
                Ok(())
            }
        })
    }

    /// Create an agent through a named factory; the agent id doubles as the
    /// session id, matching upstream's 1:1 relationship.
    pub fn create(
        &self,
        factory: &str,
        id: impl Into<String>,
    ) -> std::result::Result<AgentRecord, AgentError> {
        let id = id.into();
        let factory = self
            .state
            .read()
            .expect("agent registry poisoned")
            .factories
            .get(factory)
            .cloned()
            .ok_or_else(|| AgentError::UnknownFactory(factory.to_string()))?;
        let mut record = factory(&id).map_err(|error| AgentError::NotFound(error.to_string()))?;
        record.session_id = id.clone();
        let mut guard = self.state.write().expect("agent registry poisoned");
        if guard.agents.contains_key(&id) {
            return Err(AgentError::Duplicate(id));
        }
        guard.agents.insert(id, record.clone());
        Ok(record)
    }

    /// Register an existing agent record.
    pub fn register(&self, record: AgentRecord) -> std::result::Result<Disposer, AgentError> {
        let id = record.id.clone();
        let state = self.state.clone();
        {
            let mut guard = state.write().expect("agent registry poisoned");
            if guard.agents.contains_key(&id) {
                return Err(AgentError::Duplicate(id));
            }
            guard.agents.insert(id.clone(), record);
        }
        Ok(Disposer::new(move || {
            let state = state.clone();
            let id = id.clone();
            async move {
                state
                    .write()
                    .expect("agent registry poisoned")
                    .agents
                    .remove(&id);
                Ok(())
            }
        }))
    }

    pub fn get(&self, id: &str) -> Option<AgentRecord> {
        self.state
            .read()
            .expect("agent registry poisoned")
            .agents
            .get(id)
            .cloned()
    }

    pub fn list(&self) -> Vec<AgentRecord> {
        let guard = self.state.read().expect("agent registry poisoned");
        guard.agents.values().cloned().collect()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<AgentRegistry> {
    let registry = AgentRegistry::new();
    ctx.provide("agents", registry.clone()).await?;
    Ok(registry)
}
