//! Reusable Rust port of `@deepseek-ai/dsh-tool-bash`.
//!
//! Source: `packages/shell/tool-bash/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The tool consumes `ctx.shell`; when `ctx.jobs` is composed, background
//! calls register a job and return its id immediately.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::jobs::{JobHandle, JobRegistry, JobStatus};
use crate::services::shell::{Shell, ShellRequest, ShellResult};
use crate::{Context, Result};

#[derive(Debug, Clone)]
pub struct ToolBashConfig {
    pub enable_run_in_background: bool,
    pub promote_on_timeout: bool,
}

impl Default for ToolBashConfig {
    fn default() -> Self {
        Self {
            enable_run_in_background: true,
            promote_on_timeout: true,
        }
    }
}

pub const DESCRIPTION: &str = "Execute a bash command in a persistent shell session. Returns stdout/stderr and the exit status.";

#[derive(Debug, Clone, PartialEq)]
pub struct BashArgs {
    pub command: String,
    pub description: Option<String>,
    pub workdir: Option<String>,
    pub run_in_background: bool,
}

/// Validate and normalize model-facing bash arguments.
pub fn parse_bash_args(
    args: &serde_json::Value,
    config: &ToolBashConfig,
) -> std::result::Result<BashArgs, ToolError> {
    let command = args
        .get("command")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("command must be a string".to_string()))?;
    if command.trim().is_empty() {
        return Err(ToolError::InvalidArgs(
            "command must not be empty".to_string(),
        ));
    }
    let description = args
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let workdir = args
        .get("workdir")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let run_in_background = args
        .get("run_in_background")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if run_in_background && !config.enable_run_in_background {
        return Err(ToolError::InvalidArgs(
            "run_in_background is disabled for this deployment".to_string(),
        ));
    }
    Ok(BashArgs {
        command: command.to_string(),
        description,
        workdir,
        run_in_background,
    })
}

fn render_result(result: &ShellResult) -> String {
    let mut text = result.stdout.clone();
    if !result.stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&result.stderr);
    }
    if result.exit_code != 0 {
        text.push_str(&format!("\n(exit code {})", result.exit_code));
    }
    text
}

fn foreground_handler(shell: Shell) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let shell = shell.clone();
        Box::pin(async move {
            let config = ToolBashConfig::default();
            let input = parse_bash_args(&args, &config)?;
            let mut request = ShellRequest::new(&input.command);
            if let Some(dir) = &input.workdir {
                request = request.in_dir(dir);
            }
            let result = shell
                .run_with_timeout(&request)
                .await
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            Ok(ToolOutput {
                content: serde_json::json!({
                    "stdout": result.stdout,
                    "stderr": result.stderr,
                    "exit_code": result.exit_code,
                }),
                rendered: vec![render_result(&result)],
            })
        })
    })
}

fn background_handler(shell: Shell, jobs: JobRegistry) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let shell = shell.clone();
        let jobs = jobs.clone();
        Box::pin(async move {
            let input = parse_bash_args(&args, &ToolBashConfig::default())?;
            let handle = jobs
                .start(Some("bash".to_string()))
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            let mut request = ShellRequest::new(&input.command);
            if let Some(dir) = &input.workdir {
                request = request.in_dir(dir);
            }
            match shell.run(&request) {
                Ok(result) => {
                    handle.append(result.stdout.as_bytes());
                    handle.append(result.stderr.as_bytes());
                    handle.settle(if result.success {
                        JobStatus::Succeeded
                    } else {
                        JobStatus::Failed
                    });
                }
                Err(error) => {
                    handle.append(error.to_string().as_bytes());
                    handle.settle(JobStatus::Failed);
                }
            }
            Ok(ToolOutput {
                content: serde_json::json!({ "job_id": handle.id() }),
                rendered: vec![format!("Started background job {}", handle.id())],
            })
        })
    })
}

/// The `dsh-tool-bash` plugin. `inject = ['tools', 'shell']`.
pub struct ToolBash {
    pub config: ToolBashConfig,
}

#[async_trait]
impl crate::Plugin for ToolBash {
    fn name(&self) -> &str {
        "tool-bash"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "shell"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let tools = ctx.require::<ToolRegistry>("tools").await?;
        let shell = ctx.require::<Shell>("shell").await?;
        let jobs = ctx.get::<JobRegistry>("jobs");
        let config = self.config.clone();

        ctx.effect(move |scope| {
            let tools = tools.clone();
            let shell = shell.clone();
            let jobs = jobs.clone();
            let config = config.clone();
            async move {
                let handler: ToolHandler = match jobs {
                    Some(jobs) if config.enable_run_in_background => {
                        background_handler((*shell).clone(), (*jobs).clone())
                    }
                    _ => foreground_handler((*shell).clone()),
                };
                let disposer = tools.register(ToolDefinition::new("bash", DESCRIPTION, handler));
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with the shared tools and shell seams.
pub async fn mount(ctx: &Context, shell: Shell, config: ToolBashConfig) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    if ctx.get::<Shell>("shell").is_none() {
        ctx.provide("shell", shell).await?;
    }
    let _ = crate::Fiber::mount_retained(ctx, Box::new(ToolBash { config })).await?;
    Ok((*tools).clone())
}

/// Helper for hosts that want the settled handle of a background job.
pub fn job_handle(jobs: &JobRegistry, output: &ToolOutput) -> Option<JobHandle> {
    output
        .content
        .get("job_id")
        .and_then(|value| value.as_u64())
        .and_then(|id| jobs.get(id))
}
