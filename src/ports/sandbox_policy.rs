//! Reusable Rust port of `@deepseek-ai/dsh-sandbox-policy` (phase 1).
//!
//! Source: `packages/sandbox/sandbox-policy/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Phase 1 provides the policy service only; enforcement (runner chain) is
//! deferred. Starts `read-only` (fail-safe), unlike local PermissionPresets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::services::permission_presets::SandboxMode;

/// Why a sandbox escalation was denied (swarm review R2-FG03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationDenial {
    NotWider {
        current: SandboxMode,
        requested: SandboxMode,
    },
    NoJustification,
}

impl std::fmt::Display for EscalationDenial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotWider { current, requested } => write!(
                f,
                "SANDBOX_ESCALATION_DENIED: not wider than current {current:?} (requested {requested:?})"
            ),
            Self::NoJustification => {
                f.write_str("SANDBOX_ESCALATION_DENIED: justification required")
            }
        }
    }
}

/// Rank used for strictly-wider checks. Kept private so `SandboxMode`'s public
/// comparison contract is unchanged (no `Ord` derive).
fn rank(mode: SandboxMode) -> u8 {
    match mode {
        SandboxMode::ReadOnly => 0,
        SandboxMode::WorkspaceWrite => 1,
        SandboxMode::DangerFullAccess => 2,
    }
}

/// Validate a `sandbox_permissions` + `justification` escalation pair. Pure:
/// returns the requested mode on success with no side effects (swarm review
/// R2-FG03).
pub fn escalate(
    current: SandboxMode,
    requested: SandboxMode,
    justification: &str,
) -> std::result::Result<SandboxMode, EscalationDenial> {
    if justification.trim().is_empty() {
        return Err(EscalationDenial::NoJustification);
    }
    if rank(requested) <= rank(current) {
        return Err(EscalationDenial::NotWider { current, requested });
    }
    Ok(requested)
}
use crate::services::sessions::{Session, SessionEvent, SessionProjectionRegistry};
use crate::{Context, Disposer, Error, Result};

pub const PROJECTION_KEY: &str = "sandboxMode";
pub const SANDBOX_INVALID_ROOT: &str = "SANDBOX_INVALID_ROOT";

/// Configuration for the sandbox policy.
#[derive(Debug, Clone, PartialEq)]
pub struct SandboxPolicyConfig {
    pub mode: SandboxMode,
    pub workspace_root: PathBuf,
    pub additional_writable_roots: Vec<PathBuf>,
}

impl Default for SandboxPolicyConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::ReadOnly,
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            additional_writable_roots: Vec::new(),
        }
    }
}

/// A resolved policy for one session and optional explicit override.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPolicy {
    pub mode: SandboxMode,
    pub workspace_root: PathBuf,
    pub additional_writable_roots: Vec<PathBuf>,
    pub session_id: Option<String>,
}

/// The sandbox policy service.
#[derive(Clone)]
pub struct SandboxPolicyService {
    config: SandboxPolicyConfig,
    projections: SessionProjectionRegistry,
    writable_roots: Arc<std::sync::RwLock<HashSet<PathBuf>>>,
}

fn parse_mode(raw: &str) -> Option<SandboxMode> {
    match raw {
        "read-only" => Some(SandboxMode::ReadOnly),
        "workspace-write" => Some(SandboxMode::WorkspaceWrite),
        "danger-full-access" => Some(SandboxMode::DangerFullAccess),
        _ => None,
    }
}

/// Projection fold: last `sandbox/mode` event wins; other events are ignored.
pub fn sandbox_mode_fold(state: serde_json::Value, event: &SessionEvent) -> serde_json::Value {
    if event.event_type == "sandbox/mode"
        && let Some(raw) = event.data.get("mode").and_then(|value| value.as_str())
        && let Some(mode) = parse_mode(raw)
    {
        return serde_json::json!(mode.as_str());
    }
    state
}

/// Validate an absolute, NUL-free path.
pub fn is_valid_path(path: &Path) -> bool {
    path.is_absolute() && !path.to_string_lossy().contains('\0')
}

impl SandboxPolicyService {
    /// Mode override recorded on the session (if any).
    pub fn override_of(&self, session: &Session) -> Option<SandboxMode> {
        self.projections
            .view(PROJECTION_KEY, session)
            .and_then(|value| value.as_str().and_then(parse_mode))
    }

