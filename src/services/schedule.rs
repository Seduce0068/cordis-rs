//! `ctx.schedule`: durable task scheduling service.
//!
//! Upstream: `packages/schedule/schedule/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! This port keeps records in memory and supports the same management verbs
//! (create / list / view / pause / resume / cancel) plus due-task lookup.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleStatus {
    Active,
    Paused,
    Cancelled,
}

impl ScheduleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleRecord {
    pub id: String,
    pub title: String,
    pub prompt: String,
    /// Unix timestamp (milliseconds) of the next delivery.
    pub next_due_ms: u64,
    pub status: ScheduleStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleError {
    NotFound(String),
    Cancelled(String),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "schedule not found: {id}"),
            Self::Cancelled(id) => write!(f, "schedule is cancelled: {id}"),
        }
    }
}

#[derive(Default)]
struct ScheduleState {
    records: BTreeMap<String, ScheduleRecord>,
    next_id: u64,
}

#[derive(Default, Clone)]
pub struct ScheduleService {
    state: Arc<RwLock<ScheduleState>>,
}

impl ScheduleService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &self,
        title: impl Into<String>,
        prompt: impl Into<String>,
        next_due_ms: u64,
    ) -> ScheduleRecord {
        let mut guard = self.state.write().expect("schedule state poisoned");
        guard.next_id += 1;
        let record = ScheduleRecord {
            id: format!("sched-{}", guard.next_id),
            title: title.into(),
            prompt: prompt.into(),
            next_due_ms,
            status: ScheduleStatus::Active,
        };
        guard.records.insert(record.id.clone(), record.clone());
        record
    }

    pub fn get(&self, id: &str) -> Option<ScheduleRecord> {
        self.state
            .read()
            .expect("schedule state poisoned")
            .records
            .get(id)
            .cloned()
    }

    pub fn list(&self) -> Vec<ScheduleRecord> {
        self.state
            .read()
            .expect("schedule state poisoned")
            .records
            .values()
            .cloned()
            .collect()
    }

    fn set_status(
        &self,
        id: &str,
        status: ScheduleStatus,
    ) -> std::result::Result<ScheduleRecord, ScheduleError> {
        let mut guard = self.state.write().expect("schedule state poisoned");
        let record = guard
            .records
            .get_mut(id)
            .ok_or_else(|| ScheduleError::NotFound(id.to_string()))?;
        record.status = status;
        Ok(record.clone())
    }

    /// Generic status transition used by management tools.
    pub fn set_status_public(
        &self,
        id: &str,
        status: ScheduleStatus,
    ) -> std::result::Result<ScheduleRecord, ScheduleError> {
        self.set_status(id, status)
    }

    pub fn pause(&self, id: &str) -> std::result::Result<ScheduleRecord, ScheduleError> {
        self.set_status(id, ScheduleStatus::Paused)
    }

    pub fn resume(&self, id: &str) -> std::result::Result<ScheduleRecord, ScheduleError> {
        self.set_status(id, ScheduleStatus::Active)
    }

    pub fn cancel(&self, id: &str) -> std::result::Result<ScheduleRecord, ScheduleError> {
        self.set_status(id, ScheduleStatus::Cancelled)
    }

    /// Active records whose next delivery time has passed, in id order.
    pub fn due(&self, now_ms: u64) -> Vec<ScheduleRecord> {
        let guard = self.state.read().expect("schedule state poisoned");
        let mut due: Vec<_> = guard
            .records
            .values()
            .filter(|record| {
                record.status == ScheduleStatus::Active && record.next_due_ms <= now_ms
            })
            .cloned()
            .collect();
        due.sort_by(|a, b| a.next_due_ms.cmp(&b.next_due_ms).then(a.id.cmp(&b.id)));
        due
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<ScheduleService> {
    let service = ScheduleService::new();
    ctx.provide("schedule", service.clone()).await?;
    Ok(service)
}
