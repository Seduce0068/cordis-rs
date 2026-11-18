//! Reusable Rust port of `@deepseek-ai/dsh-session-title-first-prompt-llm`.
//!
//! Source: `packages/session/session-title-first-prompt-llm/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Titles a session from its first human message only.

use crate::core_services::{
    ChatMessage, LlmRequest, LlmService, SessionTitles, TitleProviderConfig, frame_messages,
    title_from_output, title_system_prompt,
};
use crate::{Context, Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct FirstPromptTitleConfig {
    pub target_words: usize,
    pub target_cjk_characters: usize,
    pub provider: String,
    pub model: String,
}

impl Default for FirstPromptTitleConfig {
    fn default() -> Self {
        Self {
            target_words: 5,
            target_cjk_characters: 10,
            provider: String::new(),
            model: String::new(),
        }
    }
}

impl FirstPromptTitleConfig {
    /// Validate: targets must be positive and provider/model non-blank
    /// (upstream requires a positive integer for each target).
    pub fn validate(&self) -> std::result::Result<(), Error> {
        if self.target_words == 0 || self.target_cjk_characters == 0 {
            return Err(Error::Other(
                "title targets must be positive integers".to_string(),
            ));
        }
        if self.provider.trim().is_empty() || self.model.trim().is_empty() {
            return Err(Error::Other(
                "provider and model must be non-blank".to_string(),
            ));
        }
        Ok(())
    }
}

/// Strip 1-3 leading/trailing asterisks from a line (whole-line emphasis).
pub fn strip_emphasis(line: &str) -> &str {
    let trimmed = line.trim();
    let mut start = 0;
    let mut end = trimmed.len();
    while start < end && start < 3 && trimmed.as_bytes()[start] == b'*' {
        start += 1;
    }
    let mut trailing = 0;
    while end > start + trailing && trailing < 3 && trimmed.as_bytes()[end - 1 - trailing] == b'*' {
        trailing += 1;
    }
    end -= trailing;
    &trimmed[start..end]
}

/// Pick the first non-empty line and remove emphasis. `title_from_output`
/// already implements the same whole-line rule; this thin wrapper makes the
/// chosen behaviour explicit for this port.
pub fn first_title_line(output: &str) -> Option<String> {
    title_from_output(output).map(|title| strip_emphasis(&title).to_string())
}

pub struct SessionTitleFirstPromptLlm {
    pub config: FirstPromptTitleConfig,
}

impl SessionTitleFirstPromptLlm {
    pub fn new(config: FirstPromptTitleConfig) -> Self {
        Self { config }
    }

    fn as_title_config(&self) -> TitleProviderConfig {
        TitleProviderConfig {
            target_words: self.config.target_words,
            target_cjk_characters: self.config.target_cjk_characters,
        }
    }

    /// Generate a title using only the first message. Returns false when no
    /// usable title was produced or messages were empty.
    pub async fn generate(
        &self,
        llm: &LlmService,
        titles: &SessionTitles,
        session_id: &str,
        messages: &[String],
    ) -> std::result::Result<bool, Error> {
        self.config.validate()?;
        let Some(first) = messages.first() else {
            return Ok(false);
        };
        let request = LlmRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage::system(title_system_prompt(&self.as_title_config(), false)),
                ChatMessage::user(frame_messages(std::slice::from_ref(first), None)),
            ],
        };
        let response = llm.stream(Some(&self.config.provider), request).await?;
        let Some(title) = first_title_line(&response.text) else {
            return Ok(false);
        };
        Ok(titles.set_generated(session_id, &title, self.config.target_cjk_characters))
    }
}

#[async_trait::async_trait]
impl crate::Plugin for SessionTitleFirstPromptLlm {
    fn name(&self) -> &str {
        "session-title-first-prompt-llm"
    }

    fn inject(&self) -> &[&str] {
        // Local service names; upstream uses sessionTitle/llm/sessions.
        &["sessionTitles", "llm", "sessions"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        self.config.validate()?;
        let _ = ctx.require::<SessionTitles>("sessionTitles").await?;
        let _ = ctx.require::<LlmService>("llm").await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context, config: FirstPromptTitleConfig) -> Result<()> {
    config.validate()?;
    let fiber =
        crate::Fiber::mount_retained(ctx, Box::new(SessionTitleFirstPromptLlm::new(config)))
            .await?;
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
