//! `ctx.workingDirectory`: per-agent working directory service.
//!
//! Upstream: `packages/context/working-directory/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{Error, Result};

#[derive(Default, Clone)]
struct WorkingDirectoryState {
    directories: Arc<RwLock<HashMap<String, PathBuf>>>,
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

/// Mount the service on a context.
pub async fn provide(ctx: &crate::Context) -> Result<WorkingDirectory> {
    let directory = WorkingDirectory::new();
    ctx.provide("workingDirectory", directory.clone()).await?;
    Ok(directory)
}
