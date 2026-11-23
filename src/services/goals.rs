//! `ctx.goals`: event-sourced same-session goal state.
//!
//! Upstream: `packages/goal/goal/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! The store folds revisioned events into a live objective and enforces a
//! compare-and-set `revision` for edits (stale writes are rejected).

use std::sync::{Arc, RwLock};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub objective: String,
    pub max_rounds: u32,
    pub status: GoalStatus,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GoalError {
    NotFound,
    StaleRevision { expected: u64, actual: u64 },
    EmptyObjective,
}

impl std::fmt::Display for GoalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "no goal is set"),
            Self::StaleRevision { expected, actual } => {
                write!(
                    f,
                    "stale goal revision: expected {expected}, found {actual}"
                )
            }
            Self::EmptyObjective => write!(f, "goal objective must not be empty"),
        }
    }
}

impl std::error::Error for GoalError {}

#[derive(Debug, Clone)]
enum GoalEvent {
    Set {
        objective: String,
        max_rounds: u32,
    },
    Edit {
        objective: String,
        max_rounds: u32,
        revision: u64,
    },
    Pause {
        revision: u64,
    },
    Resume {
        revision: u64,
    },
    Complete {
        revision: u64,
    },
    Clear,
}

#[derive(Default, Clone)]
struct GoalState {
    goal: Option<Goal>,
    next_revision: u64,
    events: Vec<GoalEvent>,
}

/// Event-sourced goal store.
#[derive(Default, Clone)]
pub struct GoalStore {
    state: Arc<RwLock<GoalState>>,
}

impl GoalStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one event. The caller passes a view with an empty `events` vec,
    /// so `apply` must not inspect history; the real log is appended by
    /// `commit` only after validation succeeds (swarm review R16-RA04).
    fn apply(state: &mut GoalState, event: &GoalEvent) -> std::result::Result<(), GoalError> {
        match event {
            GoalEvent::Set {
                objective,
                max_rounds,
            } => {
                if objective.trim().is_empty() {
                    return Err(GoalError::EmptyObjective);
                }
                state.next_revision += 1;
                state.goal = Some(Goal {
                    objective: objective.clone(),
                    max_rounds: *max_rounds,
                    status: GoalStatus::Active,
                    revision: state.next_revision,
                });
            }
            GoalEvent::Edit {
                objective,
                max_rounds,
                revision,
            } => {
                if objective.trim().is_empty() {
                    return Err(GoalError::EmptyObjective);
                }
                let current = state.goal.as_ref().ok_or(GoalError::NotFound)?;
                if current.revision != *revision {
                    return Err(GoalError::StaleRevision {
                        expected: *revision,
                        actual: current.revision,
                    });
                }
                state.next_revision += 1;
                state.goal = Some(Goal {
                    objective: objective.clone(),
                    max_rounds: *max_rounds,
                    status: GoalStatus::Active,
                    revision: state.next_revision,
                });
            }
            GoalEvent::Pause { revision } => {
                let current = state.goal.as_ref().ok_or(GoalError::NotFound)?;
                if current.revision != *revision {
                    return Err(GoalError::StaleRevision {
                        expected: *revision,
                        actual: current.revision,
                    });
                }
                if let Some(goal) = &mut state.goal {
                    goal.status = GoalStatus::Paused;
                }
            }
            GoalEvent::Resume { revision } => {
                let current = state.goal.as_ref().ok_or(GoalError::NotFound)?;
                if current.revision != *revision {
                    return Err(GoalError::StaleRevision {
                        expected: *revision,
                        actual: current.revision,
                    });
                }
                if let Some(goal) = &mut state.goal {
                    goal.status = GoalStatus::Active;
                }
            }
            GoalEvent::Complete { revision } => {
                let current = state.goal.as_ref().ok_or(GoalError::NotFound)?;
                if current.revision != *revision {
                    return Err(GoalError::StaleRevision {
                        expected: *revision,
                        actual: current.revision,
                    });
                }
                if let Some(goal) = &mut state.goal {
                    goal.status = GoalStatus::Completed;
                }
            }
            GoalEvent::Clear => {
                state.goal = None;
            }
        }
        Ok(())
    }

    fn commit(&self, event: GoalEvent) -> std::result::Result<(), GoalError> {
        let mut state = self.state.write().expect("goal state poisoned");
        // Apply against a copy of just the goal fields, then swap on success:
        // validation errors leave the store unchanged (swarm review R14-RB09)
        // without deep-copying the whole event history (R15-AS07).
        let mut view = GoalState {
            goal: state.goal.clone(),
            next_revision: state.next_revision,
            events: Vec::new(),
        };
        Self::apply(&mut view, &event)?;
        state.goal = view.goal;
        state.next_revision = view.next_revision;
        state.events.push(event);
        Ok(())
    }

    pub fn set(
        &self,
        objective: impl Into<String>,
        max_rounds: u32,
    ) -> std::result::Result<Goal, GoalError> {
        self.commit(GoalEvent::Set {
            objective: objective.into(),
            max_rounds,
        })?;
        self.get().ok_or(GoalError::NotFound)
    }

    pub fn edit(
        &self,
        objective: impl Into<String>,
        max_rounds: u32,
        revision: u64,
    ) -> std::result::Result<Goal, GoalError> {
        self.commit(GoalEvent::Edit {
            objective: objective.into(),
            max_rounds,
            revision,
        })?;
        self.get().ok_or(GoalError::NotFound)
    }

    pub fn pause(&self, revision: u64) -> std::result::Result<Goal, GoalError> {
        self.commit(GoalEvent::Pause { revision })?;
        self.get().ok_or(GoalError::NotFound)
    }

    pub fn resume(&self, revision: u64) -> std::result::Result<Goal, GoalError> {
        self.commit(GoalEvent::Resume { revision })?;
        self.get().ok_or(GoalError::NotFound)
    }

    pub fn complete(&self, revision: u64) -> std::result::Result<Goal, GoalError> {
        self.commit(GoalEvent::Complete { revision })?;
        self.get().ok_or(GoalError::NotFound)
    }

    pub fn clear(&self) {
        let _ = self.try_clear();
    }

    /// Clear the goal, propagating a commit failure (swarm review R8-ER01).
    pub fn try_clear(&self) -> std::result::Result<(), GoalError> {
        self.commit(GoalEvent::Clear)
    }

    pub fn get(&self) -> Option<Goal> {
        self.state.read().expect("goal state poisoned").goal.clone()
    }

    pub fn event_count(&self) -> usize {
        self.state.read().expect("goal state poisoned").events.len()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<GoalStore> {
    provide_with_disposer(ctx).await.map(|(store, _)| store)
}

/// Register the service and return its disposer so callers can roll back a
/// partially completed mount (swarm review R14-RB07).
pub async fn provide_with_disposer(ctx: &crate::Context) -> Result<(GoalStore, crate::Disposer)> {
    let store = GoalStore::new();
    let disposer = ctx.provide("goals", store.clone()).await?;
    Ok((store, disposer))
}
