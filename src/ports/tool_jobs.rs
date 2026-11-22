//! Reusable Rust port of `@deepseek-ai/dsh-tool-jobs`.
//!
//! Source: `packages/jobs/tool-jobs/src/index.ts` (+ render.ts) in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers `job_output`, `job_list` and `job_kill` over the shared jobs
//! seam. Completion-notice delivery is a documented phase-2 gap.

use std::sync::Arc;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::jobs::{JobError, JobRegistry, JobStatus};
use crate::{Context, Result};

pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
pub const MAX_WAIT_TIMEOUT_MS: u64 = 600_000;
pub const INLINE_OUTPUT_BYTES: usize = 2048;

fn status_line(id: u64, status: JobStatus, output_len: usize) -> String {
    format!(
        "job {id}: {} ({} bytes)",
        match status {
            JobStatus::Running => "running",
            JobStatus::Stopping => "stopping",
            JobStatus::Succeeded => "succeeded",
            JobStatus::Failed => "failed",
            JobStatus::Killed => "killed",
        },
        output_len
    )
}

fn owner(args: &serde_json::Value) -> Option<&str> {
    args.get("session_id").and_then(|value| value.as_str())
}

fn job_id(args: &serde_json::Value) -> std::result::Result<u64, ToolError> {
    args.get("job_id")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| ToolError::InvalidArgs("job_id must be a number".to_string()))
}

fn job_output_handler(jobs: JobRegistry) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let jobs = jobs.clone();
        Box::pin(async move {
            let id = job_id(&args)?;
            let caller = owner(&args);
            let wait = args
                .get("wait")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let timeout = args
                .get("timeout_ms")
                .and_then(|value| value.as_u64())
                .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
                .min(MAX_WAIT_TIMEOUT_MS);
            if wait {
                // `wait` is a blocking Condvar wait; keep it off the async
                // worker. A timeout returns the current state, not an error.
                let wait_jobs = jobs.clone();
                let wait_caller = caller.map(str::to_string);
                let _ = tokio::task::spawn_blocking(move || {
                    wait_jobs.wait(id, wait_caller.as_deref(), timeout)
                })
                .await
                .map_err(|error| ToolError::Execution(error.to_string()))?
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            }
            let view = jobs
                .view(id, caller)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            let (text, _) = jobs
                .read(id, 0, caller)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            Ok(ToolOutput {
                content: serde_json::json!({
                    "job": {
                        "id": view.id,
                        "status": format!("{:?}", view.status),
                        "output_len": view.output_len,
                    },
                    "text": text,
                }),
                rendered: vec![status_line(view.id, view.status, view.output_len), text],
            })
        })
    })
}

fn job_list_handler(jobs: JobRegistry) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let jobs = jobs.clone();
        Box::pin(async move {
            let views = jobs.list(owner(&args));
            let rendered: Vec<String> = views
                .iter()
                .map(|view| status_line(view.id, view.status, view.output_len))
                .collect();
            let rows: Vec<_> = views
                .iter()
                .map(|view| {
                    serde_json::json!({
                        "id": view.id,
                        "status": format!("{:?}", view.status),
                        "output_len": view.output_len,
                    })
                })
                .collect();
            Ok(ToolOutput {
                content: serde_json::json!({ "jobs": rows }),
                rendered: if rendered.is_empty() {
                    vec!["(no background jobs)".to_string()]
                } else {
                    rendered
                },
            })
        })
    })
}

fn job_kill_handler(jobs: JobRegistry) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let jobs = jobs.clone();
        Box::pin(async move {
            let id = job_id(&args)?;
            let caller = owner(&args);
            // `kill` itself only flips state and does not stop a process; a
            // second kill on Stopping must also read as already-finished.
            let view = jobs
                .view(id, caller)
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            if matches!(
                view.status,
                JobStatus::Stopping | JobStatus::Succeeded | JobStatus::Failed | JobStatus::Killed
            ) {
                return Ok(ToolOutput::text("already-finished"));
            }
            match jobs.kill(id, caller) {
                Ok(()) => Ok(ToolOutput::text("cancellation-requested")),
                Err(JobError::AlreadyTerminal(_)) => Ok(ToolOutput::text("already-finished")),
                Err(error) => Err(ToolError::Execution(error.to_string())),
            }
        })
    })
}

/// The `dsh-tool-jobs` plugin.
pub struct ToolJobs;

#[async_trait::async_trait]
impl crate::Plugin for ToolJobs {
    fn name(&self) -> &str {
        "tool-jobs"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "jobs", "systemPrompt"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx.require::<ToolRegistry>("tools").await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let jobs = ctx.require::<JobRegistry>("jobs").await?;
    let tools_for_effect = tools.clone();
    ctx.effect(move |scope| {
        let tools = tools_for_effect.clone();
        let jobs = jobs.clone();
        async move {
            let defs = [
                (
                    "job_output",
                    "Read a background job",
                    job_output_handler((*jobs).clone()),
                ),
                (
                    "job_list",
                    "List background jobs",
                    job_list_handler((*jobs).clone()),
                ),
                (
                    "job_kill",
                    "Cancel a background job",
                    job_kill_handler((*jobs).clone()),
                ),
            ];
            for (name, description, handler) in defs {
                let disposer = tools.register(ToolDefinition::new(name, description, handler));
                scope.on_dispose(move || async move { disposer.dispose().await });
            }
            Ok(())
        }
    })
    .await?;
    Ok((*tools).clone())
}
