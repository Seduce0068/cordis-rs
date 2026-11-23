//! A minimal ReAct agent loop over the shared LLM and tools seams.
//!
//! Upstream `ctx.agentLoop` owns agent creation; the step driver lives in the
//! agent itself. This port condenses both into one testable loop: prepare a
//! request, stream a turn, execute requested tool calls, repeat until the
//! model returns no calls or `max_steps` is reached. On reaching the step cap,
//! the current step's tool calls are still executed before the turn ends; no
//! parsed call is left unexecuted (verified by the loop body).
//! Reference: `docs/agent-lifecycle.md` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted data.

use std::sync::Arc;

use crate::Result;
use crate::core_services::{ToolArgs, ToolError, ToolExecContext, ToolOutput, ToolRegistry};
use crate::services::llm::{ChatMessage, LlmRequest, LlmResponse, LlmService};
use crate::services::sessions::Session;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: ToolArgs,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, args: ToolArgs) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentStep {
    pub response: LlmResponse,
    pub tool_results: Vec<std::result::Result<ToolOutput, ToolError>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRun {
    pub steps: Vec<AgentStep>,
    pub final_text: String,
}

/// A provider that turns a model response into tool calls. Upstream parses
/// structured tool-call blocks; a host supplies its own parser here.
pub type CallParser = Arc<dyn Fn(&str) -> Vec<ToolCall> + Send + Sync>;

fn no_calls(_text: &str) -> Vec<ToolCall> {
    Vec::new()
}

fn session_event(session: &Option<Session>, event_type: &str, data: serde_json::Value) {
    if let Some(session) = session {
        session.append(event_type, data);
    }
}

/// Called after each committed step so a host can checkpoint the session.
pub type CheckpointHook = Arc<dyn Fn(&Session) -> crate::Result<()> + Send + Sync>;

pub struct AgentLoop {
    llm: LlmService,
    tools: ToolRegistry,
    /// Tool calls pass through this pipeline so cancellation checks always
    /// apply. A host can inject approval/retry/timeout middleware via
    /// `with_pipeline`; the default pipeline has no approval answerer and is
    /// therefore fail-open for approval unless the host installs one.
    pipeline: Option<crate::services::tools::ToolPipeline>,
    /// Caller identity attached to tool executions (audit / scope separation).
    caller: String,
    /// Optional session id attached to tool executions.
    session_id: Option<String>,
    model: Option<String>,
    max_steps: usize,
    parse_calls: CallParser,
    session: Option<Session>,
    checkpoint: Option<CheckpointHook>,
    system_prompt: Option<crate::services::system_prompt::SystemPrompt>,
    goals: Option<crate::services::goals::GoalStore>,
    compaction: Option<crate::services::compaction::CompactionEngine>,
    retry: Option<crate::services::retry::RetryPolicy>,
}

impl AgentLoop {
    pub fn new(llm: LlmService, tools: ToolRegistry) -> Self {
        Self {
            llm,
            tools,
            // Always route through the pipeline: even a default one enforces
            // the cancelled-execution check that direct invoke bypasses.
            pipeline: Some(crate::services::tools::ToolPipeline::default()),
            // Default keeps the legacy caller; session scope stays off unless
            // explicitly opted in (mcp scope visibility depends on it).
            caller: "agent-1".to_string(),
            session_id: None,
            model: None,
            max_steps: 4,
            parse_calls: Arc::new(no_calls),
            session: None,
            checkpoint: None,
            system_prompt: None,
            goals: None,
            compaction: None,
            retry: None,
        }
    }

    /// Set the caller identity attached to tool executions (audit / scope).
    pub fn with_caller(mut self, caller: impl Into<String>) -> Self {
        self.caller = caller.into();
        self
    }

    /// Opt in to session scoping for tool executions. By default tool calls
    /// carry no session id, preserving the legacy Agent/Global scope.
    pub fn with_tool_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Route tool calls through the shared pipeline so middleware (approval,
    /// retry, timeout policy) applies. Without this, calls use direct invoke.
    pub fn with_pipeline(mut self, pipeline: crate::services::tools::ToolPipeline) -> Self {
        self.pipeline = Some(pipeline);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    /// Replace the call parser. The previous hook is simply dropped; any
    /// external cleanup it owned is not invoked (swarm review R13-OW09).
    pub fn with_call_parser(mut self, parser: CallParser) -> Self {
        self.parse_calls = parser;
        self
    }

    /// Record turn/step events on this session and derive messages from it.
    /// `Session` is an Arc-backed handle: passing `session.clone()` shares the
    /// same log with the caller rather than making an independent copy. Two
    /// loops given the same handle will interleave their events (swarm review
    /// R13-OW03).
    pub fn with_session(mut self, session: Session) -> Self {
        self.session = Some(session);
        self
    }

    /// Assemble a system message from the shared prompt registry each turn.
    pub fn with_system_prompt(
        mut self,
        prompt: crate::services::system_prompt::SystemPrompt,
    ) -> Self {
        self.system_prompt = Some(prompt);
        self
    }

    /// Inject the live goal as the `goal` assembly variable.
    pub fn with_goals(mut self, goals: crate::services::goals::GoalStore) -> Self {
        self.goals = Some(goals);
        self
    }

    /// Retry LLM requests on retryable provider errors.
    pub fn with_retry(mut self, retry: crate::services::retry::RetryPolicy) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Run automatic pressure compaction before each step.
    pub fn with_compaction(
        mut self,
        compaction: crate::services::compaction::CompactionEngine,
    ) -> Self {
        self.compaction = Some(compaction);
        self
    }

    /// Checkpoint the session after each committed step. Replacing an earlier
    /// hook drops it without invoking cleanup (swarm review R13-OW09).
    pub fn with_checkpoint(mut self, hook: CheckpointHook) -> Self {
        self.checkpoint = Some(hook);
        self
    }

    /// Always close the turn on failure, mirroring upstream's durable
    /// `turn/end` contract. Cleanup errors are intentionally non-fatal here;
    /// the original LLM error is what the caller receives.
    fn finish_turn_with_error(&self, reason: &str) {
        if let Some(session) = &self.session {
            session.append(
                "turn/end",
                serde_json::json!({ "reason": { "kind": reason } }),
            );
            if let Some(checkpoint) = &self.checkpoint {
                let _ = checkpoint(session);
            }
        }
    }

    /// Run one turn: stream, parse calls, execute them, and repeat while the
    /// parser keeps producing calls.
    pub async fn run(&self, prompt: impl Into<String>) -> Result<AgentRun> {
        let prompt = prompt.into();
        let mut messages = Vec::new();
        let mut steps = Vec::new();
        let mut final_text = String::new();

        // Assemble the system prompt with the live goal as a variable.
        if let Some(system_prompt) = &self.system_prompt {
            use crate::services::system_prompt::AssembleContext;
            use std::collections::BTreeMap;
            let mut variables = BTreeMap::new();
            if let Some(goal) = self.goals.as_ref().and_then(|goals| goals.get()) {
                variables.insert("goal".to_string(), goal.objective);
                variables.insert("goal_status".to_string(), goal.status.as_str().to_string());
            }
            let assembly = system_prompt
                .assemble(&AssembleContext { variables })
                .map_err(|error| crate::Error::Other(error.to_string()))?;
            if !assembly.text.trim().is_empty() {
                messages.push(ChatMessage::system(assembly.text));
            }
        }
        messages.push(ChatMessage::user(prompt.clone()));

        if let Some(session) = &self.session {
            session.append("turn/start", serde_json::json!({}));
            session.append("user/message", serde_json::json!({ "text": prompt }));
        }

        for step_index in 0..self.max_steps {
            // Retry budget is per-step: a retry consumed in one step must not
            // starve later steps (team review finding).
            let mut retry_attempt = 0u32;
            // Automatic pressure compaction at the step boundary, mirroring
            // upstream's `agent/pre-step` policy listener.
            if let (Some(session), Some(compaction)) = (&self.session, &self.compaction) {
                use crate::services::compaction::CompactionTrigger;
                let result = compaction.maybe_compact(session, CompactionTrigger::Pressure)?;
                if let Some(replacement) = result.replacement {
                    // Keep the system prefix and the most recent messages,
                    // replacing the compacted span with one summary message.
                    let system_count = messages
                        .iter()
                        .take_while(|message| message.role == "system")
                        .count();
                    let tail_start = messages.len().saturating_sub(result.retain_tail);
                    let mut compacted: Vec<ChatMessage> = messages[..system_count].to_vec();
                    compacted.push(replacement);
                    compacted.extend_from_slice(&messages[tail_start..]);
                    messages = compacted;
                }
            }
            if let Some(session) = &self.session {
                session.append("step/start", serde_json::json!({ "step": step_index + 1 }));
            }
            let request = LlmRequest {
                model: self.model.clone().unwrap_or_default(),
                messages: messages.clone(),
            };
            let response = loop {
                match self
                    .llm
                    .stream(self.model.as_deref(), request.clone())
                    .await
                {
                    Ok(value) => break value,
                    Err(error) => {
                        let decision = self
                            .retry
                            .as_ref()
                            .map(|policy| policy.should_retry_error(&error, retry_attempt))
                            .unwrap_or(crate::services::retry::RetryDecision::Stop);
                        if decision == crate::services::retry::RetryDecision::Stop {
                            self.finish_turn_with_error("llm_error");
                            return Err(error);
                        }
                        let delay = self
                            .retry
                            .as_ref()
                            .map(|policy| policy.backoff_ms(retry_attempt + 1, 0.5))
                            .unwrap_or(0);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        retry_attempt += 1;
                    }
                }
            };
            let calls = (self.parse_calls)(&response.text);
            final_text = response.text.clone();
            messages.push(ChatMessage::assistant(response.text.clone()));
            if let Some(session) = &self.session {
                session.append(
                    "assistant/message",
                    serde_json::json!({ "text": response.text }),
                );
            }

            let mut tool_results = Vec::new();
            for call in calls {
                let exec = ToolExecContext {
                    agent: Some(self.caller.clone()),
                    session: self.session_id.clone(),
                    signal_cancelled: false,
                    cancel_flag: None,
                };
                let result = match &self.pipeline {
                    Some(pipeline) => pipeline.run(&self.tools, &call.name, call.args, exec).await,
                    None => self.tools.invoke(&call.name, call.args, exec).await,
                };
                tool_results.push(result.clone());
                let rendered = match result {
                    Ok(output) => output.rendered.join("\n"),
                    Err(error) => error.to_string(),
                };
                session_event(
                    &self.session,
                    "tool/call",
                    serde_json::json!({ "tool": call.name }),
                );
                session_event(
                    &self.session,
                    "tool/result",
                    serde_json::json!({ "text": rendered }),
                );
                messages.push(ChatMessage::user(rendered));
            }

            steps.push(AgentStep {
                response,
                tool_results,
            });
            if let Some(session) = &self.session {
                session.append("step/end", serde_json::json!({ "step": step_index + 1 }));
                if let Some(checkpoint) = &self.checkpoint {
                    checkpoint(session)?;
                }
            }
            if messages
                .last()
                .map(|m| m.role == "assistant")
                .unwrap_or(true)
            {
                break;
            }
        }

        if let Some(session) = &self.session {
            session.append("turn/end", serde_json::json!({}));
            if let Some(checkpoint) = &self.checkpoint {
                checkpoint(session)?;
            }
        }

        Ok(AgentRun { steps, final_text })
    }
}
