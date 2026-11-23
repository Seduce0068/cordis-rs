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
    /// Canonicalised once at construction so a symlinked root cannot point
    /// outside the intended boundary later (R3-DE03 adversarial review).
    canonical_root: Option<PathBuf>,
}

impl FileSystem {
    pub fn local() -> Self {
        Self::default()
    }

    /// Confine every operation under `root`. The root is canonicalised once
    /// here; a failure yields a handle that fails closed on every operation.
    pub fn scoped(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let canonical_root = std::fs::canonicalize(&root).ok();
        Self {
            state: Arc::new(RwLock::new(FsState::default())),
            root: Some(root),
            canonical_root,
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
                let canonical_root = self.canonical_root.as_ref().ok_or_else(|| {
                    Error::Other("filesystem root could not be canonicalised".to_string())
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
                if !canonical.starts_with(canonical_root) {
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
        // Stream lines one at a time: never materialise the whole file. Only
        // the requested window is retained; every other line is counted then
        // dropped (swarm review R3-IV06). BufReader::lines() matches
        // str::lines() semantics, so total_lines stays identical.
        use std::io::BufRead;
        let resolved = self.resolve(path)?;
        let file =
            std::fs::File::open(&resolved).map_err(|error| Error::Other(error.to_string()))?;
        let reader = std::io::BufReader::new(file);
        let mut total_lines = 0usize;
        let start = offset - 1;
        let end = start.saturating_add(limit);
        let mut rendered: Vec<String> = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|error| Error::Other(error.to_string()))?;
            let index = total_lines;
            total_lines += 1;
            if index < start || index >= end {
                continue;
            }
            rendered.push(if line.chars().count() > max_line_length {
                format!(
                    "{}…",
                    line.chars().take(max_line_length).collect::<String>()
                )
            } else {
                line
            });
        }
        Ok(ReadWindow {
            content: rendered.join("\n"),
            total_lines,
            start_line: offset,
            lines_returned: rendered.len(),
        })
    }

    pub fn write_text(&self, path: &str, content: &str) -> Result<FsWriteOutcome> {
        let resolved = self.resolve(path)?;
        let existed = resolved.exists();
        let previous = self.version(path);
        let version = self.reserve_version(&resolved, None)?;
        if let Some(error) = self.write_text_inner(&resolved, content) {
            self.restore_version(&resolved, previous);
            return Err(Error::Other(error));
        }
        Ok(FsWriteOutcome {
            operation: if existed {
                WriteOperation::Update
            } else {
                WriteOperation::Create
            },
            version,
        })
    }

    /// File I/O only (tmp+rename + ancestor rollback). Returns the new version
    /// reserved by the caller and an error message when the write failed.
    fn write_text_inner(&self, resolved: &std::path::Path, content: &str) -> Option<String> {
        let existed = resolved.exists();
        // Record ancestors that do not exist yet so a failed write can remove
        // them again (swarm review R14-RB03). Existing ones are never touched.
        let mut created_dirs: Vec<std::path::PathBuf> = Vec::new();
        if let Some(parent) = resolved.parent() {
            let mut cursor = parent.to_path_buf();
            while !cursor.as_os_str().is_empty() && !cursor.exists() {
                created_dirs.push(cursor.clone());
                if !cursor.pop() {
                    break;
                }
            }
            if let Err(error) = std::fs::create_dir_all(parent) {
                return Some(error.to_string());
            }
        }
        // Write a sibling temp then rename: a failure never leaves a partial
        // target file (same pattern as sessions.rs / loader.rs).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp = resolved.with_file_name(format!(
            ".{}.{}.{}.tmp",
            resolved.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id(),
            nanos
        ));
        let write_result =
            std::fs::write(&temp, content).and_then(|_| std::fs::rename(&temp, resolved));
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temp);
            // Deepest-first; remove_dir is a no-op if a concurrent process
            // created something inside, so another writer's files survive.
            for dir in created_dirs {
                let _ = std::fs::remove_dir(dir);
            }
            return Some(error.to_string());
        }
        let _ = existed;
        None
    }

    /// Atomically check the expected version and reserve the next one under a
    /// single write guard (swarm review R14-RB04).
    fn reserve_version(&self, resolved: &std::path::Path, expected: Option<u64>) -> Result<u64> {
        let mut guard = self.state.write().expect("fs state poisoned");
        let current = guard.versions.get(resolved).copied();
        if let (Some(expected), Some(current)) = (expected, current)
            && expected != current
        {
            return Err(Error::Other(format!(
                "version mismatch: expected {expected}, found {current}"
            )));
        }
        let next = current.unwrap_or(0) + 1;
        guard.versions.insert(resolved.to_path_buf(), next);
        Ok(next)
    }

    fn restore_version(&self, resolved: &std::path::Path, previous: Option<u64>) {
        let mut guard = self.state.write().expect("fs state poisoned");
        match previous {
            Some(version) => {
                guard.versions.insert(resolved.to_path_buf(), version);
            }
            None => {
                guard.versions.remove(resolved);
            }
        }
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
        // Reserve the version immediately before writing so a concurrent
        // writer cannot slip between the check and the increment (swarm review
        // R14-RB04).
        let previous = self.version(path);
        let version = self.reserve_version(&resolved, expected_version)?;
        if let Some(error) = self.write_text_inner(&resolved, &updated) {
            self.restore_version(&resolved, previous);
            return Err(Error::Other(error));
        }
        Ok(FsEditOutcome {
            version,
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

/// Register a confined filesystem under `root`. Production hosts should use
/// this instead of `provide()` (which registers an unrestricted local FS for
/// backward compatibility) — swarm review R3-DE03.
pub async fn provide_scoped(ctx: &crate::Context, root: impl AsRef<Path>) -> Result<FileSystem> {
    let fs = FileSystem::scoped(root);
    ctx.provide("fs", fs.clone()).await?;
    Ok(fs)
}
