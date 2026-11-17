//! `dsh-file-reference-local`: workspace file search for `@` mentions.
//!
//! Upstream: `packages/context/file-reference-local/src/search.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Recursively indexes a workspace, skips well-known build/cache directories
//! and returns deterministic prefix matches.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const DEFAULT_MAX_RESULTS: usize = 20;
pub const DEFAULT_MAX_ENTRIES: usize = 50_000;
pub const DEFAULT_EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "dist",
    "build",
    "out",
    "coverage",
    "target",
    ".next",
    ".nuxt",
    ".turbo",
    ".venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".gradle",
];

#[derive(Debug, Clone, PartialEq)]
pub struct FileSearchConfig {
    pub max_results: usize,
    pub max_entries: usize,
    pub excluded_directories: Vec<String>,
}

impl Default for FileSearchConfig {
    fn default() -> Self {
        Self {
            max_results: DEFAULT_MAX_RESULTS,
            max_entries: DEFAULT_MAX_ENTRIES,
            excluded_directories: DEFAULT_EXCLUDED_DIRECTORIES
                .iter()
                .map(|name| name.to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileSearchCandidate {
    pub path: String,
    pub is_directory: bool,
}

/// `WorkspaceFileSearch`: indexes a root once and answers prefix queries.
#[derive(Debug, Clone)]
pub struct WorkspaceFileSearch {
    root: PathBuf,
    config: FileSearchConfig,
    entries: Vec<FileSearchCandidate>,
}

impl WorkspaceFileSearch {
    pub fn new(
        root: impl Into<PathBuf>,
        config: FileSearchConfig,
    ) -> std::result::Result<Self, Error> {
        if config.max_results == 0 {
            return Err(Error::Other(
                "file search maxResults must be positive".to_string(),
            ));
        }
        if config.max_entries == 0 {
            return Err(Error::Other(
                "file search maxEntries must be positive".to_string(),
            ));
        }
        if config
            .excluded_directories
            .iter()
            .any(|name| name.is_empty() || name.contains('/') || name.contains('\\'))
        {
            return Err(Error::Other(
                "excludedDirectories entries must be non-empty basenames".to_string(),
            ));
        }
        let root = root.into();
        let mut entries = Vec::new();
        let excluded: BTreeSet<&str> = config
            .excluded_directories
            .iter()
            .map(String::as_str)
            .collect();
        index_dir(&root, &root, &excluded, config.max_entries, &mut entries);
        Ok(Self {
            root,
            config,
            entries,
        })
    }

    /// Return up to `max_results` deterministic candidates whose relative path
    /// starts with `query`. Shorter paths sort first, then lexicographically.
    pub fn search(&self, query: &str) -> Vec<FileSearchCandidate> {
        let needle = query.trim_start_matches("./");
        let mut matches: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.path.starts_with(needle))
            .collect();
        matches.sort_by(|a, b| {
            a.path
                .len()
                .cmp(&b.path.len())
                .then_with(|| a.path.cmp(&b.path))
        });
        matches
            .into_iter()
            .take(self.config.max_results)
            .cloned()
            .collect()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn index_dir(
    root: &Path,
    dir: &Path,
    excluded: &BTreeSet<&str>,
    max_entries: usize,
    out: &mut Vec<FileSearchCandidate>,
) {
    if out.len() >= max_entries {
        return;
    }
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = read_dir.filter_map(|entry| entry.ok()).collect();
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        if out.len() >= max_entries {
            return;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        let is_dir = path.is_dir();
        if is_dir && excluded.contains(name.as_ref()) {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        out.push(FileSearchCandidate {
            path: relative.to_string_lossy().to_string(),
            is_directory: is_dir,
        });
        if is_dir {
            index_dir(root, &path, excluded, max_entries, out);
        }
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<WorkspaceFileSearch> {
    let search = WorkspaceFileSearch::new(".", FileSearchConfig::default())?;
    ctx.provide("fileSearch", search.clone()).await?;
    Ok(search)
}
