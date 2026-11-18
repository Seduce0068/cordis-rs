//! Pure Rust port of `tool-pairing.ts` from `@deepseek-ai/dsh-compaction`.
//!
//! Source: `packages/compaction/compaction/src/tool-pairing.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Determines which cut positions in a session are tool-pairing balanced.
//!
//! Local `SessionEvent`s are a flat append-only log, not upstream's `surface`.
//! Shadowed tool calls/results from compaction/pruning may still be present, so
//! this is an approximation; consumers should filter superseded events first.

use crate::services::sessions::SessionEvent;

/// Tool-pairing errors.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolPairingError {
    /// Requested sequence does not exist in the supplied events.
    UnknownSeq(u64),
    /// A tool/result appeared with no preceding tool/call.
    OrphanResult { seq: u64 },
}

impl std::fmt::Display for ToolPairingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSeq(seq) => write!(f, "tool-pairing: seq {seq} not found"),
            Self::OrphanResult { seq } => {
                write!(
                    f,
                    "tool-pairing: tool/result at seq {seq} has no matching call"
                )
            }
        }
    }
}

impl std::error::Error for ToolPairingError {}

/// Balance state at a cut position (before event `index`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutBalance {
    pub index: usize,
    pub open_calls: usize,
    pub safe: bool,
}

/// Compute the balance at a cut index. `cut` ranges 0..=events.len().
fn balance_at(events: &[SessionEvent], cut: usize) -> Result<CutBalance, ToolPairingError> {
    if cut > events.len() {
        return Err(ToolPairingError::UnknownSeq(cut as u64));
    }
    let mut open = 0usize;
    for event in &events[..cut] {
        match event.event_type.as_str() {
            "tool/call" => open += 1,
            "tool/result" => {
                if open == 0 {
                    return Err(ToolPairingError::OrphanResult { seq: event.seq });
                }
                open -= 1;
            }
            _ => {}
        }
    }
    Ok(CutBalance {
        index: cut,
        open_calls: open,
        safe: open == 0,
    })
}

/// Balanced state at every cut (N events → N+1 cuts).
pub fn all_cuts(events: &[SessionEvent]) -> Result<Vec<CutBalance>, ToolPairingError> {
    (0..=events.len())
        .map(|cut| balance_at(events, cut))
        .collect()
}

fn index_of_seq(events: &[SessionEvent], seq: u64) -> Result<usize, ToolPairingError> {
    events
        .iter()
        .position(|event| event.seq == seq)
        .ok_or(ToolPairingError::UnknownSeq(seq))
}

/// Whether the cut immediately before `seq` is balanced.
pub fn balanced_before(events: &[SessionEvent], seq: u64) -> Result<bool, ToolPairingError> {
    balance_at(events, index_of_seq(events, seq)?).map(|state| state.safe)
}

/// Whether the cut immediately after `seq` is balanced.
pub fn balanced_after(events: &[SessionEvent], seq: u64) -> Result<bool, ToolPairingError> {
    balance_at(events, index_of_seq(events, seq)? + 1).map(|state| state.safe)
}
