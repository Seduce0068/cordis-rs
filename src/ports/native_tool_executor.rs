//! Reusable Rust port of `@deepseek-ai/dsh-tool-executor-native`.
//!
//! Source: `packages/core/tool-executor-native/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Provides a small executor selector around the existing registry/pipeline.

use std::sync::Arc;

use crate::core_services::{ToolError, ToolExecContext, ToolOutput, ToolPipeline, ToolRegistry};
use crate::{Context, Result};

pub const SERVICE_NAME: &str = "nativeToolExecutor";
pub const DEFAULT_MAX_PARALLEL: usize = 10;

/// A model-facing tool schema (simplified projection of `projectToolSchema`).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// Simplified parameter schema. Local `ToolDefinition` has no introspectable
    /// argument schema, so this is `{"type":"object","properties":{}}` unless a
    /// host supplies one via a side table.
    pub parameters: serde_json::Value,
}

/// Single-slot executor holder (stand-in for upstream scoped selector).
/// Only one executor may be registered at a time.
#[derive(Clone, Default)]
pub struct ExecutorSelector {
    current: Arc<std::sync::Mutex<Option<NativeToolExecutor>>>,
}

impl ExecutorSelector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, executor: NativeToolExecutor) -> bool {
        let mut guard = self.current.lock().expect("executor selector poisoned");
        if guard.is_some() {
            return false;
        }
        *guard = Some(executor);
        true
    }

    pub fn current(&self) -> Option<NativeToolExecutor> {
        self.current
            .lock()
            .expect("executor selector poisoned")
            .clone()
    }

    pub fn clear(&self) {
        *self.current.lock().expect("executor selector poisoned") = None;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallSpec {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionOutcome {
    pub name: String,
    pub result: std::result::Result<ToolOutput, String>,
}

/// Native executor wrapping registry + pipeline with bounded parallelism.
#[derive(Clone)]
pub struct NativeToolExecutor {
    registry: ToolRegistry,
    pipeline: ToolPipeline,
    max_parallel: usize,
}

impl NativeToolExecutor {
    pub fn new(registry: ToolRegistry, pipeline: ToolPipeline) -> Self {
        Self {
            registry,
            pipeline,
            max_parallel: DEFAULT_MAX_PARALLEL,
        }
    }

    pub fn with_max_parallel(mut self, max_parallel: usize) -> Self {
        self.max_parallel = max_parallel.max(1);
        self
    }

    /// Project registered tools into model-facing minimal schemas.
    pub fn describe(&self) -> Vec<ToolSchema> {
        self.registry
            .names()
            .into_iter()
            .map(|name| {
                let description = self
                    .registry
                    .get(&name)
                    .map(|definition| definition.description().to_string())
                    .unwrap_or_default();
                ToolSchema {
                    name,
                    description,
                    parameters: serde_json::json!({ "type": "object", "properties": {} }),
                }
            })
            .collect()
    }

    /// Execute calls concurrently, bounded by `max_parallel`, preserving input
    /// order in the returned outcomes.
    pub async fn execute(
        &self,
        calls: Vec<ToolCallSpec>,
        exec: ToolExecContext,
    ) -> Vec<ExecutionOutcome> {
        let indexed = calls.into_iter().enumerate();
        let mut stream = futures::stream::iter(indexed.map(|(index, call)| {
            let registry = self.registry.clone();
            let pipeline = self.pipeline.clone();
            let exec = exec.clone();
            async move {
                let name = call.name.clone();
                let result = pipeline
                    .run(&registry, &call.name, call.args, exec)
                    .await
                    .map_err(|error: ToolError| error.to_string());
                (index, ExecutionOutcome { name, result })
            }
        }))
        .buffer_unordered(self.max_parallel);
        use futures::StreamExt;
        let mut indexed_outcomes = Vec::new();
        while let Some(item) = stream.next().await {
            indexed_outcomes.push(item);
        }
        // `buffer_unordered` completes out of order; restore model order.
        indexed_outcomes.sort_by_key(|(index, _)| *index);
        indexed_outcomes
            .into_iter()
            .map(|(_, outcome)| outcome)
            .collect()
    }
}

/// The `dsh-tool-executor-native` plugin (selection is a local service).
pub struct NativeToolExecutorPlugin;

#[async_trait::async_trait]
impl crate::Plugin for NativeToolExecutorPlugin {
    fn name(&self) -> &str {
        "tool-executor-native"
    }

    fn inject(&self) -> &[&str] {
        &["tools"]
    }

    async fn apply(&self, _ctx: &Context) -> Result<()> {
        // Registration is owned by `mount` on the root context: the fiber
        // child would otherwise create a second executor with a shadowed
        // config and register it twice in the same holder.
        Ok(())
    }
}

/// Mount the executor selector service on a context.
pub async fn mount(ctx: &Context, max_parallel: usize) -> Result<NativeToolExecutor> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let pipeline = ctx.get::<ToolPipeline>("toolPipeline");
    let pipeline = pipeline
        .as_deref()
        .cloned()
        .unwrap_or_else(ToolPipeline::default);
    let holder = ctx
        .get::<ExecutorSelector>("toolExecutor")
        .map(|arc| (*arc).clone())
        .unwrap_or_default();
    if ctx.get::<ExecutorSelector>("toolExecutor").is_none() {
        let _ = ctx.provide("toolExecutor", holder.clone()).await?;
    }
    let executor =
        NativeToolExecutor::new((*tools).clone(), pipeline).with_max_parallel(max_parallel);
    holder.register(executor.clone());
    ctx.provide(SERVICE_NAME, executor.clone()).await?;
    let fiber = crate::Fiber::mount_retained(ctx, Box::new(NativeToolExecutorPlugin)).await?;
    ctx.effect(move |scope| {
        let fiber = fiber.clone();
        async move {
            scope.on_dispose(move || async move {
                fiber.unload().await?;
                Ok(())
            });
            Ok(())
        }
    })
    .await?;
    Ok(executor)
}
