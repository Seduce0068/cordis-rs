//! Reusable Rust port of `@deepseek-ai/dsh-command-compact`.
//!
//! Source: `packages/compaction/command-compact/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! `inject = ['commands', 'compaction', 'sessions']`; `/compact` takes no args.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{
    CommandDefinition, CommandHandler, CommandInvocation, CommandRegistry, CommandResult,
};
use crate::services::compaction::CompactionEngine;
use crate::services::sessions::SessionStore;
use crate::{Context, Result};

pub const USAGE: &str = "Usage: /compact (no arguments)";

fn handler(compaction: CompactionEngine, sessions: SessionStore) -> CommandHandler {
    Arc::new(move |invocation: CommandInvocation| {
        let compaction = compaction.clone();
        let sessions = sessions.clone();
        Box::pin(async move {
            if !invocation.raw_input.trim().is_empty() {
                return CommandResult::error(USAGE);
            }
            let Some(session_id) = invocation.session.as_deref() else {
                return CommandResult::error("No active session to compact.");
            };
            let Some(session) = sessions.get(session_id) else {
                return CommandResult::error("Session is unavailable.");
            };
            match compaction.compact_now(&session) {
                Ok(result) => CommandResult::success(format!(
                    "Compacted {} event(s). Pressure: {:.2}",
                    result.shadowed_events, result.pressure
                )),
                Err(error) => CommandResult::error(format!("Compaction failed: {error}")),
            }
        })
    })
}

/// The `dsh-command-compact` plugin. `sessions` is passed explicitly because
/// the command handler needs the live store, not just a dependency signal.
pub struct CommandCompact {
    pub sessions: SessionStore,
}

#[async_trait]
impl crate::Plugin for CommandCompact {
    fn name(&self) -> &str {
        "command-compact"
    }

    fn inject(&self) -> &[&str] {
        &["commands", "compaction"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let commands = ctx.require::<CommandRegistry>("commands").await?;
        let compaction = ctx.require::<CompactionEngine>("compaction").await?;
        let sessions = self.sessions.clone();
        ctx.effect(move |scope| {
            let commands = commands.clone();
            let compaction = compaction.clone();
            let sessions = sessions.clone();
            async move {
                let disposer = commands.register(CommandDefinition::new(
                    "/compact",
                    "Compact older conversation history",
                    handler((*compaction).clone(), sessions),
                ));
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with the shared commands, compaction and sessions seams.
pub async fn mount(ctx: &Context, sessions: SessionStore) -> Result<CommandRegistry> {
    let commands = ctx.require::<CommandRegistry>("commands").await?;
    let _ = crate::Fiber::mount_retained(
        ctx,
        Box::new(CommandCompact {
            sessions: sessions.clone(),
        }),
    )
    .await?;
    Ok((*commands).clone())
}
