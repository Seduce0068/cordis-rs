//! Core `ctx.commands` service: human-facing slash commands.
//!
//! Upstream: `packages/interaction/commands/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::sync::{Arc, RwLock};

use crate::Result;
use crate::context::Disposer;

#[derive(Debug, Clone, PartialEq)]
pub struct CommandInvocation {
    pub raw_input: String,
    pub agent: Option<String>,
    pub session: Option<String>,
}

impl CommandInvocation {
    pub fn new(raw_input: impl Into<String>) -> Self {
        Self {
            raw_input: raw_input.into(),
            agent: None,
            session: None,
        }
    }

    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        self.agent = Some(agent.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandResult {
    Success { text: String },
    Error { text: String },
}

impl CommandResult {
    pub fn success(text: impl Into<String>) -> Self {
        Self::Success { text: text.into() }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::Error { text: text.into() }
    }
}

pub type CommandHandler = Arc<
    dyn Fn(CommandInvocation) -> crate::events::BoxFuture<'static, CommandResult> + Send + Sync,
>;

#[derive(Clone)]
pub struct CommandDefinition {
    name: String,
    description: String,
    handler: CommandHandler,
}

impl CommandDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        handler: CommandHandler,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            handler,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

#[derive(Default)]
struct RegistryState {
    commands: Vec<(u64, CommandDefinition)>,
    next_id: u64,
}

/// `ctx.commands`: registry of slash commands available to a human.
#[derive(Default, Clone)]
pub struct CommandRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl CommandRegistry {
    pub fn register(&self, definition: CommandDefinition) -> Disposer {
        let state = self.state.clone();
        let mut guard = state.write().expect("command registry poisoned");
        let id = guard.next_id;
        guard.next_id += 1;
        guard.commands.push((id, definition.clone()));
        drop(guard);

        Disposer::new(move || {
            let state = state.clone();
            async move {
                state
                    .write()
                    .expect("command registry poisoned")
                    .commands
                    .retain(|(current, _)| *current != id);
                Ok(())
            }
        })
    }

    pub fn names(&self) -> Vec<String> {
        self.state
            .read()
            .expect("command registry poisoned")
            .commands
            .iter()
            .map(|(_, definition)| definition.name().to_string())
            .collect()
    }

    pub fn description(&self, name: &str) -> Option<String> {
        self.state
            .read()
            .expect("command registry poisoned")
            .commands
            .iter()
            .find(|(_, definition)| definition.name() == name)
            .map(|(_, definition)| definition.description().to_string())
    }

    pub async fn invoke(&self, name: &str, invocation: CommandInvocation) -> Option<CommandResult> {
        let handler = self
            .state
            .read()
            .expect("command registry poisoned")
            .commands
            .iter()
            .find(|(_, definition)| definition.name() == name)
            .map(|(_, definition)| definition.handler.clone());
        match handler {
            Some(handler) => Some(handler(invocation).await),
            None => None,
        }
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<CommandRegistry> {
    let registry = CommandRegistry::default();
    ctx.provide("commands", registry.clone()).await?;
    Ok(registry)
}
