//! `ctx.toolResultPruner`: deterministic head/middle/tail pruning.
//!
//! Upstream: `packages/compaction/compaction-tool-result-pruner/src/index.ts`
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! When a tool result exceeds the character budget the middle is replaced by
//! a marker while a fixed head and tail are preserved.

use crate::Result;

/// Fixed marker inserted in place of the removed middle span.
/// Matches upstream `PRUNE_MARKER` (37 code points).
pub const PRUNE_MARKER: &str = "\n\n[... tool result middle pruned ...]\n\n";

#[derive(Debug, Clone, PartialEq)]
pub struct PrunerConfig {
    /// Characters kept at the start of the text.
    pub head_chars: usize,
    /// Characters kept at the end of the text.
    pub tail_chars: usize,
    /// Total character budget; pruning runs only above this.
    pub threshold_chars: usize,
}

impl Default for PrunerConfig {
    fn default() -> Self {
        Self {
            head_chars: 4096,
            tail_chars: 1024,
            threshold_chars: 8192,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PruneError {
    InvalidConfig,
    RemovalNotFound,
}

impl std::fmt::Display for PruneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig => write!(f, "head + marker + tail must be below the threshold"),
            Self::RemovalNotFound => write!(f, "failed to locate the removed text span"),
        }
    }
}

/// The result of pruning one text block.
#[derive(Debug, Clone, PartialEq)]
pub struct PrunedText {
    pub text: String,
    pub removed_chars: usize,
}

/// Deterministic pruner.
#[derive(Debug, Clone)]
pub struct ToolResultPruner {
    config: PrunerConfig,
}

impl ToolResultPruner {
    pub fn new(config: PrunerConfig) -> std::result::Result<Self, PruneError> {
        let marker_len = PRUNE_MARKER.chars().count();
        if config.head_chars + config.tail_chars + marker_len > config.threshold_chars {
            return Err(PruneError::InvalidConfig);
        }
        Ok(Self { config })
    }

    /// Prune a single text value. Returns `None` when it is within budget.
    pub fn prune_text(&self, text: &str) -> Option<PrunedText> {
        let points: Vec<char> = text.chars().collect();
        let total = points.len();
        if total <= self.config.threshold_chars {
            return None;
        }

        let head = self.config.head_chars.min(total);
        let tail_start = total.saturating_sub(self.config.tail_chars).max(head);
        let head_text: String = points[..head].iter().collect();
        let tail_text: String = points[tail_start..].iter().collect();
        let removed = tail_start.saturating_sub(head);
        let rendered = format!("{head_text}{PRUNE_MARKER}{tail_text}");
        Some(PrunedText {
            text: rendered,
            removed_chars: removed,
        })
    }

    /// Prune every string value in a JSON tool result, returning a new value.
    pub fn prune_json(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(text) => match self.prune_text(text) {
                Some(pruned) => serde_json::Value::String(pruned.text),
                None => value.clone(),
            },
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(|item| self.prune_json(item)).collect())
            }
            serde_json::Value::Object(map) => {
                let mut result = serde_json::Map::new();
                for (key, item) in map {
                    result.insert(key.clone(), self.prune_json(item));
                }
                serde_json::Value::Object(result)
            }
            other => other.clone(),
        }
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<ToolResultPruner> {
    let pruner = ToolResultPruner::new(PrunerConfig::default())
        .map_err(|error| crate::Error::Other(error.to_string()))?;
    ctx.provide("toolResultPruner", pruner.clone()).await?;
    Ok(pruner)
}
