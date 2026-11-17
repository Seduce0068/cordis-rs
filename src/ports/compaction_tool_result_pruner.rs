//! Reusable Rust port of `@deepseek-ai/dsh-compaction-tool-result-pruner`.
//!
//! Source: `packages/compaction/compaction-tool-result-pruner/src/index.ts`
//! (+ config.ts, types.ts) in deepseek-ai/deepseek-harness (MIT), read as
//! untrusted reference data. The head/middle/tail core already lives in
//! `services/pruner.rs`; this port provides the service wiring and the
//! shadow-price → replacement event sequence for a session.

use crate::services::pruner::{PrunedText, PrunerConfig, ToolResultPruner};
use crate::services::sessions::{Session, SessionEvent};
use crate::{Context, Result};

pub const SERVICE_NAME: &str = "toolResultPruner";
pub const SHADOW_PRICE_EVENT: &str = "compaction/prune";
pub const REPLACEMENT_EVENT: &str = "tool/result";

/// Measure only text-block code points (upstream `measureContent`).
pub fn measure_content(blocks: &[serde_json::Value]) -> usize {
    blocks
        .iter()
        .map(|block| {
            block
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| text.chars().count())
                .unwrap_or(0)
        })
        .sum()
}

/// Prune text blocks, asserting the upstream invariant that the rendered
/// result is both within the threshold and strictly shorter than the input.
pub fn prune_content(
    pruner: &ToolResultPruner,
    blocks: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .map(|block| {
            let Some(text) = block.get("text").and_then(|value| value.as_str()) else {
                return block.clone();
            };
            match pruner.prune_text(text) {
                None => block.clone(),
                Some(PrunedText { text: pruned, .. }) => {
                    let mut next = block.clone();
                    if let Some(map) = next.as_object_mut() {
                        map.insert("text".to_string(), serde_json::json!(pruned));
                    }
                    next
                }
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct PruneSessionResult {
    pub pruned: usize,
    pub chars_removed: usize,
    pub events: Vec<SessionEvent>,
}

/// Apply pruning to a session's `tool/result` events. Each pruned event gets a
/// shadow-price marker immediately followed by its replacement, both carrying
/// the source event seq (upstream `sourceEventSeqs` + surfaceOp replace).
pub fn prune_session(pruner: &ToolResultPruner, session: &Session) -> PruneSessionResult {
    let mut pruned = 0usize;
    let mut chars_removed = 0usize;
    let mut events = Vec::new();
    for event in session.events() {
        if event.event_type != REPLACEMENT_EVENT {
            continue;
        }
        let Some(text) = event.data.get("text").and_then(|value| value.as_str()) else {
            continue;
        };
        if let Some(result) = pruner.prune_text(text) {
            chars_removed += result.removed_chars;
            events.push(session.append(
                SHADOW_PRICE_EVENT,
                serde_json::json!({ "sourceEventSeqs": [event.seq] }),
            ));
            events.push(session.append(
                REPLACEMENT_EVENT,
                serde_json::json!({
                    "text": result.text,
                    "sourceEventSeqs": [event.seq],
                    "op": "replace",
                }),
            ));
            pruned += 1;
        }
    }
    PruneSessionResult {
        pruned,
        chars_removed,
        events,
    }
}

/// Mount the service on a context, registering the disposer as an effect.
pub async fn provide(ctx: &Context, config: PrunerConfig) -> Result<ToolResultPruner> {
    let pruner =
        ToolResultPruner::new(config).map_err(|error| crate::Error::Other(error.to_string()))?;
    // `Context::provide` already registers the disposer as a fiber effect,
    // so no second effect wrapper is needed (reviewer finding).
    let _disposer = ctx.provide(SERVICE_NAME, pruner.clone()).await?;
    Ok(pruner)
}

/// The `dsh-compaction-tool-result-pruner` plugin.
pub struct CompactionToolResultPruner;

#[async_trait::async_trait]
impl crate::Plugin for CompactionToolResultPruner {
    fn name(&self) -> &str {
        "compaction-tool-result-pruner"
    }

    fn inject(&self) -> &[&str] {
        &["tokenMeter"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx
            .require::<crate::services::token_meter::TokenMeter>("tokenMeter")
            .await?;
        provide(ctx, PrunerConfig::default()).await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context) -> Result<()> {
    let _fiber = crate::Fiber::mount(ctx, Box::new(CompactionToolResultPruner)).await?;
    Ok(())
}
