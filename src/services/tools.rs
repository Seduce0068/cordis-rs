//! Core `ctx.tools` service: a named tool registry with an execution pipeline.
//!
//! Upstream keeps tool registration and the shared `tools/*` pipeline in one
//! service. Plugins register capabilities; the agent loop selects and calls
//! them. The pipeline is intentionally generic here: observers can wrap a
//! call (around), and post-execute observers can transform or spill results.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::events::BoxFuture;
use crate::primitives::Disposer;

pub type ToolHandler = Arc<
    dyn Fn(
            ToolArgs,
            ToolExecContext,
        ) -> BoxFuture<'static, std::result::Result<ToolOutput, ToolError>>
        + Send
        + Sync,
>;

pub type ToolArgs = serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolExecContext {
    pub agent: Option<String>,
    pub session: Option<String>,
    pub signal_cancelled: bool,
    /// Optional shared cancellation flag. A host may set it while the tool
    /// body is running; the pipeline observes it after the body completes and
    /// reclassifies a successful result as cancelled (upstream
    /// `toolAbortedResult(prior)` semantics). The body is never cut short.
    pub cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl PartialEq for ToolExecContext {
    fn eq(&self, other: &Self) -> bool {
        self.agent == other.agent
            && self.session == other.session
            && self.signal_cancelled == other.signal_cancelled
            && match (&self.cancel_flag, &other.cancel_flag) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

impl ToolExecContext {
    /// True when cancellation was signalled before dispatch or while running.
    pub fn is_cancelled(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.signal_cancelled
            || self
                .cancel_flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    pub fn agent(name: impl Into<String>) -> Self {
        Self {
            agent: Some(name.into()),
            session: None,
            signal_cancelled: false,
            cancel_flag: None,
        }
    }

    pub fn anonymous() -> Self {
        Self {
            agent: None,
            session: None,
            signal_cancelled: false,
            cancel_flag: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub content: serde_json::Value,
    pub rendered: Vec<String>,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            content: serde_json::Value::String(text.clone()),
            rendered: vec![text],
        }
    }

    /// Normalize a tool error into an observable output for post-execute
    /// observers. All codes start with `TOOL_` so `is_error()` succeeds.
    pub fn from_error(error: &ToolError) -> Self {
        let code = match error {
            ToolError::UnknownTool(_) => "TOOL_UNKNOWN",
            ToolError::InvalidArgs(_) => "TOOL_INVALID_ARGS",
            ToolError::Cancelled(_) => "TOOL_CANCELLED",
            ToolError::Execution(_) => "TOOL_EXECUTION_ERROR",
        };
        Self {
            content: serde_json::json!({ "code": code, "message": error.to_string() }),
            rendered: vec![error.to_string()],
        }
    }

    /// Model-facing error flag (e.g. a structured TOOL_TIMEOUT result).
    /// Implemented as a method so existing struct literals stay valid. Note:
    /// a tool that returns `"code": "TOOL_TIMEOUT"` as ordinary data would
    /// also be flagged; hosts needing strict attribution should use a typed
    /// field in a future breaking release.
    pub fn is_error(&self) -> bool {
        self.content
            .get("code")
            .and_then(|value| value.as_str())
            .is_some_and(|code| code.starts_with("TOOL_") || code.ends_with("_ERROR"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    UnknownTool(String),
    InvalidArgs(String),
    Execution(String),
    Cancelled(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTool(name) => write!(f, "unknown tool: {name}"),
            Self::InvalidArgs(message) => write!(f, "invalid arguments: {message}"),
            Self::Execution(message) => write!(f, "tool execution failed: {message}"),
            Self::Cancelled(message) => write!(f, "tool call cancelled: {message}"),
        }
    }
}

impl std::error::Error for ToolError {}

#[derive(Clone)]
pub struct ToolDefinition {
    name: String,
    description: String,
    handler: ToolHandler,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        handler: ToolHandler,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            handler,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

#[derive(Default)]
struct RegistryState {
    next_id: u64,
    tools: HashMap<String, (u64, ToolDefinition)>,
}

/// The shared tool registry. Cloneable so several plugins can hold a handle.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl ToolRegistry {
    pub fn register(&self, definition: ToolDefinition) -> Disposer {
        let state = self.state.clone();
        let mut guard = state.write().expect("tool registry poisoned");
        let id = guard.next_id;
        guard.next_id += 1;
        guard
            .tools
            .insert(definition.name().to_string(), (id, definition.clone()));
        drop(guard);

        Disposer::new(move || {
            let state = state.clone();
            let name = definition.name().to_string();
            async move {
                let mut guard = state.write().expect("tool registry poisoned");
                if guard.tools.get(&name).map(|(id, _)| *id) == Some(id) {
                    guard.tools.remove(&name);
                }
                Ok(())
            }
        })
    }

    pub fn names(&self) -> Vec<String> {
        let guard = self.state.read().expect("tool registry poisoned");
        let mut names: Vec<_> = guard.tools.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn get(&self, name: &str) -> Option<ToolDefinition> {
        self.state
            .read()
            .expect("tool registry poisoned")
            .tools
            .get(name)
            .map(|(_, definition)| definition.clone())
    }

    pub async fn invoke(
        &self,
        name: &str,
        args: ToolArgs,
        exec: ToolExecContext,
    ) -> std::result::Result<ToolOutput, ToolError> {
        let definition = self
            .get(name)
            .ok_or_else(|| ToolError::UnknownTool(name.to_string()))?;
        (definition.handler)(args, exec).await
    }
}

/// A hook that wraps a tool call, mirroring upstream `tools/call` waterfall.
pub type ToolMiddleware = Arc<
    dyn Fn(
            ToolArgs,
            ToolExecContext,
            ToolNext,
        ) -> BoxFuture<'static, std::result::Result<ToolOutput, ToolError>>
        + Send
        + Sync,
>;

pub type ToolNext = Arc<
    dyn Fn(
            ToolArgs,
            ToolExecContext,
        ) -> BoxFuture<'static, std::result::Result<ToolOutput, ToolError>>
        + Send
        + Sync,
>;

/// Convert a hook deny effect into a middleware that blocks tool execution.
/// `Proceed` / `Inject` delegate to the inner tool (context injection is a
/// phase-2 concern and is intentionally dropped). Register via
/// `ToolPipeline::use_middleware` to opt into hook enforcement.
pub fn hook_deny_middleware(effect: crate::plugin_ports::HookEffect) -> ToolMiddleware {
    Arc::new(move |_args, _exec, next| {
        let effect = effect.clone();
        Box::pin(async move {
            match effect {
                crate::plugin_ports::HookEffect::Reject(reason)
                | crate::plugin_ports::HookEffect::Block(reason) => {
                    Err(ToolError::Execution(format!("blocked by hook: {reason}")))
                }
                crate::plugin_ports::HookEffect::Ask(_)
                | crate::plugin_ports::HookEffect::Inject(_)
                | crate::plugin_ports::HookEffect::Proceed => next(_args, _exec).await,
            }
        })
    })
}

/// Post-execute observer, mirroring upstream `tools/post-execute`.
pub type ToolPostExecute = Arc<
    dyn Fn(ToolOutput) -> BoxFuture<'static, std::result::Result<ToolOutput, ToolError>>
        + Send
        + Sync,
>;

#[derive(Default)]
struct PipelineState {
    middleware: Vec<ToolMiddleware>,
    post: Vec<ToolPostExecute>,
    approval: Option<crate::services::approval::ApprovalService>,
    /// Opt-in fail-closed gate: deny execution when no ApprovalService is
    /// installed (default false preserves existing fail-open behavior).
    approval_required: bool,
    retry: Option<crate::services::retry::RetryPolicy>,
}

/// Shared execution pipeline. Registered hooks are reversible effects.
#[derive(Default, Clone)]
pub struct ToolPipeline {
    state: Arc<RwLock<PipelineState>>,
}

impl ToolPipeline {
    /// Install a fail-closed approval gate at the innermost execution point.
    /// With no answerer registered the request is denied (upstream default).
    pub fn with_approval(&self, approval: crate::services::approval::ApprovalService) {
        self.state.write().expect("tool pipeline poisoned").approval = Some(approval);
    }

    /// Opt into fail-closed behavior: when no `ApprovalService` is installed,
    /// tool execution is denied instead of running unapproved. The default is
    /// fail-open for backwards compatibility with existing hosts and tests.
    pub fn require_approval(&self) -> &Self {
        self.state
            .write()
            .expect("tool pipeline poisoned")
            .approval_required = true;
        self
    }

    /// Install the production control set: fail-closed approval, an optional
    /// hook-deny gate, and the sandbox gate. Middleware runs in registration
    /// order (`run()` wraps with `.rev()`), so the sandbox gate registered last
    /// runs after the hook gate but still before approval/retry (which live in
    /// the innermost `next`).
    ///
    /// `sandbox_policy` is a snapshot resolved at install time; re-install on
    /// session changes. Pass the actual `hook_effect` (from `map_outcome`) — a
    /// `Proceed` effect registers no deny middleware, since a no-op hook gate
    /// would be a misleading control.
    pub fn install_default_controls(
        &self,
        hook_effect: Option<crate::plugin_ports::HookEffect>,
        sandbox_policy: crate::plugin_ports::ResolvedPolicy,
    ) -> Vec<Disposer> {
        self.require_approval();
        let mut disposers = Vec::new();
        if let Some(effect) = hook_effect
            && !matches!(
                effect,
                crate::plugin_ports::HookEffect::Proceed | crate::plugin_ports::HookEffect::Ask(_)
            )
        {
            disposers.push(self.use_middleware(hook_deny_middleware(effect)));
        }
        disposers.push(
            self.use_middleware(crate::plugin_ports::sandbox_policy_middleware(
                sandbox_policy,
            )),
        );
        disposers
    }

    /// Retry tool executions on retryable HTTP-style errors.
    pub fn with_retry(&self, retry: crate::services::retry::RetryPolicy) {
        self.state.write().expect("tool pipeline poisoned").retry = Some(retry);
    }

    pub fn use_middleware(&self, hook: ToolMiddleware) -> Disposer {
        let state = self.state.clone();
        let hook_ptr = hook.clone();
        state
            .write()
            .expect("tool pipeline poisoned")
            .middleware
            .push(hook);
        Disposer::new(move || {
            let state = state.clone();
            async move {
                state
                    .write()
                    .expect("tool pipeline poisoned")
                    .middleware
                    .retain(|current| !Arc::ptr_eq(current, &hook_ptr));
                Ok(())
            }
        })
    }

    pub fn on_post_execute(&self, hook: ToolPostExecute) -> Disposer {
        let state = self.state.clone();
        let hook_ptr = hook.clone();
        state
            .write()
            .expect("tool pipeline poisoned")
            .post
            .push(hook);
        Disposer::new(move || {
            let state = state.clone();
            async move {
                state
                    .write()
                    .expect("tool pipeline poisoned")
                    .post
                    .retain(|current| !Arc::ptr_eq(current, &hook_ptr));
                Ok(())
            }
        })
    }

    pub async fn run(
        &self,
        registry: &ToolRegistry,
        name: &str,
        args: ToolArgs,
        exec: ToolExecContext,
    ) -> std::result::Result<ToolOutput, ToolError> {
        let hooks = self
            .state
            .read()
            .expect("tool pipeline poisoned")
            .middleware
            .clone();
        let post = self
            .state
            .read()
            .expect("tool pipeline poisoned")
            .post
            .clone();

        let (approval, retry, approval_required) = {
            let state = self.state.read().expect("tool pipeline poisoned");
            (
                state.approval.clone(),
                state.retry.clone(),
                state.approval_required,
            )
        };
        let registry = registry.clone();
        let tool_name = name.to_string();
        let mut next: ToolNext = Arc::new(move |args, exec| {
            let registry = registry.clone();
            let tool_name = tool_name.clone();
            let approval = approval.clone();
            let retry = retry.clone();
            Box::pin(async move {
                if approval.is_none() && approval_required {
                    return Err(ToolError::Execution(format!(
                        "tool {tool_name} blocked: approval unavailable"
                    )));
                }
                if let Some(approval) = approval {
                    use crate::services::approval::{ApprovalDecision, ApprovalRequest};
                    let mut request = ApprovalRequest::new(
                        format!("tool:{tool_name}"),
                        format!("Execute tool {tool_name} with the supplied arguments"),
                    );
                    // Surface the model-supplied command to the answerer so it
                    // can make a real allow/deny decision (ADR-012). Never use
                    // `args["command"]` (panics on a missing key).
                    if tool_name == "bash"
                        && let Some(command) = args.get("command").and_then(|v| v.as_str())
                    {
                        request = request.with_details(command.escape_debug().to_string());
                    }
                    match approval.request(&request) {
                        Ok(outcome) if outcome.decision == ApprovalDecision::Allow => {}
                        Ok(_) => {
                            return Err(ToolError::Execution(format!(
                                "tool {tool_name} denied by approval policy"
                            )));
                        }
                        Err(error) => {
                            return Err(ToolError::Execution(format!(
                                "tool {tool_name} blocked: {error}"
                            )));
                        }
                    }
                }
                let mut attempt = 0;
                loop {
                    match registry
                        .invoke(&tool_name, args.clone(), exec.clone())
                        .await
                    {
                        Ok(output) => return Ok(output),
                        Err(error) => {
                            // Do not re-dispatch after cancellation was
                            // signalled during a previous attempt (R4 review).
                            if exec.is_cancelled() {
                                return Err(error);
                            }
                            let decision = retry
                                .as_ref()
                                .map(|policy| {
                                    policy.should_retry_message(&error.to_string(), attempt)
                                })
                                .unwrap_or(crate::services::retry::RetryDecision::Stop);
                            if decision == crate::services::retry::RetryDecision::Stop {
                                return Err(error);
                            }
                            let delay = retry
                                .as_ref()
                                .map(|policy| policy.backoff_ms(attempt + 1, 0.5))
                                .unwrap_or(0);
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            attempt += 1;
                        }
                    }
                }
            })
        });
        for hook in hooks.into_iter().rev() {
            let hook = hook.clone();
            let inner = next.clone();
            next = Arc::new(move |args, exec| {
                let hook = hook.clone();
                let inner = inner.clone();
                let args_clone = args.clone();
                let exec_clone = exec.clone();
                Box::pin(async move { hook(args_clone, exec_clone, inner).await })
            });
        }

        // A cancelled execution must never reach the tool body.
        if exec.is_cancelled() {
            return Err(ToolError::Cancelled("execution was cancelled".to_string()));
        }
        let cancelled_during_run = exec.cancel_flag.clone();
        let mut execution = next(args, exec).await;
        // Upstream `toolAbortedResult(prior)`: let the body finish, then
        // reclassify a successful result when cancellation was signalled
        // mid-flight. Failures keep their original identity.
        if execution.is_ok()
            && cancelled_during_run
                .as_ref()
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
        {
            execution = Err(ToolError::Cancelled(
                "execution was cancelled during tool body".to_string(),
            ));
        }
        // Upstream runs post-execute observers even for thrown tools. Normalize
        // the failure into an observable output, run the observers, then
        // restore the original error (unless an observer itself fails).
        let original_error = execution.as_ref().err().cloned();
        let mut output = match execution {
            Ok(output) => output,
            Err(error) => ToolOutput::from_error(&error),
        };
        for observer in post {
            output = observer(output).await?;
        }
        match original_error {
            Some(error) => Err(error),
            None => Ok(output),
        }
    }
}

/// Mount the two services on a context.
pub async fn provide(ctx: &crate::Context) -> Result<(ToolRegistry, ToolPipeline)> {
    let registry = ToolRegistry::default();
    let pipeline = ToolPipeline::default();
    ctx.provide("tools", registry.clone()).await?;
    ctx.provide("toolPipeline", pipeline.clone()).await?;
    Ok((registry, pipeline))
}

/// Helper for host code that cannot await (e.g. a GPUI click handler).
pub fn invoke_blocking(
    registry: &ToolRegistry,
    pipeline: &ToolPipeline,
    name: &str,
    args: ToolArgs,
    exec: ToolExecContext,
) -> std::result::Result<ToolOutput, ToolError> {
    // A blocking call from inside a Tokio runtime would stall the only thread
    // of a current-thread runtime, so `tokio::time::sleep` in the retry path
    // could never wake (or panic without a reactor). Fail closed and direct
    // callers to the async API (swarm review R2-BV).
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(ToolError::Execution(
            "invoke_blocking must not be called from a Tokio runtime; use ToolPipeline::run(...).await"
                .to_string(),
        ));
    }
    futures::executor::block_on(pipeline.run(registry, name, args, exec))
}
