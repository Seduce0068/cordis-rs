//! `dsh-file-reference`: file mention grammar helpers.
//!
//! Upstream: `packages/context/file-reference/src/grammar.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Mentions use `@path` or `@"path with spaces"`; directories keep a trailing
//! slash so completion can descend.

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAtToken {
    pub raw: String,
    pub quoted: bool,
}

/// Format a file mention for insertion. Returns `None` for paths the grammar
/// cannot represent safely (control characters or embedded quotes). A quoted
/// `Directory` mention intentionally omits the trailing quote (the trailing
/// `/` terminates the token); only `File` mentions close the quote (swarm
/// review R14-ST02).
pub fn format_file_mention(
    path: &str,
    kind: ReferenceKind,
    preserve_quote: bool,
) -> Option<String> {
    if path.chars().any(|ch| ch.is_control() || ch == '"') {
        return None;
    }
    let rendered = if kind == ReferenceKind::Directory {
        format!("{path}/")
    } else {
        path.to_string()
    };
    let quoted = preserve_quote || rendered.chars().any(char::is_whitespace);
    if !quoted {
        return Some(format!("@{rendered}"));
    }
    if kind == ReferenceKind::Directory {
        Some(format!("@\"{rendered}"))
    } else {
        Some(format!("@\"{rendered}\""))
    }
}

/// Detect an active mention token at a cursor position in a line.
pub fn active_at_token(line: &str, cursor_col: usize) -> Option<ActiveAtToken> {
    // `cursor_col` is a character column; convert it to a byte offset at a
    // valid UTF-8 boundary (swarm review R8-IV01).
    let byte_end = line
        .char_indices()
        .nth(cursor_col)
        .map(|(byte, _)| byte)
        .unwrap_or(line.len());
    let before = &line[..byte_end];
    if let Some(index) = before.rfind("@\"") {
        let raw = &before[index + 2..];
        if !raw.contains('"') {
            return Some(ActiveAtToken {
                raw: raw.to_string(),
                quoted: true,
            });
        }
    }
    if let Some(index) = before.rfind('@') {
        let raw = &before[index + 1..];
        if !raw.chars().any(char::is_whitespace) {
            return Some(ActiveAtToken {
                raw: raw.to_string(),
                quoted: false,
            });
        }
    }
    None
}

/// Expose the grammar helpers as a seam service.
#[derive(Default, Clone)]
pub struct FileReferences;

impl FileReferences {
    pub const fn new() -> Self {
        Self
    }

    pub fn format(&self, path: &str, kind: ReferenceKind, preserve_quote: bool) -> Option<String> {
        format_file_mention(path, kind, preserve_quote)
    }

    pub fn active(&self, line: &str, cursor_col: usize) -> Option<ActiveAtToken> {
        active_at_token(line, cursor_col)
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<FileReferences> {
    let service = FileReferences::new();
    ctx.provide("fileReferences", service.clone()).await?;
    Ok(service)
}
