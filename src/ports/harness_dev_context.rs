//! Reusable Rust port of `@deepseek-ai/dsh-harness-dev-context`.
//!
//! Source: `packages/context/harness-dev-context/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Supplies runtime-context values (implementation checkout and Web GUI URL)
//! as `systemPrompt` assembly variables so sections can interpolate them.

use crate::core_services::AssembleContext;

pub const HARNESS_SOURCE_KEY: &str = "harness_source";
pub const WEB_GUI_URL_KEY: &str = "web_gui_url";

pub const HARNESS_SOURCE_PROMPT: &str =
    "The runtime context names the DeepSeek Harness implementation checkout.";
pub const WEB_SURFACE_PROMPT: &str = "You are interacting with the user through the DeepSeek Harness Web GUI whose URL the runtime context names.";

/// Build an assembly context carrying the two harness runtime-context values.
pub fn with_harness_context(
    mut context: AssembleContext,
    checkout: &str,
    web_gui_url: &str,
) -> AssembleContext {
    context
        .variables
        .insert(HARNESS_SOURCE_KEY.to_string(), checkout.to_string());
    context
        .variables
        .insert(WEB_GUI_URL_KEY.to_string(), web_gui_url.to_string());
    context
}

/// Read a runtime-context value from an assembly context.
pub fn harness_context_value<'a>(context: &'a AssembleContext, key: &str) -> Option<&'a str> {
    context.variables.get(key).map(String::as_str)
}
