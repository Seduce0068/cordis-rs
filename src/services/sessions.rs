//! Core `ctx.sessions` and `ctx.sessionProjections` services.
//!
//! Upstream keeps an in-memory append-only log per session and lets plugins
//! register fold units that derive current state from the event stream.
//! Source: `packages/core/session/src/index.ts` and
//! `docs/capability-seams.md` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data.

use std::sync::{Arc, Mutex, RwLock};

use crate::Result;
use crate::primitives::Disposer;

/// One committed session event (branded seq per session).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEvent {
    pub seq: u64,
    pub event_type: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionHeader {
    pub id: String,
    pub cwd: Option<String>,
}

/// An append-only session log. Clonable handle shared with consumers.
#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

/// seq allocation and event insertion share one lock, so concurrent `append`
/// calls cannot reorder sequences relative to the log (swarm review R2).
struct LogState {
    next_seq: u64,
    events: Vec<SessionEvent>,
}

type EventObserver = dyn Fn(&str, &SessionEvent) + Send + Sync;

struct SessionInner {
    header: SessionHeader,
    log: Mutex<LogState>,
    /// Called after each newly appended event (never during restore replay).
    observers: Mutex<Vec<Arc<EventObserver>>>,
}

impl Session {
    pub fn new(id: impl Into<String>, cwd: Option<String>) -> Self {
        Self {
            inner: Arc::new(SessionInner {
                header: SessionHeader { id: id.into(), cwd },
                log: Mutex::new(LogState {
                    next_seq: 0,
                    events: Vec::new(),
                }),
                observers: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn id(&self) -> &str {
        &self.inner.header.id
    }

    pub fn cwd(&self) -> Option<&str> {
        self.inner.header.cwd.as_deref()
    }

    pub fn append(&self, event_type: impl Into<String>, data: serde_json::Value) -> SessionEvent {
        let event = {
            let mut log = self.inner.log.lock().expect("session log poisoned");
            log.next_seq += 1;
            let event = SessionEvent {
                seq: log.next_seq,
                event_type: event_type.into(),
                data,
            };
            log.events.push(event.clone());
            event
        };
        // Notify after releasing the log lock so an observer that re-enters
        // the session cannot deadlock (swarm review R2-FG07).
        let observers = self
            .inner
            .observers
            .lock()
            .expect("session observers poisoned")
            .clone();
        for observer in observers {
            observer(self.id(), &event);
        }
        event
    }

    /// Register an observer called for every newly appended event. Restore
    /// replay (`append_with_seq`) does not notify observers.
    pub fn observe(&self, observer: Arc<EventObserver>) {
        self.inner
            .observers
            .lock()
            .expect("session observers poisoned")
            .push(observer);
    }

    /// Replay a persisted event with its original seq. Used only by JSONL
    /// restore so sequences survive a restart (swarm review R2-DI). Returns
    /// `false` when `seq` is not greater than the current cursor (duplicate or
    /// out-of-order), leaving the log untouched.
    pub fn append_with_seq(
        &self,
        seq: u64,
        event_type: impl Into<String>,
        data: serde_json::Value,
    ) -> bool {
        let mut log = self.inner.log.lock().expect("session log poisoned");
        if seq <= log.next_seq {
            return false;
        }
        log.next_seq = seq;
        log.events.push(SessionEvent {
            seq,
            event_type: event_type.into(),
            data,
        });
        true
    }

    pub fn events(&self) -> Vec<SessionEvent> {
        self.inner
            .log
            .lock()
            .expect("session log poisoned")
            .events
            .clone()
    }

    pub fn seq(&self) -> u64 {
        self.inner
            .log
            .lock()
            .expect("session log poisoned")
            .next_seq
    }
}

#[derive(Default)]
struct StoreState {
    next_id: u64,
    sessions: Vec<Session>,
    /// Observers installed on every session created from now on.
    observers: Vec<Arc<EventObserver>>,
}

/// `ctx.sessions`: owns live sessions by id.
#[derive(Default, Clone)]
pub struct SessionStore {
    state: Arc<RwLock<StoreState>>,
}

impl SessionStore {
    /// Create a session. An explicitly supplied id must be unique.
    pub fn create(&self, id: Option<String>, cwd: Option<String>) -> crate::Result<Session> {
        let mut guard = self.state.write().expect("session store poisoned");
        let explicit = id.is_some();
        let id = id.unwrap_or_else(|| {
            loop {
                let candidate = format!("session-{}", guard.next_id);
                guard.next_id += 1;
                if !guard.sessions.iter().any(|s| s.id() == candidate) {
                    return candidate;
                }
            }
        });
        if guard.sessions.iter().any(|session| session.id() == id) {
            return Err(crate::Error::DuplicateSession(id));
        }
        // The auto-id branch already advanced the cursor while choosing the
        // candidate; advancing again here skips every other id (swarm review
        // R10-TX01).
        if explicit {
            guard.next_id += 1;
        }
        let session = Session::new(id, cwd);
        for observer in &guard.observers {
            session.observe(observer.clone());
        }
        guard.sessions.push(session.clone());
        Ok(session)
    }

    /// Observe events appended to all current and future sessions.
    pub fn observe_all(&self, observer: Arc<EventObserver>) {
        let sessions = {
            let mut guard = self.state.write().expect("session store poisoned");
            guard.observers.push(observer.clone());
            guard.sessions.clone()
        };
        // Drop the store lock before touching each Session's own observer
        // lock: an observer may re-enter the store (swarm review R11-LC04).
        for session in sessions {
            session.observe(observer.clone());
        }
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        self.state
            .read()
            .expect("session store poisoned")
            .sessions
            .iter()
            .find(|session| session.id() == id)
            .cloned()
    }

    pub fn list(&self) -> Vec<Session> {
        self.state
            .read()
            .expect("session store poisoned")
            .sessions
            .clone()
    }

    /// Fork a session up to an inclusive boundary sequence (1-based locally).
    pub fn fork(
        &self,
        source: &Session,
        boundary: u64,
        id: Option<String>,
    ) -> crate::Result<Session> {
        let events = source.events();
        if boundary > events.len() as u64 || boundary == 0 {
            return Err(crate::Error::InvalidBoundary {
                session_id: source.id().to_string(),
                boundary,
                reason: format!("boundary must be between 1 and {}", events.len()),
            });
        }
        if let Some(child_id) = &id
            && self.get(child_id).is_some()
        {
            return Err(crate::Error::DuplicateSession(child_id.clone()));
        }
        let forked = self.create(id, source.cwd().map(str::to_string))?;
        for event in events.into_iter().filter(|event| event.seq <= boundary) {
            forked.append(event.event_type, event.data);
        }
        Ok(forked)
    }
}

/// A projection fold: derive a current value from one event. The `Fn` bound
/// means the fold cannot mutate captured state directly; a stateful fold must
/// capture shared state through `Arc<Mutex<_>>` / `Arc<RwLock<_>>` (swarm
/// review R13-OW08).
pub type ProjectionFold =
    Arc<dyn Fn(serde_json::Value, &SessionEvent) -> serde_json::Value + Send + Sync>;

#[derive(Default)]
struct ProjectionState {
    /// (key, generation id, fold). The id fences disposers so an older
    /// registration cannot remove a later one with the same key (R2-BV).
    entries: Vec<(String, u64, ProjectionFold)>,
    next_id: u64,
}

/// `ctx.sessionProjections`: registry of named fold units.
#[derive(Default, Clone)]
pub struct SessionProjectionRegistry {
    state: Arc<RwLock<ProjectionState>>,
}

impl SessionProjectionRegistry {
    /// Register a fold under `key` (last registration wins). The returned
    /// disposer removes only this registration, even if the key was later
    /// re-registered.
    pub fn register(&self, key: impl Into<String>, fold: ProjectionFold) -> Disposer {
        let key = key.into();
        let mut guard = self.state.write().expect("session projections poisoned");
        guard.next_id += 1;
        let id = guard.next_id;
        if let Some(existing) = guard
            .entries
            .iter_mut()
            .find(|(current, _, _)| current == &key)
        {
            *existing = (key, id, fold);
        } else {
            guard.entries.push((key, id, fold));
        }
        drop(guard);
        let state = self.state.clone();
        Disposer::new(move || {
            let state = state.clone();
            async move {
                state
                    .write()
                    .expect("session projections poisoned")
                    .entries
                    .retain(|(_, current_id, _)| *current_id != id);
                Ok(())
            }
        })
    }

    pub fn has(&self, key: &str) -> bool {
        self.state
            .read()
            .expect("session projections poisoned")
            .entries
            .iter()
            .any(|(current, _, _)| current == key)
    }

    /// Fold all events of a session into the named projection.
    pub fn view(&self, key: &str, session: &Session) -> Option<serde_json::Value> {
        // Clone the fold and drop the read guard before invoking user code:
        // a fold that registers/unregisters a projection would otherwise
        // deadlock on this RwLock (swarm review R2-BV).
        let fold = {
            let state = self.state.read().expect("session projections poisoned");
            state
                .entries
                .iter()
                .find(|(current, _, _)| current == key)
                .map(|(_, _, fold)| fold.clone())?
        };
        let mut value = serde_json::Value::Null;
        for event in session.events() {
            value = fold(value, &event);
        }
        Some(value)
    }
}

/// Tracks how many session events have already been persisted. A checkpoint
/// hook uses this to append only the newly committed tail instead of
/// re-scanning the whole log on every step.
#[derive(Default, Clone)]
pub struct CheckpointCursor {
    written: Arc<std::sync::Mutex<usize>>,
    /// Monotonic counter for unique temp-file names across repeated saves.
    save_seq: Arc<std::sync::atomic::AtomicU64>,
}

impl CheckpointCursor {
    pub fn new() -> Self {
        Self {
            written: Arc::new(std::sync::Mutex::new(0)),
            save_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Events committed since the last `commit` call.
    pub fn pending(&self, session: &Session) -> Vec<SessionEvent> {
        // Hold the cursor lock while taking the session snapshot so a
        // concurrent commit() cannot make us slice with a stale `written`
        // (swarm R2-BV TOCTOU).
        let written = self.written.lock().expect("checkpoint cursor poisoned");
        let events = session.events();
        if *written >= events.len() {
            Vec::new()
        } else {
            events[*written..].to_vec()
        }
    }

    /// Advance the cursor by the number of events successfully persisted.
    pub fn commit(&self, count: usize) {
        let mut written = self.written.lock().expect("checkpoint cursor poisoned");
        *written = written.saturating_add(count);
    }

    pub fn written(&self) -> usize {
        *self.written.lock().expect("checkpoint cursor poisoned")
    }

    /// Persist the cursor so a restarted process resumes from the same event.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> crate::Result<()> {
        let path = path.as_ref();
        let written = *self.written.lock().expect("checkpoint cursor poisoned");
        let json = serde_json::json!({ "written": written });

        // Write to a sibling temp file and rename it into place. A crash
        // between write and rename leaves the previous cursor intact rather
        // than a half-written file that would reset progress to zero.
        // Use a unique sibling temp name: a fixed `.tmp` suffix collides when
        // two cursors (or two processes) save the same path concurrently, and
        // one rename could then persist the other's data (swarm R2-DI).
        let unique = format!(
            "{}.{}.{}.tmp",
            path.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id(),
            self.save_seq
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let temp = path.with_file_name(unique);
        std::fs::write(&temp, json.to_string())
            .map_err(|error| crate::Error::Other(error.to_string()))?;
        std::fs::rename(&temp, path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            crate::Error::Other(error.to_string())
        })
    }

    /// Restore a cursor previously written by `save`. Missing files are a
    /// fresh start (`Ok` with zero), but malformed/corrupt files are an error
    /// so data loss is not silently masked (fail closed).
    pub fn try_load(path: impl AsRef<std::path::Path>) -> std::result::Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::new());
            }
            Err(error) => return Err(format!("cannot read checkpoint cursor: {error}")),
        };
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| format!("invalid checkpoint cursor JSON: {error}"))?;
        let written = value
            .get("written")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| "checkpoint cursor missing 'written'".to_string())?;
        let written = usize::try_from(written)
            .map_err(|_| "checkpoint cursor 'written' out of range".to_string())?;
        Ok(Self {
            written: Arc::new(std::sync::Mutex::new(written)),
            save_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// Backwards-compatible loader that falls back to zero on malformed data.
    /// Prefer `try_load` when corruption should be surfaced.
    pub fn load(path: impl AsRef<std::path::Path>) -> Self {
        Self::try_load(path).unwrap_or_default()
    }
}

/// Mount both services on a context.
pub async fn provide(ctx: &crate::Context) -> Result<(SessionStore, SessionProjectionRegistry)> {
    let sessions = SessionStore::default();
    let projections = SessionProjectionRegistry::default();
    ctx.provide("sessions", sessions.clone()).await?;
    ctx.provide("sessionProjections", projections.clone())
        .await?;
    Ok((sessions, projections))
}
