//! Reusable Rust port of `@deepseek-ai/dsh-command-feedback`.
//!
//! Records one authoritative `feedback/record` event on a session through the
//! human-facing `/feedback` command. Source:
//! `packages/feedback/command-feedback/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{
    CommandDefinition, CommandHandler, CommandInvocation, CommandRegistry, CommandResult,
};
use crate::core_services::{LoggedSessionEvent, Session};
use crate::{Context, Result};

pub const USAGE: &str = "Usage: /feedback <text>";

pub const FEEDBACK_CATEGORIES: &[&str] = &[
    "task-result",
    "instruction-following",
    "product-interaction",
    "service-stability",
    "resource-cost",
    "security-privacy-permission",
    "other",
];

#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackRecord {
    pub text: Option<String>,
    pub category: Option<String>,
}

/// Port of upstream `recordFeedback`: trim text, drop blank text, append the
/// authoritative log-only event. No model work is started.
pub fn record_feedback(session: &Session, entry: &FeedbackRecord) {
    let text = entry.text.as_deref().unwrap_or("").trim().to_string();
    let mut data = serde_json::Map::new();
    if !text.is_empty() {
        data.insert("text".to_string(), serde_json::Value::String(text));
    }
    if let Some(category) = &entry.category {
        data.insert(
            "category".to_string(),
            serde_json::Value::String(category.clone()),
        );
    }
    session.append("feedback/record", serde_json::Value::Object(data));
}

/// Port of the command's `executeFeedbackCommand`.
fn execute_command(invocation: &CommandInvocation) -> Option<(String, FeedbackRecord)> {
    let text = invocation.raw_input.trim();
    if text.is_empty() {
        return None;
    }
    Some((
        text.to_string(),
        FeedbackRecord {
            text: Some(text.to_string()),
            category: None,
        },
    ))
}

/// The `dsh-command-feedback` plugin. `inject = ['commands']`.
pub struct CommandFeedback {
    /// Optional session to record into; when absent the invocation's session
    /// id is looked up through the shared session store.
    pub session: Option<Session>,
}

#[async_trait]
impl crate::Plugin for CommandFeedback {
    fn name(&self) -> &str {
        "command-feedback"
    }

    fn inject(&self) -> &[&str] {
        &["commands"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let commands = ctx.require::<CommandRegistry>("commands").await?;
        let session = self.session.clone();
        let handler: CommandHandler = Arc::new(move |invocation| {
            let session = session.clone();
            Box::pin(async move {
                match execute_command(&invocation) {
                    None => CommandResult::error(format!("Feedback text is required. {USAGE}")),
                    Some((text, record)) => {
                        let session_id = match &session {
                            Some(session) => {
                                record_feedback(session, &record);
                                session.id().to_string()
                            }
                            None => match &invocation.session {
                                Some(id) => id.clone(),
                                None => {
                                    return CommandResult::error(
                                        "feedback requires an owning session".to_string(),
                                    );
                                }
                            },
                        };
                        CommandResult::success(format!(
                            "Feedback recorded for session {session_id}: {text}"
                        ))
                    }
                }
            })
        });
        ctx.effect(move |scope| {
            let commands = commands.clone();
            let handler = handler.clone();
            async move {
                let definition = CommandDefinition::new(
                    "/feedback",
                    "Record feedback about this session (log-only, no model work).",
                    handler,
                );
                let disposer = commands.register(definition);
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with a session for feedback recording.
pub async fn mount(ctx: &Context, session: Session) -> Result<CommandRegistry> {
    let commands = crate::core_services::provide_commands(ctx).await?;
    let _ = crate::Fiber::mount_retained(
        ctx,
        Box::new(CommandFeedback {
            session: Some(session),
        }),
    )
    .await?;
    Ok(commands)
}

/// Helper to read recorded feedback events from a session.
pub fn feedback_events(session: &Session) -> Vec<LoggedSessionEvent> {
    session
        .events()
        .into_iter()
        .filter(|event| event.event_type == "feedback/record")
        .collect()
}
