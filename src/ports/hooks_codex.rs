//! Reusable Rust port of `@deepseek-ai/dsh-hooks-codex` (phase 1).
//!
//! Source: `packages/experimental/hooks-codex/src/index.ts` (+ config.ts) in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Phase 1 wires PreToolUse and PostToolUse with Codex dialect payloads.
//! Config parsing, selection, merge and audit are shared with the Claude port.

use crate::ports::hooks_claude_code::{
    HookEntry, HookEvent, HooksConfig, audit_pair, map_outcome, parse_config, select_hooks,
};
use crate::services::hook_protocol::{
    HookOutput, MergedDecision, MergedHookOutcome, merge_hook_outputs,
};
use crate::{Context, Result};

pub const DEFAULT_TIMEOUT_MS: u64 = 600_000;
pub const DEFAULT_STDERR_SUMMARY_CHARS: usize = 500;

/// Codex dialect: tool hook payload uses snake_case keys and only deny is
/// honoured (ask / rewrite are not implemented in phase 1).
#[derive(Debug, Clone, PartialEq)]
pub struct CodexToolPayload {
    pub tool_name: String,
    pub command: Option<String>,
}

/// Build a snake_case payload from a tool name and optional command.
pub fn tool_payload(tool_name: &str, command: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "tool_name": tool_name,
        "command": command,
    })
}

/// Build a merged hook outcome for a Codex deny/allow decision.
pub fn codex_outcome(deny: bool, reason: Option<&str>) -> MergedHookOutcome {
    merge_hook_outputs(std::slice::from_ref(&HookOutput {
        decision: if deny {
            MergedDecision::Deny
        } else {
            MergedDecision::Allow
        },
        reason: reason.map(str::to_string),
        stop: false,
        context: None,
    }))
}

/// Map a Codex outcome to its phase-1 effect, reusing the Claude dialect
/// mapping (PreToolUse deny → reject, PostToolUse deny → block).
pub fn codex_effect(
    event: HookEvent,
    deny: bool,
    reason: Option<&str>,
) -> crate::ports::hooks_claude_code::HookEffect {
    map_outcome(event, &codex_outcome(deny, reason))
}

/// Load a Codex hooks config. Invalid JSON is a no-op (empty config); invalid
/// matchers are rejected (fail closed, shared with the Claude port).
pub fn load_codex_config(
    value: Option<&serde_json::Value>,
) -> std::result::Result<HooksConfig, String> {
    match value {
        None => Ok(HooksConfig::default()),
        Some(value) => parse_config(value),
    }
}

/// Select matching entries for a tool hook (Codex uses the same regex matcher).
pub fn select_tool_hooks<'a>(
    config: &'a HooksConfig,
    event: HookEvent,
    tool_name: &str,
) -> Vec<&'a HookEntry> {
    select_hooks(config, event, tool_name)
}

/// Build the audit pair for a Codex hook execution (log-only).
pub fn codex_audit_pair(
    event: HookEvent,
    command: &str,
    output: &HookOutput,
    run_event_seq: Option<u64>,
) -> Vec<(String, serde_json::Value)> {
    audit_pair(event, command, output, run_event_seq)
        .into_iter()
        .map(|(name, mut data)| {
            // Keep the event names shared with the Claude port but tag the
            // dialect so a mixed composition can distinguish the two bridges.
            if let Some(map) = data.as_object_mut() {
                map.insert("dialect".to_string(), serde_json::json!("codex"));
            }
            (name, data)
        })
        .collect()
}

/// The `dsh-hooks-codex` plugin.
pub struct HooksCodex {
    pub config: HooksConfig,
}

#[async_trait::async_trait]
impl crate::Plugin for HooksCodex {
    fn name(&self) -> &str {
        "hooks-codex"
    }

    fn inject(&self) -> &[&str] {
        &["shell", "agents", "workingDirectory"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx
            .require::<crate::services::shell::Shell>("shell")
            .await?;
        let _ = ctx
            .require::<crate::services::agents::AgentRegistry>("agents")
            .await?;
        let _ = ctx
            .require::<crate::services::working_directory::WorkingDirectory>("workingDirectory")
            .await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context, config: HooksConfig) -> Result<()> {
    let fiber = crate::Fiber::mount(ctx, Box::new(HooksCodex { config })).await?;
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
    Ok(())
}
