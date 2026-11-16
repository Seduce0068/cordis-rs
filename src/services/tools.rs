//! Core `ctx.tools` service: a named tool registry with an execution pipeline.
//!
//! Upstream keeps tool registration and the shared `tools/*` pipeline in one
//! service. Plugins register capabilities; the agent loop selects and calls
//! them. The pipeline is intentionally generic here: observers can wrap a
//! call (around), and post-execute observers can transform or spill results.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::context::Disposer;
use crate::events::BoxFuture;

pub type ToolHandler = Arc<
    dyn Fn(
            ToolArgs,
            ToolExecContext,
        ) -> BoxFuture<'static, std::result::Result<ToolOutput, ToolError>>
        + Send
        + Sync,
>;

pub type ToolArgs = serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecContext {
    pub agent: Option<String>,
    pub session: Option<String>,
    pub signal_cancelled: bool,
}

impl ToolExecContext {
    pub fn agent(name: impl Into<String>) -> Self {
        Self {
            agent: Some(name.into()),
            session: None,
            signal_cancelled: false,
        }
    }

    pub fn anonymous() -> Self {
        Self {
            agent: None,
            session: None,
            signal_cancelled: false,
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
}

/// Shared execution pipeline. Registered hooks are reversible effects.
#[derive(Default, Clone)]
pub struct ToolPipeline {
    state: Arc<RwLock<PipelineState>>,
}

impl ToolPipeline {
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

        let registry = registry.clone();
        let tool_name = name.to_string();
        let mut next: ToolNext = Arc::new(move |args, exec| {
            let registry = registry.clone();
            let tool_name = tool_name.clone();
            Box::pin(async move { registry.invoke(&tool_name, args, exec).await })
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

        let mut output = next(args, exec).await?;
        for observer in post {
            output = observer(output).await?;
        }
        Ok(output)
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
    futures::executor::block_on(pipeline.run(registry, name, args, exec))
}
