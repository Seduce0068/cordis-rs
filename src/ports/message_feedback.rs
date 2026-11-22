//! Reusable Rust port of `@deepseek-ai/dsh-message-feedback`.
//!
//! Records per-assistant-message ratings as log-only session events. Keeps
//! the upstream invariants: positive safe-integer `maxNoteBytes`, non-empty
//! note text, and a `note-too-large` rejection before any event is appended.
//! Source: `packages/feedback/message-feedback/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::services::sessions::Session;
use crate::{Context, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Positive,
    Negative,
}

impl Rating {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackItem {
    pub message_id: String,
    pub rating: Rating,
    pub note: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeedbackError {
    SessionNotFound(String),
    NoteTooLarge { max: usize, actual: usize },
    EmptyNote,
    VersionConflict { message_id: String },
}

impl std::fmt::Display for FeedbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound(id) => write!(f, "session not found: {id}"),
            Self::NoteTooLarge { max, actual } => {
                write!(f, "note too large: {actual} bytes (max {max})")
            }
            Self::EmptyNote => write!(f, "note must not be empty"),
            Self::VersionConflict { message_id } => {
                write!(f, "version conflict for message {message_id}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageFeedbackConfig {
    pub max_note_bytes: usize,
}

impl Default for MessageFeedbackConfig {
    fn default() -> Self {
        Self {
            max_note_bytes: 1024,
        }
    }
}

#[derive(Default)]
struct RegistryState {
    sessions: BTreeMap<String, Session>,
}

/// The `messageFeedback` service handle.
#[derive(Default, Clone)]
pub struct MessageFeedback {
    config: MessageFeedbackConfig,
    state: Arc<RwLock<RegistryState>>,
}

impl MessageFeedback {
    pub fn new(config: MessageFeedbackConfig) -> std::result::Result<Self, Error> {
        if config.max_note_bytes == 0 {
            return Err(Error::Other(
                "message-feedback: maxNoteBytes must be a positive safe integer".to_string(),
            ));
        }
        Ok(Self {
            config,
            state: Arc::new(RwLock::new(RegistryState::default())),
        })
    }

    pub fn register_session(&self, session: Session) {
        self.state
            .write()
            .expect("feedback state poisoned")
            .sessions
            .insert(session.id().to_string(), session);
    }

    /// Put one rating on a message. Versions are monotonic per message id.
    pub fn put(
        &self,
        session_id: &str,
        item: FeedbackItem,
    ) -> std::result::Result<(), FeedbackError> {
        if let Some(note) = &item.note {
            if note.trim().is_empty() {
                return Err(FeedbackError::EmptyNote);
            }
            let actual = note.len();
            if actual > self.config.max_note_bytes {
                return Err(FeedbackError::NoteTooLarge {
                    max: self.config.max_note_bytes,
                    actual,
                });
            }
        }
        let sessions = self.state.read().expect("feedback state poisoned");
        let session = sessions
            .sessions
            .get(session_id)
            .ok_or_else(|| FeedbackError::SessionNotFound(session_id.to_string()))?;

        // Check version monotonicity against existing put events.
        let last_version = session
            .events()
            .into_iter()
            .rev()
            .find(|event| {
                event.event_type == "feedback/message-put"
                    && event.data["messageId"] == item.message_id
            })
            .and_then(|event| event.data["version"].as_str().map(str::to_string))
            .unwrap_or_default();
        // Compare numeric versions numerically so "10" > "9". For any other
        // format (e.g. upstream "v1"), keep the legacy lexicographic rule
        // (swarm review R8-IV03).
        let conflict = if last_version.is_empty() {
            false
        } else if let (Ok(current), Ok(previous)) =
            (item.version.parse::<u64>(), last_version.parse::<u64>())
        {
            current <= previous
        } else {
            item.version <= last_version
        };
        if conflict {
            return Err(FeedbackError::VersionConflict {
                message_id: item.message_id.clone(),
            });
        }

        session.append(
            "feedback/message-put",
            serde_json::json!({
                "messageId": item.message_id,
                "rating": item.rating.as_str(),
                "note": item.note,
                "version": item.version,
            }),
        );
        Ok(())
    }

    pub fn put_events(&self, session_id: &str) -> Vec<crate::core_services::LoggedSessionEvent> {
        let sessions = self.state.read().expect("feedback state poisoned");
        sessions
            .sessions
            .get(session_id)
            .map(|session| {
                session
                    .events()
                    .into_iter()
                    .filter(|event| event.event_type == "feedback/message-put")
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The `dsh-message-feedback` plugin.
pub struct MessageFeedbackPlugin {
    pub config: MessageFeedbackConfig,
}

#[async_trait]
impl crate::Plugin for MessageFeedbackPlugin {
    fn name(&self) -> &str {
        "message-feedback"
    }

    fn inject(&self) -> &[&str] {
        &[]
    }

    async fn apply(&self, _ctx: &Context) -> Result<()> {
        // Registration is owned by `mount` on the root context: a fiber child
        // would otherwise shadow the configured service with a second copy.
        Ok(())
    }
}

/// Mount the service on a context.
pub async fn mount(ctx: &Context, config: MessageFeedbackConfig) -> Result<MessageFeedback> {
    let service = MessageFeedback::new(config.clone())?;
    ctx.provide("messageFeedback", service.clone()).await?;
    let fiber =
        crate::Fiber::mount_retained(ctx, Box::new(MessageFeedbackPlugin { config })).await?;
    // Retain the fiber so its effect scope is not dropped without cleanup.
    let effect_ctx = ctx.clone();
    effect_ctx
        .effect(move |scope| {
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
