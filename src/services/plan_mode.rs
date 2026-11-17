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

#[derive(Default, Clone)]
pub struct PlanMode {
    state: Arc<RwLock<PlanState>>,
}

impl PlanMode {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a plan/mode event. Last event wins when folding.
    pub fn set(&self, session_id: &str, active: bool) -> PlanModeEvent {
        let mut guard = self.state.write().expect("plan mode poisoned");
        guard.next_seq += 1;
        let event = PlanModeEvent {
            seq: guard.next_seq,
            active,
        };
        guard
            .logs
            .entry(session_id.to_string())
            .or_default()
            .push(event.clone());
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
