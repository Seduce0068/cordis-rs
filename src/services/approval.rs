//! `ctx.approval`: one-shot permission decisions over a waterfall.
//!
//! Upstream: `packages/interaction/user-approval/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Answerers are registered as listeners; the first non-`None` decision wins.
//! With no answerer, requests fail closed (matching upstream's default).

use std::sync::{Arc, RwLock};

use crate::primitives::Disposer;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequest {
    pub action: String,
    pub description: String,
    /// Optional untrusted command text for the answerer to inspect (ADR-012).
    details: Option<String>,
}

impl ApprovalRequest {
    pub fn new(action: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            description: description.into(),
            details: None,
        }
    }

    /// Attach details (e.g. a model-supplied bash command). Builder-style so
    /// existing `ApprovalRequest::new` callers stay unchanged.
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn details(&self) -> Option<&str> {
        self.details.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalOutcome {
    pub decision: ApprovalDecision,
    pub answerer: Option<String>,
}

pub type Answerer = Arc<dyn Fn(&ApprovalRequest) -> Option<ApprovalDecision> + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalError {
    Unavailable,
    Cancelled,
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "approval unavailable (fail-closed)"),
            Self::Cancelled => write!(f, "approval request cancelled"),
        }
    }
}

/// Deployment policy for approval prompts, mirroring the `approval/policy`
/// knob set by `ctx.permissionPresets`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalMode {
    /// Ask registered answerers; fail closed if none answers.
    #[default]
    Ask,
    /// Prompts are disabled: every request is denied.
    Never,
}

#[derive(Default)]
struct ApprovalState {
    answerers: Vec<(u64, String, Answerer)>,
    next_id: u64,
    policy: ApprovalMode,
}

/// The shared approval seam.
#[derive(Default, Clone)]
pub struct ApprovalService {
    state: Arc<RwLock<ApprovalState>>,
}

impl ApprovalService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an answerer. Higher-priority (larger id) answerers are asked
    /// first, mirroring the upstream waterfall's most-recent-listener order.
    pub fn add_answerer(&self, name: impl Into<String>, answerer: Answerer) -> Disposer {
        let state = self.state.clone();
        let mut guard = state.write().expect("approval state poisoned");
        let id = guard.next_id;
        guard.next_id += 1;
        guard.answerers.push((id, name.into(), answerer));
        drop(guard);

        Disposer::new(move || {
            let state = state.clone();
            async move {
                state
                    .write()
                    .expect("approval state poisoned")
                    .answerers
                    .retain(|(current, _, _)| *current != id);
                Ok(())
            }
        })
    }

    /// Set the approval policy (usually driven by permission presets).
    pub fn set_policy(&self, policy: ApprovalMode) {
        self.state.write().expect("approval state poisoned").policy = policy;
    }

    pub fn policy(&self) -> ApprovalMode {
        self.state.read().expect("approval state poisoned").policy
    }

    /// Ask the answerers to decide. Returns `Unavailable` when none answers,
    /// so callers fail closed by default. Under `Never` every request is denied
    /// without consulting an answerer.
    pub fn request(
        &self,
        req: &ApprovalRequest,
    ) -> std::result::Result<ApprovalOutcome, ApprovalError> {
        // Answerers are user callbacks (e.g. a UI prompt) and may re-enter this
        // service to call `add_answerer` or `set_policy`. Snapshot everything
        // needed under the read guard, then drop the guard before invoking any
        // callback: holding it would deadlock against those write-lock calls.
        let (policy, answerers) = {
            let guard = self.state.read().expect("approval state poisoned");
            (guard.policy, guard.answerers.clone())
        };

        if policy == ApprovalMode::Never {
            return Ok(ApprovalOutcome {
                decision: ApprovalDecision::Deny,
                answerer: Some("policy:never".to_string()),
            });
        }
        for (_, name, answerer) in answerers.iter().rev() {
            if let Some(decision) = answerer(req) {
                return Ok(ApprovalOutcome {
                    decision,
                    answerer: Some(name.clone()),
                });
            }
        }
        Err(ApprovalError::Unavailable)
    }

    pub fn answerer_names(&self) -> Vec<String> {
        self.state
            .read()
            .expect("approval state poisoned")
            .answerers
            .iter()
            .map(|(_, name, _)| name.clone())
            .collect()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<ApprovalService> {
    let service = ApprovalService::new();
    ctx.provide("approval", service.clone()).await?;
    Ok(service)
}

/// Helper: convert an unavailable approval into a harness Error for tools.
pub fn require_approval(
    service: &ApprovalService,
    req: &ApprovalRequest,
) -> Result<ApprovalOutcome> {
    service
        .request(req)
        .map_err(|error| Error::Other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// An answerer that registers another answerer and flips the policy takes
    /// write locks. It must not deadlock against `request()`'s read guard.
    /// The request runs on a worker thread with a receive timeout so a
    /// regression fails the test instead of hanging the suite.
    #[test]
    fn answerer_can_reenter_add_answerer_and_set_policy() {
        let service = ApprovalService::new();
        let reentrant = service.clone();
        let _outer = service.add_answerer(
            "reentrant",
            Arc::new(move |req: &ApprovalRequest| {
                assert_eq!(req.action, "escalate");
                // Both calls need the write lock; while `request()` held its
                // read guard this re-entrant path would deadlock.
                let _nested = reentrant.add_answerer("nested", Arc::new(|_| None));
                reentrant.set_policy(ApprovalMode::Never);
                Some(ApprovalDecision::Allow)
            }),
        );

        let req = ApprovalRequest::new("escalate", "reentrancy check");
        let (tx, rx) = mpsc::channel();
        let worker = service.clone();
        let handle = thread::spawn(move || {
            let result = worker.request(&req);
            let _ = tx.send(result);
        });

        let outcome = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request() deadlocked: answerer re-entered the service");
        let outcome = outcome.expect("the reentrant answerer should have decided");
        assert_eq!(outcome.decision, ApprovalDecision::Allow);
        assert_eq!(outcome.answerer.as_deref(), Some("reentrant"));
        handle.join().expect("worker thread panicked");

        // The re-entrant mutations took effect, and the nested answerer added
        // mid-request was not part of the current request's snapshot.
        assert_eq!(service.policy(), ApprovalMode::Never);
        // `answerer_names()` reports insertion order (the request waterfall
        // asks in reverse), so the outer answerer comes first here.
        assert_eq!(service.answerer_names(), ["reentrant", "nested"]);
    }

    /// A `Never` policy is snapshotted too: it must still deny without ever
    /// consulting an answerer, even when one is registered.
    #[test]
    fn policy_never_still_denies_after_snapshot() {
        let service = ApprovalService::new();
        let _answerer = service.add_answerer(
            "never-called",
            Arc::new(|_| panic!("answerer must not be consulted under ApprovalMode::Never")),
        );
        service.set_policy(ApprovalMode::Never);

        let outcome = service
            .request(&ApprovalRequest::new("anything", "policy check"))
            .expect("Never produces a denial, not an error");
        assert_eq!(outcome.decision, ApprovalDecision::Deny);
        assert_eq!(outcome.answerer.as_deref(), Some("policy:never"));
    }
}
