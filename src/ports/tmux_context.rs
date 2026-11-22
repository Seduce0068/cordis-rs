//! Reusable Rust port of `@deepseek-ai/dsh-tmux-context`.
//!
//! Source: `packages/context/tmux-context/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers a `tmuxContext` session projection and exposes pure step-1
//! sampling / change-detection helpers. The host drives the pre-step query;
//! this port does not register a listener itself.

use std::sync::Arc;

use crate::core_services::SessionProjectionRegistry;
use crate::services::time_context::validate_refresh_interval;
use crate::services::tmux_context::{TmuxContext, TmuxLocation, format_reading};
use crate::{Context, Result};

pub const TMUX_PROJECTION_KEY: &str = "tmuxContext";
pub const TMUX_DEFAULT_REFRESH_INTERVAL_MS: u64 = 600_000;

#[derive(Debug, Clone, PartialEq)]
pub struct TmuxContextPluginConfig {
    pub refresh_interval_ms: u64,
}

impl Default for TmuxContextPluginConfig {
    fn default() -> Self {
        Self {
            refresh_interval_ms: TMUX_DEFAULT_REFRESH_INTERVAL_MS,
        }
    }
}

/// Decide whether to re-inject at a step: tmux context is step-1 only and
/// must differ from the last injected state.
pub fn should_sample(helper: &TmuxContext, step: u64, current: &TmuxLocation) -> bool {
    if step != 1 {
        return false;
    }
    helper.should_inject(current)
}

/// Build the durable context line for a sampled tmux location.
pub fn sample_reading(turn: u64, step: u64, location: &TmuxLocation, timestamp: &str) -> String {
    format_reading(turn, step, location, timestamp)
}

/// The `dsh-tmux-context` plugin (projection wiring; host supplies the query).
pub struct TmuxContextPlugin {
    pub config: TmuxContextPluginConfig,
}

#[async_trait::async_trait]
impl crate::Plugin for TmuxContextPlugin {
    fn name(&self) -> &str {
        "tmux-context"
    }

    fn inject(&self) -> &[&str] {
        &["agents", "sessionProjections"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let projections = ctx
            .require::<SessionProjectionRegistry>("sessionProjections")
            .await?;
        let _ = ctx
            .require::<crate::services::agents::AgentRegistry>("agents")
            .await?;
        validate_refresh_interval(self.config.refresh_interval_ms)?;

        let disposer = projections.register(
            TMUX_PROJECTION_KEY,
            Arc::new(
                |state: serde_json::Value, event: &crate::services::sessions::SessionEvent| {
                    let mut value = if state.is_null() {
                        serde_json::json!({ "samples": 0 })
                    } else {
                        state
                    };
                    if event.event_type == "user/message"
                        && event
                            .data
                            .get("source")
                            .and_then(|source| source.get("kind"))
                            .and_then(|kind| kind.as_str())
                            == Some("tmux-context")
                    {
                        let samples = value
                            .get("samples")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0)
                            .saturating_add(1);
                        value["samples"] = serde_json::json!(samples);
                    }
                    value
                },
            ),
        );
        let ctx_for_effect = ctx.clone();
        ctx.effect(move |scope| {
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
        .await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context, config: TmuxContextPluginConfig) -> Result<()> {
    let _ = crate::Fiber::mount_retained(ctx, Box::new(TmuxContextPlugin { config })).await?;
    Ok(())
}

// Re-export the pure helpers for hosts and tests.
