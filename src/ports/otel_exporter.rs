//! OTLP exporter seam for telemetry (phase 1).
//!
//! Source: `packages/telemetry/` in deepseek-ai/deepseek-harness (MIT), read as
//! untrusted reference data. Adds a redacting exporter boundary and a fake
//! collector for local verification, without changing `EventLogReporter`.

use std::sync::{Arc, RwLock};

use crate::services::telemetry::{EventRecord, TelemetryError};

/// Attribute keys that may carry real values to an exporter. Everything else
/// is replaced with `[redacted]` before leaving this module.
pub const ALLOWED_ATTRIBUTE_KEYS: [&str; 4] = ["session_id", "tool", "status", "duration_ms"];

/// Export one batch of (already redacted) records.
pub trait OTelExporter: Send + Sync + 'static {
    fn export(&self, records: &[EventRecord]) -> Result<(), TelemetryError>;
}

/// Replace untrusted attribute values with `[redacted]` unless the key is on
/// the allowlist. Event names are sanitized (control characters removed).
pub fn redact_record(record: &EventRecord) -> EventRecord {
    let name: String = record.name.chars().filter(|ch| !ch.is_control()).collect();
    let attributes = record
        .attributes
        .iter()
        .map(|(key, value)| {
            let safe = ALLOWED_ATTRIBUTE_KEYS.contains(&key.as_str());
            (
                key.clone(),
                if safe {
                    value.clone()
                } else {
                    "[redacted]".to_string()
                },
            )
        })
        .collect();
    EventRecord {
        name,
        attributes,
        seq: record.seq,
    }
}

/// Wrap an exporter so redaction happens exactly once, at the seam, regardless
/// of the concrete exporter implementation.
pub struct RedactingExporter {
    inner: Arc<dyn OTelExporter>,
}

impl RedactingExporter {
    pub fn new(inner: Arc<dyn OTelExporter>) -> Self {
        Self { inner }
    }
}

impl OTelExporter for RedactingExporter {
    fn export(&self, records: &[EventRecord]) -> Result<(), TelemetryError> {
        let redacted: Vec<EventRecord> = records.iter().map(redact_record).collect();
        // Telemetry failures must never fail the host operation.
        let _ = self.inner.export(&redacted);
        Ok(())
    }
}

/// In-memory collector for local verification: records every exported batch.
#[derive(Clone, Default)]
pub struct FakeCollector {
    batches: Arc<RwLock<Vec<Vec<EventRecord>>>>,
}

impl FakeCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn batches(&self) -> Vec<Vec<EventRecord>> {
        self.batches
            .read()
            .expect("fake collector poisoned")
            .clone()
    }
}

impl OTelExporter for FakeCollector {
    fn export(&self, records: &[EventRecord]) -> Result<(), TelemetryError> {
        self.batches
            .write()
            .expect("fake collector poisoned")
            .push(records.to_vec());
        Ok(())
    }
}

/// Build an OTLP/HTTP JSON log payload from records (no network I/O here).
pub fn otlp_payload(records: &[EventRecord]) -> serde_json::Value {
    serde_json::json!({
        "resourceLogs": [{
            "scopeLogs": [{
                "logRecords": records.iter().map(|record| serde_json::json!({
                    "timeUnixNano": "0",
                    "severityNumber": 9,
                    "name": record.name,
                    "body": { "stringValue": record.name },
                    "attributes": record.attributes.iter().map(|(key, value)| {
                        serde_json::json!({ "key": key, "value": { "stringValue": value } })
                    }).collect::<Vec<_>>(),
                })).collect::<Vec<_>>()
            }]
        }]
    })
}
