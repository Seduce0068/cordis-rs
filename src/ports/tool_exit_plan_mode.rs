//! Reusable Rust port of the `exit_plan_mode` tool from
//! `packages/plan/plan-mode` in deepseek-ai/deepseek-harness (MIT), read as
//! untrusted reference data. Only admitted while plan mode is active; an
//! approved call exits plan mode and returns the plan text for execution.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{ToolDefinition, ToolError, ToolOutput, ToolRegistry};
use crate::services::plan_mode::PlanMode;
use crate::{Context, Result};

pub const EXIT_PLAN_MODE: &str = "exit_plan_mode";

#[derive(Debug, Clone, PartialEq)]
pub enum ExitPlanError {
    NotInPlanMode,
    InvalidPlan,
}

impl std::fmt::Display for ExitPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInPlanMode => write!(f, "exit_plan_mode is only available in plan mode"),
            Self::InvalidPlan => write!(f, "plan must be non-empty and start with a # heading"),
        }
    }
}

/// Validate a plan: non-empty markdown beginning with a `#` heading.
pub fn validate_plan(plan: &str) -> std::result::Result<(), ExitPlanError> {
    let trimmed = plan.trim();
    if trimmed.is_empty() || !trimmed.starts_with('#') {
        return Err(ExitPlanError::InvalidPlan);
    }
    Ok(())
}

fn handler(plan_mode: PlanMode) -> crate::core_services::ToolHandler {
    Arc::new(move |args, exec| {
        let plan_mode = plan_mode.clone();
        Box::pin(async move {
            let Some(session_id) = exec.session.as_deref() else {
                return Err(ToolError::InvalidArgs(
                    "requires a calling agent".to_string(),
                ));
            };
            if !plan_mode.is_active(session_id) {
                return Ok(ToolOutput::text(
                    "PLAN_NOT_ACTIVE: exit_plan_mode is only available in plan mode",
                ));
            }
            let plan = args
                .get("plan")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if let Err(error) = validate_plan(plan) {
                return Ok(ToolOutput::text(error.to_string()));
            }
            let approved = args
                .get("approved")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if approved {
                plan_mode.exit(session_id);
                Ok(ToolOutput::text(
                    "Plan approved — plan mode exited; carry out the plan starting with your next step.",
                ))
            } else {
                Ok(ToolOutput::text(
                    "Keep planning — revise and present again.",
                ))
            }
        })
    })
}

/// The plan-mode exit tool plugin.
pub struct ToolExitPlanMode;

#[async_trait]
impl crate::Plugin for ToolExitPlanMode {
    fn name(&self) -> &str {
        "tool-exit-plan-mode"
    }

    fn inject(&self) -> &[&str] {
        &[]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx.require::<ToolRegistry>("tools").await?;
        let _ = ctx.require::<PlanMode>("planMode").await?;
        Ok(())
    }
}

/// Mount with the shared tools and planMode seams.
pub async fn mount(ctx: &Context) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let plan_mode = ctx.require::<PlanMode>("planMode").await?;
    let tools_for_effect = tools.clone();
    let _ = ctx
        .effect(move |scope| {
            let tools = tools_for_effect.clone();
            let plan_mode = plan_mode.clone();
            async move {
                let disposer = tools.register(ToolDefinition::new(
                    EXIT_PLAN_MODE,
                    "Present the plan for review and exit plan mode on approval.",
                    handler((*plan_mode).clone()),
                ));
                scope.on_dispose(move || async move {
                    let _ = disposer.dispose().await;
                    Ok(())
                });
                Ok(())
            }
        })
        .await;
    Ok((*tools).clone())
}
