//! Core `ctx.sessions` and `ctx.sessionProjections` services.
//!
//! Upstream keeps an in-memory append-only log per session and lets plugins
//! register fold units that derive current state from the event stream.
//! Source: `packages/core/session/src/index.ts` and
//! `docs/capability-seams.md` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::context::Disposer;

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

struct SessionInner {
    header: SessionHeader,
    seq: AtomicU64,
    events: RwLock<Vec<SessionEvent>>,
}

impl Session {
    pub fn new(id: impl Into<String>, cwd: Option<String>) -> Self {
        Self {
            inner: Arc::new(SessionInner {
                header: SessionHeader { id: id.into(), cwd },
                seq: AtomicU64::new(0),
                events: RwLock::new(Vec::new()),
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
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let event = SessionEvent {
            seq,
            event_type: event_type.into(),
            data,
        };
        self.inner
            .events
            .write()
            .expect("session events poisoned")
            .push(event.clone());
        event
    }

    pub fn events(&self) -> Vec<SessionEvent> {
        self.inner
            .events
            .read()
            .expect("session events poisoned")
            .clone()
    }

    pub fn seq(&self) -> u64 {
        self.inner.seq.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct StoreState {
    next_id: u64,
    sessions: Vec<Session>,
}

/// `ctx.sessions`: owns live sessions by id.
#[derive(Default, Clone)]
pub struct SessionStore {
    state: Arc<RwLock<StoreState>>,
}

impl SessionStore {
    pub fn create(&self, id: Option<String>, cwd: Option<String>) -> Session {
        let mut guard = self.state.write().expect("session store poisoned");
        let id = id.unwrap_or_else(|| format!("session-{}", guard.next_id));
        guard.next_id += 1;
        let session = Session::new(id, cwd);
        guard.sessions.push(session.clone());
        session
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

    /// Fork a session up to a boundary sequence, mirroring upstream `fork`.
    pub fn fork(&self, source: &Session, boundary: u64, id: Option<String>) -> Session {
        let forked = self.create(id, source.cwd().map(str::to_string));
        for event in source
            .events()
            .into_iter()
            .filter(|event| event.seq <= boundary)
        {
            forked.append(event.event_type, event.data);
        }
        forked
    }
}

/// A projection fold: derive a current value from one event.
pub type ProjectionFold =
    Arc<dyn Fn(serde_json::Value, &SessionEvent) -> serde_json::Value + Send + Sync>;

/// `ctx.sessionProjections`: registry of named fold units.
#[derive(Default, Clone)]
pub struct SessionProjectionRegistry {
    state: Arc<RwLock<Vec<(String, ProjectionFold)>>>,
}

impl SessionProjectionRegistry {
    pub fn register(&self, key: impl Into<String>, fold: ProjectionFold) -> Disposer {
        let key = key.into();
        let mut guard = self.state.write().expect("session projections poisoned");
        guard.push((key.clone(), fold));
        drop(guard);
        let state = self.state.clone();
        Disposer::new(move || {
            let state = state.clone();
            let key = key.clone();
            async move {
                state
                    .write()
                    .expect("session projections poisoned")
                    .retain(|(current, _)| current != &key);
                Ok(())
            }
        })
    }

    pub fn has(&self, key: &str) -> bool {
        self.state
            .read()
            .expect("session projections poisoned")
            .iter()
            .any(|(current, _)| current == key)
    }

    /// Fold all events of a session into the named projection.
    pub fn view(&self, key: &str, session: &Session) -> Option<serde_json::Value> {
        let folds = self.state.read().expect("session projections poisoned");
        let fold = folds
            .iter()
            .find(|(current, _)| current == key)
            .map(|(_, fold)| fold)?;
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
}

impl CheckpointCursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Events committed since the last `commit` call.
    pub fn pending(&self, session: &Session) -> Vec<SessionEvent> {
        let events = session.events();
        let written = *self.written.lock().expect("checkpoint cursor poisoned");
        if written >= events.len() {
            Vec::new()
        } else {
            events[written..].to_vec()
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
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, json.to_string())
            .map_err(|error| crate::Error::Other(error.to_string()))?;
        std::fs::rename(&temp, path).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            crate::Error::Other(error.to_string())
        })
    }

    /// Restore a cursor previously written by `save`. A missing or malformed
    /// file starts from zero so a fresh run checkpoints the whole log.
    pub fn load(path: impl AsRef<std::path::Path>) -> Self {
        let written = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value["written"].as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        Self {
            written: Arc::new(std::sync::Mutex::new(written)),
        }
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
