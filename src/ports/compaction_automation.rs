//! Reusable Rust port of `@deepseek-ai/dsh-compaction/basic` automation.
//!
//! Source: `packages/compaction/compaction/src/` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Phase 1: trigger detection + existing engine reuse; LLM summarization stays
//! a deterministic pruning stub (phase 2).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::services::compaction::{CompactionEngine, CompactionTrigger};
use crate::services::sessions::SessionStore;
use crate::{Context, Error, Result};

/// Configuration for the automation listener.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionAutomationConfig {
    pub auto: bool,
    pub pressure_threshold: f64,
    pub overflow_code: String,
    pub max_overflow_retries: u32,
}

impl Default for CompactionAutomationConfig {
    fn default() -> Self {
        Self {
            auto: true,
            pressure_threshold: 0.8,
            overflow_code: "CONTEXT_WINDOW_EXCEEDED".to_string(),
            max_overflow_retries: 1,
        }
    }
}

/// Result of one automation attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum AutomationOutcome {
    Disabled,
    Noop,
    Compacted {
        shadowed_events: usize,
        pressure: f64,
    },
    RetryLimit,
}

#[derive(Debug, Default)]
struct AutomationState {
    overflow_retries: HashMap<String, u32>,
    compactions: usize,
}

/// The compaction automation service.
#[derive(Clone)]
pub struct CompactionAutomation {
    config: CompactionAutomationConfig,
    engine: Arc<CompactionEngine>,
    sessions: Arc<SessionStore>,
    state: Arc<RwLock<AutomationState>>,
}

impl CompactionAutomation {
    pub fn config(&self) -> &CompactionAutomationConfig {
        &self.config
    }

    /// Drive compaction for a session on a trigger.
    pub fn drive(&self, session_id: &str, trigger: CompactionTrigger) -> Result<AutomationOutcome> {
        if !self.config.auto {
            return Ok(AutomationOutcome::Disabled);
        }
        let Some(session) = self.sessions.get(session_id) else {
            return Ok(AutomationOutcome::Noop);
        };
        // Phase 1: the engine gates ContextOverflow by the pressure threshold
        // too. Preserve that behavior and document it; phase 2 will let
        // overflow bypass the threshold.
        let result = self.engine.maybe_compact(&session, trigger)?;
        if !result.compacted {
            return Ok(AutomationOutcome::Noop);
        }
        if trigger == CompactionTrigger::ContextOverflow {
            let mut state = self
                .state
                .write()
                .expect("compaction automation state poisoned");
            let retries = state
                .overflow_retries
                .entry(session_id.to_string())
                .or_insert(0);
            if *retries >= self.config.max_overflow_retries {
                return Ok(AutomationOutcome::RetryLimit);
            }
            *retries += 1;
        }
        // Persist the replacement as a user message tagged with the upstream
        // checkpoint source marker (compaction_checkpoint port).
        if let Some(replacement) = result.replacement {
            let marker = crate::plugin_ports::checkpoint_source_json(
                &crate::plugin_ports::compact_checkpoint_source(session_id, None),
            );
            // TokenMeter and the compaction engine read `data.text` (not
            // `data.content` blocks), so the replacement must use `text` to
            // actually lower pressure on the next measure.
            session.append(
                "user/message",
                serde_json::json!({
                    "text": replacement.content,
                    "source": marker,
                }),
            );
        }
        self.state
            .write()
            .expect("compaction automation state poisoned")
            .compactions += 1;
        Ok(AutomationOutcome::Compacted {
            shadowed_events: result.shadowed_events,
            pressure: result.pressure,
        })
    }
}

/// Plugin wrapper for the automation service.
pub struct CompactionAutomationPlugin;

#[async_trait::async_trait]
impl crate::Plugin for CompactionAutomationPlugin {
    fn name(&self) -> &str {
        "compaction-automation"
    }

    fn inject(&self) -> &[&str] {
        &["compaction", "sessions"]
    }

    async fn apply(&self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

/// Mount the automation service, reusing the registered compaction engine.
pub async fn mount(
    ctx: &Context,
    config: CompactionAutomationConfig,
) -> Result<CompactionAutomation> {
    if !config.pressure_threshold.is_finite()
        || config.pressure_threshold <= 0.0
        || config.pressure_threshold > 1.0
    {
        return Err(Error::Other(
            "pressure_threshold must be in (0, 1]".to_string(),
        ));
    }
    if config.overflow_code.is_empty() {
        return Err(Error::Other("overflow_code must be non-empty".to_string()));
    }
    // Keep the legacy synchronous lookup: tests/cordis.rs and
    // tests/core_services.rs require mount to fail fast when dependencies are
    // absent. R1 suggested require() to honour inject, but that would hang on
    // a context with no providers (swarm review R1-ST01, reverted).
    let engine = ctx
        .get::<CompactionEngine>("compaction")
        .ok_or_else(|| Error::Other("compaction service not available".to_string()))?;
    let sessions = ctx
        .get::<SessionStore>("sessions")
        .ok_or_else(|| Error::Other("sessions service not available".to_string()))?;
    let service = CompactionAutomation {
        config,
        engine,
        sessions,
        state: Arc::new(RwLock::new(AutomationState::default())),
    };
    ctx.provide("compactionAutomation", service.clone()).await?;
    let fiber = crate::Fiber::mount_retained(ctx, Box::new(CompactionAutomationPlugin)).await?;
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
    Ok(service)
}
