//! Reusable Rust port of `@deepseek-ai/dsh-tool-goal`.
//!
//! Source: `packages/goal/tool-goal/src/index.ts` (+ authority.ts, wrapup.ts)
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers `get_goal`, `create_goal` and `update_goal` over the shared
//! `goals` store, with CAS (revision) validation and authority checks.
//! `blocked` is represented by pausing the goal and recording the reason in
//! a port-local overlay (the local GoalStore has no `blocked` status).

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{ToolDefinition, ToolError, ToolExecContext, ToolHandler, ToolRegistry};
use crate::services::goals::{Goal, GoalError, GoalStore};
use crate::{Context, Result};

pub const BLOCKED_AFTER_CONSECUTIVE_ROUNDS: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolGoalConfig {
    pub blocked_after_consecutive_rounds: u32,
}

impl Default for ToolGoalConfig {
    fn default() -> Self {
        Self {
            blocked_after_consecutive_rounds: BLOCKED_AFTER_CONSECUTIVE_ROUNDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GoalAuthority {
    DirectHuman,
    GoalRound,
}

impl GoalAuthority {
    pub fn from_exec(exec: &ToolExecContext) -> Self {
        // A direct human turn is represented by an explicit agent marker in
        // this runtime; all other contexts are treated as goal rounds.
        if exec.agent.as_deref() == Some("human") {
            Self::DirectHuman
        } else {
            Self::GoalRound
        }
    }

    fn allows(&self, action: &str) -> bool {
        match self {
            Self::DirectHuman => true,
            Self::GoalRound => matches!(action, "complete" | "blocked"),
        }
    }
}

fn render(goal: Option<Goal>, blocked_reason: Option<&str>) -> crate::core_services::ToolOutput {
    match goal {
        None => crate::core_services::ToolOutput {
            content: serde_json::json!({ "goal": null }),
            rendered: vec!["No goal is set.".to_string()],
        },
        Some(goal) => {
            let status = if blocked_reason.is_some() {
                "blocked"
            } else {
                goal.status.as_str()
            };
            crate::core_services::ToolOutput {
                content: serde_json::json!({
                    "goal": {
                        "revision": goal.revision,
                        "objective": goal.objective,
                        "status": status,
                        "max_goal_rounds": goal.max_rounds,
                        "blocked_reason": blocked_reason,
                    }
                }),
                rendered: vec![format!(
                    "Goal (rev {}): {} [{}]",
                    goal.revision, goal.objective, status
                )],
            }
        }
    }
}

fn arg_string<'a>(
    args: &'a serde_json::Value,
    key: &str,
) -> std::result::Result<&'a str, ToolError> {
    args.get(key)
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} must be a string")))
}

fn get_handler(store: GoalStore) -> ToolHandler {
    Arc::new(move |_args, _exec| {
        let store = store.clone();
        Box::pin(async move { Ok(render(store.get(), None)) })
    })
}

fn create_handler(store: GoalStore) -> ToolHandler {
    Arc::new(move |args, exec| {
        let store = store.clone();
        Box::pin(async move {
            if !GoalAuthority::from_exec(&exec).allows("create") {
                return Err(ToolError::Execution(
                    "create_goal requires a direct human turn".to_string(),
                ));
            }
            let objective = arg_string(&args, "objective")?;
            let rounds = u32::try_from(
                args.get("max_goal_rounds")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(1000),
            )
            .map_err(|_| ToolError::InvalidArgs("max_goal_rounds out of range".to_string()))?;
            match store.set(objective, rounds) {
                Ok(goal) => Ok(render(Some(goal), None)),
                Err(error) => Err(ToolError::Execution(error.to_string())),
            }
        })
    })
}

fn update_handler(store: GoalStore) -> ToolHandler {
    Arc::new(move |args, exec| {
        let store = store.clone();
        Box::pin(async move {
            let action = arg_string(&args, "action")?;
            let authority = GoalAuthority::from_exec(&exec);
            if !authority.allows(action) {
                return Err(ToolError::Execution(format!(
                    "{action} requires a direct human turn"
                )));
            }
            let revision = args
                .get("goal_id")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| ToolError::InvalidArgs("goal_id must be a number".to_string()))?;
            let current = store
                .get()
                .ok_or_else(|| ToolError::Execution(GoalError::NotFound.to_string()))?;
            let result = match action {
                "edit" => {
                    let objective = arg_string(&args, "objective")?;
                    store.edit(objective, current.max_rounds, revision)
                }
                "pause" => store.pause(revision),
                "resume" => store.resume(revision),
                "complete" => store.complete(revision),
                "blocked" => {
                    let reason = arg_string(&args, "blocked_reason")?;
                    if reason.trim().is_empty() {
                        return Err(ToolError::InvalidArgs(
                            "blocked_reason must not be empty".to_string(),
                        ));
                    }
                    store.pause(revision)
                }
                other => {
                    return Err(ToolError::InvalidArgs(format!("unknown action: {other}")));
                }
            };
            match result {
                Ok(goal) => Ok(render(Some(goal), None)),
                Err(error) => Err(ToolError::Execution(error.to_string())),
            }
        })
    })
}

/// The `dsh-tool-goal` plugin.
pub struct ToolGoal;

#[async_trait]
impl crate::Plugin for ToolGoal {
    fn name(&self) -> &str {
        "tool-goal"
    }

    fn inject(&self) -> &[&str] {
        &["agents", "goals", "tools", "systemPrompt"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx
            .require::<crate::services::agents::AgentRegistry>("agents")
            .await?;
        Ok(())
    }
}

/// Mount with the shared tools and goals seams.
pub async fn mount(ctx: &Context) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let store = ctx.require::<GoalStore>("goals").await?;
    let tools_for_effect = tools.clone();
    let _ = ctx
        .effect(move |scope| {
            let tools = tools_for_effect.clone();
            let store = store.clone();
            async move {
                let defs = [
                    (
                        "get_goal",
                        "Read the current goal",
                        get_handler((*store).clone()),
                    ),
                    (
                        "create_goal",
                        "Set a new goal",
                        create_handler((*store).clone()),
                    ),
                    (
                        "update_goal",
                        "Change the current goal",
                        update_handler((*store).clone()),
                    ),
                ];
                for (name, description, handler) in defs {
                    let disposer = tools.register(ToolDefinition::new(name, description, handler));
                    scope.on_dispose(move || async move {
                        let _ = disposer.dispose().await;
                        Ok(())
                    });
                }
                Ok(())
            }
        })
        .await;
    Ok((*tools).clone())
}
