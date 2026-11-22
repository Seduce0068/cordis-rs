//! Pure Rust port of `@deepseek-ai/dsh-fs-patch` (Codex-style patches).
//!
//! Source: `packages/fs/fs-patch/src/{parse,apply,types}.ts` in
//! deepseek-ai/deepseek-harness, read as untrusted reference data. The
//! parser/matcher design derives from OpenAI Codex rust-v0.158.0 apply-patch
//! (Apache-2.0, Copyright 2025 OpenAI); see THIRD_PARTY_NOTICES.md.

const BEGIN: &str = "*** Begin Patch";
const END: &str = "*** End Patch";
const EOF_MARKER: &str = "*** End of File";
const MOVE: &str = "*** Move to: ";
const ADD: &str = "*** Add File: ";
const DELETE: &str = "*** Delete File: ";
const UPDATE: &str = "*** Update File: ";

/// One contiguous update.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchHunk {
    pub line_number: usize,
    pub context: Option<String>,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub is_end_of_file: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatchOperation {
    Add {
        path: String,
        line_number: usize,
        content: String,
    },
    Delete {
        path: String,
        line_number: usize,
    },
    Update {
        path: String,
        line_number: usize,
        move_to: Option<String>,
        hunks: Vec<PatchHunk>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchParseError {
    pub line_number: usize,
    pub reason: String,
}

impl std::fmt::Display for PatchParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Invalid patch at line {}: {}",
            self.line_number, self.reason
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HunkApplyFailure {
    pub hunk_line_number: usize,
    pub stage: &'static str,
    pub search_start: usize,
    pub pattern: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApplyHunksResult {
    Ok { after: String },
    Failure { failure: HunkApplyFailure },
}

/// Unicode White_Space as used by Codex (BOM excluded).
fn is_white_space(ch: char) -> bool {
    ch.is_whitespace()
        || ch == '\u{85}'
        || matches!(ch, '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{feff}')
}

fn trim(value: &str) -> &str {
    value.trim_matches(is_white_space)
}

fn trim_end(value: &str) -> &str {
    value.trim_end_matches(is_white_space)
}

#[derive(Clone)]
struct InputLine {
    text: String,
    number: usize,
}

fn patch_lines(patch: &str) -> std::result::Result<Vec<InputLine>, PatchParseError> {
    let leading_len = patch.len() - patch.trim_start_matches(is_white_space).len();
    let offset = patch[..leading_len].matches('\n').count();
    let body = trim(patch);
    let mut lines: Vec<InputLine> = body
        .split('\n')
        .enumerate()
        .map(|(index, text)| InputLine {
            text: text.to_string(),
            number: offset + index + 1,
        })
        .collect();
    let first = lines[0].text.as_str();
    let last = lines[lines.len() - 1].text.as_str();
    if matches!(first, "<<EOF" | "<<'EOF'" | "<<\"EOF\"")
        && last.ends_with("EOF")
        && lines.len() >= 4
    {
        lines = lines[1..lines.len() - 1].to_vec();
    }
    if trim(&lines[0].text) != BEGIN {
        return Err(PatchParseError {
            line_number: lines[0].number,
            reason: format!("Expected '{BEGIN}'"),
        });
    }
    let end_index = lines.len() - 1;
    if trim(&lines[end_index].text) != END {
        return Err(PatchParseError {
            line_number: lines[end_index].number,
            reason: format!("Expected '{END}'"),
        });
    }
    for line in lines.iter_mut().take(end_index) {
        if line.text.ends_with('\r') {
            line.text.pop();
        }
    }
    Ok(lines)
}

fn empty_hunk(hunk: &PatchHunk) -> bool {
    hunk.old_lines.is_empty() && hunk.new_lines.is_empty()
}

fn ensure_update(
    operation: Option<&PatchOperation>,
    line_number: usize,
) -> std::result::Result<(), PatchParseError> {
    let Some(PatchOperation::Update {
        path,
        line_number: op_line,
        hunks,
        ..
    }) = operation
    else {
        return Ok(());
    };
    let Some(last) = hunks.last() else {
        return Err(PatchParseError {
            line_number: *op_line,
            reason: format!("Update for '{path}' requires at least one hunk"),
        });
    };
    if empty_hunk(last) {
        return Err(PatchParseError {
            line_number,
            reason: "Update hunk must contain context, added, or removed lines".to_string(),
        });
    }
    Ok(())
}

fn read_update(
    operation: &mut PatchOperation,
    line: &InputLine,
) -> std::result::Result<(), PatchParseError> {
    let text = trim_end(&line.text).to_string();
    let previous_eof = match operation {
        PatchOperation::Update { hunks, .. } => {
            hunks.last().is_some_and(|hunk| hunk.is_end_of_file)
        }
        _ => false,
    };
    let marker = text == "@@" || text.starts_with("@@ ");
    if previous_eof {
        if text.is_empty() {
            return Ok(());
        }
        if !marker {
            return Err(PatchParseError {
                line_number: line.number,
                reason: "Expected '@@' after '*** End of File'".to_string(),
            });
        }
    }
    let move_to = match operation {
        PatchOperation::Update { move_to, hunks, .. } => Some((move_to, hunks)),
        _ => None,
    };
    if let Some((move_to, hunks)) = move_to
        && hunks.is_empty()
        && move_to.is_none()
        && text.starts_with(MOVE)
    {
        *move_to = Some(text[MOVE.len()..].to_string());
        return Ok(());
    }
    if marker {
        if let PatchOperation::Update { hunks, .. } = operation {
            if hunks.last().is_some_and(empty_hunk) {
                return Err(PatchParseError {
                    line_number: line.number,
                    reason: "Update hunk must contain context, added, or removed lines".to_string(),
                });
            }
            let mut hunk = PatchHunk {
                line_number: line.number,
                context: None,
                old_lines: Vec::new(),
                new_lines: Vec::new(),
                is_end_of_file: false,
            };
            if let Some(context) = text.strip_prefix("@@ ") {
                hunk.context = Some(context.to_string());
            }
            hunks.push(hunk);
        }
        return Ok(());
    }
    if text == EOF_MARKER {
        if let PatchOperation::Update { hunks, .. } = operation
            && let Some(last) = hunks.last_mut()
        {
            if empty_hunk(last) {
                return Err(PatchParseError {
                    line_number: line.number,
                    reason: "Update hunk must contain context, added, or removed lines".to_string(),
                });
            }
            last.is_end_of_file = true;
        }
        return Ok(());
    }
    let prefix = line.text.chars().next();
    if !line.text.is_empty() && !matches!(prefix, Some(' ') | Some('+') | Some('-')) {
        return Err(PatchParseError {
            line_number: line.number,
            reason: "Expected a context (' '), added ('+'), removed ('-') line, or '@@'"
                .to_string(),
        });
    }
    let content = line.text.get(1..).unwrap_or("").to_string();
    if let PatchOperation::Update { hunks, .. } = operation {
        if hunks.is_empty() {
            hunks.push(PatchHunk {
                line_number: line.number,
                context: None,
                old_lines: Vec::new(),
                new_lines: Vec::new(),
                is_end_of_file: false,
            });
        }
        let hunk = hunks.last_mut().unwrap();
        match prefix {
            Some('+') => hunk.new_lines.push(content),
            Some('-') => hunk.old_lines.push(content),
            _ => {
                hunk.old_lines.push(content.clone());
                hunk.new_lines.push(content);
            }
        }
    }
    Ok(())
}

/// Parse a complete Codex-style patch without file I/O.
pub fn parse_patch(patch: &str) -> std::result::Result<Vec<PatchOperation>, PatchParseError> {
    let lines = patch_lines(patch)?;
    let mut operations: Vec<PatchOperation> = Vec::new();
    let mut ended = false;
    let last_index = lines.len() - 1;
    for (index, line) in lines.iter().enumerate().skip(1) {
        let header = if matches!(operations.last(), Some(PatchOperation::Update { .. }))
            && index != last_index
        {
            trim_end(&line.text).to_string()
        } else {
            trim(&line.text).to_string()
        };
        if ended && index != last_index {
            if !trim(&line.text).is_empty() {
                return Err(PatchParseError {
                    line_number: line.number,
                    reason: format!("Unexpected content after '{END}'"),
                });
            }
            continue;
        }
        if header == END {
            ensure_update(operations.last(), line.number)?;
            ended = true;
        } else if let Some(path) = header.strip_prefix(ADD) {
            ensure_update(operations.last(), line.number)?;
            operations.push(PatchOperation::Add {
                path: path.to_string(),
                line_number: line.number,
                content: String::new(),
            });
        } else if let Some(path) = header.strip_prefix(DELETE) {
            ensure_update(operations.last(), line.number)?;
            operations.push(PatchOperation::Delete {
                path: path.to_string(),
                line_number: line.number,
            });
        } else if let Some(path) = header.strip_prefix(UPDATE) {
            ensure_update(operations.last(), line.number)?;
            operations.push(PatchOperation::Update {
                path: path.to_string(),
                line_number: line.number,
                move_to: None,
                hunks: Vec::new(),
            });
        } else if operations.is_empty() && header.starts_with("*** Environment ID:") {
            return Err(PatchParseError {
                line_number: line.number,
                reason: "Environment ID headers are unsupported; the caller selects the filesystem provider".to_string(),
            });
        } else if let Some(PatchOperation::Add { content, .. }) = operations.last_mut() {
            if !line.text.starts_with('+') {
                return Err(PatchParseError {
                    line_number: line.number,
                    reason: "Expected an Add File, Delete File, or Update File header; added content must start with +".to_string(),
                });
            }
            content.push_str(&line.text[1..]);
            content.push('\n');
        } else if matches!(operations.last(), Some(PatchOperation::Update { .. })) {
            if let Some(op) = operations.last_mut() {
                read_update(op, line)?;
            }
        } else {
            return Err(PatchParseError {
                line_number: line.number,
                reason: "Expected an Add File, Delete File, or Update File header; added content must start with +".to_string(),
            });
        }
    }
    Ok(operations)
}

fn normalize_punctuation(value: &str) -> String {
    trim(value)
        .chars()
        .map(|ch| match ch {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{2018}'..='\u{201B}' => '\'',
            '\u{201C}'..='\u{201F}' => '"',
            '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

fn seek_sequence(lines: &[String], pattern: &[String], start: usize, eof: bool) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let last = lines.len() - pattern.len();
    let first = if eof { last } else { start };
    let passes: [fn(&str) -> String; 4] = [
        |value| value.to_string(),
        |value| trim_end(value).to_string(),
        |value| trim(value).to_string(),
        normalize_punctuation,
    ];
    for normalize in passes {
        let expected: Vec<String> = pattern.iter().map(|line| normalize(line)).collect();
        for index in first..=last {
            if expected
                .iter()
                .enumerate()
                .all(|(offset, line)| normalize(&lines[index + offset]) == *line)
            {
                return Some(index);
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
struct Replacement {
    start: usize,
    old_length: usize,
    new_lines: Vec<String>,
}

/// Apply hunks to LF-normalized text without I/O or input mutation.
pub fn apply_hunks(before: &str, hunks: &[PatchHunk]) -> ApplyHunksResult {
    let mut original: Vec<String> = before.split('\n').map(str::to_string).collect();
    if original.last().is_some_and(|line| line.is_empty()) {
        original.pop();
    }
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut line_index = 0usize;

    for hunk in hunks {
        if let Some(context) = &hunk.context {
            let pattern = vec![context.clone()];
            let Some(found) = seek_sequence(&original, &pattern, line_index, false) else {
                return ApplyHunksResult::Failure {
                    failure: HunkApplyFailure {
                        hunk_line_number: hunk.line_number,
                        stage: "context",
                        search_start: line_index,
                        pattern,
                    },
                };
            };
            line_index = found + 1;
        }
        if hunk.old_lines.is_empty() {
            let start = if original.last().is_some_and(|line| line.is_empty()) {
                original.len() - 1
            } else {
                original.len()
            };
            replacements.push(Replacement {
                start,
                old_length: 0,
                new_lines: hunk.new_lines.clone(),
            });
            continue;
        }

        let mut pattern = hunk.old_lines.clone();
        let mut new_lines = hunk.new_lines.clone();
        let mut found = seek_sequence(&original, &pattern, line_index, hunk.is_end_of_file);
        if found.is_none() && pattern.last().is_some_and(|line| line.is_empty()) {
            pattern.pop();
            if new_lines.last().is_some_and(|line| line.is_empty()) {
                new_lines.pop();
            }
            found = seek_sequence(&original, &pattern, line_index, hunk.is_end_of_file);
        }
        let Some(found) = found else {
            return ApplyHunksResult::Failure {
                failure: HunkApplyFailure {
                    hunk_line_number: hunk.line_number,
                    stage: "oldLines",
                    search_start: line_index,
                    pattern: hunk.old_lines.clone(),
                },
            };
        };
        replacements.push(Replacement {
            start: found,
            old_length: pattern.len(),
            new_lines,
        });
        line_index = found + pattern.len();
    }

    replacements.sort_by_key(|replacement| replacement.start);
    let mut lines = original;
    for replacement in replacements.into_iter().rev() {
        let mut next = Vec::with_capacity(lines.len());
        next.extend_from_slice(&lines[..replacement.start]);
        next.extend(replacement.new_lines);
        next.extend_from_slice(&lines[replacement.start + replacement.old_length..]);
        lines = next;
    }
    if lines.last().is_some_and(|line| !line.is_empty()) {
        lines.push(String::new());
    }
    ApplyHunksResult::Ok {
        after: lines.join("\n"),
    }
}

/// The `dsh-fs-patch` library port (pure helpers, no plugin registration).
pub struct FsPatchLibrary;

impl FsPatchLibrary {
    pub fn name() -> &'static str {
        "fs-patch"
    }
}
