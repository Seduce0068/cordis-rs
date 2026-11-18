//! Pure Rust port of `@deepseek-ai/dsh-llm/call-config`.
//!
//! Source: `packages/llm/llm/src/call-config.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Field-wise equality for request-header state affecting cache reuse.

/// Provider, model, reasoning effort and sampling scalars of one call.
#[derive(Debug, Clone)]
pub struct LlmCallConfig {
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub stop: Option<Vec<String>>,
}

/// Field-wise equality (upstream `callConfigEquals`). `stop` is compared
/// element-wise when both present; `None` equals only `None`. Two `NaN`
/// temperatures are unequal (upstream `!==` semantics).
pub fn call_config_eq(a: &LlmCallConfig, b: &LlmCallConfig) -> bool {
    if a.provider != b.provider
        || a.model != b.model
        || a.reasoning_effort != b.reasoning_effort
        || !optional_float_eq(a.temperature, b.temperature)
        || a.max_tokens != b.max_tokens
    {
        return false;
    }
    match (&a.stop, &b.stop) {
        (None, None) => true,
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn optional_float_eq(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(left), Some(right)) => left == right, // NaN != NaN
        _ => false,
    }
}