    /// Resolve with precedence: explicit > session override > config default.
    pub fn resolve(
        &self,
        session: Option<&Session>,
        explicit: Option<SandboxMode>,
    ) -> ResolvedPolicy {
        let session_mode = session.and_then(|session| self.override_of(session));
        let mode = explicit.or(session_mode).unwrap_or(self.config.mode);
        let workspace_root = session
            .and_then(|session| session.cwd())
            .map(PathBuf::from)
            .filter(|path| is_valid_path(path))
            .unwrap_or_else(|| self.config.workspace_root.clone());
        let mut roots = self.config.additional_writable_roots.clone();
        let registered = self
            .writable_roots
            .read()
            .expect("writable roots poisoned")
            .clone();
        for root in registered {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        ResolvedPolicy {
            mode,
            workspace_root,
            additional_writable_roots: roots,
            session_id: session.map(|session| session.id().to_string()),
        }
    }

    /// Register an additional writable root for the service lifetime.
    pub fn register_writable_root(&self, root: PathBuf) -> std::result::Result<Disposer, Error> {
        if !is_valid_path(&root) {
            return Err(Error::Other(format!(
                "{SANDBOX_INVALID_ROOT}: {}",
                root.display()
            )));
        }
        self.writable_roots
            .write()
            .expect("writable roots poisoned")
            .insert(root.clone());
        let roots = self.writable_roots.clone();
        Ok(Disposer::new(move || {
            roots
                .write()
                .expect("writable roots poisoned")
                .remove(&root);
            async { Ok(()) }
        }))
    }
}

/// Tool names that mutate the filesystem or execute commands. A `ReadOnly`
/// policy blocks these at the pre-execute point.
pub const WRITE_TOOLS: [&str; 6] = [
    "bash",
    "fs",
    "str_replace_editor",
    "apply_patch",
    "edit",
    "write",
];

/// Whether a tool name mutates state (fail-safe: unknown names are read-only).
pub fn is_write_tool(name: &str) -> bool {
    WRITE_TOOLS.contains(&name)
}

/// Build a pre-execute middleware that enforces a resolved sandbox policy.
/// `ReadOnly` blocks write tools; other modes delegate. Opt-in: register via
/// `ToolPipeline::use_middleware` (phase 1, no runner-level confinement).
pub fn sandbox_policy_middleware(
    resolved: ResolvedPolicy,
) -> crate::services::tools::ToolMiddleware {
    std::sync::Arc::new(move |_args, _exec, next| {
        let blocked = resolved.mode == SandboxMode::ReadOnly;
        Box::pin(async move {
            if blocked {
                return Err(crate::services::tools::ToolError::Execution(
                    "blocked by sandbox policy: read-only mode".to_string(),
                ));
            }
            next(_args, _exec).await
        })
    })
}

/// Render the model-facing policy context (upstream wording).
pub fn render_policy_context(policy: &ResolvedPolicy) -> String {
    match policy.mode {
        SandboxMode::ReadOnly => {
            "## Sandbox\n- Mode: read-only\n- Only read operations are permitted.".to_string()
        }
        SandboxMode::WorkspaceWrite => format!(
            "## Sandbox\n- Mode: workspace-write\n- Writable roots: {}",
            policy
                .additional_writable_roots
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SandboxMode::DangerFullAccess => {
            "## Sandbox\n- Mode: danger-full-access\n- All operations are permitted.".to_string()
        }
    }
}

/// A sandbox-policy plugin injecting `sessionProjections`.
pub struct SandboxPolicyPlugin {
    pub config: SandboxPolicyConfig,
}

#[async_trait::async_trait]
impl crate::Plugin for SandboxPolicyPlugin {
    fn name(&self) -> &str {
        "sandbox-policy"
    }

    fn inject(&self) -> &[&str] {
        &["sessionProjections"]
    }

    async fn apply(&self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

/// Mount the policy service, registering its projection fold as an effect.
pub async fn mount(ctx: &Context, config: SandboxPolicyConfig) -> Result<SandboxPolicyService> {
    let projections = ctx
        .require::<SessionProjectionRegistry>("sessionProjections")
        .await?;
    if projections.has(PROJECTION_KEY) {
        return Err(Error::Other(format!(
            "sandbox-policy: projection `{PROJECTION_KEY}` already registered"
        )));
    }
    let disposer = projections.register(PROJECTION_KEY, Arc::new(sandbox_mode_fold));
    let service = SandboxPolicyService {
        config,
        projections: (*projections).clone(),
        writable_roots: Arc::new(std::sync::RwLock::new(HashSet::new())),
    };
    ctx.provide("sandboxPolicy", service.clone()).await?;
    let fiber = crate::Fiber::mount_retained(
        ctx,
        Box::new(SandboxPolicyPlugin {
            config: service.config.clone(),
        }),
    )
    .await?;
    ctx.effect(move |scope| {
        let disposer = disposer.clone();
        let fiber = fiber.clone();
        async move {
            scope.on_dispose(move || async move {
                disposer.dispose().await?;
                fiber.unload().await?;
                Ok(())
            });
            Ok(())
        }
    })
    .await?;
    Ok(service)
}
