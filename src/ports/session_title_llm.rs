//! Reusable Rust port of `session-title-all-prompts-llm`.
//!
//! Source: `packages/experimental/session-title-all-prompts-llm/src/index.ts`
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! `inject = ['sessionTitles', 'llm']`: asks the LLM for a title from all
//! human messages, then stores the parsed result on `ctx.sessionTitles`.

use async_trait::async_trait;

use crate::core_services::{
    ChatMessage, LlmRequest, LlmService, SessionTitles, TitleProviderConfig, frame_messages,
    title_from_output, title_system_prompt,
};
use crate::{Context, Error, Result};

pub struct SessionTitleLlm {
    pub config: TitleProviderConfig,
    pub model: Option<String>,
}

impl SessionTitleLlm {
    pub fn new(config: TitleProviderConfig) -> Self {
        Self {
            config,
            model: None,
        }
    }

    /// Generate and store a title from human messages. Returns `false` when
    /// the model produced no usable title (the fallback title is preserved).
    pub async fn generate(
        &self,
        llm: &LlmService,
        titles: &SessionTitles,
        session_id: &str,
        messages: &[String],
    ) -> std::result::Result<bool, Error> {
        if messages.is_empty() {
            return Ok(false);
        }
        let current = titles.get(session_id);
        let request = LlmRequest {
            model: self.model.clone().unwrap_or_default(),
            messages: vec![
                ChatMessage::system(title_system_prompt(&self.config, current.is_some())),
                ChatMessage::user(frame_messages(messages, current.as_deref())),
            ],
        };
        let response = llm.stream(self.model.as_deref(), request).await?;
        match title_from_output(&response.text) {
            Some(title) => {
                Ok(titles.set_generated(session_id, &title, self.config.target_cjk_characters))
            }
            None => Ok(false),
        }
    }
}

#[async_trait]
impl crate::Plugin for SessionTitleLlm {
    fn name(&self) -> &str {
        "session-title-all-prompts-llm"
    }

    fn inject(&self) -> &[&str] {
        &["sessionTitles", "llm"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        // Provider is stateless; the seams are verified by inject resolution.
        let _ = ctx.require::<SessionTitles>("sessionTitles").await?;
        let _ = ctx.require::<LlmService>("llm").await?;
        Ok(())
    }
}

/// Mount the provider plugin.
pub async fn mount(ctx: &Context, config: TitleProviderConfig) -> Result<()> {
    let _ = crate::Fiber::mount_retained(ctx, Box::new(SessionTitleLlm::new(config))).await?;
    Ok(())
}
