//! Reusable Rust port of `@deepseek-ai/dsh-session-turn-outline`.
//!
//! Source: `packages/session/session-turn-outline/src/projection.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! A pure fold over turn boundaries, first prompts and final responses that
//! builds the whole-log turn outline used by chat rails.

use std::sync::Arc;

use crate::core_services::SessionProjectionRegistry;
use crate::{Context, Result};

pub const TURN_OUTLINE_PROJECTION_KEY: &str = "turnOutline";
pub const PROMPT_PREVIEW_LIMIT: usize = 50;
pub const RESPONSE_PREVIEW_LIMIT: usize = 120;

/// Space-join text blocks, collapse whitespace and clip with an ellipsis.
fn preview(blocks: Option<&serde_json::Value>, limit: usize) -> String {
    let Some(blocks) = blocks.and_then(|value| value.as_array()) else {
        return String::new();
    };
    let mut text = String::new();
    let mut unread = false;
    for block in blocks {
        if block.get("type").and_then(|value| value.as_str()) != Some("text") {
            continue;
        }
        let content = block
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if text.chars().count() >= limit * 2 {
            unread = true;
            break;
        }
        let chars: Vec<char> = content.chars().collect();
        let clipped = chars.len() > limit * 2;
        let chunk: String = if clipped {
            chars[..limit * 2].iter().collect()
        } else {
            content.to_string()
        };
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&chunk);
        if clipped {
            unread = true;
            break;
        }
    }
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = normalized.chars().count();
    if count > limit.saturating_sub(1) {
        let kept: String = normalized.chars().take(limit - 1).collect();
        return format!("{}…", kept.trim_end());
    }
    if unread {
        return format!("{normalized}…");
    }
    normalized
}

/// Fold one session event into the turn-outline state (`{turns, draft}`).
pub fn fold(
    state: serde_json::Value,
    event: &crate::services::sessions::SessionEvent,
) -> serde_json::Value {
    let mut turns: Vec<serde_json::Value> = state
        .get("turns")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let draft = state
        .get("draft")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();

    match event.event_type.as_str() {
        "turn/start" => {
            let turn = event
                .data
                .get("turn")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let last_turn = turns
                .last()
                .and_then(|entry| entry.get("turn").and_then(|value| value.as_u64()))
                .map(|value| value as i64)
                .unwrap_or(-1);
            if turn as i64 <= last_turn {
                return state;
            }
            turns.push(serde_json::json!({
                "turn": turn, "seq": event.seq, "prompt": "", "response": ""
            }));
            serde_json::json!({ "turns": turns, "draft": "" })
        }
        "user/message" => {
            if event
                .data
                .get("source")
                .and_then(|source| source.get("kind"))
                .and_then(|kind| kind.as_str())
                != Some("user")
            {
                return state;
            }
            let Some(last) = turns.last_mut() else {
                return state;
            };
            if last.get("prompt").and_then(|value| value.as_str()) != Some("") {
                return state;
            }
            let prompt = preview(event.data.get("content"), PROMPT_PREVIEW_LIMIT);
            if prompt.is_empty() {
                return state;
            }
            last["prompt"] = serde_json::json!(prompt);
            serde_json::json!({ "turns": turns, "draft": draft })
        }
        "assistant/message" => {
            let content = event
                .data
                .get("message")
                .and_then(|message| message.get("content"));
            let next = preview(content, RESPONSE_PREVIEW_LIMIT);
            if next.is_empty() || next == draft {
                return state;
            }
            serde_json::json!({ "turns": state.get("turns").cloned().unwrap_or_default(), "draft": next })
        }
        "turn/end" => {
            if draft.is_empty() {
                return state;
            }
            let Some(last) = turns.last_mut() else {
                return state;
            };
            if last.get("response").and_then(|value| value.as_str()) == Some(draft.as_str()) {
                return serde_json::json!({ "turns": state.get("turns").cloned().unwrap_or_default(), "draft": "" });
            }
            last["response"] = serde_json::json!(draft);
            serde_json::json!({ "turns": turns, "draft": "" })
        }
        _ => state,
    }
}

/// The `dsh-session-turn-outline` plugin.
pub struct SessionTurnOutline;

#[async_trait::async_trait]
impl crate::Plugin for SessionTurnOutline {
    fn name(&self) -> &str {
        "session-turn-outline"
    }

    fn inject(&self) -> &[&str] {
        &["sessionProjections"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let projections = ctx
            .require::<SessionProjectionRegistry>("sessionProjections")
            .await?;
        let disposer = projections.register(TURN_OUTLINE_PROJECTION_KEY, Arc::new(fold));
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
    let fiber = crate::Fiber::mount(ctx, Box::new(SessionTurnOutline)).await?;
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
