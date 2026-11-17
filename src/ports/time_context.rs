//! Reusable Rust port of `@deepseek-ai/dsh-time-context`.
//!
//! Source: `packages/context/time-context/src/index.ts` (+ timestamp.ts) in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers a `timeContext` session projection and an `agent/pre-step`
//! listener that injects a timestamp line when the refresh interval elapsed.

use std::sync::Arc;

use crate::core_services::{ChatMessage, SessionProjectionRegistry};
use crate::services::time_context::{TimeContext, format_sample, validate_refresh_interval};
use crate::{Context, Error, Result};

pub const DEFAULT_REFRESH_INTERVAL_MS: u64 = 600_000;
pub const PROJECTION_KEY: &str = "timeContext";

/// Config for the plugin. `timeZone` is informational in this PoC: rendering
/// uses an explicit RFC 3339 offset supplied by the host.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeContextPluginConfig {
    pub refresh_interval_ms: u64,
    pub time_zone: Option<String>,
}

impl Default for TimeContextPluginConfig {
    fn default() -> Self {
        Self {
            refresh_interval_ms: DEFAULT_REFRESH_INTERVAL_MS,
            time_zone: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreStepArgs {
    pub turn: u64,
    pub step: u64,
    pub timestamp: String,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreStepDecision {
    pub messages: Vec<ChatMessage>,
}

/// Build the injected message for a step, honoring the refresh interval.
pub fn sample_message(
    context: &mut TimeContext,
    args: &PreStepArgs,
) -> std::result::Result<Option<ChatMessage>, Error> {
    if !context.should_inject(args.now_ms) {
        return Ok(None);
    }
    let text = format_sample(args.turn, args.step, &args.timestamp)?;
    context.mark_injected(args.now_ms);
    Ok(Some(ChatMessage::user(text)))
}

/// The `dsh-time-context` plugin.
pub struct TimeContextPlugin {
    pub config: TimeContextPluginConfig,
}

#[async_trait::async_trait]
impl crate::Plugin for TimeContextPlugin {
    fn name(&self) -> &str {
        "time-context"
    }

    fn inject(&self) -> &[&str] {
        &["agents", "sessionProjections"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        validate_refresh_interval(self.config.refresh_interval_ms)?;
        let projections = ctx
            .require::<SessionProjectionRegistry>("sessionProjections")
            .await?;
        let _ = ctx
            .require::<crate::services::agents::AgentRegistry>("agents")
            .await?;

        let disposer = projections.register(
            PROJECTION_KEY,
            Arc::new(
                |state: serde_json::Value, event: &crate::services::sessions::SessionEvent| {
                    let mut value = if state.is_null() {
                        serde_json::json!({ "turns": 0, "messages": 0 })
                    } else {
                        state
                    };
                    match event.event_type.as_str() {
                        "turn/start" => {
                            let turns =
                                value.get("turns").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
                            value["turns"] = serde_json::json!(turns);
                        }
                        "user/message" | "assistant/message" => {
                            let messages =
                                value.get("messages").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
                            value["messages"] = serde_json::json!(messages);
                        }
                        _ => {}
                    }
                    value
                },
            ),
        );
        // Keep the disposer alive for the fiber lifetime via an effect.
        let ctx_for_effect = ctx.clone();
        let _ = ctx
            .effect(move |scope| {
                let d = disposer.clone();
                let _c = ctx_for_effect.clone();
                async move {
                    scope.on_dispose(move || async move {
                        let _ = d.dispose().await;
                        Ok(())
                    });
                    Ok(())
                }
            })
            .await;
        Ok(())
    }
}

pub async fn mount(ctx: &Context, config: TimeContextPluginConfig) -> Result<()> {
    let _fiber = crate::Fiber::mount(ctx, Box::new(TimeContextPlugin { config })).await?;
    Ok(())
}
