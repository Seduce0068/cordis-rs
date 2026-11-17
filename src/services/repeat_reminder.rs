//! `dsh-repeat-tool-reminder`: advisory repeat-call detector.
//!
//! Upstream: `packages/guard/repeat-tool-reminder/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Detects consecutive identical tool calls (name + deep-sorted arguments)
//! and emits a gentle reminder at the first threshold, a detailed one later.

use std::collections::BTreeMap;

use crate::{Error, Result};

pub const DEFAULT_THRESHOLDS: &[usize] = &[3, 5, 8];
pub const DEFAULT_MAX_ARG_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct ReminderConfig {
    pub thresholds: Vec<usize>,
    pub max_arg_chars: usize,
}

impl Default for ReminderConfig {
    fn default() -> Self {
        Self {
            thresholds: DEFAULT_THRESHOLDS.to_vec(),
            max_arg_chars: DEFAULT_MAX_ARG_CHARS,
        }
    }
}

/// Validate thresholds: non-empty, integers >= 2, no duplicates. Return sorted.
pub fn validate_thresholds(thresholds: &[usize]) -> std::result::Result<Vec<usize>, Error> {
    if thresholds.is_empty() {
        return Err(Error::Other(
            "repeat-tool-reminder: thresholds must not be empty".to_string(),
        ));
    }
    let mut sorted = thresholds.to_vec();
    for value in &sorted {
        if *value < 2 {
            return Err(Error::Other(format!(
                "repeat-tool-reminder: invalid threshold {value} — every threshold must be >= 2"
            )));
        }
    }
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() != thresholds.len() {
        return Err(Error::Other(
            "repeat-tool-reminder: duplicate thresholds".to_string(),
        ));
    }
    Ok(sorted)
}

/// Deep key-sort a JSON value so argument objects differing only in key order
/// canonicalize identically.
pub fn sort_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(sort_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, serde_json::Value> = map
                .iter()
                .map(|(key, item)| (key.clone(), sort_json_value(item)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        other => other.clone(),
    }
}

/// Canonical string form of a tool call's arguments.
pub fn canonicalize(arguments: &serde_json::Value) -> String {
    sort_json_value(arguments).to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub detailed: bool,
    pub text: String,
    pub run_length: usize,
}

#[derive(Debug, Clone)]
pub struct RepeatToolReminder {
    config: ReminderConfig,
    last_key: Option<String>,
    run_length: usize,
}

impl RepeatToolReminder {
    pub fn new(config: ReminderConfig) -> std::result::Result<Self, Error> {
        Ok(Self {
            config: ReminderConfig {
                thresholds: validate_thresholds(&config.thresholds)?,
                ..config
            },
            last_key: None,
            run_length: 0,
        })
    }

    /// Observe one tool call. Returns a reminder when a threshold is crossed.
    pub fn observe(&mut self, name: &str, arguments: &serde_json::Value) -> Option<Reminder> {
        let key = format!("{name}:{}", canonicalize(arguments));
        if self.last_key.as_deref() == Some(key.as_str()) {
            self.run_length += 1;
        } else {
            self.last_key = Some(key);
            self.run_length = 1;
        }
        let first = self
            .config
            .thresholds
            .first()
            .copied()
            .unwrap_or(usize::MAX);
        let matched = self.config.thresholds.contains(&self.run_length);
        if !matched {
            return None;
        }
        let detailed = self.run_length != first;
        let chars: String = canonicalize(arguments)
            .chars()
            .take(self.config.max_arg_chars)
            .collect();
        let text = if detailed {
            format!(
                "Repeated tool call detected:\n{name} called {} times with: {chars}\nThe repeated calls are not making progress.",
                self.run_length
            )
        } else {
            "You are repeating the exact same tool call with identical arguments. Consider a different action.".to_string()
        };
        Some(Reminder {
            detailed,
            text,
            run_length: self.run_length,
        })
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<RepeatToolReminder> {
    let service = RepeatToolReminder::new(ReminderConfig::default())?;
    ctx.provide("repeatReminder", service.clone()).await?;
    Ok(service)
}
