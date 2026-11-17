//! `dsh-hook-protocol` matcher and most-restrictive outcome merge.
//!
//! Upstream: `packages/experimental/hook-protocol/src/matcher.ts` and
//! `merge.ts` in deepseek-ai/deepseek-harness (MIT), read as untrusted
//! reference data. This port provides the loader-side matcher validation and
//! the fold that collapses matched hook outputs into one decision.

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergedDecision {
    None,
    Allow,
    Ask,
    Deny,
}

impl MergedDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Allow => 1,
            Self::Ask => 2,
            Self::Deny => 3,
        }
    }

    fn from_rank(rank: u8) -> Self {
        match rank {
            3 => Self::Deny,
            2 => Self::Ask,
            1 => Self::Allow,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HookOutput {
    pub decision: MergedDecision,
    pub reason: Option<String>,
    pub stop: bool,
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MergedHookOutcome {
    pub decision: MergedDecision,
    pub reasons: Vec<String>,
    pub stop: bool,
    pub contexts: Vec<String>,
}

/// Fold hook outputs into the most restrictive decision.
/// Precedence: `deny` > `ask` > `allow` > `none`. Only reasons belonging to
/// the winning rank are surfaced (upstream behaviour).
pub fn merge_hook_outputs(outputs: &[HookOutput]) -> MergedHookOutcome {
    let max_rank = outputs
        .iter()
        .map(|output| output.decision.rank())
        .max()
        .unwrap_or(0);
    let winning = MergedDecision::from_rank(max_rank);
    let reasons = outputs
        .iter()
        .filter(|output| output.decision == winning)
        .filter_map(|output| output.reason.clone())
        .collect();
    let contexts = outputs
        .iter()
        .filter_map(|output| output.context.clone())
        .collect();
    MergedHookOutcome {
        decision: winning,
        reasons,
        stop: outputs.iter().any(|output| output.stop),
        contexts,
    }
}

/// Match-all sentinels: absent, empty and `*`.
pub fn is_match_all(matcher: Option<&str>) -> bool {
    matches!(matcher, None | Some("") | Some("*"))
}

/// Literal-mode matcher: pipe-separated alternatives, exact match.
/// Regex-mode uses an unanchored `regex::Regex`; invalid patterns are
/// non-matches at runtime and rejected by `matcher_diagnostic` at config load.
pub fn matches(matcher: Option<&str>, query: &str, literal: bool) -> bool {
    if is_match_all(matcher) {
        return true;
    }
    let Some(pattern) = matcher else {
        return true;
    };
    if literal {
        pattern.split('|').any(|alternative| alternative == query)
    } else {
        match regex::Regex::new(pattern) {
            Ok(re) => re.is_match(query),
            Err(_) => false, // invalid regexes are non-matches at runtime
        }
    }
}

/// Validate a matcher before a config bridge accepts it. Returns `None` for
/// match-all sentinels and valid patterns, otherwise a stable diagnostic.
pub fn matcher_diagnostic(matcher: Option<&str>, literal: bool) -> Option<String> {
    if is_match_all(matcher) {
        return None;
    }
    let pattern = matcher?;
    if literal {
        return None; // any literal pattern is valid, including empty alternatives
    }
    regex::Regex::new(pattern)
        .err()
        .map(|error| format!("invalid regex matcher {pattern:?}: {error}"))
}

pub async fn provide(ctx: &crate::Context) -> Result<()> {
    // Library-only port: no service is registered.
    let _ = ctx;
    Ok(())
}
