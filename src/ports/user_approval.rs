//! Pure Rust port of `@deepseek-ai/dsh-user-approval` normalization.
//!
//! Source: `packages/interaction/user-approval/src/` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Read-side fail-closed normalization for approval outcomes. The writable
//! `ApprovalService` is left untouched.

use crate::services::approval::{ApprovalDecision, ApprovalError, ApprovalOutcome};

/// Approved policies (upstream `APPROVAL_POLICIES`).
pub const APPROVAL_POLICIES: [&str; 2] = ["ask", "never"];

/// Normalized outcomes (upstream `OUTCOMES`).
pub const APPROVAL_OUTCOMES: [&str; 4] = ["allowed-once", "rejected", "cancelled", "unavailable"];

/// Model-facing sentences for each policy.
pub const NEVER_SENTENCE: &str = "Prompts are disabled; the request is rejected.";
pub const ASK_SENTENCE: &str = "Awaiting approval.";

/// Normalize an answerer result into a decision. Every error and an explicit
/// deny become `Deny` (fail-closed); only `Ok(Allow)` yields `Allow`.
pub fn normalize_approval_outcome(
    outcome: std::result::Result<ApprovalOutcome, ApprovalError>,
) -> ApprovalDecision {
    match outcome {
        Ok(outcome) if outcome.decision == ApprovalDecision::Allow => ApprovalDecision::Allow,
        _ => ApprovalDecision::Deny,
    }
}

/// Map a decision to its upstream outcome label.
pub fn outcome_label(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Allow => "allowed-once",
        ApprovalDecision::Deny => "rejected",
    }
}
