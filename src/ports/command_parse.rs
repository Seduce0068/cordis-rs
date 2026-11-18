//! Pure Rust port of `parseCommand` from `@deepseek-ai/dsh-commands`.
//!
//! Source: `packages/interaction/commands/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Strictly parses a slash command, preserving the separator whitespace in
//! `rawInput`.

/// A parsed slash command borrowing from the input line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedCommand<'a> {
    /// Lowercase command name without the leading slash.
    pub name: &'a str,
    /// Everything after the name, including the separator whitespace.
    pub raw_input: &'a str,
}

impl<'a> ParsedCommand<'a> {
    /// Name as registered in the local `CommandRegistry` (leading slash).
    pub fn registered_name(&self) -> String {
        format!("/{}", self.name)
    }
}

/// Parse a line like upstream's `^\/([a-z][a-z0-9_-]*)(?=$|[\t\n\r ])`.
/// Returns `None` when the line is not a valid slash command.
pub fn parse_command(line: &str) -> Option<ParsedCommand<'_>> {
    let bytes = line.as_bytes();
    // Exactly one leading slash; `//goal` is rejected (do not normalize).
    if bytes.first() != Some(&b'/') || bytes.get(1) == Some(&b'/') {
        return None;
    }
    let rest = &line[1..];
    if rest.is_empty() {
        return None;
    }
    let mut bytes = rest.bytes().enumerate().peekable();
    let first = bytes.peek().copied()?;
    let (_, first_byte) = first;
    if !first_byte.is_ascii_lowercase() {
        return None;
    }
    let mut end = 1usize; // index into `rest`
    while let Some((offset, byte)) = bytes.peek().copied() {
        let is_name =
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-';
        if !is_name {
            break;
        }
        end = offset + 1;
        bytes.next();
    }
    let name = &rest[..end];
    let remainder = &rest[end..];
    // The char after the name must be end-of-line or a separator.
    if let Some(separator) = remainder.as_bytes().first()
        && !matches!(separator, b'\t' | b'\n' | b'\r' | b' ')
    {
        return None;
    }
    Some(ParsedCommand {
        name,
        raw_input: remainder,
    })
}
