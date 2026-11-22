//! `ctx.sessionQuery`: exact reads, traces, filters and text search over
//! stored sessions.
//!
//! Upstream: `packages/session-query/session-query/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The concrete SQLite backend adds FTS5; this port provides deterministic
//! in-memory search over the same event vocabulary.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::services::sessions::Session;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionHit {
    pub session_id: String,
    pub event_seq: u64,
    pub event_type: String,
    pub text: String,
}

#[derive(Default)]
struct EngineState {
    sessions: BTreeMap<String, Session>,
}

/// In-memory session query engine.
#[derive(Default, Clone)]
pub struct SessionQueryEngine {
    state: Arc<RwLock<EngineState>>,
}

impl SessionQueryEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index `session`. `Session` clones share their inner event state, so
    /// later `append`s on the original are visible here. `refresh` re-indexes
    /// explicitly and `remove` drops the entry (swarm review R2-DI02).
    pub fn index(&self, session: Session) {
        self.state
            .write()
            .expect("session query state poisoned")
            .sessions
            .insert(session.id().to_string(), session);
    }

    /// Replace the indexed snapshot of a session (equivalent to re-indexing).
    pub fn refresh(&self, session: Session) {
        self.index(session);
    }

    /// Remove a session from the index. Returns true if one was present. This
    /// affects the index only; the underlying `SessionStore` is untouched
    /// (swarm review R12-ST06).
    pub fn remove(&self, session_id: &str) -> bool {
        self.state
            .write()
            .expect("session query state poisoned")
            .sessions
            .remove(session_id)
            .is_some()
    }

    pub fn get(&self, session_id: &str) -> Option<Session> {
        self.state
            .read()
            .expect("session query state poisoned")
            .sessions
            .get(session_id)
            .cloned()
    }

    /// Return events of one session in seq order (trace).
    pub fn trace(&self, session_id: &str) -> Vec<crate::services::sessions::SessionEvent> {
        self.get(session_id)
            .map(|session| session.events())
            .unwrap_or_default()
    }

    /// Filter events by exact type across all indexed sessions.
    pub fn filter(&self, event_type: &str) -> Vec<SessionHit> {
        let guard = self.state.read().expect("session query state poisoned");
        let mut hits = Vec::new();
        for (session_id, session) in &guard.sessions {
            for event in session.events() {
                if event.event_type == event_type {
                    hits.push(SessionHit {
                        session_id: session_id.clone(),
                        event_seq: event.seq,
                        event_type: event.event_type.clone(),
                        text: event
                            .data
                            .get("text")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string(),
                    });
                }
            }
        }
        hits
    }

    /// Case-insensitive substring search over event text fields.
    pub fn search(&self, query: &str) -> Vec<SessionHit> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        let guard = self.state.read().expect("session query state poisoned");
        let mut hits = Vec::new();
        for (session_id, session) in &guard.sessions {
            for event in session.events() {
                let text = event
                    .data
                    .get("text")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if text.to_lowercase().contains(&needle) {
                    hits.push(SessionHit {
                        session_id: session_id.clone(),
                        event_seq: event.seq,
                        event_type: event.event_type.clone(),
                        text: text.to_string(),
                    });
                }
            }
        }
        hits
    }

    pub fn session_ids(&self) -> Vec<String> {
        self.state
            .read()
            .expect("session query state poisoned")
            .sessions
            .keys()
            .cloned()
            .collect()
    }
}

pub async fn provide(ctx: &crate::Context) -> crate::Result<SessionQueryEngine> {
    let engine = SessionQueryEngine::new();
    ctx.provide("sessionQuery", engine.clone()).await?;
    Ok(engine)
}
