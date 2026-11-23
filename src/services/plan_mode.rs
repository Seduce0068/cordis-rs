//! `ctx.planMode`: per-session collaborative planning state.
//!
//! Upstream: `packages/plan/plan-mode/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Plan mode is logged per session (`plan/mode` events); this port keeps the
//! same "last event wins" semantics with an in-memory log.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;

/// Validation error for the fallible plan-mode API (swarm review R10-IV05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanModeError {
    EmptySessionId,
}

impl std::fmt::Display for PlanModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySessionId => f.write_str("plan mode session_id must not be empty"),
        }
    }
}

impl std::error::Error for PlanModeError {}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanModeEvent {
    pub seq: u64,
    pub active: bool,
}

#[derive(Default)]
struct PlanState {
    logs: BTreeMap<String, Vec<PlanModeEvent>>,
    next_seq: u64,
}

/// Default cap on retained plan/mode events per session.
pub const DEFAULT_MAX_EVENTS: usize = 100;

#[derive(Clone)]
pub struct PlanMode {
    state: Arc<RwLock<PlanState>>,
    max_events: usize,
}

impl Default for PlanMode {
    fn default() -> Self {
        Self {
            state: Arc::new(RwLock::new(PlanState::default())),
            max_events: DEFAULT_MAX_EVENTS,
        }
    }
}

impl PlanMode {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the retained-event cap (builder keeps `new()` unchanged).
    pub fn with_max_events(mut self, max_events: usize) -> Self {
        assert!(max_events > 0, "max_events must be positive");
        self.max_events = max_events;
        self
    }

    /// Append a plan/mode event. Last event wins when folding. An empty
    /// `session_id` is rejected (fail-closed); use [`Self::try_set`] to handle
    /// that case without panicking (swarm review R10-IV05).
    ///
    /// `set`, `enter`, and `exit` panic on an empty or whitespace-only
    /// `session_id`; callers must validate or use `try_set` / `try_enter` /
    /// `try_exit` (swarm review R17-PN02).
    pub fn set(&self, session_id: &str, active: bool) -> PlanModeEvent {
        self.try_set(session_id, active)
            .expect("session_id must not be empty")
    }

    pub fn enter(&self, session_id: &str) -> PlanModeEvent {
        self.set(session_id, true)
    }

    pub fn exit(&self, session_id: &str) -> PlanModeEvent {
        self.set(session_id, false)
    }

    /// Fallible variant of [`Self::set`]: rejects an empty `session_id` before
    /// consuming a sequence number (swarm review R10-IV05).
    pub fn try_set(
        &self,
        session_id: &str,
        active: bool,
    ) -> std::result::Result<PlanModeEvent, PlanModeError> {
        if session_id.trim().is_empty() {
            return Err(PlanModeError::EmptySessionId);
        }
        let mut guard = self.state.write().expect("plan mode poisoned");
        guard.next_seq += 1;
        let event = PlanModeEvent {
            seq: guard.next_seq,
            active,
        };
        let log = guard.logs.entry(session_id.to_string()).or_default();
        log.push(event.clone());
        if log.len() > self.max_events {
            let drop_count = log.len() - self.max_events;
            log.drain(..drop_count);
        }
        Ok(event)
    }

    /// Fallible variant of [`Self::enter`].
    pub fn try_enter(&self, session_id: &str) -> std::result::Result<PlanModeEvent, PlanModeError> {
        self.try_set(session_id, true)
    }

    /// Fallible variant of [`Self::exit`].
    pub fn try_exit(&self, session_id: &str) -> std::result::Result<PlanModeEvent, PlanModeError> {
        self.try_set(session_id, false)
    }

    /// Fold the log: a session with no events is not in plan mode.
    pub fn is_active(&self, session_id: &str) -> bool {
        self.state
            .read()
            .expect("plan mode poisoned")
            .logs
            .get(session_id)
            .and_then(|events| events.last())
            .map(|event| event.active)
            .unwrap_or(false)
    }

    pub fn trace(&self, session_id: &str) -> Vec<PlanModeEvent> {
        self.state
            .read()
            .expect("plan mode poisoned")
            .logs
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<PlanMode> {
    let service = PlanMode::new();
    ctx.provide("planMode", service.clone()).await?;
    Ok(service)
}
