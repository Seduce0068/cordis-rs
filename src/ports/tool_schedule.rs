//! Reusable Rust port of `@deepseek-ai/dsh-tool-schedule`.
//!
//! Source: `packages/schedule/tool-schedule/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers `schedule_create`, `schedule_list`, `schedule_view` and
//! `schedule_edit` over the shared `ctx.schedule` service.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::schedule::{ScheduleService, ScheduleStatus};
use crate::{Context, Result};

fn arg_string<'a>(
    args: &'a serde_json::Value,
    key: &str,
) -> std::result::Result<&'a str, ToolError> {
    args.get(key)
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} must be a string")))
}

fn create_handler(service: ScheduleService) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let service = service.clone();
        Box::pin(async move {
            let title = arg_string(&args, "title")?;
            let prompt = arg_string(&args, "prompt")?;
            let due = args
                .get("due_ms")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| ToolError::InvalidArgs("due_ms must be a number".to_string()))?;
            let record = service.create(title, prompt, due);
            Ok(ToolOutput {
                content: serde_json::json!({
                    "id": record.id,
                    "status": record.status.as_str(),
                }),
                rendered: vec![format!("Created schedule {}", record.id)],
            })
        })
    })
}

fn list_handler(service: ScheduleService) -> ToolHandler {
    Arc::new(move |_args, _exec| {
        let service = service.clone();
        Box::pin(async move {
            let records = service.list();
            let values: Vec<serde_json::Value> = records
                .iter()
                .map(|record| {
                    serde_json::json!({
                        "id": record.id,
                        "title": record.title,
                        "status": record.status.as_str(),
                        "next_due_ms": record.next_due_ms,
                    })
                })
                .collect();
            let rendered: Vec<String> = records
                .iter()
                .map(|record| {
                    format!(
                        "{} [{}]: {}",
                        record.id,
                        record.status.as_str(),
                        record.title
                    )
                })
                .collect();
            Ok(ToolOutput {
                content: serde_json::json!({ "schedules": values }),
                rendered: if rendered.is_empty() {
                    vec!["No schedules.".to_string()]
                } else {
                    rendered
                },
            })
        })
    })
}

fn view_handler(service: ScheduleService) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let service = service.clone();
        Box::pin(async move {
            let id = arg_string(&args, "id")?;
            match service.get(id) {
                Some(record) => Ok(ToolOutput {
                    content: serde_json::json!({
                        "id": record.id,
                        "title": record.title,
                        "prompt": record.prompt,
                        "status": record.status.as_str(),
                        "next_due_ms": record.next_due_ms,
                    }),
                    rendered: vec![format!("{}: {}", record.id, record.title)],
                }),
                None => Ok(ToolOutput::text(format!("schedule_not_found: {id}"))),
            }
        })
    })
}

fn edit_handler(service: ScheduleService) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let service = service.clone();
        Box::pin(async move {
            let id = arg_string(&args, "id")?;
            let action = arg_string(&args, "action")?;
            let status = match action {
                "pause" => ScheduleStatus::Paused,
                "resume" => ScheduleStatus::Active,
                "cancel" => ScheduleStatus::Cancelled,
                _ => return Err(ToolError::InvalidArgs("invalid action".to_string())),
            };
            match service.get(id) {
                Some(_) => {
                    // Propagate persistence failures instead of silently
                    // reporting success (swarm review R2-ER). Use Execution,
                    // not Cancelled: the latter means the tool call itself was
                    // cancelled and would be picked up by retry/cancel logic.
                    service
                        .set_status_public(id, status)
                        .map_err(|error| ToolError::Execution(error.to_string()))?;
                    Ok(ToolOutput::text(format!("{id} -> {}", status.as_str())))
                }
                None => Ok(ToolOutput::text(format!("schedule_not_found: {id}"))),
            }
        })
    })
}

/// The `dsh-tool-schedule` plugin.
pub struct ToolSchedule;

#[async_trait]
impl crate::Plugin for ToolSchedule {
    fn name(&self) -> &str {
        "tool-schedule"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "schedule"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx.require::<ToolRegistry>("tools").await?;
        let _ = ctx.require::<ScheduleService>("schedule").await?;
        Ok(())
    }
}

/// Mount with the shared tools and schedule seams.
pub async fn mount(ctx: &Context) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let schedule = ctx.require::<ScheduleService>("schedule").await?;
    let definitions = [
        (
            "schedule_create",
            "Create a schedule",
            create_handler((*schedule).clone()),
        ),
        (
            "schedule_list",
            "List schedules",
            list_handler((*schedule).clone()),
        ),
        (
            "schedule_view",
            "View one schedule",
            view_handler((*schedule).clone()),
        ),
        (
            "schedule_edit",
            "Change a schedule status",
            edit_handler((*schedule).clone()),
        ),
    ];
    let tools_for_effect = tools.clone();
    let _ = ctx
        .effect(move |scope| {
            let tools = tools_for_effect.clone();
            let definitions = definitions.clone();
            async move {
                for (name, description, handler) in definitions {
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
