//! Reusable Rust port of `@deepseek-ai/dsh-tool-fs` (read / write / edit).
//!
//! Source: `packages/fs/tool-fs/src/{index,read,write,edit}.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Sandbox escalation, image reads and apply_patch are intentionally out of
//! scope for this port; the three text tools and their argument validation
//! are faithful to upstream.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::fs::FileSystem;
use crate::{Context, Result};

pub const READ_LIMIT: usize = 2000;
pub const STREAM_MIN_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ToolFsConfig {
    pub read: bool,
    pub write: bool,
    pub edit: bool,
    pub read_limit: usize,
    pub read_max_line_length: usize,
}

impl Default for ToolFsConfig {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
            edit: true,
            read_limit: READ_LIMIT,
            read_max_line_length: 2000,
        }
    }
}

/// Port of `parseReadArgs`.
pub fn parse_read_args(
    args: &serde_json::Value,
    max_limit: usize,
) -> std::result::Result<(String, usize, usize), ToolError> {
    let file_path = args
        .get("file_path")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("file_path must be a string".to_string()))?;
    if file_path.trim().is_empty() {
        return Err(ToolError::InvalidArgs(
            "file_path must be a non-empty string".to_string(),
        ));
    }
    let positive = |value: &serde_json::Value,
                    name: &str|
     -> std::result::Result<usize, ToolError> {
        let number = value
            .as_u64()
            .ok_or_else(|| ToolError::InvalidArgs(format!("{name} must be a positive integer")))?;
        if number == 0 {
            return Err(ToolError::InvalidArgs(format!(
                "{name} must be a positive integer"
            )));
        }
        usize::try_from(number).map_err(|_| ToolError::InvalidArgs(format!("{name} is too large")))
    };
    let offset = match args.get("offset") {
        Some(value) => positive(value, "offset")?,
        None => 1,
    };
    let limit = match args.get("limit") {
        Some(value) => positive(value, "limit")?,
        None => max_limit,
    };
    if limit > max_limit {
        return Err(ToolError::InvalidArgs(format!(
            "limit must be less than or equal to {max_limit}"
        )));
    }
    Ok((file_path.to_string(), offset, limit))
}

/// Port of `parseWriteArgs`.
pub fn parse_write_args(
    args: &serde_json::Value,
) -> std::result::Result<(String, String), ToolError> {
    let file_path = args
        .get("file_path")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("file_path must be a string".to_string()))?;
    if file_path.trim().is_empty() {
        return Err(ToolError::InvalidArgs(
            "file_path must be a non-empty string".to_string(),
        ));
    }
    let content = args
        .get("content")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("content must be a string".to_string()))?;
    Ok((file_path.to_string(), content.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditInput {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

/// Port of `parseEditArgs`.
pub fn parse_edit_args(args: &serde_json::Value) -> std::result::Result<EditInput, ToolError> {
    let file_path = args
        .get("file_path")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("file_path must be a string".to_string()))?;
    if file_path.trim().is_empty() {
        return Err(ToolError::InvalidArgs(
            "file_path must be a non-empty string".to_string(),
        ));
    }
    let old_string = args
        .get("old_string")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("old_string must be a string".to_string()))?;
    if old_string.is_empty() {
        return Err(ToolError::InvalidArgs(
            "old_string must be a non-empty string".to_string(),
        ));
    }
    let new_string = args
        .get("new_string")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs("new_string must be a string".to_string()))?;
    if old_string == new_string {
        return Err(ToolError::InvalidArgs(
            "old_string and new_string must differ".to_string(),
        ));
    }
    let replace_all = args
        .get("replace_all")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Ok(EditInput {
        file_path: file_path.to_string(),
        old_string: old_string.to_string(),
        new_string: new_string.to_string(),
        replace_all,
    })
}

fn render_read(path: &str, window: &crate::services::fs::ReadWindow) -> String {
    let mut output = format!(
        "<path>{path}</path>\n<type>file</type>\n<content>\n{}\n</content>",
        window.content
    );
    if window.total_lines > window.lines_returned {
        output.push_str(&format!(
            "\n(showing lines {}-{} of {})",
            window.start_line,
            window.start_line + window.lines_returned - 1,
            window.total_lines
        ));
    }
    output
}

fn render_write(path: &str, operation: &crate::services::fs::WriteOperation) -> String {
    let verb = match operation {
        crate::services::fs::WriteOperation::Create => "Created",
        crate::services::fs::WriteOperation::Update => "Updated",
    };
    format!("<path>{path}</path>\n<type>file</type>\n<content>\n{verb} file\n</content>")
}

