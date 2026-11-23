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
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Set when a write/flush fails: further appends are rejected so a retry
    /// cannot create a duplicate seq line (swarm review R14-RB05).
    poisoned: Arc<AtomicBool>,
}

impl JsonlSessionHandle {
    fn check_not_poisoned(&self) -> std::result::Result<(), PersistenceError> {
        if self.poisoned.load(Ordering::Acquire) {
            Err(PersistenceError::Corrupt {
                path: self.path.to_string_lossy().to_string(),
                reason: "session log is poisoned after a failed flush".to_string(),
            })
        } else {
            Ok(())
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, event: &LoggedSessionEvent) -> Result<()> {
        // Use the same poison check as read/read_recovered so every access
        // path reports the same error kind (swarm review R16-RA01).
        self.check_not_poisoned().map_err(Error::from)?;
        let opened = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| Error::Other(error.to_string()));
        let mut file = match opened {
            Ok(file) => file,
            Err(error) => {
                self.poisoned.store(true, Ordering::Release);
                return Err(error);
            }
        };
        let line = serde_json::json!({
            "seq": event.seq,
            "type": event.event_type,
            "data": event.data,
        });
        let written = writeln!(file, "{}", line)
            .and_then(|_| file.flush())
            .map_err(|error| Error::Other(error.to_string()));
        if written.is_err() {
            self.poisoned.store(true, Ordering::Release);
        }
        written
    }

    /// Decode one persisted line with strict schema checks. Missing or
    /// wrong-typed `seq` / `type` is corruption, never a sentinel value
    /// (swarm review R2-DI, high).
    fn decode_line(
        &self,
        line: &str,
        index: usize,
    ) -> std::result::Result<PersistedEvent, PersistenceError> {
        let corrupt = |reason: String| PersistenceError::Corrupt {
            path: self.path.to_string_lossy().to_string(),
            reason: format!("line {}: {reason}", index + 1),
        };
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| corrupt(error.to_string()))?;
        let seq = value
            .get("seq")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| corrupt("missing or invalid seq".to_string()))?;
        let event_type = value
            .get("type")
            .and_then(|value| value.as_str())
            .ok_or_else(|| corrupt("missing or invalid type".to_string()))?
            .to_string();
        let data = value
            .get("data")
            .cloned()
            .ok_or_else(|| corrupt("missing data".to_string()))?;
        Ok(PersistedEvent {
            seq,
            event_type,
            data,
        })
    }

    pub fn read(&self) -> std::result::Result<Vec<PersistedEvent>, PersistenceError> {
        self.check_not_poisoned()?;
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
            events.push(self.decode_line(&line, index)?);
        }
        Ok(events)
    }
}

/// Result of a recovery replay: valid events plus the 1-based line where a
/// torn/corrupt write was detected. `None` means the log was complete.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryReport {
    pub events: Vec<PersistedEvent>,
    pub truncated_at_line: Option<usize>,
    /// Why the log was truncated (torn JSON or schema violation), when known.
    pub reason: Option<String>,
}

impl JsonlSessionHandle {
    /// Replay the log, stopping at the first malformed line instead of
    /// failing. The truncation point is always reported so callers cannot
    /// silently continue with an incomplete log. This never modifies the file.
    pub fn read_recovered(&self) -> std::result::Result<RecoveryReport, PersistenceError> {
        self.check_not_poisoned()?;
        let file =
            File::open(&self.path).map_err(|_| PersistenceError::NotFound(self.id.clone()))?;
        let mut events = Vec::new();
        let mut truncated_at_line = None;
        let mut reason = None;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| PersistenceError::Corrupt {
                path: self.path.to_string_lossy().to_string(),
                reason: error.to_string(),
            })?;
            if line.trim().is_empty() {
                continue; // tolerate a trailing blank line
            }
            match self.decode_line(&line, index) {
                Ok(event) => events.push(event),
                Err(error) => {
                    // Stop here: anything after a torn or malformed write is
                    // untrustworthy. Both the line and the reason are reported
                    // so a host can decide whether to accept the recovery.
                    truncated_at_line = Some(index + 1);
                    reason = Some(error.to_string());
                    break;
                }
            }
        }
        Ok(RecoveryReport {
            events,
            truncated_at_line,
            reason,
        })
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
    /// Number of observer writes that failed (swarm review R2-FG07).
    failures: Arc<std::sync::atomic::AtomicU64>,
}

