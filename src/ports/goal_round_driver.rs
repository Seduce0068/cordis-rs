//! Reusable Rust port of `@deepseek-ai/dsh-goal-round-driver` (phase 1).
//!
//! Source: `packages/goal/goal-round-driver/src/` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Drives an active goal by appending round prompts to its session.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::services::agents::AgentRegistry;
use crate::services::goals::{GoalStatus, GoalStore};
use crate::services::sessions::SessionStore;
use crate::{Context, Result};

/// Outcome of one `drive` call.
#[derive(Debug, Clone, PartialEq)]
pub enum DriveOutcome {
    Noop,
    Queued { round: u32 },
    Blocked,
}

/// Internal action decided under the state lock; the durable append is then
/// performed after dropping the guard so observers can re-enter the driver
/// (swarm review R11-LC01).
enum DriveAction {
    Noop,
    Blocked {
        goal_id: String,
        round: u32,
    },
    Queue {
        goal_id: String,
        round: u32,
        text: String,
        session_id: String,
    },
}

#[derive(Debug, Clone)]
struct RoundAttempt {
    goal_revision: u64,
}

#[derive(Debug, Default)]
struct DriverState {
    attempt: Option<RoundAttempt>,
    rounds: HashMap<String, u32>,
    blocked: HashSet<String>,
    competing: bool,
}

/// The goal round driver service.
#[derive(Clone)]
pub struct GoalRoundDriver {
    state: Arc<RwLock<DriverState>>,
}

impl Default for GoalRoundDriver {
    fn default() -> Self {
        Self {
            state: Arc::new(RwLock::new(DriverState::default())),
        }
    }
}

impl GoalRoundDriver {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(DriverState::default())),
        }
    }

    /// Render the round prompt (upstream `renderGoalRoundPrompt` wording).
    pub fn prompt(objective: &str, round: u32, max_rounds: u32) -> String {
        format!(
            "<goal_round>\nObjective: {objective}\nRound: {round}/{max_rounds}\n\nContinue working toward the objective in this same session. Before claiming completion, gather evidence that the whole objective is achieved, then report.\n</goal_round>"
        )
    }

    /// Drive one round for an agent's active goal. Call on step end, goal
    /// change, or explicitly from the host.
    pub fn drive(&self, ctx: &Context, agent_id: &str) -> Result<DriveOutcome> {
        let Some(agents) = ctx.get::<AgentRegistry>("agents") else {
            return Ok(DriveOutcome::Noop);
        };
        let Some(agent) = agents.get(agent_id) else {
            return Ok(DriveOutcome::Noop);
        };
        let Some(goals) = ctx.get::<GoalStore>("goals") else {
            return Ok(DriveOutcome::Noop);
        };
        let Some(goal) = goals.get() else {
            return Ok(DriveOutcome::Noop);
        };
        let Some(sessions) = ctx.get::<SessionStore>("sessions") else {
            return Ok(DriveOutcome::Noop);
        };

        let goal_id = format!("agent-{agent_id}");
        // Phase 1: decide and commit state under the lock. Round numbering and
        // competing reset must stay inside the critical section (R11-LC01).
        let action = {
            let mut state = self.state.write().expect("goal driver state poisoned");
            if goal.status != GoalStatus::Active || state.blocked.contains(&goal_id) {
                state.attempt = None;
                DriveAction::Noop
            } else if state.attempt.is_some() && !state.competing {
                DriveAction::Noop
            } else if state.competing {
                state.attempt = None;
                state.competing = false;
                DriveAction::Noop
            } else {
                let round = state.rounds.get(&goal_id).copied().unwrap_or(0) + 1;
                if round > goal.max_rounds {
                    state.blocked.insert(goal_id.clone());
                    state.attempt = None;
                    DriveAction::Blocked { goal_id, round }
                } else if let Some(attempt) = &state.attempt
                    && attempt.goal_revision != goal.revision
                {
                    state.attempt = None;
                    DriveAction::Noop
                } else {
                    state.attempt = Some(RoundAttempt {
                        goal_revision: goal.revision,
                    });
                    state.rounds.insert(goal_id.clone(), round);
                    DriveAction::Queue {
                        goal_id,
                        round,
                        text: Self::prompt(&goal.objective, round, goal.max_rounds),
                        session_id: agent.session_id.clone(),
                    }
                }
            }
        };

        // Phase 2: append with no state lock held so an observer calling
        // note_competing() cannot deadlock (swarm review R11-LC01).
        match action {
            DriveAction::Noop => Ok(DriveOutcome::Noop),
            DriveAction::Blocked { goal_id, round } => {
                if let Some(session) = sessions.get(&agent.session_id) {
                    session.append(
                        "goal/change",
                        serde_json::json!({
                            "kind": "goal/change", "version": 1, "operation": "block",
                            "goal": {
                                "id": goal_id, "revision": goal.revision + 1,
                                "objective": goal.objective, "phase": "blocked",
                                "maxGoalRounds": goal.max_rounds,
                                "blockedReason": {
                                    "code": "round-limit",
                                    "message": format!("round limit {} reached", goal.max_rounds)
                                }
                            },
                            "roundsStarted": round - 1, "createdAt": 0, "updatedAt": 0
                        }),
                    );
                }
                Ok(DriveOutcome::Blocked)
            }
            DriveAction::Queue {
                goal_id,
                round,
                text,
                session_id,
            } => {
                if let Some(session) = sessions.get(&session_id) {
                    session.append(
                        "user/message",
                        serde_json::json!({
                            "content": [{"type": "text", "text": text}],
                            "source": {"kind": "goal", "goalId": goal_id, "revision": goal.revision, "round": round}
                        }),
                    );
                }
                Ok(DriveOutcome::Queued { round })
            }
        }
    }

    /// Record a competing (human) input after a queued round. Approximates
    /// upstream inbox fencing: the next `drive` drops the attempt.
    pub fn note_competing(&self) {
        self.state
            .write()
            .expect("goal driver state poisoned")
            .competing = true;
    }
}

/// Plugin wrapper for the round driver.
pub struct GoalRoundDriverPlugin {
    pub driver: GoalRoundDriver,
}

#[async_trait::async_trait]
impl crate::Plugin for GoalRoundDriverPlugin {
    fn name(&self) -> &str {
        "goal-round-driver"
    }

    fn inject(&self) -> &[&str] {
        &["agents", "goals", "sessions"]
    }

    async fn apply(&self, _ctx: &Context) -> Result<()> {
        Ok(())
    }
}

/// Mount the driver as a service, retaining its fiber in an effect.
pub async fn mount(ctx: &Context) -> Result<GoalRoundDriver> {
    let driver = GoalRoundDriver::new();
    ctx.provide("goalRoundDriver", driver.clone()).await?;
    let fiber = crate::Fiber::mount_retained(
        ctx,
        Box::new(GoalRoundDriverPlugin {
            driver: driver.clone(),
        }),
    )
    .await?;
    ctx.effect(move |scope| {
        let fiber = fiber.clone();
        async move {
            scope.on_dispose(move || async move {
                fiber.unload().await?;
                Ok(())
            });
            Ok(())
        }
    })
    .await?;
    Ok(driver)
}
