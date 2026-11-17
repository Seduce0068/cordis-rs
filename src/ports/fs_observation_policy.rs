//! Reusable Rust port of `@deepseek-ai/dsh-fs-observation-policy`.
//!
//! Source: `packages/fs/fs-observation-policy/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! A no-clobber guard: a tool must observe a file before editing/deleting it,
//! and writes carry a version obtained from the observation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    Present { version: u64 },
    Absent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WriteIntent {
    CreateIfAbsent,
    ReplaceIfVersion { version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    NotObserved,
    NotFound,
    StaleVersion { expected: u64, actual: u64 },
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotObserved => write!(f, "FS_NOT_OBSERVED"),
            Self::NotFound => write!(f, "FS_NOT_FOUND"),
            Self::StaleVersion { expected, actual } => {
                write!(f, "FS_STALE_VERSION: expected {expected}, actual {actual}")
            }
        }
    }
}

/// Per-owner observation table. Upstream keys by event actor; here the host
/// supplies an owner id explicitly.
#[derive(Clone)]
pub struct FsObservationPolicy {
    entries: Arc<Mutex<HashMap<String, HashMap<String, Observation>>>>,
}

impl Default for FsObservationPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl FsObservationPolicy {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record an observation for `path` under `owner` (fs/observed).
    pub fn observe(&self, owner: &str, path: &str, observation: Observation) {
        self.entries
            .lock()
            .expect("observation policy poisoned")
            .entry(owner.to_string())
            .or_default()
            .insert(path.to_string(), observation);
    }

    fn get(&self, owner: &str, path: &str) -> Option<Observation> {
        self.entries
            .lock()
            .expect("observation policy poisoned")
            .get(owner)
            .and_then(|map| map.get(path))
            .cloned()
    }

    /// fs/edit-intent / fs/delete-intent: require a prior observation.
    pub fn edit_intent(&self, owner: &str, path: &str) -> std::result::Result<u64, PolicyError> {
        match self.get(owner, path) {
            None => Err(PolicyError::NotObserved),
            Some(Observation::Absent) => Err(PolicyError::NotFound),
            Some(Observation::Present { version }) => Ok(version),
        }
    }

    /// fs/write-intent: choose create or versioned replace from observation.
    pub fn write_intent(
        &self,
        owner: &str,
        path: &str,
    ) -> std::result::Result<WriteIntent, PolicyError> {
        match self.get(owner, path) {
            None => Err(PolicyError::NotObserved),
            Some(Observation::Absent) => Ok(WriteIntent::CreateIfAbsent),
            Some(Observation::Present { version }) => Ok(WriteIntent::ReplaceIfVersion { version }),
        }
    }

    /// Clear all observations (dispose / HMR safety).
    pub fn clear(&self) {
        self.entries
            .lock()
            .expect("observation policy poisoned")
            .clear();
    }
}

/// The `dsh-fs-observation-policy` plugin (seam is a state helper; inject = []).
pub struct FsObservationPolicyPlugin;

#[async_trait::async_trait]
impl crate::Plugin for FsObservationPolicyPlugin {
    fn name(&self) -> &str {
        "fs-observation-policy"
    }

    fn inject(&self) -> &[&str] {
        &[]
    }

    async fn apply(&self, _ctx: &crate::Context) -> Result<()> {
        Ok(())
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<FsObservationPolicy> {
    let policy = FsObservationPolicy::new();
    ctx.provide("fsObservationPolicy", policy.clone()).await?;
    Ok(policy)
}
