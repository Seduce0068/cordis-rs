//! Reusable Rust port of `@deepseek-ai/dsh-command-goal`.
//!
//! Registers the `/goal` human command over the shared `commands` seam and the
//! event-sourced `ctx.goals` store. Source:
//! `packages/goal/command-goal/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{
    CommandDefinition, CommandHandler, CommandInvocation, CommandRegistry, CommandResult,
};
use crate::services::goals::{GoalError, GoalStore};
use crate::{Context, Result};

pub const USAGE: &str = "Usage: /goal [<objective>|clear|edit <objective>|pause|resume]";
pub const DEFAULT_MAX_GOAL_ROUNDS: u32 = 1000;

fn render(goal: Option<crate::services::goals::Goal>) -> CommandResult {
    match goal {
        None => CommandResult::error("No goal is set."),
        Some(goal) => CommandResult::success(format!(
            "Goal (rev {}): {} [{}]",
            goal.revision,
            goal.objective,
            goal.status.as_str()
        )),
    }
}

fn handler(store: GoalStore) -> CommandHandler {
    Arc::new(move |invocation: CommandInvocation| {
        let store = store.clone();
        Box::pin(async move {
            let input = invocation.raw_input.trim();
            if input.is_empty() {
                return render(store.get());
            }
            if input == "clear" {
                store.clear();
                return CommandResult::success("Goal cleared.");
            }
            if input == "pause" || input == "resume" {
                let Some(goal) = store.get() else {
                    return CommandResult::error("No goal is set.");
                };
                let result = if input == "pause" {
                    store.pause(goal.revision)
                } else {
                    store.resume(goal.revision)
                };
                return match result {
                    Ok(goal) => render(Some(goal)),
                    Err(error) => CommandResult::error(error.to_string()),
                };
            }
            if let Some(objective) = input.strip_prefix("edit ") {
                let Some(goal) = store.get() else {
                    return CommandResult::error("No goal is set.");
                };
                return match store.edit(objective.trim(), goal.max_rounds, goal.revision) {
                    Ok(goal) => render(Some(goal)),
                    Err(error) => CommandResult::error(error.to_string()),
                };
            }
            match store.set(input, DEFAULT_MAX_GOAL_ROUNDS) {
                Ok(goal) => render(Some(goal)),
                Err(GoalError::EmptyObjective) => CommandResult::error(USAGE),
                Err(error) => CommandResult::error(error.to_string()),
            }
        })
    })
}

/// The `dsh-command-goal` plugin. `inject = ['commands', 'goals']`.
pub struct CommandGoal;

#[async_trait]
impl crate::Plugin for CommandGoal {
    fn name(&self) -> &str {
        "command-goal"
    }

    fn inject(&self) -> &[&str] {
        &["commands", "goals"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let commands = ctx.require::<CommandRegistry>("commands").await?;
        let store = ctx.require::<GoalStore>("goals").await?;
        ctx.effect(move |scope| {
            let commands = commands.clone();
            let handler = handler((*store).clone());
            async move {
                let disposer = commands.register(CommandDefinition::new(
                    "/goal",
                    "Set or inspect the session goal (same-session persistence).",
                    handler,
                ));
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with the shared commands and goals seams.
pub async fn mount(ctx: &Context) -> Result<GoalStore> {
    let store = if let Some(existing) = ctx.get::<GoalStore>("goals") {
        (*existing).clone()
    } else {
        crate::core_services::provide_goals(ctx).await?
    };
    if ctx.get_untyped("commands").is_none() {
        crate::core_services::provide_commands(ctx).await?;
    }
    let _ = crate::Fiber::mount_retained(ctx, Box::new(CommandGoal)).await?;
    Ok(store)
}