fn render_edit(path: &str, replace_all: bool) -> String {
    if replace_all {
        format!("The file {path} has been updated. All occurrences were successfully replaced.")
    } else {
        format!("The file {path} has been updated successfully.")
    }
}

fn read_handler(fs: FileSystem, config: ToolFsConfig) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let fs = fs.clone();
        let config = config.clone();
        Box::pin(async move {
            let (path, offset, limit) = parse_read_args(&args, config.read_limit)?;
            let window = fs
                .read_window(&path, offset, limit, config.read_max_line_length)
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            let rendered = render_read(&path, &window);
            Ok(ToolOutput {
                content: serde_json::json!({
                    "path": path,
                    "content": window.content,
                    "total_lines": window.total_lines,
                }),
                rendered: vec![rendered],
            })
        })
    })
}

fn write_handler(fs: FileSystem) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let fs = fs.clone();
        Box::pin(async move {
            let (path, content) = parse_write_args(&args)?;
            let outcome = fs
                .write_text(&path, &content)
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            Ok(ToolOutput {
                content: serde_json::json!({
                    "path": path,
                    "operation": match outcome.operation {
                        crate::services::fs::WriteOperation::Create => "create",
                        crate::services::fs::WriteOperation::Update => "update",
                    },
                }),
                rendered: vec![render_write(&path, &outcome.operation)],
            })
        })
    })
}

fn edit_handler(fs: FileSystem) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let fs = fs.clone();
        Box::pin(async move {
            let input = parse_edit_args(&args)?;
            let expected_version = fs.version(&input.file_path);
            let outcome = fs
                .edit_text(
                    &input.file_path,
                    &input.old_string,
                    &input.new_string,
                    input.replace_all,
                    expected_version,
                )
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            Ok(ToolOutput {
                content: serde_json::json!({
                    "path": input.file_path,
                    "replacements": outcome.replacements,
                    "version": outcome.version,
                }),
                rendered: vec![render_edit(&input.file_path, input.replace_all)],
            })
        })
    })
}

/// The `dsh-tool-fs` plugin. `inject = ['tools', 'fs']`.
pub struct ToolFs {
    pub config: ToolFsConfig,
}

#[async_trait]
impl crate::Plugin for ToolFs {
    fn name(&self) -> &str {
        "tool-fs"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "fs"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let tools = ctx.require::<ToolRegistry>("tools").await?;
        let fs = ctx.require::<FileSystem>("fs").await?;
        let config = self.config.clone();

        ctx.effect(move |scope| {
            let tools = tools.clone();
            let fs = fs.clone();
            let config = config.clone();
            async move {
                if config.read {
                    let disposer = tools.register(ToolDefinition::new(
                        "read",
                        "Read a file with a bounded line window.",
                        read_handler((*fs).clone(), config.clone()),
                    ));
                    scope.on_dispose(move || async move { disposer.dispose().await });
                }
                if config.write {
                    let disposer = tools.register(ToolDefinition::new(
                        "write",
                        "Create or overwrite a file with full content.",
                        write_handler((*fs).clone()),
                    ));
                    scope.on_dispose(move || async move { disposer.dispose().await });
                }
                if config.edit {
                    let disposer = tools.register(ToolDefinition::new(
                        "edit",
                        "Replace a literal string in a file (unique match by default).",
                        edit_handler((*fs).clone()),
                    ));
                    scope.on_dispose(move || async move { disposer.dispose().await });
                }
                Ok(())
            }
        })
        .await
    }
}

/// Mount with a filesystem and the shared tools seam.
pub async fn mount(ctx: &Context, fs: FileSystem, config: ToolFsConfig) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    if ctx.get_untyped("fs").is_none() {
        ctx.provide("fs", fs).await?;
    }
    let fiber = crate::Fiber::mount_retained(ctx, Box::new(ToolFs { config })).await?;
    // Keep the fiber alive for the context's lifetime. `register_fiber` only
    // stores a Weak, so dropping the mount handle would remove it from the
    // dependent-refresh registry and prevent unload on inject changes.
    ctx.effect(move |scope| {
        let fiber = fiber.clone();
        async move {
            scope.on_dispose(move || async move {
                fiber.unload().await?;
                Ok(())
            });
            Ok(())
        }
    })
    .await?;
    Ok((*tools).clone())
}
