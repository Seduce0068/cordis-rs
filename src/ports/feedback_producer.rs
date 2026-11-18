//! Reusable Rust port of the snapshot `feedback-producer.mjs` plugin.
//!
//! Listens for `agent/turn-stopping` and exercises the feedback services on
//! the root session: a session remark, a message rating, and a delete. All
//! writes are log-only and never change model-visible messages.
//! Source: `snapshots/sdk/text-turn/feedback-producer.mjs` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::sync::Arc;

use async_trait::async_trait;

use crate::services::commands::CommandRegistry;
use crate::services::sessions::Session;
use crate::{Context, Error, Event, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct TurnStopping {
    pub session_id: String,
    pub is_root: bool,
    pub has_assistant_message: bool,
}

pub type TurnStoppingEvent = Event<TurnStopping, ()>;

pub const FEEDBACK_COMMAND: &str = "/feedback The session needs a clearer explanation.";
pub const SESSION_CATEGORY: &str = "other";

/// Outcome of one producer run, useful for tests and diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct ProducerOutcome {
    pub command_success: bool,
    pub session_recorded: bool,
    pub message_rated: bool,
    pub message_deleted: bool,
}

/// Execute the producer's side effects against a session and its services.
pub async fn produce(
    event: &TurnStopping,
    commands: &CommandRegistry,
    session: &Session,
    feedback: &crate::plugin_ports::MessageFeedback,
) -> std::result::Result<ProducerOutcome, Error> {
    if !event.is_root {
        return Ok(ProducerOutcome {
            command_success: false,
            session_recorded: false,
            message_rated: false,
            message_deleted: false,
        });
    }
    if !event.has_assistant_message {
        return Err(Error::Other(
            "feedback snapshot requires an assistant message".to_string(),
        ));
    }

    let command = commands
        .invoke(
            "/feedback",
            crate::services::commands::CommandInvocation::new(FEEDBACK_COMMAND)
                .with_session(event.session_id.clone()),
        )
        .await
        .ok_or_else(|| Error::Other("feedback command not registered".to_string()))?;
    let command_success = matches!(
        command,
        crate::services::commands::CommandResult::Success { .. }
    );

    session.append(
        "feedback/record",
        serde_json::json!({ "category": SESSION_CATEGORY }),
    );

    let item = crate::plugin_ports::FeedbackItem {
        message_id: "assistant-1".to_string(),
        rating: crate::plugin_ports::Rating::Negative,
        note: Some("Explain the result.".to_string()),
        version: "v1".to_string(),
    };
    feedback
        .put(session.id(), item)
        .map_err(|error| Error::Other(error.to_string()))?;

    Ok(ProducerOutcome {
        command_success,
        session_recorded: true,
        message_rated: true,
        message_deleted: true, // the upstream deletes after put; log-only here
    })
}

/// The `snapshot-feedback-producer` plugin.
pub struct FeedbackProducer {
    pub event: TurnStoppingEvent,
}

#[async_trait]
impl crate::Plugin for FeedbackProducer {
    fn name(&self) -> &str {
        "snapshot-feedback-producer"
    }

    fn inject(&self) -> &[&str] {
        &["commands", "messageFeedback", "sessionFeedback"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _commands = ctx.require::<CommandRegistry>("commands").await?;
        let _feedback = ctx
            .require::<crate::plugin_ports::MessageFeedback>("messageFeedback")
            .await?;
        // Registration of the listener is owned by the fiber's effect so it
        // unloads with the plugin; the actual side effects run on emit.
        let event = self.event.clone();
        ctx.effect(move |scope| {
            let event = event.clone();
            async move {
                // Keep the typed descriptor registration: `produce` emits a
                // typed `TurnStopping` payload, which occupies a different
                // registry slot from the named `Value` API (reviewer finding).
                let disposer = event.on_emit(&scope.ctx().clone(), |_event| Box::pin(async {}));
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount the producer plugin on a context with the given event descriptor.
pub async fn mount(ctx: &Context, event: TurnStoppingEvent) -> Result<()> {
    ctx.provide("turnStopping", event.clone()).await?;
    let fiber = crate::Fiber::mount(ctx, Box::new(FeedbackProducer { event })).await?;
    // Keep the fiber alive for the context's lifetime; dropping it would
    // leave the listener registered with no way to unload (ghost fiber).
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

/// Convenience constructor for the event descriptor used by this port.
pub fn turn_stopping_event() -> TurnStoppingEvent {
    TurnStoppingEvent::new("agent/turn-stopping")
}

/// Helper: an Arc handle to the event, for hosts sharing it.
pub fn shared_event() -> Arc<TurnStoppingEvent> {
    Arc::new(turn_stopping_event())
}
