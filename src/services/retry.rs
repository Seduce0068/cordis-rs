//! `llm-retry` policy executor: provider-routed retry decisions.
//!
//! Upstream: `packages/llm/llm-retry/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The policy owns exponential backoff with jitter and retryable error codes;
//! scheduling/waiting is left to the caller.

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter_ratio: f64,
    pub retryable_codes: Vec<u32>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 500,
            max_delay_ms: 30_000,
            jitter_ratio: 0.2,
            retryable_codes: vec![408, 429, 500, 502, 503, 504],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    Stop,
}

impl RetryPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide whether an error code at a given retry count should be retried.
    pub fn should_retry(&self, code: u32, retry: u32) -> RetryDecision {
        if retry >= self.max_retries {
            return RetryDecision::Stop;
        }
        if self.retryable_codes.contains(&code) {
            RetryDecision::Retry
        } else {
            RetryDecision::Stop
        }
    }

    /// Extract an HTTP status code from an error message (e.g. `429`).
    pub fn error_code(error: &crate::Error) -> Option<u32> {
        let text = error.to_string();
        for token in text.split(|ch: char| !ch.is_ascii_digit()) {
            if token.len() == 3
                && let Ok(code) = token.parse::<u32>()
                && (100..600).contains(&code)
            {
                return Some(code);
            }
        }
        None
    }

    /// Decide from a harness error. Unknown codes are not retried.
    pub fn should_retry_error(&self, error: &crate::Error, retry: u32) -> RetryDecision {
        match Self::error_code(error) {
            Some(code) => self.should_retry(code, retry),
            None => RetryDecision::Stop,
        }
    }

    /// Decide from a tool error message carrying an HTTP-style code.
    pub fn should_retry_message(&self, message: &str, retry: u32) -> RetryDecision {
        let error = crate::Error::Other(message.to_string());
        self.should_retry_error(&error, retry)
    }

    /// Deterministic backoff for a retry number (1-based). `random` supplies
    /// a value in `[0, 1)` for jitter; passing `0.5` yields the midpoint.
    pub fn backoff_ms(&self, retry: u32, random: f64) -> u64 {
        let exponent = retry.saturating_sub(1).min(1024);
        let exponential = self
            .initial_delay_ms
            .saturating_mul(1u64 << exponent.min(20))
            .min(self.max_delay_ms);
        let clamped = random.clamp(0.0, 1.0);
        // Guard against NaN / out-of-range jitter_ratio: a negative jitter
        // would cast to 0 and produce a busy retry loop (swarm review R3-CF06).
        let ratio = if self.jitter_ratio.is_finite() {
            self.jitter_ratio.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // Keep a floor so a misconfigured ratio can never collapse the delay
        // to zero (busy retry). With a valid ratio the floor never binds.
        let jitter = (1.0 - ratio + 2.0 * ratio * clamped).max(0.1);
        ((exponential as f64) * jitter)
            .min(self.max_delay_ms as f64)
            .max(0.0) as u64
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<RetryPolicy> {
    let policy = RetryPolicy::new();
    ctx.provide("retryPolicy", policy.clone()).await?;
    Ok(policy)
}
