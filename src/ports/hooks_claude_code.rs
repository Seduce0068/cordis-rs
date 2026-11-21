//! Reusable Rust port of `@deepseek-ai/dsh-hooks-claude-code` (phase 1).
//!
//! Source: `packages/experimental/hooks-claude-code/src/index.ts` (+ config.ts)
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Phase 1 wires SessionStart / PreToolUse / PostToolUse; Stop and subagent
//! hooks are deferred. The shared execution/merging lives in `hook_protocol`.

use crate::services::hook_protocol::{HookOutput, MergedDecision, MergedHookOutcome, matches};
use crate::{Context, Result};

pub const DEFAULT_TIMEOUT_MS: u64 = 600_000;
pub const DEFAULT_STDERR_SUMMARY_CHARS: usize = 500;
pub const AUDIT_INVOKED: &str = "hook/invoked";
pub const AUDIT_RESULT: &str = "hook/result";

/// Three hook events implemented in phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    SessionStart,
    PreToolUse,
    PostToolUse,
}

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "SessionStart" => Some(Self::SessionStart),
            "PreToolUse" => Some(Self::PreToolUse),
            "PostToolUse" => Some(Self::PostToolUse),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HookEntry {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HooksConfig {
    pub hooks: Vec<HookEntry>,
    pub default_timeout_ms: u64,
    pub stderr_summary_max_chars: usize,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            hooks: Vec::new(),
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            stderr_summary_max_chars: DEFAULT_STDERR_SUMMARY_CHARS,
        }
    }
}

/// Parse the `hooks.json` shape `{ "hooks": { "Event": [{ matcher, hooks: [{command}] }] } }`.
pub fn parse_config(value: &serde_json::Value) -> std::result::Result<HooksConfig, String> {
    let mut config = HooksConfig::default();
    if let Some(timeout) = value.get("defaultTimeoutMs").and_then(|v| v.as_u64()) {
        // Reject zero (immediate timeout) and values that cannot fit usize
        // on this target (swarm review R6-IV07).
        if timeout == 0 || timeout > usize::MAX as u64 {
            return Err(format!("defaultTimeoutMs out of range: {timeout}"));
        }
        config.default_timeout_ms = timeout;
    }
    if let Some(chars) = value.get("stderrSummaryMaxChars").and_then(|v| v.as_u64()) {
        if chars == 0 || chars > usize::MAX as u64 {
            return Err(format!("stderrSummaryMaxChars out of range: {chars}"));
        }
        config.stderr_summary_max_chars = chars as usize;
    }
    let Some(hooks) = value.get("hooks").and_then(|v| v.as_object()) else {
        return Ok(config);
    };
    for (event_name, groups) in hooks {
        let Some(event) = HookEvent::parse(event_name) else {
            continue; // Stop / Subagent hooks are deferred in phase 1.
        };
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let matcher = group
                .get("matcher")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            // Fail closed: a malformed regex matcher would silently never
            // match at dispatch time, disabling deny hooks (security review).
            if let Some(diagnostic) =
                crate::core_services::matcher_diagnostic(matcher.as_deref(), false)
            {
                return Err(diagnostic);
            }
            let Some(entries) = group.get("hooks").and_then(|v| v.as_array()) else {
                continue;
            };
            for entry in entries {
                if let Some(command) = entry.get("command").and_then(|v| v.as_str()) {
                    if command.trim().is_empty() {
                        return Err("hook command must be non-blank".to_string());
                    }
                    config.hooks.push(HookEntry {
                        event,
                        matcher: matcher.clone(),
                        command: command.to_string(),
                    });
                }
            }
        }
    }
    Ok(config)
}

/// Select entries matching an event + tool/query name.
pub fn select_hooks<'a>(
    config: &'a HooksConfig,
    event: HookEvent,
    query: &str,
) -> Vec<&'a HookEntry> {
    config
        .hooks
        .iter()
        .filter(|entry| entry.event == event)
        .filter(|entry| {
            entry
                .matcher
                .as_deref()
                .is_none_or(|matcher| matches(Some(matcher), query, false))
        })
        .collect()
}

/// Map a merged phase-1 outcome to its effect. `updatedInput` and
/// `systemMessage` are intentionally unimplemented (warn-only upstream).
/// `MergedDecision::Ask` is not mapped to deny: delegating to an approval
/// answer flow is deferred, and pre-step SessionStart injection is likewise
/// not wired into `agent_loop` in phase 1 (deny must not be silently ignored
/// by hosts — they should call this helper and handle `Reject`).
#[derive(Debug, Clone, PartialEq)]
pub enum HookEffect {
    Proceed,
    Reject(String),
    Block(String),
    Inject(String),
}

pub fn map_outcome(event: HookEvent, outcome: &MergedHookOutcome) -> HookEffect {
    let reason = outcome.reasons.first().cloned().unwrap_or_default();
    if outcome.decision == MergedDecision::Deny {
        return match event {
            HookEvent::PreToolUse => HookEffect::Reject(reason),
            HookEvent::PostToolUse => HookEffect::Block(reason),
            HookEvent::SessionStart => HookEffect::Reject(reason),
        };
    }
    if let Some(context) = outcome.contexts.first() {
        return HookEffect::Inject(context.clone());
    }
    HookEffect::Proceed
}

/// Build an audit pair for a hook execution. Both events are log-only (not
/// `user/message`) so hook text never enters compaction summaries.
/// `run_event_seq` links the result back to the invocation in an append-only
/// log; readers must not assume adjacency when hooks run in parallel.
pub fn audit_pair(
    event: HookEvent,
    command: &str,
    output: &HookOutput,
    run_event_seq: Option<u64>,
) -> Vec<(String, serde_json::Value)> {
    vec![
        (
            AUDIT_INVOKED.to_string(),
            serde_json::json!({ "event": event.as_str(), "command": command }),
        ),
        (
            AUDIT_RESULT.to_string(),
            serde_json::json!({
                "event": event.as_str(),
                "command": command,
                "decision": output.decision.as_str(),
                "reason": output.reason,
                "run_event_seq": run_event_seq,
            }),
        ),
    ]
}

/// The `dsh-hooks-claude-code` plugin (phase 1).
pub struct HooksClaudeCodePlugin {
    pub config: HooksConfig,
}

#[async_trait::async_trait]
impl crate::Plugin for HooksClaudeCodePlugin {
    fn name(&self) -> &str {
        "hooks-claude-code"
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
    let _ = crate::Fiber::mount_retained(ctx, Box::new(HooksClaudeCodePlugin { config })).await?;
    Ok(())
}
