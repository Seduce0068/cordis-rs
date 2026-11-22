//! `dsh-time-context`: source-attributed timestamp lines for request turns.
//!
//! Upstream: `packages/context/time-context/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The upstream uses `Intl.DateTimeFormat`; this port requires an RFC 3339
//! timestamp with an explicit UTC offset and renders the sampling line.

use crate::{Error, Result};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimeContextConfig {
    /// Minimum spacing between injected time lines, in milliseconds.
    pub refresh_interval_ms: u64,
}

/// Validate the refresh interval: a non-negative safe integer.
pub fn validate_refresh_interval(ms: u64) -> std::result::Result<(), Error> {
    if ms > i64::MAX as u64 {
        return Err(Error::Other(format!(
            "time-context: refreshIntervalMs must be a non-negative safe integer, got {ms}"
        )));
    }
    Ok(())
}

/// Render the sampling line. `timestamp` must carry a UTC offset (RFC 3339).
pub fn format_sample(turn: u64, step: u64, timestamp: &str) -> std::result::Result<String, Error> {
    if !has_utc_offset(timestamp) {
        return Err(Error::Other(format!(
            "time-context: timestamp must include a UTC offset: {timestamp}"
        )));
    }
    Ok(format!(
        "Time sampled while preparing turn {turn}, step {step}: {timestamp}\n"
    ))
}

/// Check for a trailing `Z` or `+HH:MM` / `-HH:MM` offset.
pub fn has_utc_offset(value: &str) -> bool {
    if value.ends_with('Z') {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() < 6 {
        return false;
    }
    // `get` returns None when the offset lands inside a multi-byte char
    // (swarm review R8-IV01).
    let Some(offset) = value.get(bytes.len() - 6..) else {
        return false;
    };
    matches!(bytes[bytes.len() - 6], b'+' | b'-')
        && offset.as_bytes()[3] == b':'
        && offset[1..3].chars().all(|ch| ch.is_ascii_digit())
        && offset[4..6].chars().all(|ch| ch.is_ascii_digit())
}

#[derive(Debug, Clone)]
pub struct TimeContext {
    config: TimeContextConfig,
    /// Shared so clones (including the one registered by `provide`) observe
    /// the same injection state (swarm review R8-AP02).
    last_injected_ms: Arc<RwLock<Option<u64>>>,
}

impl TimeContext {
    pub fn new(config: TimeContextConfig) -> std::result::Result<Self, Error> {
        validate_refresh_interval(config.refresh_interval_ms)?;
        Ok(Self {
            config,
            last_injected_ms: Arc::new(RwLock::new(None)),
        })
    }

    /// Decide whether a sample should be injected at `now_ms`, honoring the
    /// refresh interval. First sample is always injected.
    pub fn should_inject(&self, now_ms: u64) -> bool {
        match *self.last_injected_ms.read().expect("time context poisoned") {
            None => true,
            Some(last) => now_ms.saturating_sub(last) >= self.config.refresh_interval_ms,
        }
    }

    /// Record an injected sample.
    pub fn mark_injected(&self, now_ms: u64) {
        *self
            .last_injected_ms
            .write()
            .expect("time context poisoned") = Some(now_ms);
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<TimeContext> {
    let service = TimeContext::new(TimeContextConfig::default())?;
    ctx.provide("timeContext", service.clone()).await?;
    Ok(service)
}
