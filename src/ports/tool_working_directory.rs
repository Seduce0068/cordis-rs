//! Reusable Rust port of `@deepseek-ai/dsh-tool-working-directory`.
//!
//! Reads or changes the calling session's working directory through the
//! shared `tools` seam. Source:
//! `packages/session/tool-working-directory/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::core_services::{
    ToolDefinition, ToolError, ToolExecContext, ToolHandler, ToolOutput, ToolRegistry,
};
use crate::{Context, Error, Result};

#[derive(Default, Clone)]
struct WorkingDirectoryState {
    directories: Arc<RwLock<std::collections::HashMap<String, PathBuf>>>,
}

/// The `workingDirectory` seam: per-agent working directories.
#[derive(Default, Clone)]
pub struct WorkingDirectory {
    state: WorkingDirectoryState,
}

impl WorkingDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure(&self, agent: &str, initial: impl AsRef<Path>) -> PathBuf {
        let mut guard = self
            .state
            .directories
            .write()
            .expect("working directory poisoned");
        guard
            .entry(agent.to_string())
            .or_insert_with(|| initial.as_ref().to_path_buf())
            .clone()
    }

    /// Set a relative or absolute path. Relative paths resolve against the
    /// agent's current directory. Returns the new absolute path.
    pub fn set(&self, agent: &str, path: impl AsRef<Path>) -> Result<PathBuf> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(Error::Other("cd must not be empty".to_string()));
        }
        let mut guard = self
            .state
            .directories
            .write()
            .expect("working directory poisoned");
        let current = guard
            .entry(agent.to_string())
            .or_insert_with(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
        let next = if path.is_absolute() {
            path.to_path_buf()
        } else {
            current.join(path)
        };
        *current = next.clone();
        Ok(next)
    }

    pub fn get(&self, agent: &str) -> Option<PathBuf> {
        self.state
            .directories
            .read()
            .expect("working directory poisoned")
            .get(agent)
            .cloned()
    }
}

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
    let directory = WorkingDirectory::new();
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    ctx.provide("workingDirectory", directory.clone()).await?;
    let _fiber = crate::Fiber::mount(
        ctx,
        Box::new(ToolWorkingDirectory {
            working_directory: directory.clone(),
        }),
    )
    .await?;
    let _ = tools;
    Ok(directory)
}
