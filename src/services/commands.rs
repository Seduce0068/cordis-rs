//! Core `ctx.commands` service: human-facing slash commands.
//!
//! Upstream: `packages/interaction/commands/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::primitives::Disposer;

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
        let name = name.into();
        // Match parse_command's registered_name contract: a leading slash and
        // `[a-z][a-z0-9_-]*`, so parsed input can find the command (swarm
        // review R5-IV05).
        let valid = name.strip_prefix('/').is_some_and(|rest| {
            let mut chars = rest.chars();
            chars.next().is_some_and(|first| first.is_ascii_lowercase())
                && chars.all(|ch| {
                    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
                })
        });
        assert!(valid, "invalid command name: {name}");
        Self {
            name,
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
    commands: HashMap<String, (u64, CommandDefinition)>,
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
        let name = definition.name().to_string();
        // Match ToolRegistry: last registration wins, and an older disposer
        // must not remove a newer definition with the same name (swarm review
        // R5-AP03).
        guard
            .commands
            .insert(name.clone(), (id, definition.clone()));
        drop(guard);

        Disposer::new(move || {
            let state = state.clone();
            async move {
                let mut guard = state.write().expect("command registry poisoned");
                if guard.commands.get(&name).map(|(current, _)| *current) == Some(id) {
                    guard.commands.remove(&name);
                }
                Ok(())
            }
        })
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .state
            .read()
            .expect("command registry poisoned")
            .commands
            .values()
            .map(|(_, definition)| definition.name().to_string())
            .collect();
        // HashMap iteration order is not stable; keep the human-facing list
        // deterministic (swarm review R8-AP04).
        names.sort();
        names
    }

    pub fn description(&self, name: &str) -> Option<String> {
        self.state
            .read()
            .expect("command registry poisoned")
            .commands
            .get(name)
            .map(|(_, definition)| definition.description().to_string())
    }

    pub async fn invoke(&self, name: &str, invocation: CommandInvocation) -> Option<CommandResult> {
        let handler = self
            .state
            .read()
            .expect("command registry poisoned")
            .commands
            .get(name)
            .map(|(_, definition)| definition.handler.clone());
        match handler {
            Some(handler) => Some(handler(invocation).await),
            None => None,
        }
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<CommandRegistry> {
    provide_with_disposer(ctx)
        .await
        .map(|(registry, _)| registry)
}

/// Register the service and return its disposer so callers can roll back a
/// partially completed mount (swarm review R14-RB07).
pub async fn provide_with_disposer(
    ctx: &crate::Context,
) -> Result<(CommandRegistry, crate::Disposer)> {
    let registry = CommandRegistry::default();
    let disposer = ctx.provide("commands", registry.clone()).await?;
    Ok((registry, disposer))
}
