//! Reusable Rust port of `@deepseek-ai/dsh-tool-subagent`.
//!
//! Registers a model-facing `subagent` tool on `ctx.tools` that delegates a
//! prompt to a named `ctx.subagents` provider with a depth cap.
//! Source: `packages/subagent/tool-subagent/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::subagents::{SubagentRegistry, SubagentRequest};
use crate::{Context, Result};

#[derive(Debug, Clone)]
pub struct ToolSubagentConfig {
    pub provider: String,
    pub tool_name: String,
    pub max_depth: usize,
}

impl Default for ToolSubagentConfig {
    fn default() -> Self {
        Self {
            provider: "spawn".to_string(),
            tool_name: "subagent".to_string(),
            max_depth: 1,
        }
    }
}

fn handler(subagents: SubagentRegistry, config: ToolSubagentConfig) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let subagents = subagents.clone();
        let config = config.clone();
        Box::pin(async move {
            let prompt = args
                .get("prompt")
                .and_then(|value| value.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("prompt must be a string".to_string()))?;
            if prompt.trim().is_empty() {
                return Err(ToolError::InvalidArgs(
                    "prompt must not be empty".to_string(),
                ));
            }
            let depth = args
                .get("depth")
                .and_then(|value| value.as_u64())
                .unwrap_or(1) as usize;
            let result = subagents
                .delegate(
                    &config.provider,
                    SubagentRequest {
                        prompt: prompt.to_string(),
                        depth,
                    },
                    config.max_depth,
                )
                .await
                .map_err(|error| ToolError::Execution(error.to_string()))?;
            Ok(ToolOutput {
                content: serde_json::json!({
                    "text": result.text,
                    "provider": result.provider,
                }),
                rendered: vec![result.text],
            })
        })
    })
}

/// The `dsh-tool-subagent` plugin.
pub struct ToolSubagent {
    pub config: ToolSubagentConfig,
}

#[async_trait]
impl crate::Plugin for ToolSubagent {
    fn name(&self) -> &str {
        "tool-subagent"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "subagents"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let tools = ctx.require::<ToolRegistry>("tools").await?;
        let subagents = ctx.require::<SubagentRegistry>("subagents").await?;
        let config = self.config.clone();
        ctx.effect(move |scope| {
            let tools = tools.clone();
            let subagents = subagents.clone();
            let config = config.clone();
            async move {
                let disposer = tools.register(ToolDefinition::new(
                    config.tool_name.clone(),
                    "Delegate a task to a child agent.",
                    handler((*subagents).clone(), config),
                ));
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with the shared tools and subagents seams.
pub async fn mount(ctx: &Context, config: ToolSubagentConfig) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    if ctx.get::<SubagentRegistry>("subagents").is_none() {
        crate::core_services::provide_subagents(ctx).await?;
    }
    let _fiber = crate::Fiber::mount(ctx, Box::new(ToolSubagent { config })).await?;
    Ok((*tools).clone())
}
