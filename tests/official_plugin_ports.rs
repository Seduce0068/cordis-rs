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

#[tokio::test]
async fn tool_str_replace_editor_handles_four_commands() {
    use cordis_rs::core_services::{ToolExecContext, provide_fs, provide_tools};
    use cordis_rs::plugin_ports::mount_tool_str_replace_editor;

    let ctx = Context::root("str-replace-editor");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    provide_fs(&ctx).await.unwrap();
    mount_tool_str_replace_editor(&ctx).await.unwrap();
    assert!(tools.names().contains(&"str_replace_editor".to_string()));

    let dir = std::env::temp_dir().join(format!("str-replace-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("sample.txt");
    let _ = std::fs::remove_file(&path);
    let path_str = path.to_string_lossy().to_string();
    let exec = || ToolExecContext::agent("a");

    let created = tools
        .invoke(
            "str_replace_editor",
            serde_json::json!({ "command": "create", "path": path_str, "file_text": "alpha\nbeta\ngamma" }),
            exec(),
        )
        .await
        .unwrap();
    assert!(created.rendered[0].contains("Created"));
    assert!(
        tools
            .invoke(
                "str_replace_editor",
                serde_json::json!({ "command": "create", "path": path_str, "file_text": "dup" }),
                exec(),
            )
            .await
            .is_err()
    );

    let viewed = tools
        .invoke(
            "str_replace_editor",
            serde_json::json!({ "command": "view", "path": path_str, "view_range": [2, 3] }),
            exec(),
        )
        .await
        .unwrap();
    assert!(viewed.rendered[0].contains("beta"));
    assert!(!viewed.rendered[0].contains("alpha"));

    let replaced = tools
        .invoke(
            "str_replace_editor",
            serde_json::json!({ "command": "str_replace", "path": path_str, "old_str": "beta", "new_str": "BETA" }),
            exec(),
        )
        .await
        .unwrap();
    assert!(replaced.rendered[0].contains("Updated"));
    assert!(tools
        .invoke(
            "str_replace_editor",
            serde_json::json!({ "command": "str_replace", "path": path_str, "old_str": "missing", "new_str": "x" }),
            exec(),
        )
        .await
        .is_err());

    let inserted = tools
        .invoke(
            "str_replace_editor",
            serde_json::json!({ "command": "insert", "path": path_str, "insert_line": 1, "new_str": "inserted" }),
            exec(),
        )
        .await
        .unwrap();
    assert!(inserted.rendered[0].contains("Inserted"));
    assert!(std::fs::read_to_string(&path).unwrap().contains("inserted"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn time_context_plugin_registers_projection() {
    use cordis_rs::core_services::{Session, provide_agents, provide_sessions};
    use cordis_rs::plugin_ports::{TimeContextPluginConfig, mount_time_context};

    let ctx = Context::root("time-context-port");
    provide_agents(&ctx).await.unwrap();
    let (sessions, projections) = provide_sessions(&ctx).await.unwrap();
    mount_time_context(
        &ctx,
        TimeContextPluginConfig {
            refresh_interval_ms: 1000,
            time_zone: Some("Asia/Tokyo".to_string()),
        },
    )
    .await
    .unwrap();

    let session = sessions.create(Some("tc-1".to_string()), None).unwrap();
    session.append("turn/start", serde_json::json!({}));
    session.append("user/message", serde_json::json!({ "text": "hi" }));
    let view = projections.view("timeContext", &session);
    assert!(view.is_some());
    let view = view.unwrap();
    assert_eq!(view["turns"], 1);
    assert_eq!(view["messages"], 1);
    let _ = Session::new("unused", None);
}

#[tokio::test]
async fn tmux_context_projects_samples_and_gates_step_one() {
    use cordis_rs::core_services::{TmuxContext, provide_agents, provide_sessions};
    use cordis_rs::plugin_ports::{
        TmuxContextPluginConfig, mount_tmux_context, sample_reading, should_sample,
    };

    let ctx = Context::root("tmux-context-port");
    provide_agents(&ctx).await.unwrap();
    let (sessions, projections) = provide_sessions(&ctx).await.unwrap();
    mount_tmux_context(&ctx, TmuxContextPluginConfig::default())
        .await
        .unwrap();

    let session = sessions.create(Some("tmux-1".to_string()), None).unwrap();
    session.append(
        "user/message",
        serde_json::json!({
            "source": { "kind": "tmux-context" },
            "text": "tmux location (turn 1):\nmain:0"
        }),
    );
    let view = projections.view("tmuxContext", &session).unwrap();
    assert_eq!(view["samples"], 1);

    let location = cordis_rs::core_services::parse_location(
        "main\\t0\\tshell\\t1\\t%3\\t1\\t0\\t=,80,24,0,0,0",
    )
    .unwrap();
    let helper = TmuxContext::new();
    assert!(should_sample(&helper, 1, &location));
    assert!(!should_sample(&helper, 2, &location));
    assert!(sample_reading(1, 1, &location, "2026-11-18T05:00:00+09:00").contains("session=main"));
}

#[tokio::test]
async fn tool_jobs_registers_three_tools_and_kill_two_branches() {
    use cordis_rs::core_services::{JobStatus, ToolExecContext, provide_jobs, provide_tools};
    use cordis_rs::plugin_ports::mount_tool_jobs;

    let ctx = Context::root("tool-jobs-port");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let jobs = provide_jobs(&ctx).await.unwrap();
    mount_tool_jobs(&ctx).await.unwrap();

    let mut names = tools.names();
    names.sort();
    assert_eq!(names, ["job_kill", "job_list", "job_output"]);

    let exec = || ToolExecContext::agent("a");
    let handle = jobs.start(Some("a".to_string())).unwrap();
    let id = handle.id();
    handle.append("first output");

    let list = tools
        .invoke("job_list", serde_json::json!({ "session_id": "a" }), exec())
        .await
        .unwrap();
    assert_eq!(list.content["jobs"].as_array().unwrap().len(), 1);

    let output = tools
        .invoke(
            "job_output",
            serde_json::json!({ "job_id": id, "session_id": "a" }),
            exec(),
        )
        .await
        .unwrap();
    assert_eq!(output.content["job"]["status"], "Running");
    assert!(output.rendered[1].contains("first output"));

    let killed = tools
        .invoke(
            "job_kill",
            serde_json::json!({ "job_id": id, "session_id": "a" }),
            exec(),
        )
        .await
        .unwrap();
    assert_eq!(killed.rendered[0], "cancellation-requested");
    assert_eq!(handle.status(), JobStatus::Killed);

    let again = tools
        .invoke(
            "job_kill",
            serde_json::json!({ "job_id": id, "session_id": "a" }),
            exec(),
        )
        .await
        .unwrap();
    assert_eq!(again.rendered[0], "already-finished");

    let empty = tools
        .invoke(
            "job_list",
            serde_json::json!({ "session_id": "other" }),
            exec(),
        )
        .await
        .unwrap();
    assert_eq!(empty.rendered[0], "(no background jobs)");
}

#[tokio::test]
async fn timeout_policy_converts_only_own_timer_to_structured_result() {
    use cordis_rs::core_services::{ToolDefinition, ToolError, ToolOutput, provide_tools};
    use cordis_rs::plugin_ports::{
        TOOL_TIMEOUT_CODE, TimeoutPolicyPluginConfig, mount_timeout_policy, timeout_output,
    };
    use std::sync::Arc;

    let structured = timeout_output(&cordis_rs::core_services::tool_timeout_result(50));
    assert_eq!(structured.content["code"], TOOL_TIMEOUT_CODE);
    assert_eq!(structured.content["timed_out"], true);

    let ctx = Context::root("timeout-policy-port");
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    mount_timeout_policy(
        &ctx,
        TimeoutPolicyPluginConfig {
            default_timeout_ms: Some(30),
        },
    )
    .await
    .unwrap();

    let slow = Arc::new(
        move |_args: serde_json::Value, _exec: cordis_rs::core_services::ToolExecContext| {
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Ok(ToolOutput::text("done")) as Result<ToolOutput, ToolError>
            }) as cordis_rs::events::BoxFuture<'static, Result<ToolOutput, ToolError>>
        },
    );
    let fast = Arc::new(
        move |_args: serde_json::Value, _exec: cordis_rs::core_services::ToolExecContext| {
            Box::pin(async move { Ok(ToolOutput::text("done")) as Result<ToolOutput, ToolError> })
                as cordis_rs::events::BoxFuture<'static, Result<ToolOutput, ToolError>>
        },
    );
    let _slow_d = registry.register(ToolDefinition::new("slow", "slow", slow));
    let _fast_d = registry.register(ToolDefinition::new("fast", "fast", fast));

    let slow_result = pipeline
        .run(
            &registry,
            "slow",
            serde_json::json!({}),
            cordis_rs::core_services::ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(slow_result.content["code"], TOOL_TIMEOUT_CODE);

    let fast_result = pipeline
        .run(
            &registry,
            "fast",
            serde_json::json!({}),
            cordis_rs::core_services::ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(fast_result.rendered[0], "done");
}

#[tokio::test]
async fn compaction_image_offload_selects_and_validates() {
    use cordis_rs::plugin_ports::{
        Decision, IMAGE_OFFLOAD_REQUIRED_CODE, decide, offload_payload, select_oldest_after,
        validate_payload,
    };

    let candidates = [(3u64, vec![0usize, 2]), (7, vec![1]), (9, vec![0])];
    let done = [(3, vec![0])];
    let selected = select_oldest_after(&candidates, &done, 2, 0);
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].seq, 3);
    assert_eq!(selected[0].image_indexes, vec![2]);
    assert_eq!(selected[1].seq, 7);

    // Checkpoint boundary skips already-summarized events.
    let after_checkpoint = select_oldest_after(&candidates, &[], 5, 7);
    assert_eq!(after_checkpoint.len(), 1);
    assert_eq!(after_checkpoint[0].seq, 9);

    let payload = offload_payload(&selected);
    let round_trip = validate_payload(&payload).unwrap();
    assert_eq!(round_trip, selected);

    // Validation rejects malformed payloads.
    assert!(validate_payload(&serde_json::json!({})).is_err());
    assert!(validate_payload(&serde_json::json!({ "targets": [] })).is_err());
    assert!(
        validate_payload(&serde_json::json!({ "targets": [{ "seq": -1, "imageIndexes": [0] }] }))
            .is_err()
    );
    assert!(
        validate_payload(&serde_json::json!({ "targets": [{ "seq": 1, "imageIndexes": [2, 2] }] }))
            .is_err()
    );

    assert_eq!(
        decide(Some(IMAGE_OFFLOAD_REQUIRED_CODE), true, false),
        Decision::Retry
    );
    assert_eq!(
        decide(Some(IMAGE_OFFLOAD_REQUIRED_CODE), true, true),
        Decision::SummaryHandled
    );
    assert_eq!(decide(Some("OTHER"), true, false), Decision::Noop);
    assert_eq!(
        decide(Some(IMAGE_OFFLOAD_REQUIRED_CODE), false, false),
        Decision::Noop
    );
}

#[tokio::test]
async fn compaction_tool_result_pruner_emits_adjacent_shadow_and_replacement() {
    use cordis_rs::core_services::{
        PrunerConfig, provide_agents, provide_sessions, provide_token_meter,
    };
    use cordis_rs::plugin_ports::{
        REPLACEMENT_EVENT, SHADOW_PRICE_EVENT, measure_content,
        mount_compaction_tool_result_pruner, provide_tool_result_pruner, prune_content,
        prune_session,
    };

    let ctx = Context::root("pruner-port");
    provide_agents(&ctx).await.unwrap();
    provide_token_meter(&ctx).await.unwrap();
    let (_sessions, _) = provide_sessions(&ctx).await.unwrap();
    mount_compaction_tool_result_pruner(&ctx).await.unwrap();
    let pruner = provide_tool_result_pruner(
        &ctx,
        PrunerConfig {
            head_chars: 8,
            tail_chars: 8,
            threshold_chars: 60,
        },
    )
    .await
    .unwrap();

    let long = "a".repeat(80);
    let blocks = vec![serde_json::json!({ "type": "text", "text": long })];
    assert_eq!(measure_content(&blocks), 80);
    let pruned_blocks = prune_content(&pruner, &blocks);
    assert!(pruned_blocks[0]["text"].as_str().unwrap().len() < 80);
    let short = vec![serde_json::json!({ "type": "text", "text": "tiny" })];
    assert_eq!(prune_content(&pruner, &short), short);

    let session = ctx
        .require::<cordis_rs::core_services::SessionStore>("sessions")
        .await
        .unwrap();
    let session = session
        .create(Some("prune-session".to_string()), None)
        .unwrap();
    session.append(
        REPLACEMENT_EVENT,
        serde_json::json!({ "text": "z".repeat(80) }),
    );
    let result = prune_session(&pruner, &session);
    assert_eq!(result.pruned, 1);
    assert!(result.chars_removed > 0);
    assert_eq!(result.events.len(), 2);
    assert_eq!(result.events[0].event_type, SHADOW_PRICE_EVENT);
    assert_eq!(result.events[1].event_type, REPLACEMENT_EVENT);
    assert_eq!(
        result.events[1].data["sourceEventSeqs"][0],
        result.events[0].data["sourceEventSeqs"][0]
    );
    assert_eq!(result.events[1].data["op"], "replace");
    assert!(result.events[1].seq == result.events[0].seq + 1);
}

#[tokio::test]
async fn fs_patch_parses_and_applies_codex_style_patch() {
    use cordis_rs::plugin_ports::{ApplyHunksResult, PatchOperation, apply_hunks, parse_patch};

    let patch =
        "*** Begin Patch\n*** Update File: hello.txt\n@@ greeting\n-hello\n+hi\n*** End Patch";
    let ops = parse_patch(patch).unwrap();
    assert_eq!(ops.len(), 1);
    let PatchOperation::Update { path, hunks, .. } = &ops[0] else {
        panic!("expected update");
    };
    assert_eq!(path, "hello.txt");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].context.as_deref(), Some("greeting"));
    assert_eq!(hunks[0].old_lines, vec!["hello"]);
    assert_eq!(hunks[0].new_lines, vec!["hi"]);

    let result = apply_hunks("greeting\nhello\n", hunks);
    assert_eq!(
        result,
        ApplyHunksResult::Ok {
            after: "greeting\nhi\n".to_string()
        }
    );

    // Add + delete + heredoc wrapper.
    let wrapped = "<<EOF\n*** Begin Patch\n*** Add File: new.txt\n+one\n+two\n*** Delete File: old.txt\n*** End Patch\nEOF";
    let ops = parse_patch(wrapped).unwrap();
    assert_eq!(ops.len(), 2);
    let PatchOperation::Add { content, .. } = &ops[0] else {
        panic!("expected add");
    };
    assert_eq!(content, "one\ntwo\n");
    assert!(matches!(ops[1], PatchOperation::Delete { .. }));

    // Malformed patch reports the original line number.
    let error = parse_patch("not a patch\n*** End Patch").unwrap_err();
    assert_eq!(error.line_number, 1);

    // Unmatched hunk returns a structured failure (not a panic).
    let failure = apply_hunks("other\ntext\n", hunks);
    match failure {
        ApplyHunksResult::Failure { failure } => {
            assert_eq!(failure.stage, "context");
            assert_eq!(failure.pattern, vec!["greeting"]);
        }
        _ => panic!("expected failure"),
    }
}

#[tokio::test]
async fn mcp_resources_registers_three_tools_and_validates_server() {
    use cordis_rs::core_services::{ToolError, ToolExecContext, provide_tools};
    use cordis_rs::plugin_ports::{LIST_RESOURCES, READ_RESOURCE, mount_mcp_resources};

    let ctx = Context::root("mcp-resources-port");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let registry = mount_mcp_resources(&ctx).await.unwrap();

    let mut names = tools.names();
    names.sort();
    assert_eq!(
        names,
        [
            "list_mcp_resource_templates",
            "list_mcp_resources",
            "read_mcp_resource"
        ]
    );

    // Unregistered server fails as InvalidArgs before dispatch.
    let error = tools
        .invoke(
            LIST_RESOURCES,
            serde_json::json!({ "server": "missing" }),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidArgs(_)));

    registry.register(
        "demo".to_string(),
        std::sync::Arc::new(|method, params| {
            let resources = serde_json::json!([{ "uri": "file:///a", "name": "a" }]);
            Ok(match method {
                "resources/list" => serde_json::json!({ "resources": resources }),
                "resources/read" => serde_json::json!({ "contents": [{ "uri": params["uri"] }] }),
                _ => serde_json::json!({}),
            })
        }),
    );
    assert_eq!(registry.servers(), vec!["demo"]);

    let list = tools
        .invoke(
            LIST_RESOURCES,
            serde_json::json!({ "server": "demo" }),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(list.content["resources"].as_array().unwrap().len(), 1);

    let read = tools
        .invoke(
            READ_RESOURCE,
            serde_json::json!({ "server": "demo", "uri": "file:///a" }),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(read.content["contents"][0]["uri"], "file:///a");
    assert!(read.rendered[0].contains("file:///a"));

    assert!(registry.unregister("demo"));
}

#[tokio::test]
async fn hooks_claude_code_parses_selects_and_maps_phase_one() {
    use cordis_rs::core_services::{HookOutput, MergedDecision, merge_hook_outputs};
    use cordis_rs::plugin_ports::{
        HookEffect, HookEvent, audit_pair, map_outcome, parse_config, select_hooks,
    };

    let raw = serde_json::json!({
        "defaultTimeoutMs": 5000,
        "hooks": {
            "PreToolUse": [{
                "matcher": "bash",
                "hooks": [{ "command": "check.sh" }]
            }],
            "Stop": [{ "hooks": [{ "command": "ignored.sh" }] }]
        }
    });
    let config = parse_config(&raw).unwrap();
    assert_eq!(config.hooks.len(), 1);
    assert_eq!(config.default_timeout_ms, 5000);
    assert_eq!(
        select_hooks(&config, HookEvent::PreToolUse, "bash").len(),
        1
    );
    assert!(select_hooks(&config, HookEvent::PreToolUse, "fs").is_empty());

    let deny = HookOutput {
        decision: MergedDecision::Deny,
        reason: Some("blocked".to_string()),
        stop: false,
        context: None,
    };
    assert_eq!(
        map_outcome(
            HookEvent::PreToolUse,
            &merge_hook_outputs(std::slice::from_ref(&deny))
        ),
        HookEffect::Reject("blocked".to_string())
    );
    assert_eq!(
        map_outcome(
            HookEvent::PostToolUse,
            &merge_hook_outputs(std::slice::from_ref(&deny))
        ),
        HookEffect::Block("blocked".to_string())
    );

    let pair = audit_pair(HookEvent::PreToolUse, "check.sh", &deny, Some(42));
    assert_eq!(pair.len(), 2);
    assert_eq!(pair[0].0, "hook/invoked");
    assert_eq!(pair[1].0, "hook/result");
    assert_eq!(pair[1].1["run_event_seq"], 42);
}

#[tokio::test]
async fn multiple_port_mounts_unregister_and_reload_with_fs_service() {
    use cordis_rs::core_services::provide_tools;
    use cordis_rs::plugin_ports::{mount_tool_fs, mount_tool_str_replace_editor};
    use cordis_rs::services::fs::provide as provide_fs_service;
    use std::time::Duration;

    let ctx = Context::root("multi-port-unload");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    provide_fs_service(&ctx).await.unwrap();
    mount_tool_fs(
        &ctx,
        cordis_rs::core_services::FileSystem::local(),
        Default::default(),
    )
    .await
    .unwrap();
    mount_tool_str_replace_editor(&ctx).await.unwrap();

    assert_eq!(
        tools
            .names()
            .iter()
            .filter(|name| *name == "str_replace_editor")
            .count(),
        1
    );
    assert!(tools.names().contains(&"read".to_string()));

    // Removing the shared fs service unloads both dependent fibers. Poll with
    // a bounded deadline instead of a fixed sleep.
    ctx.remove_service("fs").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let names = tools.names();
            if !names.contains(&"str_replace_editor".to_string())
                && !names.contains(&"read".to_string())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dependent tools were not unloaded after fs removal");

    // Re-provide fs: both reload without duplicate tool registrations.
    provide_fs_service(&ctx).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let names = tools.names();
            if names
                .iter()
                .filter(|name| *name == "str_replace_editor")
                .count()
                == 1
                && names.iter().filter(|name| *name == "read").count() == 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dependent tools did not reload after fs re-provide");
}

#[tokio::test]
async fn session_turn_outline_folds_turns_and_commits_draft() {
    use cordis_rs::core_services::{Session, provide_agents, provide_sessions};
    use cordis_rs::plugin_ports::mount_session_turn_outline;

    let ctx = Context::root("turn-outline-port");
    provide_agents(&ctx).await.unwrap();
    let (sessions, projections) = provide_sessions(&ctx).await.unwrap();
    mount_session_turn_outline(&ctx).await.unwrap();
    let session = sessions
        .create(Some("outline-1".to_string()), None)
        .unwrap();

    session.append("turn/start", serde_json::json!({ "turn": 1 }));
    session.append(
        "user/message",
        serde_json::json!({
            "source": { "kind": "user" },
            "content": [{ "type": "text", "text": "  hello   world  " }]
        }),
    );
    session.append(
        "assistant/message",
        serde_json::json!({
            "message": { "content": [{ "type": "text", "text": "draft answer" }] }
        }),
    );
    let before_end = projections.view("turnOutline", &session).unwrap();
    assert_eq!(before_end["turns"][0]["response"], "");

    session.append("turn/end", serde_json::json!({}));
    let view = projections.view("turnOutline", &session).unwrap();
    assert_eq!(view["turns"].as_array().unwrap().len(), 1);
    assert_eq!(view["turns"][0]["prompt"], "hello world");
    assert_eq!(view["turns"][0]["response"], "draft answer");

    // Steering message must not overwrite the first prompt; a new turn appends.
    session.append(
        "user/message",
        serde_json::json!({
            "source": { "kind": "user" },
            "content": [{ "type": "text", "text": "steering" }]
        }),
    );
    session.append("turn/start", serde_json::json!({ "turn": 2 }));
    let view = projections.view("turnOutline", &session).unwrap();
    assert_eq!(view["turns"].as_array().unwrap().len(), 2);
    assert_eq!(view["turns"][0]["prompt"], "hello world");

    // Non-user sources never fill a prompt.
    session.append(
        "user/message",
        serde_json::json!({
            "source": { "kind": "time-context" },
            "content": [{ "type": "text", "text": "injected" }]
        }),
    );
    let view = projections.view("turnOutline", &session).unwrap();
    assert_eq!(view["turns"][1]["prompt"], "");

    let _ = Session::new("unused", None);
}

#[tokio::test]
async fn deepseek_llm_api_extensions_prepares_and_accepts_idempotently() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cordis_rs::plugin_ports::{PreparedField, mount_deepseek_llm_api_extensions};

    let ctx = Context::root("llm-api-extensions");
    let registry = mount_deepseek_llm_api_extensions(&ctx).await.unwrap();

    assert!(!registry.register(" bad", Arc::new(|_| Ok(None))));
    assert!(registry.register(
        "thinking",
        Arc::new(|_| {
            Ok(Some(PreparedField {
                value: serde_json::json!({ "budget": 10 }),
                accept: None,
            }))
        })
    ));
    assert!(!registry.register("thinking", Arc::new(|_| Ok(None))));

    let calls = Arc::new(AtomicUsize::new(0));
    let accept_calls = calls.clone();
    assert!(registry.register(
        "trace",
        Arc::new(move |_| {
            let counter = accept_calls.clone();
            Ok(Some(PreparedField {
                value: serde_json::json!({ "id": "t1" }),
                accept: Some(Arc::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })),
            }))
        })
    ));

    let request = serde_json::json!({ "model": "deepseek-chat" });
    let prepared = registry.prepare(&request, false);
    assert_eq!(prepared.fields["thinking"]["budget"], 10);
    assert_eq!(prepared.fields["trace"]["id"], "t1");
    assert!(prepared.failures.is_empty());

    // Acceptance runs once and joins subsequent callers.
    assert!(prepared.accept().is_ok());
    assert!(prepared.accept().is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Deep-clone: mutating the original provider value cannot affect fields.
    let shared = Arc::new(std::sync::Mutex::new(serde_json::json!({ "n": 1 })));
    let source = shared.clone();
    registry.register(
        "clone",
        Arc::new(move |_| {
            Ok(Some(PreparedField {
                value: source.lock().unwrap().clone(),
                accept: None,
            }))
        }),
    );
    *shared.lock().unwrap() = serde_json::json!({ "n": 99 });
    let prepared = registry.prepare(&request, false);
    assert_eq!(prepared.fields["clone"]["n"], 99); // snapshot taken at prepare time

    // A failing provider is omitted and reported once.
    registry.register("broken", Arc::new(|_| Err("boom".to_string())));
    let prepared = registry.prepare(&request, false);
    assert!(prepared.fields.get("broken").is_none());
    assert_eq!(prepared.failures[0].0, "broken");
}

#[tokio::test]
async fn session_title_first_prompt_validates_and_strips_emphasis() {
    use cordis_rs::plugin_ports::{
        FirstPromptTitleConfig, first_title_line, mount_session_title_first_prompt_llm,
        strip_emphasis,
    };

    let valid = FirstPromptTitleConfig {
        provider: "deepseek".to_string(),
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };
    let ctx = Context::root("first-prompt-title");
    mount_session_title_first_prompt_llm(&ctx, valid)
        .await
        .unwrap();

    let bad = FirstPromptTitleConfig {
        target_words: 0,
        provider: "deepseek".to_string(),
        model: "m".to_string(),
        ..Default::default()
    };
    assert!(
        mount_session_title_first_prompt_llm(&Context::root("bad-title"), bad)
            .await
            .is_err()
    );

    assert_eq!(strip_emphasis("**A title**"), "A title");
    assert_eq!(strip_emphasis("*one*"), "one");
    assert_eq!(strip_emphasis("plain"), "plain");
    assert_eq!(
        first_title_line("  **Hello**\nsecond"),
        Some("Hello".to_string())
    );
    assert_eq!(first_title_line("\n\n"), None);
}
