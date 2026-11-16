//! `ctx.approval`: one-shot permission decisions over a waterfall.
//!
//! Upstream: `packages/interaction/user-approval/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Answerers are registered as listeners; the first non-`None` decision wins.
//! With no answerer, requests fail closed (matching upstream's default).

use std::sync::{Arc, RwLock};

use crate::context::Disposer;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequest {
    pub action: String,
    pub description: String,
}

impl ApprovalRequest {
    pub fn new(action: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            description: description.into(),
        }
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

#[derive(Default)]
struct ApprovalState {
    answerers: Vec<(u64, String, Answerer)>,
    next_id: u64,
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

    /// Ask the answerers to decide. Returns `Unavailable` when none answers,
    /// so callers fail closed by default.
    pub fn request(
        &self,
        req: &ApprovalRequest,
    ) -> std::result::Result<ApprovalOutcome, ApprovalError> {
        let guard = self.state.read().expect("approval state poisoned");
        for (_, name, answerer) in guard.answerers.iter().rev() {
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
