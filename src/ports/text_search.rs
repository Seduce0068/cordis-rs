//! Pure Rust port of `@deepseek-ai/dsh-util/text-search`.
//!
//! Source: `packages/util/text-search/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! In-memory fuzzy ranking over arbitrary items with four score tiers.

/// Which fields of an item are searched and how.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchKeys<T> {
    pub fuzzy: Vec<T>,
    pub substring: Vec<T>,
}

impl<T> SearchKeys<T> {
    pub fn new(fuzzy: Vec<T>, substring: Vec<T>) -> Self {
        Self { fuzzy, substring }
    }
}

/// Normalize for matching: lowercase, drop combining marks, fold fullwidth
/// ASCII and common stroke characters. Unicode NFKD proper needs a crate; this
/// is a std-only approximation covering the documented cases.
pub fn normalize(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        // Combining diacritical marks (U+0300..U+036F) are ignored.
        if ('\u{0300}'..='\u{036f}').contains(&ch) {
            continue;
        }
        let mapped = match ch {
            // Fullwidth ASCII → ASCII.
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(ch as u32 - 0xfee0).unwrap_or(ch),
            // Selected stroke letters (common in names).
            '\u{00f8}' | '\u{00d8}' => 'o',
            '\u{0142}' | '\u{0141}' => 'l',
            '\u{0111}' | '\u{0110}' => 'd',
            other => other,
        };
        for lowered in mapped.to_lowercase() {
            out.push(lowered);
        }
    }
    out
}

fn score_term(term: &str, text: &str) -> u32 {
    if text.is_empty() || term.is_empty() {
        return 0;
    }
    // Tier 4: fuzzy prefix.
    if text.starts_with(term) {
        return 4;
    }
    // Tier 3: fuzzy substring.
    if text.contains(term) {
        return 3;
    }
    // Tier 2: fuzzy subsequence, starting at a word boundary only.
    let chars: Vec<char> = text.chars().collect();
    let term_chars: Vec<char> = term.chars().collect();
    for start in 0..chars.len() {
        if start > 0 && !chars[start - 1].is_whitespace() {
            continue; // word-head only
        }
        let mut cursor = start;
        let mut matched = true;
        for wanted in &term_chars {
            while cursor < chars.len() && chars[cursor] != *wanted {
                cursor += 1;
            }
            if cursor == chars.len() {
                matched = false;
                break;
            }
            cursor += 1;
        }
        if matched {
            return 2;
        }
    }
    // Tier 1: plain substring (already covered by tier 3 for normalized text).
    if text.contains(term) {
        return 1;
    }
    0
}

fn score_item(terms: &[String], fuzzy: &[String], substring: &[String]) -> u32 {
    if terms.is_empty() {
        return 1; // empty query matches everything
    }
    let mut total = 0;
    for term in terms {
        let mut best = 0;
        for field in fuzzy.iter().chain(substring.iter()) {
            best = best.max(score_term(term, field));
        }
        if best == 0 {
            return 0; // all terms must match (AND)
        }
        total += best;
    }
    total
}

/// An in-memory search index over prepared string keys.
#[derive(Debug, Clone)]
pub struct TextSearchIndex<T: Clone> {
    items: Vec<T>,
    fuzzy: Vec<Vec<String>>,
    substring: Vec<Vec<String>>,
}

impl<T: Clone> TextSearchIndex<T> {
    /// Build an index from items and per-item key selectors.
    pub fn build(items: Vec<T>, keys: impl Fn(&T) -> SearchKeys<String>) -> Self {
        let mut fuzzy = Vec::new();
        let mut substring = Vec::new();
        for item in &items {
            let key = keys(item);
            fuzzy.push(
                key.fuzzy
                    .into_iter()
                    .map(|value| normalize(&value))
                    .collect(),
            );
            substring.push(
                key.substring
                    .into_iter()
                    .map(|value| normalize(&value))
                    .collect(),
            );
        }
        Self {
            items,
            fuzzy,
            substring,
        }
    }

    /// Rebuild with new items while keeping the same key selector.
    pub fn update(self, items: Vec<T>, keys: impl Fn(&T) -> SearchKeys<String>) -> Self {
        Self::build(items, keys)
    }

    /// Rank items by descending score, preserving build order for ties.
    pub fn search(&self, query: &str) -> Vec<(usize, u32, &T)> {
        let terms: Vec<String> = normalize(query)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let mut scored: Vec<(usize, u32)> = self
            .items
            .iter()
            .enumerate()
            .map(|(index, _)| {
                (
                    index,
                    score_item(&terms, &self.fuzzy[index], &self.substring[index]),
                )
            })
            .filter(|(_, score)| *score > 0)
            .collect();
        scored.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        scored
            .into_iter()
            .map(|(index, score)| (index, score, &self.items[index]))
            .collect()
    }
}
