//! Reusable Rust port of `@deepseek-ai/dsh-tool-working-directory`.
//!
//! Reads or changes the calling session's working directory through the
//! shared `tools` seam. Source:
//! `packages/session/tool-working-directory/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{
    ToolDefinition, ToolError, ToolExecContext, ToolHandler, ToolOutput, ToolRegistry,
};
use crate::services::working_directory::WorkingDirectory;
use crate::{Context, Result};

pub const DESCRIPTION: &str = "Read the current working directory, or change it with cd. Relative paths use the current directory. Existing shells and running processes keep their own directories.";

/// Port of the tool's `execute`.
pub fn execute(
    working_directory: &WorkingDirectory,
    args: &serde_json::Value,
    exec: &ToolExecContext,
) -> std::result::Result<ToolOutput, ToolError> {
    let agent = exec.agent.as_deref().ok_or_else(|| {
        ToolError::Execution("working_directory requires an agent Session".to_string())
    })?;
    let cd = args.get("cd").and_then(|value| value.as_str());
    if cd == Some("") {
        return Err(ToolError::InvalidArgs(
            "cd must not be empty; omit it to read the current directory".to_string(),
        ));
    }
    let cwd = match cd {
        None => working_directory.ensure(
            agent,
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
        ),
        Some(path) => working_directory
            .set(agent, path)
            .map_err(|error| ToolError::Execution(error.to_string()))?,
    };
    Ok(ToolOutput {
        content: serde_json::json!({ "cwd": cwd.to_string_lossy() }),
        rendered: vec![cwd.to_string_lossy().to_string()],
    })
}

/// The `dsh-tool-working-directory` plugin. `inject = ['tools', 'workingDirectory']`.
pub struct ToolWorkingDirectory {
    pub working_directory: WorkingDirectory,
}

#[async_trait]
impl crate::Plugin for ToolWorkingDirectory {
    fn name(&self) -> &str {
        "tool-working-directory"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "workingDirectory"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let tools = ctx.require::<ToolRegistry>("tools").await?;
        let directory = self.working_directory.clone();
        ctx.effect(move |scope| {
            let tools = tools.clone();
            let directory = directory.clone();
            async move {
                let handler: ToolHandler = Arc::new(move |args, exec| {
                    let directory = directory.clone();
                    Box::pin(async move { execute(&directory, &args, &exec) })
                });
                let disposer = tools.register(ToolDefinition::new(
                    "working_directory",
                    DESCRIPTION,
                    handler,
                ));
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

pub async fn mount(ctx: &Context) -> Result<WorkingDirectory> {
    let directory = if let Some(existing) = ctx.get::<WorkingDirectory>("workingDirectory") {
        (*existing).clone()
    } else {
        crate::core_services::provide_working_directory(ctx).await?
    };
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let _ = crate::Fiber::mount_retained(
        ctx,
        Box::new(ToolWorkingDirectory {
            working_directory: directory.clone(),
        }),
    )
    .await?;
    let _ = tools;
    Ok(directory)
}
