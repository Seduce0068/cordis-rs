//! `ctx.systemPrompt`: registry for prompt sections assembled before each step.
//!
//! Upstream: `packages/core/system-prompt/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Sections are sorted by order then name; `{{variable}}` placeholders are
//! interpolated from an assemble context. A `complete` section replaces all
//! others; more than one complete section is an error.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::primitives::Disposer;

#[derive(Debug, Clone)]
pub struct PromptSection {
    pub name: String,
    pub order: i64,
    pub text: String,
    pub interpolate: bool,
    pub complete: bool,
}

impl PromptSection {
    pub fn new(name: impl Into<String>, order: i64, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            order,
            text: text.into(),
            interpolate: true,
            complete: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AssembleContext {
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptAssembly {
    pub text: String,
    pub sections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PromptError {
    DuplicateSection(String),
    MultipleComplete,
}

impl std::fmt::Display for PromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateSection(name) => write!(f, "duplicate system prompt section: {name}"),
            Self::MultipleComplete => write!(f, "more than one complete system prompt section"),
        }
    }
}

#[derive(Default)]
struct RegistryState {
    sections: BTreeMap<String, PromptSection>,
}

/// The shared system prompt registry.
#[derive(Default, Clone)]
pub struct SystemPrompt {
    state: Arc<RwLock<RegistryState>>,
}

impl SystemPrompt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a section. Duplicate names are rejected upstream.
    pub fn section(&self, section: PromptSection) -> std::result::Result<Disposer, PromptError> {
        let state = self.state.clone();
        {
            let mut guard = state.write().expect("system prompt poisoned");
            if guard.sections.contains_key(&section.name) {
                return Err(PromptError::DuplicateSection(section.name.clone()));
            }
            guard.sections.insert(section.name.clone(), section.clone());
        }
        Ok(Disposer::new(move || {
            let state = state.clone();
            let name = section.name.clone();
            async move {
                state
                    .write()
                    .expect("system prompt poisoned")
                    .sections
                    .remove(&name);
                Ok(())
            }
        }))
    }

    /// Well-known section order buckets, mirroring upstream named orders.
    pub fn get_section_order(&self, name: &str) -> i64 {
        match name {
            "HARNESS_IDENTITY" => 0,
            "DEPLOYMENT_PERSONA_PREFIX" => 100,
            "TOOL_READ" | "TOOL_WRITE" | "TOOL_EDIT" | "TOOL_BASH" => 200,
            "DEPLOYMENT_PERSONA_SUFFIX" => 900,
            _ => 500,
        }
    }

    fn interpolate(text: &str, context: &AssembleContext) -> String {
        let mut rendered = text.to_string();
        for (name, value) in &context.variables {
            rendered = rendered.replace(&format!("{{{{{name}}}}}"), value);
        }
        rendered
    }

    /// Assemble the current prompt from all registered sections.
    pub fn assemble(
        &self,
        context: &AssembleContext,
    ) -> std::result::Result<PromptAssembly, PromptError> {
        let guard = self.state.read().expect("system prompt poisoned");
        let mut sections: Vec<&PromptSection> = guard.sections.values().collect();
        sections.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));

        let complete: Vec<&PromptSection> =
            sections.iter().copied().filter(|s| s.complete).collect();
        if complete.len() > 1 {
            return Err(PromptError::MultipleComplete);
        }

        let selected: Vec<&PromptSection> = if let Some(section) = complete.first() {
            vec![section]
        } else {
            sections
        };

        let mut rendered = Vec::new();
        for section in &selected {
            let text = if section.interpolate {
                Self::interpolate(&section.text, context)
            } else {
                section.text.clone()
            };
            if !text.trim().is_empty() {
                rendered.push(text);
            }
        }
        Ok(PromptAssembly {
            text: rendered.join("\n\n"),
            sections: selected
                .iter()
                .map(|section| section.name.clone())
                .collect(),
        })
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<SystemPrompt> {
    let service = SystemPrompt::new();
    ctx.provide("systemPrompt", service.clone()).await?;
    Ok(service)
}
