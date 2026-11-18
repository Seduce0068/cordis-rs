//! Pure Rust port of `@deepseek-ai/dsh-llm/attribution`.
//!
//! Source: `packages/llm/llm/src/attribution.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Centralizes the non-secret product identity sent as `User-Agent`.

/// Static public product identity sent to LLM providers.
#[derive(Debug, Clone, PartialEq)]
pub struct AppIdentity {
    pub product: String,
    pub version: String,
    pub url: String,
    /// Validated RFC 9110 product token appended after the comment.
    pub suffix: Option<String>,
}

impl Default for AppIdentity {
    fn default() -> Self {
        Self {
            product: "deepseek-harness".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            url: "https://github.com/deepseek-ai/deepseek-harness".to_string(),
            suffix: launch_suffix().ok().flatten(),
        }
    }
}

/// Read `DSH_USER_AGENT_SUFFIX` on each use (a launcher may set it after load).
/// Returns `None` when unset or empty, `Err` when it is not a product token.
pub fn launch_suffix() -> Result<Option<String>, String> {
    let Ok(value) = std::env::var("DSH_USER_AGENT_SUFFIX") else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if !is_product_token(&value) {
        return Err(format!(
            "DSH_USER_AGENT_SUFFIX must be an RFC 9110 product token; got {value:?}"
        ));
    }
    Ok(Some(value))
}

/// RFC 9110 §10.1.5 `product = token ["/" product-version]`.
pub fn is_product_token(value: &str) -> bool {
    let token = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'.'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'|'
                            | b'~'
                            | b'-'
                    )
            })
    };
    let mut parts = value.splitn(2, '/');
    let product = parts.next().is_some_and(token);
    match parts.next() {
        Some(version) => product && token(version),
        None => product,
    }
}

/// Render `product/version (+url)` plus ` suffix` when present.
pub fn user_agent(identity: &AppIdentity) -> String {
    let base = format!(
        "{}/{} (+{})",
        identity.product, identity.version, identity.url
    );
    match &identity.suffix {
        Some(suffix) => format!("{base} {suffix}"),
        None => base,
    }
}

/// Build the attribution headers an adapter must send on every request.
/// Header names are lowercase; omission cannot suppress attribution.
pub fn attribution_headers(identity: &AppIdentity) -> Vec<(String, String)> {
    vec![("user-agent".to_string(), user_agent(identity))]
}
