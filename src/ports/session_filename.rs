//! Pure Rust port of `@deepseek-ai/dsh-session-format/filename`.
//!
//! Source: `packages/session/session-format/src/filename.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Canonical basenames for generation-addressed session JSONL logs.

/// Validate a non-negative safe-integer session format version.
pub fn session_format_version(value: i64, label: &str) -> std::result::Result<u32, String> {
    const MAX_SAFE: i64 = 9_007_199_254_740_991;
    if !(0..=MAX_SAFE).contains(&value) {
        return Err(format!(
            "{label}: expected a non-negative safe integer, got {value}"
        ));
    }
    u32::try_from(value).map_err(|_| format!("{label}: version {value} exceeds u32 range"))
}

/// Name the raw JSONL log of one immutable Session format generation.
pub fn session_format_log_filename(version: u32) -> String {
    if version == 0 {
        "session.jsonl".to_string()
    } else {
        format!("session.v{version}.jsonl")
    }
}

/// Read the generation named by one raw JSONL log basename. Temporary,
/// uppercase, leading-zero, `.v0` and compression-suffixed names are rejected.
pub fn parse_session_format_log_filename(filename: &str) -> Option<u32> {
    let rest = filename.strip_prefix("session")?;
    let version_str = if rest == ".jsonl" {
        return Some(0);
    } else {
        rest.strip_prefix(".v")?.strip_suffix(".jsonl")?
    };
    if version_str.is_empty()
        || !version_str.bytes().all(|byte| byte.is_ascii_digit())
        || version_str.starts_with('0')
    {
        return None;
    }
    version_str.parse::<u32>().ok()
}
