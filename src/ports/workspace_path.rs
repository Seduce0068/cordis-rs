//! Pure Rust port of `@deepseek-ai/dsh-util-workspace-path`.
//!
//! Source: `packages/util/workspace-path/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! String-based path display, resolution and file-address helpers.

/// Reject control characters (C0/C1, bidi overrides) and separators in one
/// path segment. Returns the offending character for diagnostics.
pub fn validate_segment(segment: &str) -> std::result::Result<(), char> {
    if segment.is_empty() || segment == "." || segment == ".." {
        return Err('\0');
    }
    segment
        .chars()
        .find(|ch| {
            *ch == '/'
                || *ch == '\\'
                || ('\u{0000}'..='\u{001f}').contains(ch)
                || ('\u{007f}'..='\u{009f}').contains(ch)
                || ('\u{202a}'..='\u{202e}').contains(ch)
                || ('\u{2066}'..='\u{2069}').contains(ch)
        })
        .map_or(Ok(()), Err)
}

/// True for Windows drive (`C:\`) or UNC (`\\`) style paths.
pub fn is_windows_style_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
        || bytes.starts_with(b"\\\\")
}

/// True for POSIX `/` or Windows-style absolute paths.
pub fn is_absolute_workspace_path(path: &str) -> bool {
    path.starts_with('/') || is_windows_style_path(path)
}

/// Resolve a workspace-relative path against a cwd, dropping leading `./`.
pub fn resolve_workspace_path(cwd: Option<&str>, path: &str) -> String {
    if is_absolute_workspace_path(path) {
        return path.to_string();
    }
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return path.to_string();
    };
    let separator = if is_windows_style_path(cwd) && cwd.contains('\\') {
        '\\'
    } else {
        '/'
    };
    let base = cwd.trim_end_matches(['/', '\\']);
    let relative = path.trim_start_matches(['.', '/', '\\']);
    format!("{base}{separator}{relative}")
}

/// Abbreviate a POSIX home directory to `~` for display.
pub fn abbreviate_home_path(path: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|home| !home.is_empty()) else {
        return path.to_string();
    };
    if is_windows_style_path(path) || is_windows_style_path(home) {
        return path.to_string();
    }
    let root = home.trim_end_matches('/');
    if root.is_empty() || root == "/" {
        return path.to_string();
    }
    if path.trim_end_matches('/') == root {
        return "~".to_string();
    }
    if let Some(rest) = path.strip_prefix(&format!("{root}/")) {
        return format!("~/{rest}");
    }
    path.to_string()
}

/// Final non-empty segment of a path (POSIX or Windows separators).
pub fn workspace_title_of(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let cut = trimmed
        .rfind(['/', '\\'])
        .map(|index| index + 1)
        .unwrap_or(0);
    trimmed[cut..].to_string()
}

/// Split a path into directory prefix (including trailing separator) and name.
pub fn path_parts_of(path: &str) -> (String, String) {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return (String::new(), path.to_string());
    }
    let cut = trimmed
        .rfind(['/', '\\'])
        .map(|index| index + 1)
        .unwrap_or(0);
    (trimmed[..cut].to_string(), trimmed[cut..].to_string())
}

/// Strip the workspace root from a rooted path (display only).
pub fn relativize_to_cwd(text: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return text.to_string();
    };
    let root = cwd.trim_end_matches(['/', '\\']);
    for separator in ['/', '\\'] {
        let prefix = format!("{root}{separator}");
        if let Some(rest) = text.strip_prefix(&prefix) {
            return rest.to_string();
        }
    }
    text.to_string()
}

fn encode_segment(value: &str) -> String {
    // Percent-encode bytes outside the unreserved set (session ids and path
    // segments are untrusted).
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn encode_path(value: &str) -> String {
    value
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Build a `dsh-resource://file/session/...` address for a path.
pub fn file_address_for(session_id: &str, cwd: Option<&str>, path: &str) -> String {
    const PREFIX: &str = "dsh-resource://file/";
    let normalized = path.replace('\\', "/");
    let relative = if is_absolute_workspace_path(&normalized) {
        let root = cwd
            .map(|cwd| cwd.replace('\\', "/").trim_end_matches('/').to_string())
            .unwrap_or_default();
        if !root.is_empty() && normalized == root {
            String::new()
        } else if !root.is_empty() {
            normalized
                .strip_prefix(&format!("{root}/"))
                .unwrap_or(&normalized)
                .to_string()
        } else {
            normalized.clone()
        }
    } else {
        normalized.clone()
    };
    let cleaned = relative.trim_start_matches("./");
    format!(
        "{PREFIX}session/{}/{}",
        encode_segment(session_id),
        encode_path(cleaned)
    )
}
