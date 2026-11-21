//! `dsh-session-reference`: cross-session reference URI helpers.
//!
//! Upstream: `packages/context/session-reference/src/uri.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! A reference is a `dsh-session:` URI carrying a base64url-encoded JSON
//! session id. This port implements encode/decode and markdown mention
//! formatting/parsing.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::{Error, Result};

pub const SESSION_REFERENCE_SCHEME: &str = "dsh-session:";
/// Maximum encoded payload length accepted by the decoder (R3-IV10).
pub const MAX_REFERENCE_PAYLOAD_LEN: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionReference {
    pub session_id: String,
    pub label: String,
}

/// Encode a session id as a canonical `dsh-session:` URI.
pub fn encode_session_reference_uri(session_id: &str) -> String {
    let payload = URL_SAFE_NO_PAD.encode(session_id.as_bytes());
    format!("{SESSION_REFERENCE_SCHEME}{payload}")
}

/// Decode a canonical URI. Malformed input yields an error.
pub fn decode_session_reference_uri(uri: &str) -> std::result::Result<String, Error> {
    let payload = uri
        .strip_prefix(SESSION_REFERENCE_SCHEME)
        .ok_or_else(|| Error::Other("not a session reference URI".to_string()))?;
    // Bound the input before decoding and never echo the untrusted URI (it may
    // carry a session id) into the error message (swarm review R3-IV10).
    if payload.is_empty() || payload.len() > MAX_REFERENCE_PAYLOAD_LEN {
        return Err(Error::Other(format!(
            "invalid session reference payload length: {}",
            payload.len()
        )));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| Error::Other(format!("invalid session reference payload: {error}")))?;
    let value = String::from_utf8(bytes)
        .map_err(|error| Error::Other(format!("invalid session reference utf8: {error}")))?;
    // Upstream stores a JSON string; accept both quoted JSON and bare text.
    match serde_json::from_str::<String>(&value) {
        Ok(id) => Ok(id),
        Err(_) => {
            if value.trim() == value && !value.is_empty() {
                Ok(value)
            } else {
                Err(Error::Other("invalid session reference id".to_string()))
            }
        }
    }
}

/// Format a markdown mention for a session reference.
pub fn format_session_reference_mention(reference: &SessionReference) -> String {
    let safe_label: String = reference
        .label
        .chars()
        .flat_map(|ch| match ch {
            '[' | ']' | ')' | '\n' | '\r' => vec!['\\', ch],
            _ => vec![ch],
        })
        .collect();
    format!(
        "@[{}]({})",
        safe_label,
        encode_session_reference_uri(&reference.session_id)
    )
}

/// Expose the session reference helpers as a seam service.
#[derive(Default, Clone)]
pub struct SessionReferences;

impl SessionReferences {
    pub const fn new() -> Self {
        Self
    }

    pub fn encode(&self, session_id: &str) -> String {
        encode_session_reference_uri(session_id)
    }

    pub fn decode(&self, uri: &str) -> Result<String> {
        decode_session_reference_uri(uri)
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<SessionReferences> {
    let service = SessionReferences::new();
    ctx.provide("sessionReferences", service.clone()).await?;
    Ok(service)
}
