//! `ctx.workspace`: durable workspace records with session attachment.
//!
//! Upstream: `packages/workspace/workspace/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The upstream implementation uses a storage domain and validates session
//! headers; this port keeps the registry semantics in memory.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceRecord {
    pub id: String,
    pub path: String,
    pub title: String,
    pub session_ids: Vec<String>,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceError {
    NotFound(String),
    Duplicate(String),
    Archived(String),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "workspace not found: {id}"),
            Self::Duplicate(id) => write!(f, "workspace already exists: {id}"),
            Self::Archived(id) => write!(f, "workspace is archived: {id}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

#[derive(Default)]
struct RegistryState {
    records: BTreeMap<String, WorkspaceRecord>,
    order: Vec<String>,
}

#[derive(Default, Clone)]
pub struct WorkspaceRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl WorkspaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &self,
        id: impl Into<String>,
        path: impl Into<String>,
    ) -> std::result::Result<WorkspaceRecord, WorkspaceError> {
        let id = id.into();
        let mut guard = self.state.write().expect("workspace registry poisoned");
        let was_archived = match guard.records.get(&id) {
            Some(existing) if existing.archived => true,
            Some(_) => return Err(WorkspaceError::Duplicate(id)),
            None => false,
        };
        let path = path.into();
        let title = std::path::Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&path)
            .to_string();
        let record = WorkspaceRecord {
            id: id.clone(),
            path,
            title,
            session_ids: Vec::new(),
            archived: false,
        };
        guard.records.insert(id.clone(), record.clone());
        // An archived id is reused in place: keep its original position in
        // the durable order instead of appending a duplicate (swarm review
        // R10-TX02).
        if !was_archived {
            guard.order.push(id);
        }
        Ok(record)
    }

    pub fn get(&self, id: &str) -> Option<WorkspaceRecord> {
        self.state
            .read()
            .expect("workspace registry poisoned")
            .records
            .get(id)
            .cloned()
    }

    /// List in durable registry order.
    pub fn list(&self) -> Vec<WorkspaceRecord> {
        let guard = self.state.read().expect("workspace registry poisoned");
        guard
            .order
            .iter()
            .filter_map(|id| guard.records.get(id).cloned())
            .collect()
    }

    /// Attach a session id to a workspace (idempotent).
    pub fn attach(
        &self,
        workspace_id: &str,
        session_id: impl Into<String>,
    ) -> std::result::Result<WorkspaceRecord, WorkspaceError> {
        let session_id = session_id.into();
        let mut guard = self.state.write().expect("workspace registry poisoned");
        let record = guard
            .records
            .get_mut(workspace_id)
            .ok_or_else(|| WorkspaceError::NotFound(workspace_id.to_string()))?;
        if record.archived {
            return Err(WorkspaceError::Archived(workspace_id.to_string()));
        }
        if !record.session_ids.contains(&session_id) {
            record.session_ids.push(session_id);
        }
        Ok(record.clone())
    }

    /// Sessions attached to the workspace.
    pub fn sessions_in(&self, workspace_id: &str) -> Vec<String> {
        self.get(workspace_id)
            .map(|record| record.session_ids)
            .unwrap_or_default()
    }

    pub fn archive(&self, id: &str) -> std::result::Result<(), WorkspaceError> {
        let mut guard = self.state.write().expect("workspace registry poisoned");
        let record = guard
            .records
            .get_mut(id)
            .ok_or_else(|| WorkspaceError::NotFound(id.to_string()))?;
        record.archived = true;
        Ok(())
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<WorkspaceRegistry> {
    let registry = WorkspaceRegistry::new();
    ctx.provide("workspace", registry.clone()).await?;
    Ok(registry)
}
