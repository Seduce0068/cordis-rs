//! Reusable Rust port of `@deepseek-ai/dsh-user-questions` (phase 1).
//!
//! Source: `packages/interaction/user-questions/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Phase 1 provides blocking `ask` / `answer`, a minimal projection, an
//! answerer waterfall seam and the `ask_user_question` tool. Timed waits,
//! `continued` state and `steer()` are phase 2 and intentionally absent.

use std::sync::Arc;

use crate::services::sessions::{SessionEvent, SessionProjectionRegistry};
use crate::{
    Context, Error, Event, Result,
    events::EventRegistry,
    services::tools::{
        ToolArgs, ToolDefinition, ToolError, ToolExecContext, ToolOutput, ToolRegistry,
    },
};

pub const PROJECTION_KEY: &str = "userQuestions";
pub const TOOL_NAME: &str = "ask_user_question";

/// Stable error codes (upstream `HarnessError` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserQuestionError {
    NoProvider,
    AskAborted,
    /// Reserved for phase-2 `steer()` / timed waits (upstream HarnessError
    /// code). No construction path yet; kept for public code-set stability
    /// (swarm review R2-ER06).
    CallerNotLive,
    /// Reserved for phase-2 delegated-call handling; never emitted. Kept
    /// in `code()` for public code-set stability (swarm review R18-ER03).
    DelegatedCaller,
    /// Reserved for phase-2 steer intent validation; never emitted. Kept
    /// in `code()` for public code-set stability (swarm review R18-ER03).
    BadIntent,
    EmptyQuestions,
    /// Reserved for phase-2 answer validation; never emitted. Kept in
    /// `code()` for public code-set stability (swarm review R18-ER03).
    BadAnswer,
    /// Reserved for phase-2 reply queuing; never emitted. Kept in
    /// `code()` for public code-set stability (swarm review R18-ER03).
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

/// Wire structs below use serde defaults: fields without `Option` are
/// required on deserialization; only `Option` fields may be omitted
/// (swarm review R19-DUAL4-05).
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
    registry: EventRegistry,
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
            .waterfall_with_registry(&self.registry, request, no_provider)
            .await
            .map_err(|_| UserQuestionError::ProviderError)?;
        if answer.answers.is_empty() {
            Err(UserQuestionError::NoProvider)
        } else {
            Ok(answer)
        }
    }

    /// Register an answerer via the waterfall seam.
    pub fn on_answerer<F>(&self, listener: F) -> crate::Disposer
    where
        F: Fn(
                crate::events::Shared<AskRequest>,
                crate::events::Next<AskRequest, Answer>,
            ) -> crate::events::BoxFuture<'static, crate::Result<Answer>>
            + Send
            + Sync
            + 'static,
    {
        self.answerers
            .on_waterfall_with_registry(&self.registry, listener)
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

/// Register the `ask_user_question` tool.
async fn register_tool(ctx: &Context, service: UserQuestionService) -> Result<crate::Disposer> {
    let definition = ToolDefinition::new(
        TOOL_NAME,
        "Ask the user one or more questions",
        Arc::new(move |args: ToolArgs, exec: ToolExecContext| {
            let service = service.clone();
            Box::pin(async move {
                let request: AskRequest = serde_json::from_value(args)
                    .map_err(|error| ToolError::InvalidArgs(error.to_string()))?;
                if request.questions.is_empty() {
                    return Err(ToolError::InvalidArgs("empty questions".to_string()));
                }
                let answer = service
                    .ask(request, exec.is_cancelled())
                    .await
                    .map_err(|error| ToolError::Execution(error.to_string()))?;
                let content = serde_json::to_value(&answer)
                    .map_err(|error| ToolError::Execution(error.to_string()))?;
                Ok(ToolOutput {
                    content,
                    rendered: vec![format!("{} answers", answer.answers.len())],
                })
            })
        }),
    );
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    Ok(tools.register(definition))
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
        registry: ctx.event_registry().clone(),
    };
    ctx.provide("userQuestions", service.clone()).await?;
    let tool_disposer = register_tool(ctx, service.clone()).await?;
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