impl JsonlPersistence {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            state: Arc::new(RwLock::new(StoreState::default())),
            failures: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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
        // Atomically fail if the file already exists: a separate exists()
        // check followed by File::create could truncate a concurrent create
        // (swarm review R2-DI).
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => PersistenceError::AlreadyExists(id.clone()),
                _ => PersistenceError::Corrupt {
                    path: path.to_string_lossy().to_string(),
                    reason: format!("cannot create session log: {error}"),
                },
            })?;
        let handle = JsonlSessionHandle {
            id,
            path,
            poisoned: Arc::new(AtomicBool::new(false)),
        };
        self.state
            .write()
            .expect("persistence state poisoned")
            .handles
            .insert(handle.id().to_string(), handle.clone());
        Ok(handle)
    }

    pub fn open(&self, id: &str) -> std::result::Result<JsonlSessionHandle, PersistenceError> {
        validate_session_id(id)?;
        // Keep lookup, path check and insert in one critical section so two
        // concurrent open() calls cannot create separate handles for the same
        // file (swarm review R11-RS03).
        let mut guard = self.state.write().expect("persistence state poisoned");
        if let Some(handle) = guard.handles.get(id) {
            return Ok(handle.clone());
        }
        let path = self.path_for(id);
        if !path.exists() {
            return Err(PersistenceError::NotFound(id.to_string()));
        }
        let handle = JsonlSessionHandle {
            id: id.to_string(),
            path,
            poisoned: Arc::new(AtomicBool::new(false)),
        };
        guard.handles.insert(id.to_string(), handle.clone());
        Ok(handle)
    }

    /// List known session ids, merging in-memory handles with `.jsonl` files
    /// on disk so a restarted process can see persisted sessions without an
    /// explicit `open()` (swarm review R2-DI).
    /// Number of failed observer writes.
    pub fn failures(&self) -> u64 {
        self.failures.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn list(&self) -> Vec<String> {
        let mut ids: std::collections::BTreeSet<String> = self
            .state
            .read()
            .expect("persistence state poisoned")
            .handles
            .keys()
            .cloned()
            .collect();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_file() {
                    continue;
                }
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                let Some(id) = name.strip_suffix(".jsonl") else {
                    continue;
                };
                // Only accept paths that round-trip through path_for(), which
                // rejects traversal, hidden names and embedded separators.
                if validate_session_id(id).is_err() || self.path_for(id) != entry.path() {
                    continue;
                }
                ids.insert(id.to_string());
            }
        }
        ids.into_iter().collect()
    }

    /// Replay a persisted session into an in-memory Session. Sequence gaps
    /// are tolerated by default (a concurrent process may advance the cursor);
    /// use [`Self::restore_with`] with `strict = true` to reject them.
    pub fn restore(
        &self,
        id: &str,
        cwd: Option<String>,
    ) -> std::result::Result<Session, PersistenceError> {
        self.restore_with(id, cwd, false)
    }

    /// Replay with an optional strict sequence-gap check (swarm review
    /// R10-TX03). Non-monotonic (duplicate/out-of-order) seqs are always
    /// rejected; strict mode additionally rejects gaps.
    pub fn restore_with(
        &self,
        id: &str,
        cwd: Option<String>,
        strict: bool,
    ) -> std::result::Result<Session, PersistenceError> {
        let handle = self.open(id)?;
        let session = Session::new(id, cwd);
        let mut expected = 1u64;
        for event in handle.read()? {
            if strict && event.seq != expected {
                return Err(PersistenceError::Corrupt {
                    path: handle.path().to_string_lossy().to_string(),
                    reason: format!(
                        "seq gap during restore: expected {expected}, got {}",
                        event.seq
                    ),
                });
            }
            if !session.append_with_seq(event.seq, event.event_type, event.data) {
                return Err(PersistenceError::Corrupt {
                    path: handle.path().to_string_lossy().to_string(),
                    reason: format!("non-monotonic persisted seq {} during restore", event.seq),
                });
            }
            expected = event.seq + 1;
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
    // Wire session events to JSONL when a SessionStore is present. Use `get`
    // (not `require`) so hosts without sessions keep working (swarm review
    // R2-FG07).
    if let Some(sessions) = ctx.get::<crate::services::sessions::SessionStore>("sessions") {
        let persistence = store.clone();
        sessions.observe_all(Arc::new(move |session_id, event| {
            let result = persistence
                .open(session_id)
                .or_else(|_| persistence.create(session_id));
            match result {
                Ok(handle) => {
                    if handle.append(event).is_err() {
                        persistence
                            .failures
                            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    }
                }
                Err(_) => {
                    persistence
                        .failures
                        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                }
            }
        }));
    }
    ctx.provide("sessionPersistence", store.clone()).await?;
    Ok(store)
}
