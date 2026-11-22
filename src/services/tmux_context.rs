//! `dsh-tmux-context`: opt-in tmux-location context for request turns.
//!
//! Upstream: `packages/context/tmux-context/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! This port parses the tab-separated `display-message` output into a
//! structured location and renders the sampling line.

use std::sync::{Arc, RwLock};

use crate::{Error, Result};

pub const READING_PREFIX: &str = "tmux location (turn ";
pub const FIELD_COUNT: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxLocation {
    pub session_name: String,
    pub window_index: String,
    pub window_name: String,
    pub pane_index: String,
    pub pane_id: String,
    pub window_active: String,
    pub pane_active: String,
    pub window_layout: String,
}

/// Parse a `display-message` result separated by the literal two-character `\t`.
pub fn parse_location(raw: &str) -> std::result::Result<TmuxLocation, Error> {
    let fields: Vec<&str> = raw.split("\\t").collect();
    if fields.len() != FIELD_COUNT {
        return Err(Error::Other(format!(
            "tmux location expected {FIELD_COUNT} fields, got {}",
            fields.len()
        )));
    }
    Ok(TmuxLocation {
        session_name: fields[0].to_string(),
        window_index: fields[1].to_string(),
        window_name: fields[2].to_string(),
        pane_index: fields[3].to_string(),
        pane_id: fields[4].to_string(),
        window_active: fields[5].to_string(),
        pane_active: fields[6].to_string(),
        window_layout: fields[7].to_string(),
    })
}

/// Render the durable context line for one turn/step.
pub fn format_reading(turn: u64, step: u64, location: &TmuxLocation, timestamp: &str) -> String {
    format!(
        "{READING_PREFIX}{turn}, step {step}): session={} window={}:{} pane={} ({})\n",
        location.session_name,
        location.window_index,
        location.window_name,
        location.pane_index,
        timestamp
    )
}

/// Decide whether a reading should be re-injected: only when the rendered
/// location changed since the last injection (refresh interval handled by the
/// caller, like time-context).
pub fn is_changed(previous: Option<&TmuxLocation>, current: &TmuxLocation) -> bool {
    previous != Some(current)
}

#[derive(Clone)]
pub struct TmuxContext {
    /// Shared so clones observe the same last-injected location (swarm review
    /// R8-AP02).
    last: Arc<RwLock<Option<TmuxLocation>>>,
}

impl Default for TmuxContext {
    fn default() -> Self {
        Self {
            last: Arc::new(RwLock::new(None)),
        }
    }
}

impl TmuxContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn should_inject(&self, current: &TmuxLocation) -> bool {
        let guard = self.last.read().expect("tmux context poisoned");
        is_changed(guard.as_ref(), current)
    }

    pub fn mark_injected(&self, location: TmuxLocation) {
        *self.last.write().expect("tmux context poisoned") = Some(location);
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<TmuxContext> {
    let service = TmuxContext::new();
    ctx.provide("tmuxContext", service.clone()).await?;
    Ok(service)
}
