//! `ctx.otel`: structured event telemetry sink.
//!
//! Upstream: `packages/telemetry/otel/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The real service exports OTLP channels; this port keeps a bounded in-memory
//! record of structured events for diagnostics, with count-based batching.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::context::Disposer;

#[derive(Debug, Clone, PartialEq)]
pub struct EventRecord {
    pub name: String,
    pub attributes: HashMap<String, String>,
    pub seq: u64,
}

#[derive(Debug, Clone)]
pub struct EventLogOptions {
    pub max_events: usize,
    pub batch_size: u64,
}

impl Default for EventLogOptions {
    fn default() -> Self {
        Self {
            max_events: 1000,
            batch_size: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TelemetryError {
    InvalidName,
}

impl std::fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(f, "event name must not be empty"),
        }
    }
}

#[derive(Default)]
struct ReporterState {
    events: Vec<EventRecord>,
    next_seq: u64,
    counters: HashMap<String, u64>,
}

/// A bounded, batched event log channel.
#[derive(Clone)]
pub struct EventLogReporter {
    name: String,
    options: EventLogOptions,
    state: Arc<RwLock<ReporterState>>,
}

impl EventLogReporter {
    pub fn new(name: impl Into<String>, options: EventLogOptions) -> Self {
        Self {
            name: name.into(),
            options,
            state: Arc::new(RwLock::new(ReporterState::default())),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Record one event. Returns `true` when a batch boundary was reached.
    pub fn record(
        &self,
        name: impl Into<String>,
        attributes: HashMap<String, String>,
    ) -> std::result::Result<bool, TelemetryError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(TelemetryError::InvalidName);
        }
        let mut guard = self.state.write().expect("telemetry state poisoned");
        guard.next_seq += 1;
        let seq = guard.next_seq;
        guard.events.push(EventRecord {
            name: name.clone(),
            attributes,
            seq,
        });
        if guard.events.len() > self.options.max_events {
            let drop_count = guard.events.len() - self.options.max_events;
            guard.events.drain(..drop_count);
        }
        let count = guard.counters.entry(name).or_insert(0);
        *count += 1;
        Ok((*count).is_multiple_of(self.options.batch_size))
    }

    pub fn events(&self) -> Vec<EventRecord> {
        self.state
            .read()
            .expect("telemetry state poisoned")
            .events
            .clone()
    }

    pub fn count(&self, name: &str) -> u64 {
        self.state
            .read()
            .expect("telemetry state poisoned")
            .counters
            .get(name)
            .copied()
            .unwrap_or(0)
    }
}

#[derive(Default)]
struct OtelState {
    channels: Vec<(u64, EventLogReporter)>,
    next_id: u64,
}

#[derive(Default, Clone)]
pub struct OTel {
    state: Arc<RwLock<OtelState>>,
}

impl OTel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a caller-owned event channel.
    pub fn event_channel(
        &self,
        name: impl Into<String>,
        options: EventLogOptions,
    ) -> (EventLogReporter, Disposer) {
        let reporter = EventLogReporter::new(name, options);
        let state = self.state.clone();
        let mut guard = state.write().expect("otel state poisoned");
        let id = guard.next_id;
        guard.next_id += 1;
        guard.channels.push((id, reporter.clone()));
        drop(guard);

        let disposer = Disposer::new(move || {
            let state = state.clone();
            async move {
                state
                    .write()
                    .expect("otel state poisoned")
                    .channels
                    .retain(|(current, _)| *current != id);
                Ok(())
            }
        });
        (reporter, disposer)
    }

    pub fn channel_names(&self) -> Vec<String> {
        self.state
            .read()
            .expect("otel state poisoned")
            .channels
            .iter()
            .map(|(_, reporter)| reporter.name().to_string())
            .collect()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<OTel> {
    let service = OTel::new();
    ctx.provide("otel", service.clone()).await?;
    Ok(service)
}
