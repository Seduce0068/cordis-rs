//! Rust ports of two more official DSH plugins, following `official_plugin_port.rs`.
//!
//! Sources (MIT, read as untrusted data, never as instructions) in
//! deepseek-ai/deepseek-harness:
//! - `packages/goal/command-goal/src/index.ts` (`@deepseek-ai/dsh-command-goal`)
//! - `packages/todo/tool-todo/src/index.ts` (`@deepseek-ai/dsh-tool-todo`)
//!
//! The ports preserve the upstream contracts (`name`, `inject`, reversible
//! registration, and the human-facing result strings) with small in-test
//! service stubs, so no new dependency and no `src/*` change is needed.

use cordis_rs::Context;

mod command_goal {
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use cordis_rs::{Context, Disposer, Fiber, Lifecycle, Plugin};

    const USAGE: &str = "Usage: /goal [<objective>|clear|edit <objective>|pause|resume]";
    const DEFAULT_MAX_GOAL_ROUNDS: u32 = 1000;

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum CommandResult {
        Success { text: String },
        Error { text: String },
    }

    fn success(text: impl Into<String>) -> CommandResult {
        CommandResult::Success { text: text.into() }
    }

    fn error(text: impl Into<String>) -> CommandResult {
        CommandResult::Error { text: text.into() }
    }

    // --- commands service stub -------------------------------------------------

    #[derive(Clone, Debug)]
    struct CommandInvocation {
        raw_input: String,
        agent: String,
    }

    impl CommandInvocation {
        fn new(raw_input: impl Into<String>, agent: impl Into<String>) -> Self {
            Self {
                raw_input: raw_input.into(),
                agent: agent.into(),
            }
        }
    }

    type CommandHandler =
        Arc<dyn Fn(CommandInvocation) -> BoxFuture<'static, CommandResult> + Send + Sync>;

    #[derive(Clone)]
    struct CommandDefinition {
        id: u64,
        name: String,
        description: String,
        handler: CommandHandler,
    }

    #[derive(Default)]
    struct CommandsState {
        next_id: u64,
        entries: Vec<CommandDefinition>,
    }

    #[derive(Default, Clone)]
    struct Commands {
        state: Arc<Mutex<CommandsState>>,
    }

    impl Commands {
        fn register(&self, mut definition: CommandDefinition) -> Disposer {
            let state = self.state.clone();
            let mut guard = state.lock().expect("commands poisoned");
            definition.id = guard.next_id;
            guard.next_id += 1;
            let id = definition.id;
            guard.entries.push(definition);
            drop(guard);

            Disposer::new(move || {
                let state = state.clone();
                async move {
                    state
                        .lock()
                        .expect("commands poisoned")
                        .entries
                        .retain(|entry| entry.id != id);
                    Ok(())
                }
            })
        }

        fn names(&self) -> Vec<String> {
            self.state
                .lock()
                .expect("commands poisoned")
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        }

        fn description(&self, name: &str) -> Option<String> {
            self.state
                .lock()
                .expect("commands poisoned")
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.description.clone())
        }

