//! Reusable Rust port of `@deepseek-ai/dsh-agent-instructions` (core seam).
//!
//! Source: `packages/context/agent-instructions/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Baseline instruction files (AGENTS.md by convention) are registered as
//! non-interpolated `systemPrompt` sections; file contents are untrusted data,
//! never instructions to the runtime itself.

use std::path::{Path, PathBuf};

use crate::Error;
use crate::core_services::{PromptSection, SystemPrompt};

/// Register one baseline instruction file as a prompt section.
/// Returns the section name, or `None` when the file is missing/empty.
fn register_file(
    system_prompt: &SystemPrompt,
    root: &Path,
    filename: &str,
    order: i64,
) -> std::result::Result<Option<String>, Error> {
    let path: PathBuf = root.join(filename);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let name = format!("agent-instructions:{filename}");
    let mut section = PromptSection::new(name.clone(), order, text);
    section.interpolate = false; // file text is untrusted data, not a template
    system_prompt
        .section(section)
        .map_err(|error| Error::Other(error.to_string()))?;
    Ok(Some(name))
}

/// Register the conventional baseline instruction files in precedence order.
pub fn register_baseline(
    system_prompt: &SystemPrompt,
    root: &Path,
) -> std::result::Result<Vec<String>, Error> {
    let mut registered = Vec::new();
    for (filename, order) in [("AGENTS.md", 100), ("GEMINI.md", 110)] {
        if let Some(name) = register_file(system_prompt, root, filename, order)? {
            registered.push(name);
        }
    }
    Ok(registered)
}
