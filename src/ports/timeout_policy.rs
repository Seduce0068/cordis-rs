//! Reusable Rust port of `@deepseek-ai/dsh-timeout-policy`.
//!
//! Source: `packages/guard/timeout-policy/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Wraps tool execution with a declared `timeoutMs` and converts only this
//! policy's own timer expiry into a structured TOOL_TIMEOUT result.

use std::sync::Arc;
use std::time::Duration;

use crate::core_services::{ToolArgs, ToolError, ToolExecContext, ToolOutput};
use crate::events::BoxFuture;
use crate::services::timeout_policy::{TimeoutPolicy, TimeoutResult, tool_timeout_result};
use crate::{Context, Result};

pub const POLICY_NAME: &str = "timeout-policy";
pub const TOOL_TIMEOUT_CODE: &str = "TOOL_TIMEOUT";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimeoutPolicyPluginConfig {
    /// Fallback timeout when a tool declares none. `None` means pass through.
    pub default_timeout_ms: Option<u64>,
}

/// Convert the structured timeout result into a model-facing tool output.
pub fn timeout_output(result: &TimeoutResult) -> ToolOutput {
    ToolOutput {
        content: serde_json::json!({
            "code": TOOL_TIMEOUT_CODE,
            "message": result.message,
            "timed_out": result.timed_out,
        }),
        rendered: vec![result.message.clone()],
    }
}

/// Pure helper for callers that only need a decision from measured elapsed
/// time. `timeout_of` is the single source of truth for the effective budget.
pub fn evaluate(
    policy: &TimeoutPolicy,
    declared: Option<u64>,
    elapsed_ms: u64,
) -> Option<TimeoutResult> {
    if policy.check(declared, elapsed_ms).is_some() {
        return policy.timeout_of(declared).map(tool_timeout_result);
    }
    None
}

/// The `dsh-timeout-policy` plugin. Registers a tool middleware that applies
/// the declared (or configured) budget using `tokio::time::timeout`.
pub struct TimeoutPolicyPlugin {
    pub config: TimeoutPolicyPluginConfig,
}

fn declared_timeout(args: &ToolArgs) -> Option<u64> {
    args.get("timeout_ms").and_then(|value| value.as_u64())
}

#[async_trait::async_trait]
impl crate::Plugin for TimeoutPolicyPlugin {
    fn name(&self) -> &str {
        POLICY_NAME
    }

    fn inject(&self) -> &[&str] {
        &["tools"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        use crate::services::tools::ToolPipeline;
        let pipeline = ctx.require::<ToolPipeline>("toolPipeline").await?;
        let mut policy = TimeoutPolicy::new();
        policy.default_timeout_ms = self.config.default_timeout_ms;

        // Only our own `tokio::time::timeout` expiry is classified as a
        // timeout. Tool errors pass through and external cancellation is
        // already short-circuited before middleware by the pipeline.
        let hook = Arc::new(
            move |args: ToolArgs, exec: ToolExecContext, next: crate::core_services::ToolNext| {
                let policy = policy.clone();
                Box::pin(async move {
                    let Some(budget) = policy.timeout_of(declared_timeout(&args)) else {
                        return next(args, exec).await;
                    };
                    match tokio::time::timeout(Duration::from_millis(budget), next(args, exec))
                        .await
                    {
                        Ok(result) => result,
                        Err(_elapsed) => Ok(timeout_output(&tool_timeout_result(budget))),
                    }
                }) as BoxFuture<'static, std::result::Result<ToolOutput, ToolError>>
            },
        );

        ctx.effect(move |scope| {
            let pipeline = pipeline.clone();
            let hook = hook.clone();
            async move {
                let disposer = pipeline.use_middleware(hook);
                scope.on_dispose(move || async move {
                    let _ = disposer.dispose().await;
                    Ok(())
                });
                Ok(())
            }
        })
        .await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context, config: TimeoutPolicyPluginConfig) -> Result<()> {
    let _fiber = crate::Fiber::mount(ctx, Box::new(TimeoutPolicyPlugin { config })).await?;
    Ok(())
}
