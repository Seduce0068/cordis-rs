//! `ctx.fs`: filesystem seam with bounded reads and atomic text mutations.
//!
//! Upstream: `packages/fs/fs/src/index.ts` and
//! `packages/fs/fs-local/src/index.ts` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data. This port provides a local-filesystem
//! backend with version-guarded edits; sandbox policy is intentionally left
//! to the host.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct FsInfo {
    pub exists: bool,
    pub is_file: bool,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WriteOperation {
    Create,
    Update,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FsWriteOutcome {
    pub operation: WriteOperation,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FsEditOutcome {
    pub version: u64,
    pub replacements: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadWindow {
    pub content: String,
    pub total_lines: usize,
    pub start_line: usize,
    pub lines_returned: usize,
}

#[derive(Default)]
struct FsState {
    versions: HashMap<PathBuf, u64>,
}

/// The filesystem seam. Clonable handle backed by the local filesystem.
#[derive(Default, Clone)]
pub struct FileSystem {
    state: Arc<RwLock<FsState>>,
    root: Option<PathBuf>,
}

impl FileSystem {
    pub fn local() -> Self {
        Self::default()
    }

    /// Confine every operation under `root`.
    pub fn scoped(root: impl AsRef<Path>) -> Self {
        Self {
            state: Arc::new(RwLock::new(FsState::default())),
            root: Some(root.as_ref().to_path_buf()),
        }
    }

    fn resolve(&self, path: &str) -> Result<PathBuf> {
        let candidate = PathBuf::from(path);
        match &self.root {
            None => Ok(candidate),
            Some(root) => {
                let joined = if candidate.is_absolute() {
                    candidate
                } else {
                    root.join(candidate)
                };
                let canonical_root = std::fs::canonicalize(root).map_err(|error| {
                    Error::Other(format!("cannot resolve filesystem root: {error}"))
                })?;
                // Canonicalize before checking containment. A path that does
                // not exist yet (a file about to be created) is resolved via
                // its nearest existing ancestor, so symlinks in the existing
                // prefix cannot bypass the root check (swarm review R2-ER).
                let canonical = match std::fs::canonicalize(&joined) {
                    Ok(resolved) => resolved,
                    Err(_) => {
                        let mut ancestor = joined.as_path();
                        let mut tail = Vec::new();
                        while !ancestor.exists() {
                            let Some(name) = ancestor.file_name() else {
                                return Err(Error::Other(format!(
                                    "path escapes filesystem root: {path}"
                                )));
                            };
                            tail.push(name.to_os_string());
                            let Some(parent) = ancestor.parent() else {
                                return Err(Error::Other(format!(
                                    "path escapes filesystem root: {path}"
                                )));
                            };
                            ancestor = parent;
                        }
                        let mut resolved = std::fs::canonicalize(ancestor).map_err(|error| {
                            Error::Other(format!("cannot resolve path: {error}"))
                        })?;
                        for component in tail.iter().rev() {
                            resolved.push(component);
                        }
                        resolved
                    }
                };
                if !canonical.starts_with(&canonical_root) {
                    return Err(Error::Other(format!(
                        "path escapes filesystem root: {path}"
                    )));
                }
                // Return the verified canonical path so callers operate on
                // exactly what was checked (limits symlink TOCTOU exposure).
                Ok(canonical)
            }
        }
    }

    pub fn stat(&self, path: &str) -> Result<FsInfo> {
        let resolved = self.resolve(path)?;
        match std::fs::metadata(&resolved) {
            Ok(metadata) => Ok(FsInfo {
                exists: true,
                is_file: metadata.is_file(),
                size: metadata.len(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FsInfo {
                exists: false,
                is_file: false,
                size: 0,
            }),
            Err(error) => Err(Error::Other(error.to_string())),
        }
    }

    pub fn read_text(&self, path: &str) -> Result<String> {
        let resolved = self.resolve(path)?;
        std::fs::read_to_string(&resolved).map_err(|error| Error::Other(error.to_string()))
    }

    /// Read a bounded window of lines (1-based offset and limit).
    pub fn read_window(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
        max_line_length: usize,
    ) -> Result<ReadWindow> {
        if offset < 1 || limit < 1 {
            return Err(Error::Other(
                "offset and limit must be positive".to_string(),
            ));
        }
        let text = self.read_text(path)?;
        let lines: Vec<&str> = text.lines().collect();
        let total_lines = lines.len();
        let start = (offset - 1).min(total_lines);
        let end = (start + limit).min(total_lines);
        let selected = &lines[start..end];
        let rendered: Vec<String> = selected
            .iter()
            .map(|line| {
                if line.chars().count() > max_line_length {
                    format!(
                        "{}…",
                        line.chars().take(max_line_length).collect::<String>()
                    )
                } else {
                    line.to_string()
                }
            })
            .collect();
        Ok(ReadWindow {
            content: rendered.join("\n"),
            total_lines,
            start_line: offset,
            lines_returned: end - start,
        })
    }

    pub fn write_text(&self, path: &str, content: &str) -> Result<FsWriteOutcome> {
        let resolved = self.resolve(path)?;
        let existed = resolved.exists();
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|error| Error::Other(error.to_string()))?;
        }
        std::fs::write(&resolved, content).map_err(|error| Error::Other(error.to_string()))?;
        let version = {
            let mut guard = self.state.write().expect("fs state poisoned");
            let entry = guard.versions.entry(resolved.clone()).or_insert(0);
            *entry += 1;
            *entry
        };
        Ok(FsWriteOutcome {
            operation: if existed {
                WriteOperation::Update
            } else {
                WriteOperation::Create
            },
            version,
        })
    }

    /// Literal edit with a version guard: `expected_version` must match the
    /// recorded version when provided. `replace_all` controls uniqueness.
    pub fn edit_text(
        &self,
        path: &str,
        old: &str,
        new: &str,
        replace_all: bool,
        expected_version: Option<u64>,
    ) -> Result<FsEditOutcome> {
        if old.is_empty() {
            return Err(Error::Other("old_string must be non-empty".to_string()));
        }
        if old == new {
            return Err(Error::Other(
                "old_string and new_string must differ".to_string(),
            ));
        }
        let resolved = self.resolve(path)?;
        let current_version = self
            .state
            .read()
            .expect("fs state poisoned")
            .versions
            .get(&resolved)
            .copied();
        if let (Some(expected), Some(current)) = (expected_version, current_version)
            && expected != current
        {
            return Err(Error::Other(format!(
                "version mismatch: expected {expected}, found {current}"
            )));
        }
        let text = self.read_text(path)?;
        let count = text.matches(old).count();
        if !replace_all && count != 1 {
            return Err(Error::Other(format!(
                "expected exactly one match, found {count}"
            )));
        }
        if count == 0 {
            return Err(Error::Other("no matches found".to_string()));
        }
        let updated = if replace_all {
            text.replace(old, new)
        } else {
            text.replacen(old, new, 1)
        };
        let outcome = self.write_text(path, &updated)?;
        Ok(FsEditOutcome {
            version: outcome.version,
            replacements: if replace_all { count } else { 1 },
        })
    }

    pub fn version(&self, path: &str) -> Option<u64> {
        let resolved = self.resolve(path).ok()?;
        self.state
            .read()
            .expect("fs state poisoned")
            .versions
            .get(&resolved)
            .copied()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<FileSystem> {
    let fs = FileSystem::local();
    ctx.provide("fs", fs.clone()).await?;
    Ok(fs)
}
