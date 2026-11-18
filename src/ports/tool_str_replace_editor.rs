//! Reusable Rust port of `@deepseek-ai/dsh-tool-str-replace-editor`.
//!
//! Source: `packages/fs/tool-str-replace-editor/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers a `str_replace_editor` tool with view/create/str_replace/insert
//! commands over the shared `fs` seam.

use std::sync::Arc;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::fs::FileSystem;
use crate::{Context, Result};

pub const DEFAULT_MAX_OUTPUT_CHARS: usize = 16_000;
pub const CLIP_MARKER: &str = "<response clipped>";

fn arg<'a>(args: &'a serde_json::Value, key: &str) -> std::result::Result<&'a str, ToolError> {
    args.get(key)
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} must be a string")))
}

fn clip(text: String) -> String {
    if text.chars().count() <= DEFAULT_MAX_OUTPUT_CHARS {
        return text;
    }
    let kept: String = text.chars().take(DEFAULT_MAX_OUTPUT_CHARS).collect();
    format!("{kept}\n{CLIP_MARKER}")
}

fn numbered(text: &str, range: Option<(usize, usize)>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = range.unwrap_or((1, lines.len()));
    let mut out = String::new();
    for index in start..=end.min(lines.len()) {
        out.push_str(&format!("{:>6}\t{}\n", index, lines[index - 1]));
    }
    out
}

fn parse_range(
    args: &serde_json::Value,
    line_count: usize,
) -> std::result::Result<Option<(usize, usize)>, ToolError> {
    let Some(value) = args.get("view_range") else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| ToolError::InvalidArgs("view_range must be an array".to_string()))?;
    if array.len() != 2 {
        return Err(ToolError::InvalidArgs(
            "view_range needs two elements".to_string(),
        ));
    }
    let start = array[0]
        .as_u64()
        .ok_or_else(|| ToolError::InvalidArgs("invalid view_range".to_string()))?
        as usize;
    let end_raw = array[1]
        .as_i64()
        .ok_or_else(|| ToolError::InvalidArgs("invalid view_range".to_string()))?;
    if start == 0 || start > line_count {
        return Err(ToolError::InvalidArgs(
            "view_range start out of bounds".to_string(),
        ));
    }
    let end = if end_raw == -1 {
        line_count
    } else {
        end_raw as usize
    };
    if end < start || end > line_count {
        return Err(ToolError::InvalidArgs(
            "view_range end out of bounds".to_string(),
        ));
    }
    Ok(Some((start, end)))
}

fn handler(fs: FileSystem) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let fs = fs.clone();
        Box::pin(async move {
            let command = arg(&args, "command")?;
            let path = arg(&args, "path")?;
            match command {
                "view" => {
                    let text = fs
                        .read_text(path)
                        .map_err(|e| ToolError::Execution(e.to_string()))?;
                    let range = parse_range(&args, text.lines().count())?;
                    Ok(ToolOutput::text(clip(numbered(&text, range))))
                }
                "create" => {
                    let content = arg(&args, "file_text")?;
                    if fs.stat(path).map(|info| info.exists).unwrap_or(false) {
                        return Err(ToolError::Execution("FS_ALREADY_EXISTS".to_string()));
                    }
                    fs.write_text(path, content)
                        .map_err(|e| ToolError::Execution(e.to_string()))?;
                    Ok(ToolOutput::text(format!("Created {path}")))
                }
                "str_replace" => {
                    let old = arg(&args, "old_str")?;
                    let new = arg(&args, "new_str")?;
                    let text = fs
                        .read_text(path)
                        .map_err(|e| ToolError::Execution(e.to_string()))?;
                    let occurrences = text.matches(old).count();
                    if occurrences == 0 {
                        return Err(ToolError::Execution("FS_EDIT_NOT_FOUND".to_string()));
                    }
                    if occurrences > 1 {
                        return Err(ToolError::Execution("FS_AMBIGUOUS_EDIT".to_string()));
                    }
                    let replaced = text.replacen(old, new, 1);
                    fs.write_text(path, &replaced)
                        .map_err(|e| ToolError::Execution(e.to_string()))?;
                    Ok(ToolOutput::text(format!("Updated {path}")))
                }
                "insert" => {
                    let text_to_insert = arg(&args, "new_str")?;
                    let line = args
                        .get("insert_line")
                        .and_then(|value| value.as_u64())
                        .ok_or_else(|| {
                            ToolError::InvalidArgs("insert_line must be a number".to_string())
                        })? as usize;
                    let text = fs
                        .read_text(path)
                        .map_err(|e| ToolError::Execution(e.to_string()))?;
                    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
                    if line > lines.len() {
                        return Err(ToolError::InvalidArgs(
                            "insert_line out of bounds".to_string(),
                        ));
                    }
                    lines.insert(line, text_to_insert.to_string());
                    fs.write_text(path, &lines.join("\n"))
                        .map_err(|e| ToolError::Execution(e.to_string()))?;
                    Ok(ToolOutput::text(format!("Inserted into {path}")))
                }
                other => Err(ToolError::InvalidArgs(format!("unknown command: {other}"))),
            }
        })
    })
}

/// The `dsh-tool-str-replace-editor` plugin.
pub struct ToolStrReplaceEditor;

#[async_trait::async_trait]
impl crate::Plugin for ToolStrReplaceEditor {
    fn name(&self) -> &str {
        "tool-str-replace-editor"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "fs"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let tools = ctx.require::<ToolRegistry>("tools").await?;
        let fs = ctx.require::<FileSystem>("fs").await?;
        ctx.effect(move |scope| {
            let tools = tools.clone();
            let fs = fs.clone();
            async move {
                let disposer = tools.register(ToolDefinition::new(
                    "str_replace_editor",
                    "View and edit files with precise replacements",
                    handler((*fs).clone()),
                ));
                scope.on_dispose(move || async move {
                    disposer.dispose().await?;
                    Ok(())
                });
                Ok(())
            }
        })
        .await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let _fiber = crate::Fiber::mount(ctx, Box::new(ToolStrReplaceEditor)).await?;
    Ok((*tools).clone())
}
