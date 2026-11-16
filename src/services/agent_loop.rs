//! A minimal ReAct agent loop over the shared LLM and tools seams.
//!
//! Upstream `ctx.agentLoop` owns agent creation; the step driver lives in the
//! agent itself. This port condenses both into one testable loop: prepare a
//! request, stream a turn, execute requested tool calls, repeat until the
//! model returns no calls or `max_steps` is reached.
//! Reference: `docs/agent-lifecycle.md` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted data.

use std::sync::Arc;

use crate::Result;
use crate::core_services::{ToolArgs, ToolError, ToolExecContext, ToolOutput, ToolRegistry};
use crate::services::llm::{ChatMessage, LlmRequest, LlmResponse, LlmService};

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

pub struct AgentLoop {
    llm: LlmService,
    tools: ToolRegistry,
    model: Option<String>,
    max_steps: usize,
    parse_calls: CallParser,
}

impl AgentLoop {
    pub fn new(llm: LlmService, tools: ToolRegistry) -> Self {
        Self {
            llm,
            tools,
            model: None,
            max_steps: 4,
            parse_calls: Arc::new(no_calls),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    pub fn with_call_parser(mut self, parser: CallParser) -> Self {
        self.parse_calls = parser;
        self
    }

    /// Run one turn: stream, parse calls, execute them, and repeat while the
    /// parser keeps producing calls.
    pub async fn run(&self, prompt: impl Into<String>) -> Result<AgentRun> {
        let mut messages = vec![ChatMessage::user(prompt)];
        let mut steps = Vec::new();
        let mut final_text = String::new();

        for _ in 0..self.max_steps {
            let request = LlmRequest {
                model: self.model.clone().unwrap_or_default(),
                messages: messages.clone(),
            };
            let response = self.llm.stream(self.model.as_deref(), request).await?;
            let calls = (self.parse_calls)(&response.text);
            final_text = response.text.clone();
            messages.push(ChatMessage::assistant(response.text.clone()));

            let mut tool_results = Vec::new();
            for call in calls {
                let exec = ToolExecContext::agent("agent-1");
                let result = self.tools.invoke(&call.name, call.args, exec).await;
                tool_results.push(result.clone());
                let rendered = match result {
                    Ok(output) => output.rendered.join("\n"),
                    Err(error) => error.to_string(),
                };
                messages.push(ChatMessage::user(rendered));
            }

            steps.push(AgentStep {
                response,
                tool_results,
            });
            if messages
                .last()
                .map(|m| m.role == "assistant")
                .unwrap_or(true)
            {
                break;
            }
        }

        Ok(AgentRun { steps, final_text })
    }
}
