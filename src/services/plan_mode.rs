//! `ctx.planMode`: per-session collaborative planning state.
//!
//! Upstream: `packages/plan/plan-mode/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Plan mode is logged per session (`plan/mode` events); this port keeps the
//! same "last event wins" semantics with an in-memory log.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;

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

    /// Append a plan/mode event. Last event wins when folding.
    pub fn set(&self, session_id: &str, active: bool) -> PlanModeEvent {
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
        event
    }

    pub fn enter(&self, session_id: &str) -> PlanModeEvent {
        self.set(session_id, true)
    }

    pub fn exit(&self, session_id: &str) -> PlanModeEvent {
        self.set(session_id, false)
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
