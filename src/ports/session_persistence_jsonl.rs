//! Reusable Rust port of `@deepseek-ai/dsh-session-persistence-jsonl` (core).
//!
//! Source: `packages/session/session-persistence-jsonl/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Upstream validates a fail-closed storage contract with torn-write recovery;
//! this port implements the durable JSONL core: append, read, list and close.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::services::sessions::{Session, SessionEvent as LoggedSessionEvent};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedEvent {
    pub seq: u64,
    pub event_type: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PersistenceError {
    NotFound(String),
    AlreadyExists(String),
    InvalidId(String),
    Corrupt { path: String, reason: String },
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "session {id} not found"),
            Self::AlreadyExists(id) => write!(f, "session {id} already exists"),
            Self::InvalidId(id) => write!(f, "invalid session id {id}"),
            Self::Corrupt { path, reason } => write!(f, "corrupt session log {path}: {reason}"),
        }
    }
}

impl std::error::Error for PersistenceError {}

/// Validate a session id before using it as a filename. Rejects path
/// separators, `..`, leading dots, control characters and over-long ids so a
/// caller cannot escape the configured root (fail closed).
fn validate_session_id(id: &str) -> std::result::Result<(), PersistenceError> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(PersistenceError::InvalidId(id.to_string()))
    }
}

/// Handle for one persisted session.
#[derive(Clone)]
pub struct JsonlSessionHandle {
    id: String,
    path: PathBuf,
}

impl JsonlSessionHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, event: &LoggedSessionEvent) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| Error::Other(error.to_string()))?;
        let line = serde_json::json!({
            "seq": event.seq,
            "type": event.event_type,
            "data": event.data,
        });
        writeln!(file, "{}", line).map_err(|error| Error::Other(error.to_string()))?;
        file.flush()
            .map_err(|error| Error::Other(error.to_string()))
    }

    pub fn read(&self) -> std::result::Result<Vec<PersistedEvent>, PersistenceError> {
        let file =
            File::open(&self.path).map_err(|_| PersistenceError::NotFound(self.id.clone()))?;
        let mut events = Vec::new();
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| PersistenceError::Corrupt {
                path: self.path.to_string_lossy().to_string(),
                reason: error.to_string(),
            })?;
            if line.trim().is_empty() {
                continue; // tolerate a trailing blank line
            }
            let value: serde_json::Value =
                serde_json::from_str(&line).map_err(|error| PersistenceError::Corrupt {
                    path: self.path.to_string_lossy().to_string(),
                    reason: format!("line {}: {error}", index + 1),
                })?;
            events.push(PersistedEvent {
                seq: value["seq"].as_u64().unwrap_or(0),
                event_type: value["type"].as_str().unwrap_or_default().to_string(),
                data: value["data"].clone(),
            });
        }
        Ok(events)
    }
}

#[derive(Default)]
struct StoreState {
    handles: HashMap<String, JsonlSessionHandle>,
}

/// JSONL-backed session persistence store.
#[derive(Default, Clone)]
pub struct JsonlPersistence {
    root: PathBuf,
    state: Arc<RwLock<StoreState>>,
}

impl JsonlPersistence {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            state: Arc::new(RwLock::new(StoreState::default())),
        }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.jsonl"))
    }

    pub fn create(
        &self,
        id: impl Into<String>,
    ) -> std::result::Result<JsonlSessionHandle, PersistenceError> {
        let id = id.into();
        validate_session_id(&id)?;
        std::fs::create_dir_all(&self.root).map_err(|error| PersistenceError::Corrupt {
            path: self.root.to_string_lossy().to_string(),
            reason: error.to_string(),
        })?;
        let path = self.path_for(&id);
        if path.exists() {
            return Err(PersistenceError::AlreadyExists(id));
        }
        File::create(&path).map_err(|error| PersistenceError::Corrupt {
            path: path.to_string_lossy().to_string(),
            reason: error.to_string(),
        })?;
        let handle = JsonlSessionHandle { id, path };
        self.state
            .write()
            .expect("persistence state poisoned")
            .handles
            .insert(handle.id().to_string(), handle.clone());
        Ok(handle)
    }

    pub fn open(&self, id: &str) -> std::result::Result<JsonlSessionHandle, PersistenceError> {
        validate_session_id(id)?;
        if let Some(handle) = self
            .state
            .read()
            .expect("persistence state poisoned")
            .handles
            .get(id)
        {
            return Ok(handle.clone());
        }
        let path = self.path_for(id);
        if !path.exists() {
            return Err(PersistenceError::NotFound(id.to_string()));
        }
        let handle = JsonlSessionHandle {
            id: id.to_string(),
            path,
        };
        self.state
            .write()
            .expect("persistence state poisoned")
            .handles
            .insert(id.to_string(), handle.clone());
        Ok(handle)
    }

    pub fn list(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .state
            .read()
            .expect("persistence state poisoned")
            .handles
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// Replay a persisted session into an in-memory Session.
    pub fn restore(
        &self,
        id: &str,
        cwd: Option<String>,
    ) -> std::result::Result<Session, PersistenceError> {
        let handle = self.open(id)?;
        let session = Session::new(id, cwd);
        for event in handle.read()? {
            session.append(event.event_type, event.data);
        }
        Ok(session)
    }

    pub fn close(&self, id: &str) {
        self.state
            .write()
            .expect("persistence state poisoned")
            .handles
            .remove(id);
    }
}

pub async fn mount(ctx: &crate::Context, root: impl AsRef<Path>) -> Result<JsonlPersistence> {
    let store = JsonlPersistence::new(root);
    ctx.provide("sessionPersistence", store.clone()).await?;
    Ok(store)
}
