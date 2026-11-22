//! `ctx.otel`: structured event telemetry sink.
//!
//! Upstream: `packages/telemetry/otel/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The real service exports OTLP channels; this port keeps a bounded in-memory
//! record of structured events for diagnostics, with count-based batching.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::primitives::Disposer;

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

impl EventLogOptions {
    /// Reject options that would make batching meaningless or drop every
    /// recorded event (swarm review R4-OB07).
    pub fn validate(&self) -> std::result::Result<(), TelemetryError> {
        if self.max_events == 0 || self.batch_size == 0 {
            return Err(TelemetryError::InvalidOptions);
        }
        Ok(())
    }
}

/// Default cap on distinct counter keys per reporter (swarm review R4-OB06).
pub const DEFAULT_MAX_COUNTERS: usize = 1000;
/// Bucket for event names beyond the cap so cardinality stays bounded.
pub const OTHER_COUNTER_KEY: &str = "__other__";

#[derive(Debug, Clone, PartialEq)]
pub enum TelemetryError {
    InvalidName,
    /// EventLogOptions has zero max_events or batch_size.
    InvalidOptions,
}

impl std::fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(f, "event name must not be empty"),
            Self::InvalidOptions => write!(f, "event log options must be positive"),
        }
    }
}

#[derive(Default)]
struct ReporterState {
    events: Vec<(EventRecord, u64)>,
    next_seq: u64,
    counters: HashMap<String, u64>,
    /// Total number of events silently dropped by the max_events cap.
    dropped_events: u64,
}

/// A bounded, batched event log channel.
#[derive(Clone)]
pub struct EventLogReporter {
    name: String,
    options: EventLogOptions,
    state: Arc<RwLock<ReporterState>>,
    max_counters: usize,
}

impl EventLogReporter {
    pub fn new(name: impl Into<String>, options: EventLogOptions) -> Self {
        // Fail fast on an invalid config instead of accepting it and only
        // erroring on the first record() (swarm review R3-CF07).
        assert!(
            options.validate().is_ok(),
            "invalid EventLogOptions: max_events and batch_size must be positive"
        );
        Self {
            name: name.into(),
            options,
            state: Arc::new(RwLock::new(ReporterState::default())),
            max_counters: DEFAULT_MAX_COUNTERS,
        }
    }

    /// Override the distinct-counter cap (builder, keeps EventLogOptions
    /// struct literals unchanged).
    pub fn with_max_counters(mut self, max_counters: usize) -> Self {
        // Fail fast on a zero cap, consistent with EventLogOptions::validate
        // (swarm review R2-CF03).
        assert!(max_counters > 0, "max_counters must be positive");
        self.max_counters = max_counters;
        self
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
        self.options.validate()?;
        let name = name.into();
        if name.trim().is_empty() {
            return Err(TelemetryError::InvalidName);
        }
        let name: String = name;
        let mut guard = self.state.write().expect("telemetry state poisoned");
        guard.next_seq += 1;
        let seq = guard.next_seq;
        let recorded_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        guard.events.push((
            EventRecord {
                name: name.clone(),
                attributes,
                seq,
            },
            recorded_at_ms,
        ));
        if guard.events.len() > self.options.max_events {
            let drop_count = guard.events.len() - self.options.max_events;
            guard.events.drain(..drop_count);
            guard.dropped_events += drop_count as u64;
        }
        // Bound counter cardinality: unknown names beyond the cap collapse
        // into a shared bucket. Reserve OTHER_COUNTER_KEY for internal use.
        let key = if name == OTHER_COUNTER_KEY {
            OTHER_COUNTER_KEY.to_string()
        } else if guard.counters.contains_key(&name) || guard.counters.len() < self.max_counters {
            name
        } else {
            OTHER_COUNTER_KEY.to_string()
        };
        let count = guard.counters.entry(key).or_insert(0);
        *count += 1;
        // `%` instead of `u64::is_multiple_of` (stable only since 1.87) to
        // honour the declared MSRV of 1.85.
        Ok(*count % self.options.batch_size == 0)
    }

    pub fn events(&self) -> Vec<EventRecord> {
        self.state
            .read()
            .expect("telemetry state poisoned")
            .events
            .iter()
            .map(|(record, _)| record.clone())
            .collect()
    }

    /// Records paired with their capture-time Unix milliseconds. EventRecord
    /// stays unchanged so existing struct literals keep compiling (swarm
    /// review R9-ST02).
    pub fn events_with_timestamps(&self) -> Vec<(EventRecord, u64)> {
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

    /// Number of events discarded by the `max_events` cap. Lets operators
    /// detect that early diagnostics were lost (swarm review R5-VU04).
    pub fn dropped(&self) -> u64 {
        self.state
            .read()
            .expect("telemetry state poisoned")
            .dropped_events
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
