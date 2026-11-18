//! Pure Rust port of `@deepseek-ai/dsh-goal/fold`.
//!
//! Source: `packages/goal/goal/src/fold.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Strict replay decoder and fold for durable `goal/change` events. This is
//! the read-side validator; the writable `GoalStore` is left untouched.

use std::collections::HashSet;

use crate::services::sessions::SessionEvent;

pub const GOAL_CHANGE_VERSION: u32 = 1;
const MAX_SAFE: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalPhase {
    Active,
    Paused,
    Blocked,
    Complete,
}

impl GoalPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Complete => "complete",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GoalBlockReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GoalSnapshot {
    pub id: String,
    pub revision: u64,
    pub objective: String,
    pub phase: GoalPhase,
    pub max_goal_rounds: u64,
    pub blocked_reason: Option<GoalBlockReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalRef {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GoalChange {
    Snapshot {
        operation: String,
        goal: GoalSnapshot,
        rounds_started: u64,
        created_at: u64,
        updated_at: u64,
    },
    Clear {
        cleared: GoalRef,
        cleared_at: u64,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GoalFoldState {
    pub goal: Option<GoalSnapshot>,
    pub rounds_started: u64,
    pub created_at: Option<u64>,
    pub updated_at: Option<u64>,
    pub last_ref: Option<GoalRef>,
    pub seen_goal_ids: HashSet<String>,
}

pub fn empty_goal_fold_state() -> GoalFoldState {
    GoalFoldState::default()
}

type FoldResult<T> = std::result::Result<T, String>;

fn sorted_keys(value: &serde_json::Map<String, serde_json::Value>) -> Vec<&str> {
    let mut keys: Vec<&str> = value.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

fn positive_integer(value: &serde_json::Value, field: &str) -> FoldResult<u64> {
    let number = value
        .as_i64()
        .ok_or_else(|| format!("goal change {field} must be a safe integer"))?;
    if !(1..=MAX_SAFE).contains(&number) {
        return Err(format!(
            "goal change {field} must be a positive safe integer"
        ));
    }
    Ok(number as u64)
}

fn non_negative_integer(value: &serde_json::Value, field: &str) -> FoldResult<u64> {
    let number = value
        .as_i64()
        .ok_or_else(|| format!("goal change {field} must be a safe integer"))?;
    if !(0..=MAX_SAFE).contains(&number) {
        return Err(format!(
            "goal change {field} must be a non-negative safe integer"
        ));
    }
    Ok(number as u64)
}

fn decode_block_reason(value: &serde_json::Value) -> FoldResult<GoalBlockReason> {
    let object = value
        .as_object()
        .ok_or("goal change goal.blockedReason must be a record")?;
    if sorted_keys(object) != ["code", "message"] {
        return Err(
            "goal change goal.blockedReason must have exactly code and message fields".to_string(),
        );
    }
    let code = object["code"]
        .as_str()
        .ok_or("goal change goal.blockedReason.code must be lower-kebab-case")?;
    let valid_code = code.split('-').all(|part| {
        !part.is_empty()
            && part.bytes().enumerate().all(|(i, b)| {
                if i == 0 {
                    b.is_ascii_lowercase()
                } else {
                    b.is_ascii_lowercase() || b.is_ascii_digit()
                }
            })
    }) && code.as_bytes().first().is_some_and(u8::is_ascii_lowercase);
    if !valid_code {
        return Err("goal change goal.blockedReason.code must be lower-kebab-case".to_string());
    }
    let message = object["message"]
        .as_str()
        .ok_or("goal change goal.blockedReason.message must be a string")?;
    if message.trim().is_empty() || message.trim() != message {
        return Err(
            "goal change goal.blockedReason.message must be non-empty and normalized".to_string(),
        );
    }
    Ok(GoalBlockReason {
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn decode_ref(value: &serde_json::Value) -> FoldResult<GoalRef> {
    let object = value
        .as_object()
        .ok_or("goal clear tombstone must be a record")?;
    if sorted_keys(object) != ["id", "revision"] {
        return Err("goal clear tombstone must have exactly id and revision fields".to_string());
    }
    let id = object["id"]
        .as_str()
        .ok_or("goal clear tombstone id must be a non-empty string")?;
    if id.is_empty() {
        return Err("goal clear tombstone id must be a non-empty string".to_string());
    }
    Ok(GoalRef {
        id: id.to_string(),
        revision: positive_integer(&object["revision"], "cleared.revision")?,
    })
}

fn decode_snapshot(value: &serde_json::Value) -> FoldResult<GoalSnapshot> {
    let object = value
        .as_object()
        .ok_or("goal change goal must be a record")?;
    let id = object
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("goal change goal.id must be a non-empty string")?;
    if id.is_empty() {
        return Err("goal change goal.id must be a non-empty string".to_string());
    }
    let objective = object
        .get("objective")
        .and_then(|v| v.as_str())
        .ok_or("goal change goal.objective must be a string")?;
    if objective.trim().is_empty() || objective.trim() != objective {
        return Err("goal change goal.objective must be non-empty and normalized".to_string());
    }
    let phase_str = object
        .get("phase")
        .and_then(|v| v.as_str())
        .ok_or("goal change goal.phase is invalid")?;
    let phase = match phase_str {
        "active" => GoalPhase::Active,
        "paused" => GoalPhase::Paused,
        "blocked" => GoalPhase::Blocked,
        "complete" => GoalPhase::Complete,
        _ => return Err("goal change goal.phase is invalid".to_string()),
    };
    let expected: Vec<&str> = if phase == GoalPhase::Blocked {
        vec![
            "blockedReason",
            "id",
            "maxGoalRounds",
            "objective",
            "phase",
            "revision",
        ]
    } else {
        vec!["id", "maxGoalRounds", "objective", "phase", "revision"]
    };
    if sorted_keys(object) != expected {
        return Err(format!(
            "goal change goal for phase {} must have exactly {} fields",
            phase.as_str(),
            expected.join(",")
        ));
    }
    let blocked_reason = if phase == GoalPhase::Blocked {
        Some(decode_block_reason(&object["blockedReason"])?)
    } else {
        None
    };
    Ok(GoalSnapshot {
        id: id.to_string(),
        revision: positive_integer(&object["revision"], "goal.revision")?,
        objective: objective.to_string(),
        phase,
        max_goal_rounds: positive_integer(&object["maxGoalRounds"], "goal.maxGoalRounds")?,
        blocked_reason,
    })
}

/// Decode a value that declares itself as a goal change. Unrelated values
/// return `Ok(None)`; malformed goal changes fail loudly.
pub fn decode_goal_change(value: &serde_json::Value) -> FoldResult<Option<GoalChange>> {
    let object = match value.as_object() {
        Some(object) if object.get("kind").and_then(|v| v.as_str()) == Some("goal/change") => {
            object
        }
        _ => return Ok(None),
    };
    let version = object
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or("unsupported goal change version")?;
    if version != GOAL_CHANGE_VERSION as u64 {
        return Err(format!("unsupported goal change version {version}"));
    }
    let operation = object
        .get("operation")
        .and_then(|v| v.as_str())
        .ok_or("goal change operation is invalid")?;
    if operation == "clear" {
        let allowed = ["cleared", "clearedAt", "kind", "operation", "version"];
        if sorted_keys(object) != allowed {
            return Err("goal clear change must have exactly cleared,clearedAt,kind,operation,version fields".to_string());
        }
        return Ok(Some(GoalChange::Clear {
            cleared: decode_ref(&object["cleared"])?,
            cleared_at: non_negative_integer(&object["clearedAt"], "clearedAt")?,
        }));
    }
    if !["create", "edit", "pause", "resume", "complete", "block"].contains(&operation) {
        return Err("goal change operation is invalid".to_string());
    }
    let allowed = [
        "createdAt",
        "goal",
        "kind",
        "operation",
        "roundsStarted",
        "updatedAt",
        "version",
    ];
    if sorted_keys(object) != allowed {
        return Err("goal snapshot change must have exactly createdAt,goal,kind,operation,roundsStarted,updatedAt,version fields".to_string());
    }
    let created_at = non_negative_integer(&object["createdAt"], "createdAt")?;
    let updated_at = non_negative_integer(&object["updatedAt"], "updatedAt")?;
    if updated_at < created_at {
        return Err("goal change updatedAt cannot precede createdAt".to_string());
    }
    Ok(Some(GoalChange::Snapshot {
        operation: operation.to_string(),
        goal: decode_snapshot(&object["goal"])?,
        rounds_started: non_negative_integer(&object["roundsStarted"], "roundsStarted")?,
        created_at,
        updated_at,
    }))
}

/// Return the revision identity carried by a snapshot or tombstone.
pub fn goal_change_ref(change: &GoalChange) -> GoalRef {
    match change {
        GoalChange::Clear { cleared, .. } => cleared.clone(),
        GoalChange::Snapshot { goal, .. } => GoalRef {
            id: goal.id.clone(),
            revision: goal.revision,
        },
    }
}

fn require_next_revision(current: &GoalSnapshot, next: GoalRef, operation: &str) -> FoldResult<()> {
    if next.id != current.id || next.revision != current.revision + 1 {
        return Err(format!(
            "goal {operation} must advance the current goal by one revision"
        ));
    }
    Ok(())
}

fn same_definition(current: &GoalSnapshot, next: &GoalSnapshot, operation: &str) -> FoldResult<()> {
    if next.objective != current.objective || next.max_goal_rounds != current.max_goal_rounds {
        return Err(format!(
            "goal {operation} cannot change objective or maxGoalRounds"
        ));
    }
    Ok(())
}

/// Validate and apply one decoded change to a mutable accumulator.
pub fn apply_goal_change(state: &mut GoalFoldState, change: GoalChange) -> FoldResult<()> {
    let reference = goal_change_ref(&change);
    match &change {
        GoalChange::Clear {
            cleared,
            cleared_at,
        } => {
            let current = state
                .goal
                .as_ref()
                .ok_or("goal clear requires a current goal")?
                .clone();
            require_next_revision(&current, cleared.clone(), "clear")?;
            let updated = state
                .updated_at
                .ok_or("current goal fold lacks updatedAt")?;
            if *cleared_at < updated {
                return Err(
                    "goal clear timestamp cannot precede the current goal update".to_string(),
                );
            }
            state.goal = None;
            state.rounds_started = 0;
            state.created_at = None;
            state.updated_at = None;
        }
        GoalChange::Snapshot {
            operation,
            goal,
            rounds_started,
            created_at,
            updated_at,
        } => {
            if operation == "create" {
                if goal.revision != 1
                    || goal.phase != GoalPhase::Active
                    || *rounds_started != 0
                    || state
                        .goal
                        .as_ref()
                        .is_some_and(|current| current.phase != GoalPhase::Complete)
                    || state.seen_goal_ids.contains(&goal.id)
                {
                    return Err(
                        "goal create requires a fresh active revision-one goal with zero rounds"
                            .to_string(),
                    );
                }
                state.seen_goal_ids.insert(goal.id.clone());
            } else {
                let current = state
                    .goal
                    .as_ref()
                    .ok_or_else(|| format!("goal {operation} requires a current goal"))?
                    .clone();
                require_next_revision(&current, reference.clone(), operation)?;
                let fold_created = state
                    .created_at
                    .ok_or("current goal fold lacks createdAt")?;
                let fold_updated = state
                    .updated_at
                    .ok_or("current goal fold lacks updatedAt")?;
                if *created_at != fold_created
                    || *updated_at < fold_updated
                    || *rounds_started != state.rounds_started
                {
                    return Err(format!(
                        "goal {operation} does not preserve the current counters and timestamps"
                    ));
                }
                match operation.as_str() {
                    "edit" => {
                        if goal.phase != current.phase
                            || goal.blocked_reason != current.blocked_reason
                        {
                            return Err(
                                "goal edit cannot change phase or blocked reason".to_string()
                            );
                        }
                    }
                    "pause" => {
                        same_definition(&current, goal, operation)?;
                        if current.phase != GoalPhase::Active || goal.phase != GoalPhase::Paused {
                            return Err("goal pause has an invalid phase transition".to_string());
                        }
                    }
                    "resume" => {
                        same_definition(&current, goal, operation)?;
                        if !matches!(
                            current.phase,
                            GoalPhase::Active | GoalPhase::Paused | GoalPhase::Blocked
                        ) || goal.phase != GoalPhase::Active
                            || state.rounds_started >= goal.max_goal_rounds
                        {
                            return Err("goal resume has an invalid phase transition or exhausted round budget".to_string());
                        }
                    }
                    "complete" => {
                        same_definition(&current, goal, operation)?;
                        if current.phase == GoalPhase::Complete || goal.phase != GoalPhase::Complete
                        {
                            return Err("goal complete has an invalid phase transition".to_string());
                        }
                    }
                    "block" => {
                        same_definition(&current, goal, operation)?;
                        if current.phase != GoalPhase::Active || goal.phase != GoalPhase::Blocked {
                            return Err("goal block has an invalid phase transition".to_string());
                        }
                    }
                    other => return Err(format!("unknown goal snapshot operation {other}")),
                }
            }
            state.goal = Some(goal.clone());
            state.rounds_started = *rounds_started;
            state.created_at = Some(*created_at);
            state.updated_at = Some(*updated_at);
        }
    }
    state.last_ref = Some(reference);
    Ok(())
}

/// Apply one session event to the strict durable goal fold.
pub fn apply_goal_event(state: &mut GoalFoldState, event: &SessionEvent) -> FoldResult<()> {
    if event.event_type == "goal/change" {
        let Some(change) = decode_goal_change(&event.data)? else {
            return Err(format!(
                "goal change at session event {} has an invalid kind",
                event.seq
            ));
        };
        return apply_goal_change(state, change);
    }
    if event.event_type == "user/message" {
        let source = &event.data["source"];
        if source.get("kind").and_then(|v| v.as_str()) != Some("goal") {
            return Ok(());
        }
        let current = match state.goal.as_ref() {
            Some(current) if current.phase == GoalPhase::Active => current.clone(),
            _ => {
                return Err(format!(
                    "goal round at session event {} is not the next admitted round of the active goal",
                    event.seq
                ));
            }
        };
        let goal_id = source.get("goalId").and_then(|v| v.as_str());
        let revision = source.get("revision").and_then(|v| v.as_u64());
        let round = source.get("round").and_then(|v| v.as_u64());
        if goal_id != Some(current.id.as_str())
            || revision != Some(current.revision)
            || round != Some(state.rounds_started + 1)
            || round.is_none_or(|r| r > current.max_goal_rounds)
        {
            return Err(format!(
                "goal round at session event {} is not the next admitted round of the active goal",
                event.seq
            ));
        }
        state.rounds_started = round.unwrap_or_default();
    }
    Ok(())
}

/// Fold current goal state from a contiguous session event log.
pub fn fold_goal(events: &[SessionEvent]) -> FoldResult<GoalFoldState> {
    let mut state = empty_goal_fold_state();
    for event in events {
        apply_goal_event(&mut state, event)?;
    }
    Ok(state)
}
