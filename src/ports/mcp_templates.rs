//! Pure Rust port of `@deepseek-ai/dsh-mcp-client` template utilities.
//!
//! Source: `packages/mcp/mcp-client/src/templates.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! `${credential:REF}` syntax for MCP transport configuration strings.

/// Error code for an unconfigured credential referenced by a template.
pub const MCP_CREDENTIAL_NOT_FOUND: &str = "MCP_CREDENTIAL_NOT_FOUND";

/// A parsed template segment preserving literal text verbatim.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateSegment {
    Literal(String),
    Credential(String),
}

fn is_ref_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ref_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Scan `${credential:REF}` occurrences, returning segments in template order.
/// Unknown `${...}` forms are rejected so misconfiguration fails loud.
fn parse_segments(value: &str, where_path: &str) -> Result<Vec<TemplateSegment>, String> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = value[cursor..].find("${") {
        let start = cursor + relative;
        let body_start = start + 2;
        let Some(end_relative) = value[body_start..].find('}') else {
            return Err(format!("{where_path}: unterminated `${{` at byte {start}"));
        };
        let end = body_start + end_relative;
        let body = &value[body_start..end];
        if start > cursor {
            segments.push(TemplateSegment::Literal(value[cursor..start].to_string()));
        }
        let Some(reference) = body.strip_prefix("credential:") else {
            return Err(format!(
                "{where_path}: \"${{{body}}}\" is not a template token — use ${{credential:REF}}"
            ));
        };
        let mut chars = reference.chars();
        let Some(first) = chars.next() else {
            return Err(format!("{where_path}: empty credential reference"));
        };
        if !is_ref_start(first) || !chars.all(is_ref_char) {
            return Err(format!(
                "{where_path}: \"{reference}\" is not a valid credential reference"
            ));
        }
        segments.push(TemplateSegment::Credential(reference.to_string()));
        cursor = end + 1;
    }
    if cursor < value.len() {
        segments.push(TemplateSegment::Literal(value[cursor..].to_string()));
    }
    Ok(segments)
}

/// Referenced credential names in template order (statically, no lookup).
pub fn credential_refs_in(value: &str, where_path: &str) -> Result<Vec<String>, String> {
    Ok(parse_segments(value, where_path)?
        .into_iter()
        .filter_map(|segment| match segment {
            TemplateSegment::Credential(reference) => Some(reference),
            TemplateSegment::Literal(_) => None,
        })
        .collect())
}

/// Validate that every `${...}` occurrence is a well-formed token.
pub fn assert_template_syntax(value: &str, where_path: &str) -> Result<(), String> {
    parse_segments(value, where_path).map(|_| ())
}

/// Resolve template tokens via a credential lookup closure. Resolution is
/// atomic: any unconfigured reference fails the whole string without leaking
/// partial substitutions or secret values into the error.
pub fn resolve_template_string<F>(
    value: &str,
    where_path: &str,
    lookup: F,
) -> Result<String, String>
where
    F: Fn(&str) -> Option<String>,
{
    let segments = parse_segments(value, where_path)?;
    let mut missing = Vec::new();
    let mut out = String::new();
    for segment in &segments {
        match segment {
            TemplateSegment::Literal(text) => out.push_str(text),
            TemplateSegment::Credential(reference) => match lookup(reference) {
                Some(secret) => out.push_str(&secret),
                None => missing.push(reference.clone()),
            },
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "{where_path}: credential(s) {} ({MCP_CREDENTIAL_NOT_FOUND})",
            missing.join(", ")
        ));
    }
    Ok(out)
}
