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

pub async fn provide(ctx: &crate::Context) -> Result<SessionTitles> {
    let titles = SessionTitles::new();
    ctx.provide("sessionTitles", titles.clone()).await?;
    Ok(titles)
}
