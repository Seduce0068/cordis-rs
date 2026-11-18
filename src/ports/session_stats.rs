//! Reusable Rust port of `@deepseek-ai/dsh-session-stats`.
//!
//! Source: `packages/session/session-stats/src/projection.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! A pure fold of step/tool boundaries into whole-log counters.
//!
//! Local `SessionEvent` has no wall-clock timestamp, so time-like figures are
//! reported as *event counts* (`llmEvents`, `ttftEvents`) rather than
//! fabricated milliseconds. Wall times remain zero with `approximate: true`.

use std::sync::Arc;

use crate::core_services::SessionProjectionRegistry;
use crate::{Context, Error, Result};

pub const PROJECTION_KEY: &str = "sessionStats";

fn empty_state() -> serde_json::Value {
    serde_json::json!({
        "turns": 0, "steps": 0,
        "llmMs": 0, "toolMs": 0, "ttftMs": 0, "ttftSteps": 0,
        "decodeMs": 0, "decodeTokens": 0,
        "llmEvents": 0, "ttftEvents": 0,
        "approximate": true,
        "lastTurn": null,
        "openStep": null,
        "pending": [],
        "unmatched": 0,
    })
}

fn as_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    value.and_then(|v| v.as_u64())
}

/// Fold one event into the stats state. Untrusted `data` is read defensively;
/// tool call/result pairing uses FIFO because local tool execution is serial.
pub fn fold(
    state: serde_json::Value,
    event: &crate::services::sessions::SessionEvent,
) -> serde_json::Value {
    let mut s = if state.is_null() {
        empty_state()
    } else {
        state
    };
    let data = &event.data;
    let get_u = |key: &str| as_u64(data.get(key));
    let inc = |s: &mut serde_json::Value, key: &str, by: u64| {
        let current = s.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        s[key] = serde_json::json!(current + by);
    };

    match event.event_type.as_str() {
        "step/start" => {
            let (turn, step) = (get_u("turn"), get_u("step"));
            if let (Some(turn), Some(step)) = (turn, step) {
                s["openStep"] = serde_json::json!({
                    "turn": turn, "step": step, "startSeq": event.seq, "firstTokenSeq": null,
                });
            }
            s
        }
        "assistant/attempt" => {
            let open = s
                .get("openStep")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let turn = open.get("turn").and_then(|v| v.as_u64());
            let step = open.get("step").and_then(|v| v.as_u64());
            if turn.is_none() || turn != get_u("turn") || step != get_u("step") {
                return s;
            }
            // Local streams assemble once; first-token seq equals the message seq.
            if open.get("firstTokenSeq").is_some_and(|v| v.is_null()) {
                let mut next = open.clone();
                next["firstTokenSeq"] = serde_json::json!(event.seq);
                s["openStep"] = next;
                inc(&mut s, "ttftEvents", 1);
            }
            s
        }
        "assistant/message" => {
            let open = s
                .get("openStep")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let turn = open.get("turn").and_then(|v| v.as_u64());
            let step = open.get("step").and_then(|v| v.as_u64());
            if turn.is_none() || turn != get_u("turn") || step != get_u("step") {
                return s;
            }
            inc(&mut s, "llmEvents", 1);
            let start_seq = open
                .get("startSeq")
                .and_then(|v| v.as_u64())
                .unwrap_or(event.seq);
            let first_token = open
                .get("firstTokenSeq")
                .and_then(|v| v.as_u64())
                .unwrap_or(event.seq);
            if first_token >= start_seq {
                inc(&mut s, "ttftEvents", 1);
                inc(&mut s, "ttftSteps", 1);
                if let Some(tokens) = data
                    .get("usage")
                    .and_then(|u| u.get("outputTokens"))
                    .and_then(|v| v.as_u64())
                {
                    inc(&mut s, "decodeTokens", tokens);
                }
            }
            s["openStep"] = serde_json::Value::Null;
            s
        }
        "tool/call" => {
            let mut pending: Vec<serde_json::Value> = s
                .get("pending")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            pending.push(serde_json::json!({ "seq": event.seq, "tool": data.get("tool").cloned().unwrap_or(serde_json::Value::Null) }));
            s["pending"] = serde_json::json!(pending);
            s
        }
        "tool/result" => {
            let mut pending: Vec<serde_json::Value> = s
                .get("pending")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if pending.is_empty() {
                inc(&mut s, "unmatched", 1);
                return s;
            }
            let matched = pending.remove(0); // FIFO: serial execution
            let start = matched
                .get("seq")
                .and_then(|v| v.as_u64())
                .unwrap_or(event.seq);
            if event.seq >= start {
                inc(&mut s, "toolMs", event.seq - start); // event-count units
            }
            s["pending"] = serde_json::json!(pending);
            s
        }
        "step/end" => {
            let turn = get_u("turn");
            let last = s.get("lastTurn").and_then(|v| v.as_u64());
            if turn.is_some() && turn != last {
                inc(&mut s, "turns", 1);
            }
            inc(&mut s, "steps", 1);
            s["lastTurn"] = serde_json::json!(turn);
            s["openStep"] = serde_json::Value::Null;
            s
        }
        "turn/end" => {
            let pending: Vec<serde_json::Value> = s
                .get("pending")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if !pending.is_empty() {
                inc(&mut s, "unmatched", pending.len() as u64);
                s["pending"] = serde_json::json!(Vec::<serde_json::Value>::new());
            }
            s
        }
        _ => s,
    }
}

/// Wire view: only the counters (no in-flight state).
pub fn view(state: &serde_json::Value) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for key in [
        "turns",
        "steps",
        "llmMs",
        "toolMs",
        "ttftMs",
        "ttftSteps",
        "decodeMs",
        "decodeTokens",
        "llmEvents",
        "ttftEvents",
        "unmatched",
        "approximate",
    ] {
        if let Some(value) = state.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// The `dsh-session-stats` plugin.
pub struct SessionStats;

#[async_trait::async_trait]
impl crate::Plugin for SessionStats {
    fn name(&self) -> &str {
        "session-stats"
    }

    fn inject(&self) -> &[&str] {
        &["sessionProjections"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let projections = ctx
            .require::<SessionProjectionRegistry>("sessionProjections")
            .await?;
        if projections.has(PROJECTION_KEY) {
            return Err(Error::Other(format!(
                "projection {PROJECTION_KEY} already registered"
            )));
        }
        let disposer = projections.register(PROJECTION_KEY, Arc::new(fold));
        ctx.effect(move |scope| {
            let d = disposer.clone();
            async move {
                scope.on_dispose(move || async move {
                    d.dispose().await?;
                    Ok(())
                });
                Ok(())
            }
        })
        .await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context) -> Result<()> {
    let fiber = crate::Fiber::mount_retained(ctx, Box::new(SessionStats)).await?;
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
