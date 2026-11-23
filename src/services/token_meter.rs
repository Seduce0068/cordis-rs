//! `ctx.tokenMeter`: replay-aware token measurement.
//!
//! Upstream: `packages/llm/token-meter/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Exact tokenization is provider-specific; this port uses a deterministic
//! heuristic (characters / 4, with per-message framing) so pressure decisions
//! are reproducible without a tokenizer dependency.

use crate::Result;
use crate::services::sessions::Session;

/// Deterministic token estimate for one text payload.
pub fn estimate_text(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Estimate one message including role framing overhead.
pub fn estimate_message(role: &str, content: &str) -> usize {
    estimate_text(role) + estimate_text(content) + 4
}

#[derive(Debug, Clone, PartialEq)]
pub struct TokenMeasurement {
    pub total_tokens: usize,
    pub message_count: usize,
    pub pressure: f64,
}

#[derive(Debug, Clone)]
pub struct TokenMeter {
    budget: usize,
}

impl TokenMeter {
    /// Create a meter with the given token budget. A zero budget is raised to
    /// 1 rather than disabling the meter: it keeps `pressure` finite and fails
    /// closed (immediate compaction) instead of dividing by zero (swarm review
    /// R18-BD04).
    pub fn new(budget: usize) -> Self {
        Self {
            budget: budget.max(1),
        }
    }

    /// Measure a session's current log by deriving user/assistant messages
    /// from the `user/message` and `assistant/message` events.
    pub fn measure(&self, session: &Session) -> TokenMeasurement {
        let mut total = 0usize;
        let mut count = 0usize;
        for event in session.events() {
            let role = match event.event_type.as_str() {
                "user/message" => "user",
                "assistant/message" => "assistant",
                _ => continue,
            };
            let content = event
                .data
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            total += estimate_message(role, content);
            count += 1;
        }
        TokenMeasurement {
            total_tokens: total,
            message_count: count,
            pressure: total as f64 / self.budget as f64,
        }
    }
}

impl Default for TokenMeter {
    fn default() -> Self {
        Self::new(100_000)
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<TokenMeter> {
    let meter = TokenMeter::default();
    ctx.provide("tokenMeter", meter.clone()).await?;
    Ok(meter)
}
