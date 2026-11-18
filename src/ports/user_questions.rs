//! Reusable Rust port of `@deepseek-ai/dsh-user-questions` (phase 1).
//!
//! Source: `packages/interaction/user-questions/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Phase 1 provides blocking `ask` / `answer`, a minimal projection, an
//! answerer waterfall seam. Timed waits, `continued` state, `steer()` and the
//! `ask_user_question` tool are phase 2 and intentionally absent (the tool
//! handler would need a context handle that creates an effect lifecycle cycle).

use std::sync::Arc;

use crate::services::sessions::{SessionEvent, SessionProjectionRegistry};
use crate::{Context, Error, Event, Result};

pub const PROJECTION_KEY: &str = "userQuestions";
pub const PHASE2_TOOL_NAME: &str = "ask_user_question";

/// Stable error codes (upstream `HarnessError` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserQuestionError {
    NoProvider,
    AskAborted,
    CallerNotLive,
    DelegatedCaller,
    BadIntent,
    EmptyQuestions,
    BadAnswer,
    ReplyQueued,
    ProviderError,
}

impl UserQuestionError {
    pub fn code(self) -> &'static str {
        match self {
            Self::NoProvider => "NO_PROVIDER",
            Self::AskAborted => "ASK_ABORTED",
            Self::CallerNotLive => "CALLER_NOT_LIVE",
            Self::DelegatedCaller => "DELEGATED_CALLER",
            Self::BadIntent => "BAD_INTENT",
            Self::EmptyQuestions => "EMPTY_QUESTIONS",
            Self::BadAnswer => "BAD_ANSWER",
            Self::ReplyQueued => "REPLY_QUEUED",
            Self::ProviderError => "PROVIDER_ERROR",
        }
    }
}

impl std::fmt::Display for UserQuestionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code())
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AskQuestion {
    pub id: String,
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AskRequest {
    pub questions: Vec<AskQuestion>,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnswerItem {
    pub id: String,
    pub selected: Vec<String>,
    pub custom: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Answer {
    pub answers: Vec<AnswerItem>,
}

/// Answerer waterfall terminal: no provider answered.
fn no_provider(_request: &AskRequest) -> Answer {
    // Sentinel: an answer with no items means no provider claimed the request.
    Answer {
        answers: Vec::new(),
    }
}

/// Minimal projection fold: `open` on ask, `settled` on answer.
pub fn user_questions_fold(state: serde_json::Value, event: &SessionEvent) -> serde_json::Value {
    match event.event_type.as_str() {
        "user/question-asked" => {
            let mut map = state.as_object().cloned().unwrap_or_default();
            let call_id = event
                .data
                .get("call_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            map.insert(call_id, serde_json::json!("open"));
            serde_json::Value::Object(map)
        }
        "user/question-answered" => {
            let mut map = state.as_object().cloned().unwrap_or_default();
            let call_id = event
                .data
                .get("call_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if map.contains_key(&call_id) {
                map.insert(call_id, serde_json::json!("settled"));
            }
            serde_json::Value::Object(map)
        }
        _ => state,
    }
}

type AnswererEvent = Event<AskRequest, Answer>;

/// The user-questions service.
#[derive(Clone)]
pub struct UserQuestionService {
    answerers: Arc<AnswererEvent>,
    ctx: Context,
}

impl UserQuestionService {
    /// Validate and dispatch an ask through the answerer waterfall.
    pub async fn ask(
        &self,
        request: AskRequest,
        cancelled: bool,
    ) -> std::result::Result<Answer, UserQuestionError> {
        if cancelled {
            return Err(UserQuestionError::AskAborted);
        }
        if request.questions.is_empty() {
            return Err(UserQuestionError::EmptyQuestions);
        }
        let answer = self
            .answerers
            .waterfall(&self.ctx, request, no_provider)
            .await
            .map_err(|_| UserQuestionError::ProviderError)?;
        if answer.answers.is_empty() {
            Err(UserQuestionError::NoProvider)
        } else {
            Ok(answer)
        }
    }

    /// Register an answerer via the waterfall seam.
    pub fn on_answerer<F>(&self, ctx: &Context, listener: F) -> crate::Disposer
    where
        F: Fn(
                crate::events::Shared<AskRequest>,
                crate::events::Next<AskRequest, Answer>,
            ) -> crate::events::BoxFuture<'static, crate::Result<Answer>>
            + Send
            + Sync
            + 'static,
    {
        self.answerers.on_waterfall(ctx, listener)
    }
}

/// A user-questions plugin injecting `sessionProjections` and `agents`.
pub struct UserQuestionsPlugin;

#[async_trait::async_trait]
impl crate::Plugin for UserQuestionsPlugin {
    fn name(&self) -> &str {
        "user-questions"
    }

    fn inject(&self) -> &[&str] {
        &["sessionProjections"]
    }

    async fn apply(&self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

/// Mount the service, projection fold and tool.
pub async fn mount(ctx: &Context) -> Result<UserQuestionService> {
    let projections = ctx
        .require::<SessionProjectionRegistry>("sessionProjections")
        .await?;
    if projections.has(PROJECTION_KEY) {
        return Err(Error::Other(format!(
            "user-questions: projection `{PROJECTION_KEY}` already registered"
        )));
    }
    let projection_disposer = projections.register(PROJECTION_KEY, Arc::new(user_questions_fold));
    let service = UserQuestionService {
        answerers: Arc::new(Event::new("user/question-answerer")),
        ctx: ctx.clone(),
    };
    ctx.provide("userQuestions", service.clone()).await?;
    let tool_disposer = crate::Disposer::new(|| async { Ok(()) });
    let fiber = crate::Fiber::mount_retained(ctx, Box::new(UserQuestionsPlugin)).await?;
    ctx.effect(move |scope| {
        let projection_disposer = projection_disposer.clone();
        let tool_disposer = tool_disposer.clone();
        let fiber = fiber.clone();
        async move {
            scope.on_dispose(move || async move {
                projection_disposer.dispose().await?;
                tool_disposer.dispose().await?;
                fiber.unload().await?;
                Ok(())
            });
            Ok(())
        }
    })
    .await?;
    Ok(service)
}
