//! `ctx.sessionTitles`: session title store with deterministic fallback.
//!
//! Upstream: `packages/experimental/session-title` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! LLM-backed title providers plug in via `set_generated`; when no provider
//! answered, a deterministic first-message title keeps sessions named.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;

#[derive(Default)]
struct TitleState {
    titles: BTreeMap<String, String>,
}

#[derive(Default, Clone)]
pub struct SessionTitles {
    state: Arc<RwLock<TitleState>>,
}

impl SessionTitles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, session_id: impl Into<String>, title: impl Into<String>) {
        let title = title.into();
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return;
        }
        self.state
            .write()
            .expect("session titles poisoned")
            .titles
            .insert(session_id.into(), trimmed.to_string());
    }

    pub fn get(&self, session_id: &str) -> Option<String> {
        self.state
            .read()
            .expect("session titles poisoned")
            .titles
            .get(session_id)
            .cloned()
    }

    /// Store a title from an LLM provider, truncating to the CJK character
    /// target when needed. Empty output is ignored (provider did not answer).
    pub fn set_generated(&self, session_id: &str, text: &str, max_chars: usize) -> bool {
        let first = text.lines().find(|line| !line.trim().is_empty());
        let Some(line) = first else {
            return false;
        };
        let title: String = line.trim().chars().take(max_chars.max(1)).collect();
        if title.is_empty() {
            return false;
        }
        self.set(session_id, title);
        true
    }

    /// Deterministic fallback: first human message, shortened to `max_chars`.
    pub fn derive_from_messages(&self, session_id: &str, messages: &[String], max_chars: usize) {
        if self.get(session_id).is_some() {
            return; // keep an existing, possibly LLM-generated title
        }
        let Some(first) = messages.iter().find(|text| !text.trim().is_empty()) else {
            return;
        };
        let title: String = first.trim().chars().take(max_chars.max(1)).collect();
        self.set(session_id, title);
    }

    pub fn list(&self) -> Vec<(String, String)> {
        self.state
            .read()
            .expect("session titles poisoned")
            .titles
            .iter()
            .map(|(id, title)| (id.clone(), title.clone()))
            .collect()
    }
}

/// Configuration for the LLM-backed all-prompts title provider.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleProviderConfig {
    pub target_words: usize,
    pub target_cjk_characters: usize,
}

impl Default for TitleProviderConfig {
    fn default() -> Self {
        Self {
            target_words: 6,
            target_cjk_characters: 20,
        }
    }
}

/// System instruction for title generation, including stability rules when
/// an existing title is supplied.
pub fn title_system_prompt(config: &TitleProviderConfig, has_current_title: bool) -> String {
    let mut lines = vec![
        "Create a concise title for an AI coding-assistant session from the supplied human messages.".to_string(),
        "Return only the title on one line, in plain text of natural language, with no quotes, prefix, explanation, Markdown, XML, or terminal control codes. No code is allowed.".to_string(),
        "Use the language of the messages.".to_string(),
        format!(
            "Aim for about {} words in non-CJK languages or {} CJK characters.",
            config.target_words, config.target_cjk_characters
        ),
        "If the messages give little to name, still return a short best-effort title, such as Greeting, instead of explaining.".to_string(),
    ];
    if has_current_title {
        lines.extend([
            "An existing title is supplied as currentTitle. If it still accurately describes the main topic or task, return it exactly unchanged.".to_string(),
            "Follow-up questions, acknowledgements and requests to continue do not by themselves justify a title change.".to_string(),
            "Do not reword or polish an adequate title; exact wording takes priority over length.".to_string(),
            "Change the title only when the messages materially change the main topic.".to_string(),
        ]);
    }
    lines.join("\n")
}

/// Frame human messages as JSON so user text cannot break delimiters.
pub fn frame_messages(messages: &[String], current_title: Option<&str>) -> String {
    let rendered: Vec<&str> = messages.iter().map(String::as_str).collect();
    match current_title {
        Some(title) => format!(
            "Update the session title from this JSON object:\n{}",
            serde_json::json!({ "currentTitle": title, "messages": rendered })
        ),
        None => format!(
            "Generate the session title from this JSON array of human messages:\n{}",
            serde_json::json!(rendered)
        ),
    }
}

/// Extract a title from model output: first non-empty line, stripping whole
/// line asterisk emphasis such as `**Title**`.
pub fn title_from_output(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let stripped = line
        .strip_prefix("***")
        .and_then(|value| value.strip_suffix("***"))
        .or_else(|| {
            line.strip_prefix("**")
                .and_then(|value| value.strip_suffix("**"))
        })
        .or_else(|| {
            line.strip_prefix('*')
                .and_then(|value| value.strip_suffix('*'))
        });
    let title = match stripped {
        Some(inner) if !inner.is_empty() && !inner.contains('*') => inner.to_string(),
        _ => line.to_string(),
    };
    if title.is_empty() { None } else { Some(title) }
}

pub async fn provide(ctx: &crate::Context) -> Result<SessionTitles> {
    let titles = SessionTitles::new();
    ctx.provide("sessionTitles", titles.clone()).await?;
    Ok(titles)
}
