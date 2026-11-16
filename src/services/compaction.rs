//! `ctx.compaction`: provider-neutral compaction seam.
//!
//! Upstream: `packages/compaction/compaction/src/index.ts` and
//! `compaction-basic/src/index.ts` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data. This port combines the token meter's
//! pressure with the tool-result pruner into a deterministic automatic policy:
//! over pressure, it appends a bounded summary checkpoint to the session.

use crate::Result;
use crate::services::pruner::{PrunerConfig, ToolResultPruner};
use crate::services::sessions::Session;
use crate::services::token_meter::{TokenMeasurement, TokenMeter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionTrigger {
    Pressure,
    ContextOverflow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompactionResult {
    pub compacted: bool,
    pub trigger: CompactionTrigger,
    pub pressure: f64,
    pub shadowed_events: usize,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Pressure ratio above which automatic compaction runs.
    pub pressure_threshold: f64,
    pub head_chars: usize,
    pub tail_chars: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            pressure_threshold: 0.8,
            head_chars: 1000,
            tail_chars: 1000,
        }
    }
}

#[derive(Clone)]
pub struct CompactionEngine {
    config: CompactionConfig,
    meter: TokenMeter,
    pruner: ToolResultPruner,
}

impl CompactionEngine {
    pub fn new(
        config: CompactionConfig,
        meter: TokenMeter,
    ) -> std::result::Result<Self, crate::Error> {
        let pruner = ToolResultPruner::new(PrunerConfig {
            head_chars: config.head_chars,
            tail_chars: config.tail_chars,
            threshold_chars: (config.head_chars + config.tail_chars) * 4 + 64,
        })
        .map_err(|error| crate::Error::Other(error.to_string()))?;
        Ok(Self {
            config,
            meter,
            pruner,
        })
    }

    pub fn measure(&self, session: &Session) -> TokenMeasurement {
        self.meter.measure(session)
    }

    /// Decide whether to compact and, when triggered, append a summary.
    pub fn maybe_compact(
        &self,
        session: &Session,
        trigger: CompactionTrigger,
    ) -> std::result::Result<CompactionResult, crate::Error> {
        let measurement = self.meter.measure(session);
        if measurement.pressure < self.config.pressure_threshold {
            return Ok(CompactionResult {
                compacted: false,
                trigger,
                pressure: measurement.pressure,
                shadowed_events: 0,
                summary: String::new(),
            });
        }

        // Build a bounded summary from the visible message text.
        let parts: Vec<String> = session
            .events()
            .into_iter()
            .filter(|event| {
                matches!(
                    event.event_type.as_str(),
                    "user/message" | "assistant/message"
                )
            })
            .filter_map(|event| {
                event
                    .data
                    .get("text")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .collect();
        let raw = parts.join("\n");
        let summary = match self.pruner.prune_text(&raw) {
            Some(pruned) => pruned.text,
            None => raw,
        };

        let shadowed = measurement.message_count;
        session.append(
            "compaction/checkpoint",
            serde_json::json!({
                "summary": summary,
                "shadowed_events": shadowed,
                "trigger": match trigger {
                    CompactionTrigger::Pressure => "pressure",
                    CompactionTrigger::ContextOverflow => "context-overflow",
                },
            }),
        );

        Ok(CompactionResult {
            compacted: true,
            trigger,
            pressure: measurement.pressure,
            shadowed_events: shadowed,
            summary,
        })
    }
}

pub async fn provide(ctx: &crate::Context, meter: TokenMeter) -> Result<CompactionEngine> {
    let engine = CompactionEngine::new(CompactionConfig::default(), meter)
        .map_err(|error| crate::Error::Other(error.to_string()))?;
    ctx.provide("compaction", engine.clone()).await?;
    Ok(engine)
}