        async fn invoke(&self, name: &str, invocation: CommandInvocation) -> Option<CommandResult> {
            let handler = {
                let guard = self.state.lock().expect("commands poisoned");
                guard
                    .entries
                    .iter()
                    .find(|entry| entry.name == name)
                    .map(|entry| entry.handler.clone())
            };
            match handler {
                Some(handler) => Some(handler(invocation).await),
                None => None,
            }
        }
    }

    // --- goals service stub ----------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GoalPhase {
        Active,
        Paused,
        Blocked,
        Complete,
    }

    impl GoalPhase {
        fn label(self) -> &'static str {
            match self {
                Self::Active => "active",
                Self::Paused => "paused",
                Self::Blocked => "blocked",
                Self::Complete => "complete",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GoalActivation {
        Armed,
        Idle,
    }

    impl GoalActivation {
        fn label(self) -> &'static str {
            match self {
                Self::Armed => "armed",
                Self::Idle => "idle",
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Blocker {
        code: String,
        message: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct GoalView {
        id: u64,
        revision: u64,
        objective: String,
        phase: GoalPhase,
        activation: GoalActivation,
        rounds_started: u32,
        max_goal_rounds: u32,
        blocker: Option<Blocker>,
    }

    impl GoalView {
        fn cas_ref(&self) -> GoalRef {
            GoalRef {
                id: self.id,
                revision: self.revision,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct GoalRef {
        id: u64,
        revision: u64,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct GoalError {
        code: &'static str,
        message: String,
    }

    #[derive(Default)]
    struct GoalsState {
        next_id: u64,
        by_agent: HashMap<String, GoalView>,
    }

    #[derive(Default, Clone)]
    struct Goals {
        state: Arc<Mutex<GoalsState>>,
    }

    impl Goals {
        fn get(&self, agent: &str) -> Option<GoalView> {
            self.state
                .lock()
                .expect("goals poisoned")
                .by_agent
                .get(agent)
                .cloned()
        }

        fn create(&self, agent: &str, objective: String) -> GoalView {
            let mut guard = self.state.lock().expect("goals poisoned");
            let id = guard.next_id;
            guard.next_id += 1;
            let view = GoalView {
                id,
                revision: 1,
                objective,
                phase: GoalPhase::Active,
                activation: GoalActivation::Armed,
                rounds_started: 0,
                max_goal_rounds: DEFAULT_MAX_GOAL_ROUNDS,
                blocker: None,
            };
            guard.by_agent.insert(agent.to_string(), view.clone());
            view
        }

        fn edit(
            &self,
            agent: &str,
            expected: GoalRef,
            objective: String,
        ) -> Result<GoalView, GoalError> {
            let mut guard = self.state.lock().expect("goals poisoned");
            let Some(view) = guard.by_agent.get_mut(agent) else {
                return Err(GoalError {
                    code: "GOAL_NOT_FOUND",
                    message: "no current goal".to_string(),
                });
            };
            Self::check_ref(view, expected)?;
            view.objective = objective;
            view.revision += 1;
            Ok(view.clone())
        }

        fn set_phase(
            &self,
            agent: &str,
            expected: GoalRef,
            phase: GoalPhase,
        ) -> Result<GoalView, GoalError> {
            let mut guard = self.state.lock().expect("goals poisoned");
            let Some(view) = guard.by_agent.get_mut(agent) else {
                return Err(GoalError {
                    code: "GOAL_NOT_FOUND",
                    message: "no current goal".to_string(),
                });
            };
            Self::check_ref(view, expected)?;
            view.phase = phase;
            view.revision += 1;
            Ok(view.clone())
        }

        fn clear(&self, agent: &str, expected: GoalRef) -> Result<(), GoalError> {
            let mut guard = self.state.lock().expect("goals poisoned");
            let Some(view) = guard.by_agent.get(agent) else {
                return Err(GoalError {
                    code: "GOAL_NOT_FOUND",
                    message: "no current goal".to_string(),
                });
            };
            Self::check_ref(view, expected)?;
            guard.by_agent.remove(agent);
            Ok(())
        }

        /// Test-only way to reach a disarmed activation and render its hint.
        fn disarm(&self, agent: &str, expected: GoalRef) -> GoalView {
            let mut guard = self.state.lock().expect("goals poisoned");
            let view = guard
                .by_agent
                .get_mut(agent)
                .expect("disarm needs a current goal");
            Self::check_ref(view, expected).expect("disarm needs a fresh ref");
            view.activation = GoalActivation::Idle;
            view.revision += 1;
            view.clone()
        }

        /// Test-only way to reach the durable `blocked` phase and render its reason.
        fn block(&self, agent: &str, expected: GoalRef, code: &str, message: &str) -> GoalView {
            let mut guard = self.state.lock().expect("goals poisoned");
            let view = guard
                .by_agent
                .get_mut(agent)
                .expect("block needs a current goal");
            Self::check_ref(view, expected).expect("block needs a fresh ref");
            view.phase = GoalPhase::Blocked;
            view.revision += 1;
            view.blocker = Some(Blocker {
                code: code.to_string(),
                message: message.to_string(),
            });
            view.clone()
        }

        fn check_ref(view: &GoalView, expected: GoalRef) -> Result<(), GoalError> {
            if view.cas_ref() == expected {
                Ok(())
            } else {
                Err(GoalError {
                    code: "GOAL_CONFLICT",
                    message: "goal changed since it was read".to_string(),
                })
            }
        }
    }

    // --- command logic ported from index.ts ------------------------------------

    enum GoalCommand {
        Show,
        Create { objective: String },
        Edit { objective: String },
        InvalidEdit,
        Pause,
        Resume,
        Clear,
    }

    fn parse_goal_command(raw_input: &str) -> GoalCommand {
        let input = raw_input.trim();
        if input.is_empty() {
            return GoalCommand::Show;
        }
        let control = input.to_lowercase();
        match control.as_str() {
            "clear" => return GoalCommand::Clear,
            "pause" => return GoalCommand::Pause,
            "resume" => return GoalCommand::Resume,
            "edit" => return GoalCommand::InvalidEdit,
            _ => {}
        }
        let is_edit_prefix = input
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("edit"))
            && input
                .get(4..)
                .and_then(|rest| rest.chars().next())
                .is_some_and(char::is_whitespace);
        if is_edit_prefix {
            return GoalCommand::Edit {
                objective: input[4..].trim().to_string(),
            };
        }
        GoalCommand::Create {
            objective: input.to_string(),
        }
    }

    fn command_hint(goal: &GoalView) -> &'static str {
        if goal.phase == GoalPhase::Active {
            return match goal.activation {
                GoalActivation::Armed => "/goal edit <objective>, /goal pause, /goal clear",
                GoalActivation::Idle => "/goal edit <objective>, /goal resume, /goal clear",
            };
        }
        match goal.phase {
            GoalPhase::Paused | GoalPhase::Blocked => {
                "/goal edit <objective>, /goal resume, /goal clear"
            }
            GoalPhase::Complete => "/goal <objective>, /goal clear",
            GoalPhase::Active => unreachable!("handled above"),
        }
    }

    fn render_goal(title: &str, goal: &GoalView) -> CommandResult {
        let mut lines = vec![title.to_string(), format!("Status: {}", goal.phase.label())];
        if let Some(blocker) = &goal.blocker {
            lines.push(format!("Blocker: {}: {}", blocker.code, blocker.message));
        }
        lines.push(format!("Objective: {}", goal.objective));
        lines.push(format!(
            "Rounds: {}/{}",
            goal.rounds_started, goal.max_goal_rounds
        ));
        lines.push(format!("Activation: {}", goal.activation.label()));
        lines.push(String::new());
        lines.push(format!("Commands: {}", command_hint(goal)));
        success(lines.join("\n"))
    }

    fn missing_goal(action: &str) -> CommandResult {
        error(format!(
            "No goal is currently set; /goal {action} requires one. {USAGE}"
        ))
    }

    fn invalid_state() -> CommandResult {
        error(
            "The goal command is not valid for the current state. Run /goal to view available commands.",
        )
    }

    fn execute_goal_command(goals: &Goals, invocation: &CommandInvocation) -> CommandResult {
        let current = goals.get(&invocation.agent);
        match parse_goal_command(&invocation.raw_input) {
            GoalCommand::Show => match current {
                None => success(format!("No goal is currently set.\n{USAGE}")),
                Some(goal) => render_goal("Goal", &goal),
            },
            GoalCommand::InvalidEdit => error(format!(
                "Goal editing requires a replacement objective.\n{USAGE}"
            )),
            GoalCommand::Create { objective } => {
                if let Some(existing) = current
                    .as_ref()
                    .filter(|goal| goal.phase != GoalPhase::Complete)
                {
                    return error(format!(
                        "A goal is already {}. Use /goal edit <objective> to change it or /goal clear before replacing it.",
                        existing.phase.label()
                    ));
                }
                let created = goals.create(&invocation.agent, objective);
                render_goal("Goal created", &created)
            }
            GoalCommand::Edit { objective } => {
                let Some(existing) = current else {
                    return missing_goal("edit");
                };
                if existing.phase == GoalPhase::Complete {
                    let replaced = goals.create(&invocation.agent, objective);
                    return render_goal("Goal created", &replaced);
                }
                match goals.edit(&invocation.agent, existing.cas_ref(), objective) {
                    Ok(edited) => render_goal("Goal updated", &edited),
                    Err(_) => invalid_state(),
                }
            }
            GoalCommand::Pause => {
                let Some(existing) = current else {
                    return missing_goal("pause");
                };
                match goals.set_phase(&invocation.agent, existing.cas_ref(), GoalPhase::Paused) {
                    Ok(paused) => render_goal("Goal paused", &paused),
                    Err(_) => invalid_state(),
                }
            }
            GoalCommand::Resume => {
                let Some(existing) = current else {
                    return missing_goal("resume");
                };
                match goals.set_phase(&invocation.agent, existing.cas_ref(), GoalPhase::Active) {
                    Ok(resumed) => render_goal("Goal resumed", &resumed),
                    Err(_) => invalid_state(),
                }
            }
            GoalCommand::Clear => {
                let Some(existing) = current else {
                    return success("No goal to clear.");
                };
                match goals.clear(&invocation.agent, existing.cas_ref()) {
                    Ok(()) => success("Goal cleared."),
                    Err(_) => invalid_state(),
                }
            }
        }
    }

    // --- plugin ----------------------------------------------------------------

    struct CommandGoal;

    #[async_trait]
    impl Plugin for CommandGoal {
        fn name(&self) -> &str {
            "command-goal"
        }

        fn inject(&self) -> &[&str] {
            &["commands", "goals"]
        }

        async fn apply(&self, ctx: &Context) -> cordis_rs::Result<()> {
            let commands = ctx.require::<Commands>("commands").await?;
            let goals = ctx.require::<Goals>("goals").await?;
            let commands_for_effect = commands.clone();
            let goals_for_handler = goals.clone();

            ctx.effect(move |scope| {
                let commands = commands_for_effect.clone();
                let goals = goals_for_handler.clone();
                async move {
                    let handler: CommandHandler = Arc::new(move |invocation| {
                        let goals = goals.clone();
                        Box::pin(async move { execute_goal_command(&goals, &invocation) })
                    });
                    let disposer = commands.register(CommandDefinition {
                        id: 0,
                        name: "/goal".to_string(),
                        description: "Set or view the goal for a long-running task".to_string(),
                        handler,
                    });
                    scope.on_dispose(move || async move { disposer.dispose().await });
                    Ok(())
                }
            })
            .await
        }
    }

    // --- tests -----------------------------------------------------------------

    #[tokio::test]
    async fn command_goal_waits_for_inject_registers_and_executes() {
        let ctx = Context::root("command-goal-port");
        let commands = Commands::default();
        let fiber = Fiber::mount(&ctx, Box::new(CommandGoal)).await.unwrap();

        assert_eq!(fiber.state(), Lifecycle::Pending);
        assert!(commands.names().is_empty());

        ctx.provide("commands", commands.clone()).await.unwrap();
        fiber.refresh().await.unwrap();
        assert_eq!(fiber.state(), Lifecycle::Pending);
        assert!(commands.names().is_empty());

        let goals = Goals::default();
        ctx.provide("goals", goals.clone()).await.unwrap();
        fiber.refresh().await.unwrap();
        assert_eq!(fiber.state(), Lifecycle::Active);
        assert_eq!(commands.names(), vec!["/goal".to_string()]);
        assert_eq!(
            commands.description("/goal").as_deref(),
            Some("Set or view the goal for a long-running task")
        );

        let created = commands
            .invoke(
                "/goal",
                CommandInvocation::new("Ship the Rust port", "alice"),
            )
            .await
            .expect("/goal must be registered");
        assert_eq!(
            created,
            success(concat!(
                "Goal created\n",
                "Status: active\n",
                "Objective: Ship the Rust port\n",
                "Rounds: 0/1000\n",
                "Activation: armed\n",
                "\n",
                "Commands: /goal edit <objective>, /goal pause, /goal clear",
            ))
        );
        assert_eq!(
            goals.get("alice").expect("goal stored for agent").objective,
            "Ship the Rust port"
        );
        assert!(goals.get("bob").is_none());

        let shown = commands
            .invoke("/goal", CommandInvocation::new("", "alice"))
            .await
            .unwrap();
        assert_eq!(
            shown,
            success(concat!(
                "Goal\n",
                "Status: active\n",
                "Objective: Ship the Rust port\n",
                "Rounds: 0/1000\n",
                "Activation: armed\n",
                "\n",
                "Commands: /goal edit <objective>, /goal pause, /goal clear",
            ))
        );

        let edited = commands
            .invoke(
                "/goal",
                CommandInvocation::new("  EDIT   Ship the full port  ", "alice"),
            )
            .await
            .unwrap();
        assert!(
            matches!(&edited, CommandResult::Success { text } if text.starts_with("Goal updated\n"))
        );
        assert_eq!(goals.get("alice").unwrap().objective, "Ship the full port");

        let paused = commands
            .invoke("/goal", CommandInvocation::new("pause", "alice"))
            .await
            .unwrap();
        assert!(
            matches!(&paused, CommandResult::Success { text } if text.starts_with("Goal paused\nStatus: paused\n"))
        );
        let resumed = commands
            .invoke("/goal", CommandInvocation::new("resume", "alice"))
            .await
            .unwrap();
        assert!(
            matches!(&resumed, CommandResult::Success { text } if text.starts_with("Goal resumed\nStatus: active\n"))
        );

        let idle_ref = goals.get("alice").unwrap().cas_ref();
        goals.disarm("alice", idle_ref);
        let idle = commands
            .invoke("/goal", CommandInvocation::new("", "alice"))
            .await
            .unwrap();
        assert!(
            matches!(&idle, CommandResult::Success { text } if text.contains("Activation: idle\n\nCommands: /goal edit <objective>, /goal resume, /goal clear"))
        );

        // A stale compare-and-set ref is rejected by the goals service.
        let stale = goals.edit("alice", idle_ref, "stale edit".to_string());
        assert!(matches!(stale, Err(err) if err.code == "GOAL_CONFLICT"));

        let blocked_ref = goals.get("alice").unwrap().cas_ref();
        goals.block("alice", blocked_ref, "GOAL_BLOCKED", "awaiting review");
        let blocked = commands
            .invoke("/goal", CommandInvocation::new("", "alice"))
            .await
            .unwrap();
        let CommandResult::Success { text } = blocked else {
            panic!("show must succeed");
        };
        assert!(text.contains("Status: blocked\nBlocker: GOAL_BLOCKED: awaiting review\n"));
        assert!(text.contains("Commands: /goal edit <objective>, /goal resume, /goal clear"));

        let cleared = commands
            .invoke("/goal", CommandInvocation::new("clear", "alice"))
            .await
            .unwrap();
        assert_eq!(cleared, success("Goal cleared."));
        assert!(goals.get("alice").is_none());

        // Removing one injection unloads the fiber and unregisters the command.
        ctx.remove_service("goals").unwrap();
        fiber.refresh().await.unwrap();
        assert_eq!(fiber.state(), Lifecycle::Pending);
        assert!(commands.names().is_empty());
    }

    #[tokio::test]
    async fn command_goal_rejects_bad_shapes_and_out_of_state_operations() {
        let ctx = Context::root("command-goal-errors");
        let commands = Commands::default();
        let goals = Goals::default();
        ctx.provide("commands", commands.clone()).await.unwrap();
        ctx.provide("goals", goals.clone()).await.unwrap();
        let fiber = Fiber::mount(&ctx, Box::new(CommandGoal)).await.unwrap();
        assert_eq!(fiber.state(), Lifecycle::Active);

        assert_eq!(
            commands
                .invoke("/goal", CommandInvocation::new("edit", "alice"))
                .await
                .unwrap(),
            error(format!(
                "Goal editing requires a replacement objective.\n{USAGE}"
            ))
        );
        assert_eq!(
            commands
                .invoke("/goal", CommandInvocation::new("pause", "alice"))
                .await
                .unwrap(),
            error(format!(
                "No goal is currently set; /goal pause requires one. {USAGE}"
            ))
        );
        assert_eq!(
            commands
                .invoke("/goal", CommandInvocation::new("", "alice"))
                .await
                .unwrap(),
            success(format!("No goal is currently set.\n{USAGE}"))
        );
        assert_eq!(
            commands
                .invoke("/goal", CommandInvocation::new("clear", "alice"))
                .await
                .unwrap(),
            success("No goal to clear.")
        );
        assert_eq!(
            commands
                .invoke("/unknown", CommandInvocation::new("anything", "alice"))
                .await,
            None
        );

        commands
            .invoke("/goal", CommandInvocation::new("first objective", "alice"))
            .await
            .unwrap();
        assert_eq!(
            commands
                .invoke("/goal", CommandInvocation::new("second objective", "alice"))
                .await
                .unwrap(),
            error(
                "A goal is already active. Use /goal edit <objective> to change it or /goal clear before replacing it."
            )
        );
        commands
            .invoke("/goal", CommandInvocation::new("clear", "alice"))
            .await
            .unwrap();
        assert_eq!(
            commands
                .invoke("/goal", CommandInvocation::new("resume", "alice"))
                .await
                .unwrap(),
            error(format!(
                "No goal is currently set; /goal resume requires one. {USAGE}"
            ))
        );
    }
}

mod tool_todo {
    use std::collections::HashSet;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use cordis_rs::{Context, Disposer, Fiber, Lifecycle, Plugin};

    const DESCRIPTION_HEAD: &str = "Record and update a task list to plan multi-step work and show progress; skip it for trivial single-step tasks. Add one todo per concrete step before you start. ";
    const DESCRIPTION_PARALLEL: &str = "While work remains, keep the todos being worked on `in_progress`, several only when work runs in parallel. ";
    const DESCRIPTION_SINGLE: &str = "While work remains, keep exactly one todo `in_progress`. ";
    const DESCRIPTION_TAIL: &str = "Mark each todo `completed` as soon as it is done.";

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    fn describe(allow_parallel: bool) -> String {
        let clause = if allow_parallel {
            DESCRIPTION_PARALLEL
        } else {
            DESCRIPTION_SINGLE
        };
        format!("{DESCRIPTION_HEAD}{clause}{DESCRIPTION_TAIL}")
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TodoStatus {
        Pending,
        InProgress,
        Completed,
    }

    impl TodoStatus {
        fn parse(raw: &str) -> Option<Self> {
            match raw {
                "pending" => Some(Self::Pending),
                "in_progress" => Some(Self::InProgress),
                "completed" => Some(Self::Completed),
                _ => None,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TodoItem {
        content: String,
        status: TodoStatus,
    }

    impl TodoItem {
        fn new(content: &str, status: TodoStatus) -> Self {
            Self {
                content: content.to_string(),
                status,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Counts {
        pending: usize,
        in_progress: usize,
        completed: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TodoWriteResult {
        todos: Vec<TodoItem>,
        counts: Counts,
    }

    impl TodoWriteResult {
        fn render(&self) -> String {
            format!(
                "Updated todo list: {} pending, {} in progress, {} completed.",
                self.counts.pending, self.counts.in_progress, self.counts.completed
            )
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ToolFailure {
        /// Upstream `ToolArgsError`.
        Args(String),
        /// Upstream `ToolContextUnavailableError`.
        ContextUnavailable(String),
        /// Test harness only: no tool with that name was registered.
        NotFound(String),
    }

    #[derive(Debug, Clone)]
    struct RawTodo {
        content: String,
        status: String,
    }

    impl RawTodo {
        fn new(content: &str, status: &str) -> Self {
            Self {
                content: content.to_string(),
                status: status.to_string(),
            }
        }
    }

    #[derive(Debug, Clone)]
    struct TodoWriteArgs {
        todos: Vec<RawTodo>,
    }

    #[derive(Debug, Clone)]
    struct ToolExecution {
        agent: Option<String>,
    }

    impl ToolExecution {
        fn agent(name: &str) -> Self {
            Self {
                agent: Some(name.to_string()),
            }
        }

        fn anonymous() -> Self {
            Self { agent: None }
        }
    }

    /// Port of `toTodoList`: trimmed non-empty unique content, a valid status,
    /// and at most one `in_progress` item unless parallel work is allowed.
    fn to_todo_list(raw: &[RawTodo], allow_parallel: bool) -> Result<Vec<TodoItem>, ToolFailure> {
        let mut todos = Vec::with_capacity(raw.len());
        let mut seen = HashSet::new();
        let mut active = 0usize;
        for (index, item) in raw.iter().enumerate() {
            let content = item.content.trim().to_string();
            if content.is_empty() {
                return Err(ToolFailure::Args(format!(
                    "todos[{index}].content must be a non-empty string"
                )));
            }
            if seen.contains(&content) {
                return Err(ToolFailure::Args(format!(
                    "todos[{index}].content duplicates {content:?}"
                )));
            }
            seen.insert(content.clone());
            let status = TodoStatus::parse(&item.status).ok_or_else(|| {
                ToolFailure::Args(format!(
                    "todos[{index}].status must be one of pending, in_progress, completed"
                ))
            })?;
            if status == TodoStatus::InProgress {
                active += 1;
            }
            todos.push(TodoItem { content, status });
        }
        if !allow_parallel && active > 1 {
            return Err(ToolFailure::Args(format!(
                "at most one task may be in_progress (got {active})"
            )));
        }
        Ok(todos)
    }

    // --- tools and session projection service stubs ----------------------------

    type ToolHandler = Arc<
        dyn Fn(
                TodoWriteArgs,
                ToolExecution,
            ) -> BoxFuture<'static, Result<TodoWriteResult, ToolFailure>>
            + Send
            + Sync,
    >;

    #[derive(Clone)]
    struct ToolDefinition {
        id: u64,
        name: String,
        description: String,
        handler: ToolHandler,
    }

    #[derive(Default)]
    struct ToolsState {
        next_id: u64,
        entries: Vec<ToolDefinition>,
    }

    #[derive(Default, Clone)]
    struct Tools {
        state: Arc<Mutex<ToolsState>>,
    }

    impl Tools {
        fn register(&self, mut definition: ToolDefinition) -> Disposer {
            let state = self.state.clone();
            let mut guard = state.lock().expect("tools poisoned");
            definition.id = guard.next_id;
            guard.next_id += 1;
            let id = definition.id;
            guard.entries.push(definition);
            drop(guard);

            Disposer::new(move || {
                let state = state.clone();
                async move {
                    state
                        .lock()
                        .expect("tools poisoned")
                        .entries
                        .retain(|entry| entry.id != id);
                    Ok(())
                }
            })
        }

        fn names(&self) -> Vec<String> {
            self.state
                .lock()
                .expect("tools poisoned")
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        }

        fn description(&self, name: &str) -> Option<String> {
            self.state
                .lock()
                .expect("tools poisoned")
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.description.clone())
        }

        async fn invoke(
            &self,
            name: &str,
            args: TodoWriteArgs,
            exec: ToolExecution,
        ) -> Result<TodoWriteResult, ToolFailure> {
            let handler = {
                let guard = self.state.lock().expect("tools poisoned");
                guard
                    .entries
                    .iter()
                    .find(|entry| entry.name == name)
                    .map(|entry| entry.handler.clone())
            };
            match handler {
                Some(handler) => handler(args, exec).await,
                None => Err(ToolFailure::NotFound(name.to_string())),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SessionEvent {
        TodoWrite { todos: Vec<TodoItem> },
        TurnStart,
    }

    #[derive(Default, Clone)]
    struct SessionProjections {
        registered: Arc<Mutex<Vec<String>>>,
        todos: Arc<Mutex<Option<Vec<TodoItem>>>>,
    }

    impl SessionProjections {
        fn register(&self, key: &str) -> Disposer {
            let registered = self.registered.clone();
            registered
                .lock()
                .expect("projections poisoned")
                .push(key.to_string());
            let key = key.to_string();
            Disposer::new(move || {
                let registered = registered.clone();
                async move {
                    registered
                        .lock()
                        .expect("projections poisoned")
                        .retain(|current| current != &key);
                    Ok(())
                }
            })
        }

        fn is_registered(&self, key: &str) -> bool {
            self.registered
                .lock()
                .expect("projections poisoned")
                .iter()
                .any(|current| current == key)
        }

        fn append(&self, event: SessionEvent) {
            let mut guard = self.todos.lock().expect("projection state poisoned");
            match event {
                SessionEvent::TodoWrite { todos } => *guard = Some(todos),
                SessionEvent::TurnStart => *guard = None,
            }
        }

        fn todos_view(&self) -> Option<Vec<TodoItem>> {
            self.todos
                .lock()
                .expect("projection state poisoned")
                .clone()
        }
    }

    /// Port of the tool's `execute`: validate, require an owning agent session,
    /// append the whole-list `todo/write` event, and return canonical counts.
    fn execute_todo_write(
        args: TodoWriteArgs,
        exec: &ToolExecution,
        allow_parallel: bool,
        projections: &SessionProjections,
    ) -> Result<TodoWriteResult, ToolFailure> {
        let todos = to_todo_list(&args.todos, allow_parallel)?;
        if exec.agent.is_none() {
            return Err(ToolFailure::ContextUnavailable(
                "todo_write requires an owning agent session".to_string(),
            ));
        }
        projections.append(SessionEvent::TodoWrite {
            todos: todos.clone(),
        });
        let count = |status: TodoStatus| todos.iter().filter(|todo| todo.status == status).count();
        let counts = Counts {
            pending: count(TodoStatus::Pending),
            in_progress: count(TodoStatus::InProgress),
            completed: count(TodoStatus::Completed),
        };
        Ok(TodoWriteResult { todos, counts })
    }

    // --- plugin ----------------------------------------------------------------

    struct ToolTodo {
        allow_parallel: bool,
    }

    #[async_trait]
    impl Plugin for ToolTodo {
        fn name(&self) -> &str {
            "tool-todo"
        }

        fn inject(&self) -> &[&str] {
            &["tools", "sessionProjections"]
        }

        async fn apply(&self, ctx: &Context) -> cordis_rs::Result<()> {
            let tools = ctx.require::<Tools>("tools").await?;
            let projections = ctx
                .require::<SessionProjections>("sessionProjections")
                .await?;
            let allow_parallel = self.allow_parallel;
            let tools_for_effect = tools.clone();
            let projections_for_effect = projections.clone();

            ctx.effect(move |scope| {
                let tools = tools_for_effect.clone();
                let projections = projections_for_effect.clone();
                async move {
                    let projection_disposer = projections.register("todos");
                    scope.on_dispose(move || async move { projection_disposer.dispose().await });

                    let projections_for_handler = projections.clone();
                    let handler: ToolHandler = Arc::new(move |args, exec| {
                        let projections = projections_for_handler.clone();
                        Box::pin(async move {
                            execute_todo_write(args, &exec, allow_parallel, &projections)
                        })
                    });
                    let tool_disposer = tools.register(ToolDefinition {
                        id: 0,
                        name: "todo_write".to_string(),
                        description: describe(allow_parallel),
                        handler,
                    });
                    scope.on_dispose(move || async move { tool_disposer.dispose().await });
                    Ok(())
                }
            })
            .await
        }
    }

    // --- tests -----------------------------------------------------------------

    #[tokio::test]
    async fn tool_todo_waits_for_inject_registers_and_executes() {
        let ctx = Context::root("tool-todo-port");
        let tools = Tools::default();
        let projections = SessionProjections::default();
        let fiber = Fiber::mount(
            &ctx,
            Box::new(ToolTodo {
                allow_parallel: false,
            }),
        )
        .await
        .unwrap();

        assert_eq!(fiber.state(), Lifecycle::Pending);
        assert!(tools.names().is_empty());
        assert!(!projections.is_registered("todos"));

        ctx.provide("tools", tools.clone()).await.unwrap();
        fiber.refresh().await.unwrap();
        assert_eq!(fiber.state(), Lifecycle::Pending);
        assert!(tools.names().is_empty());
        assert!(!projections.is_registered("todos"));

        ctx.provide("sessionProjections", projections.clone())
            .await
            .unwrap();
        fiber.refresh().await.unwrap();
        assert_eq!(fiber.state(), Lifecycle::Active);
        assert_eq!(tools.names(), vec!["todo_write".to_string()]);
        assert!(projections.is_registered("todos"));
        assert!(
            tools
                .description("todo_write")
                .unwrap()
                .contains("keep exactly one todo `in_progress`")
        );

        let args = TodoWriteArgs {
            todos: vec![
                RawTodo::new(" investigate ", "completed"),
                RawTodo::new("implement", "in_progress"),
                RawTodo::new("verify", "pending"),
            ],
        };
        let result = tools
            .invoke("todo_write", args, ToolExecution::agent("alice"))
            .await
            .unwrap();
        assert_eq!(
            result.render(),
            "Updated todo list: 1 pending, 1 in progress, 1 completed."
        );
        assert_eq!(
            result.todos,
            vec![
                TodoItem::new("investigate", TodoStatus::Completed),
                TodoItem::new("implement", TodoStatus::InProgress),
                TodoItem::new("verify", TodoStatus::Pending),
            ]
        );
        assert_eq!(projections.todos_view(), Some(result.todos.clone()));

        // The projection reset on a new turn is part of the upstream fold.
        projections.append(SessionEvent::TurnStart);
        assert_eq!(projections.todos_view(), None);

        assert_eq!(
            tools
                .invoke(
                    "todo_write",
                    TodoWriteArgs { todos: vec![] },
                    ToolExecution::anonymous()
                )
                .await,
            Err(ToolFailure::ContextUnavailable(
                "todo_write requires an owning agent session".to_string()
            ))
        );
        assert_eq!(
            tools
                .invoke(
                    "missing_tool",
                    TodoWriteArgs { todos: vec![] },
                    ToolExecution::agent("alice")
                )
                .await,
            Err(ToolFailure::NotFound("missing_tool".to_string()))
        );

        // Losing one injection unloads the fiber and removes both registrations.
        ctx.remove_service("sessionProjections").unwrap();
        fiber.refresh().await.unwrap();
        assert_eq!(fiber.state(), Lifecycle::Pending);
        assert!(tools.names().is_empty());
        assert!(!projections.is_registered("todos"));
    }

    #[tokio::test]
    async fn tool_todo_validates_status_duplicates_and_single_active_policy() {
        let ctx = Context::root("tool-todo-validation");
        let tools = Tools::default();
        let projections = SessionProjections::default();
        ctx.provide("tools", tools.clone()).await.unwrap();
        ctx.provide("sessionProjections", projections.clone())
            .await
            .unwrap();
        let fiber = Fiber::mount(
            &ctx,
            Box::new(ToolTodo {
                allow_parallel: false,
            }),
        )
        .await
        .unwrap();
        assert_eq!(fiber.state(), Lifecycle::Active);

        let invoke = |todos: Vec<RawTodo>| {
            let tools = tools.clone();
            async move {
                tools
                    .invoke(
                        "todo_write",
                        TodoWriteArgs { todos },
                        ToolExecution::agent("alice"),
                    )
                    .await
            }
        };

        assert_eq!(
            invoke(vec![RawTodo::new("   ", "pending")]).await,
            Err(ToolFailure::Args(
                "todos[0].content must be a non-empty string".to_string()
            ))
        );
        assert_eq!(
            invoke(vec![
                RawTodo::new("same", "pending"),
                RawTodo::new(" same ", "completed"),
            ])
            .await,
            Err(ToolFailure::Args(
                "todos[1].content duplicates \"same\"".to_string()
            ))
        );
        assert_eq!(
            invoke(vec![RawTodo::new("task", "in progess")]).await,
            Err(ToolFailure::Args(
                "todos[0].status must be one of pending, in_progress, completed".to_string()
            ))
        );
        assert_eq!(
            invoke(vec![
                RawTodo::new("first", "in_progress"),
                RawTodo::new("second", "in_progress"),
            ])
            .await,
            Err(ToolFailure::Args(
                "at most one task may be in_progress (got 2)".to_string()
            ))
        );
        assert!(projections.todos_view().is_none());

        let ok = invoke(vec![
            RawTodo::new("first", "in_progress"),
            RawTodo::new("second", "pending"),
        ])
        .await
        .unwrap();
        assert_eq!(
            ok.counts,
            Counts {
                pending: 1,
                in_progress: 1,
                completed: 0,
            }
        );
        assert!(projections.todos_view().is_some());
    }

    #[tokio::test]
    async fn tool_todo_allows_parallel_in_progress_when_configured() {
        let ctx = Context::root("tool-todo-parallel");
        let tools = Tools::default();
        let projections = SessionProjections::default();
        ctx.provide("tools", tools.clone()).await.unwrap();
        ctx.provide("sessionProjections", projections.clone())
            .await
            .unwrap();
        let fiber = Fiber::mount(
            &ctx,
            Box::new(ToolTodo {
                allow_parallel: true,
            }),
        )
        .await
        .unwrap();
        assert_eq!(fiber.state(), Lifecycle::Active);
        assert!(
            tools
                .description("todo_write")
                .unwrap()
                .contains("several only when work runs in parallel")
        );

        let result = tools
            .invoke(
                "todo_write",
                TodoWriteArgs {
                    todos: vec![
                        RawTodo::new("a", "in_progress"),
                        RawTodo::new("b", "in_progress"),
                        RawTodo::new("c", "completed"),
                    ],
                },
                ToolExecution::agent("alice"),
            )
            .await
            .unwrap();
        assert_eq!(
            result.render(),
            "Updated todo list: 0 pending, 2 in progress, 1 completed."
        );
        assert_eq!(projections.todos_view(), Some(result.todos));
    }
}

#[tokio::test]
async fn fs_observation_policy_prevents_unobserved_edits() {
    use cordis_rs::plugin_ports::{
        FsObservationPolicy, Observation, PolicyError, WriteIntent, provide_fs_observation_policy,
    };

    let ctx = Context::root("fs-observation");
    let policy: FsObservationPolicy = provide_fs_observation_policy(&ctx).await.unwrap();

    // No observation -> both intents rejected.
    assert!(matches!(
        policy.edit_intent("agent-1", "/tmp/a.txt"),
        Err(PolicyError::NotObserved)
    ));
    assert!(matches!(
        policy.write_intent("agent-1", "/tmp/a.txt"),
        Err(PolicyError::NotObserved)
    ));

    // Absent observation -> create intent, edit denied.
    policy.observe("agent-1", "/tmp/a.txt", Observation::Absent);
    assert_eq!(
        policy.write_intent("agent-1", "/tmp/a.txt").unwrap(),
        WriteIntent::CreateIfAbsent
    );
    assert!(matches!(
        policy.edit_intent("agent-1", "/tmp/a.txt"),
        Err(PolicyError::NotFound)
    ));

    // Present observation -> versioned replace and edit version.
    policy.observe("agent-1", "/tmp/a.txt", Observation::Present { version: 7 });
    assert_eq!(
        policy.write_intent("agent-1", "/tmp/a.txt").unwrap(),
        WriteIntent::ReplaceIfVersion { version: 7 }
    );
    assert_eq!(policy.edit_intent("agent-1", "/tmp/a.txt").unwrap(), 7);

    // Owner isolation and clear.
    assert!(matches!(
        policy.edit_intent("agent-2", "/tmp/a.txt"),
        Err(PolicyError::NotObserved)
    ));
    policy.clear();
    assert!(matches!(
        policy.edit_intent("agent-1", "/tmp/a.txt"),
        Err(PolicyError::NotObserved)
    ));
}

#[tokio::test]
async fn tool_session_list_registers_paginated_read_tools() {
    use cordis_rs::core_services::{
        ToolError, ToolExecContext, provide_session_query, provide_sessions, provide_tools,
        provide_workspace,
    };
    use cordis_rs::plugin_ports::{mount_tool_session_list, resolve_page};

    assert_eq!(resolve_page(None, None).unwrap().limit, 20);
    assert!(matches!(
        resolve_page(Some(0), None),
        Err(ToolError::InvalidArgs(_))
    ));
    assert!(matches!(
        resolve_page(Some(101), None),
        Err(ToolError::InvalidArgs(_))
    ));

    let ctx = Context::root("tool-session-list");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    provide_sessions(&ctx).await.unwrap();
    provide_session_query(&ctx).await.unwrap();
    provide_workspace(&ctx).await.unwrap();
    mount_tool_session_list(&ctx).await.unwrap();

    let mut names = tools.names();
    names.sort();
    assert_eq!(names, ["list_session", "list_workspace"]);

    let workspaces = tools
        .invoke(
            "list_workspace",
            serde_json::json!({ "limit": 1, "offset": 0 }),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(workspaces.content["limit"], 1);

    let sessions = tools
        .invoke(
            "list_session",
            serde_json::json!({ "limit": 5 }),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(sessions.content["limit"], 5);
    let _ = sessions;
}
