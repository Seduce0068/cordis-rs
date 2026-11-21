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

/// Maximum characters retained for event names / attribute values in an OTLP
/// payload (swarm review R4-OB02). Values beyond this are truncated with a
/// marker so external input cannot inflate the payload.
pub const MAX_NAME_LEN: usize = 256;
pub const MAX_VALUE_LEN: usize = 1024;
pub const MAX_SESSION_ID_LEN: usize = 256;
const TRUNCATION_MARKER: &str = "…[truncated]";

/// Character-based truncation (UTF-8 safe) with an idempotent marker: re-running
/// redact_record on an already-truncated value must not stack markers.
fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars: Vec<char> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    if value.ends_with(TRUNCATION_MARKER) {
        return value.to_string(); // already truncated
    }
    let budget = max_chars.saturating_sub(TRUNCATION_MARKER.chars().count());
    chars.truncate(budget);
    let mut result: String = chars.into_iter().collect();
    result.push_str(TRUNCATION_MARKER);
    result
}

/// Export one batch of (already redacted) records.
pub trait OTelExporter: Send + Sync + 'static {
    fn export(&self, records: &[EventRecord]) -> Result<(), TelemetryError>;
}

/// Replace untrusted attribute values with `[redacted]` unless the key is on
/// the allowlist. Event names are sanitized (control characters removed).
pub fn redact_record(record: &EventRecord) -> EventRecord {
    let sanitized_name: String = record.name.chars().filter(|ch| !ch.is_control()).collect();
    let name = truncate_chars(&sanitized_name, MAX_NAME_LEN);
    let attributes = record
        .attributes
        .iter()
        .map(|(key, value)| {
            let safe = ALLOWED_ATTRIBUTE_KEYS.contains(&key.as_str());
            (
                key.clone(),
                if safe {
                    // Allowlisted keys still carry untrusted values (e.g. a
                    // model-supplied tool name): strip control chars so they
                    // cannot forge log lines (R4-OB02).
                    let sanitized: String = value.chars().filter(|ch| !ch.is_control()).collect();
                    let cap = if key == "session_id" {
                        MAX_SESSION_ID_LEN
                    } else {
                        MAX_VALUE_LEN
                    };
                    truncate_chars(&sanitized, cap)
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
    /// Count of inner export failures (swarm review R4-OB04). Kept on the
    /// exporter, not the reporter: export ownership lives here. Atomic so a
    /// clone shared across threads accumulates one total.
    failures: Arc<std::sync::atomic::AtomicU64>,
}

impl Clone for RedactingExporter {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            failures: self.failures.clone(),
        }
    }
}

impl RedactingExporter {
    pub fn new(inner: Arc<dyn OTelExporter>) -> Self {
        Self {
            inner,
            failures: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Number of inner export failures observed. The export API itself keeps
    /// returning Ok so telemetry never fails the host operation (R4-OB04).
    pub fn failures(&self) -> u64 {
        self.failures.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl OTelExporter for RedactingExporter {
    fn export(&self, records: &[EventRecord]) -> Result<(), TelemetryError> {
        let redacted: Vec<EventRecord> = records.iter().map(redact_record).collect();
        // Telemetry failures must never fail the host operation, but they are
        // counted so an operator can detect a dead exporter via failures().
        if self.inner.export(&redacted).is_err() {
            self.failures
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        }
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
    // Always redact at the payload boundary: callers must not be able to leak
    // raw secrets by passing `EventLogReporter::events()` directly (R4-OB01).
    // redact_record is idempotent, so pre-redacted records stay unchanged.
    let safe: Vec<EventRecord> = records.iter().map(redact_record).collect();
    // Export-time UTC nanoseconds. EventRecord stays unchanged for backward
    // compatibility (6 struct literals in tests), so this is the export
    // timestamp; per-event timestamps need a separate ADR (R4-OB05).
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());
    serde_json::json!({
        "resourceLogs": [{
            "scopeLogs": [{
                "logRecords": safe.iter().map(|record| serde_json::json!({
                    "timeUnixNano": now_ns.clone(),
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
