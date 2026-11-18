//! Pure Rust port of `@deepseek-ai/dsh-llm/message` helpers.
//!
//! Source: `packages/llm/llm/src/message.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

/// Bound for a `notice` summary (upstream `CONTEXT_SUMMARY_MAX_CHARS`).
pub const CONTEXT_SUMMARY_MAX_CHARS: usize = 120;

/// Bound one summary by Unicode code points, appending an ellipsis when the
/// value exceeds the limit. UTF-8 safe (never slices at a byte boundary).
pub fn bound_context_summary(summary: &str) -> String {
    let chars = summary.chars();
    if chars.clone().count() <= CONTEXT_SUMMARY_MAX_CHARS {
        summary.to_string()
    } else {
        let mut out: String = chars.take(CONTEXT_SUMMARY_MAX_CHARS - 1).collect();
        out.push('…');
        out
    }
}

/// Extract text block contents in order from a JSON content array. Non-text
/// blocks (images, tool uses) are ignored entirely, separators included.
pub fn extract_text_content(content: &serde_json::Value, separator: &str) -> String {
    let Some(blocks) = content.as_array() else {
        return String::new();
    };
    let texts: Vec<&str> = blocks
        .iter()
        .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .filter(|text| !text.is_empty())
        .collect();
    texts.join(separator)
}
