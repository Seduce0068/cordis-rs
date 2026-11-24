//! `tool-call-timeout-policy`: cooperative tool-call timeout decisions.
//!
//! Upstream: `packages/guard/timeout-policy/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The runtime owns the deadline; this port supplies the model-facing result
//! text and the budget resolution rule used by the wrapper.

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct TimeoutResult {
    pub message: String,
    pub timed_out: bool,
}

/// The model-facing result for a timed-out tool call.
pub fn tool_timeout_result(timeout_ms: u64) -> TimeoutResult {
    TimeoutResult {
        message: format!("tool call timed out after {timeout_ms}ms"),
        timed_out: true,
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimeoutPolicy {
    /// Fallback budget when a tool declares none (`None` = no timeout).
    pub default_timeout_ms: Option<u64>,
}

impl TimeoutPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the effective budget for one tool: its declared value wins,
    /// then the fallback. Upstream skips wrapping when both are absent.
    pub fn timeout_of(&self, declared: Option<u64>) -> Option<u64> {
        declared.or(self.default_timeout_ms).filter(|ms| *ms > 0)
    }

    /// Decide whether an elapsed duration exceeded the budget.
    pub fn is_expired(&self, declared: Option<u64>, elapsed_ms: u64) -> bool {
        match self.timeout_of(declared) {
            Some(budget) => elapsed_ms >= budget,
            None => false,
        }
    }

    /// Build the timeout result when expired. The budget is resolved once so
    /// the reported value is exactly the one used for the expiry check
    /// (swarm review R19-TM02).
    pub fn check(&self, declared: Option<u64>, elapsed_ms: u64) -> Option<TimeoutResult> {
        let budget = self.timeout_of(declared)?;
        (elapsed_ms >= budget).then(|| tool_timeout_result(budget))
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<TimeoutPolicy> {
    let policy = TimeoutPolicy::new();
    ctx.provide("timeoutPolicy", policy.clone()).await?;
    Ok(policy)
}
