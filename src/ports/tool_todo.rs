//! Reusable Rust port of the official `@deepseek-ai/dsh-tool-todo` plugin.
//!
//! The plugin body is exposed as a library so a host (for example the GPUI
//! client) can mount it on a `Context` and drive `todo_write` through the
//! shared `tools` seam. Source: `packages/todo/tool-todo/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::context::Disposer;
use crate::events::BoxFuture;
use crate::{Context, Error, Plugin, Result};

const DESCRIPTION_HEAD: &str = "Record and update a task list to plan multi-step work and show progress; skip it for trivial single-step tasks. Add one todo per concrete step before you start. ";
const DESCRIPTION_PARALLEL: &str = "While work remains, keep the todos being worked on `in_progress`, several only when work runs in parallel. ";
const DESCRIPTION_SINGLE: &str = "While work remains, keep exactly one todo `in_progress`. ";
const DESCRIPTION_TAIL: &str = "Mark each todo `completed` as soon as it is done.";

pub fn describe(allow_parallel: bool) -> String {
    let clause = if allow_parallel {
        DESCRIPTION_PARALLEL
    } else {
        DESCRIPTION_SINGLE
    };
    format!("{DESCRIPTION_HEAD}{clause}{DESCRIPTION_TAIL}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

impl TodoItem {
    pub fn new(content: impl Into<String>, status: TodoStatus) -> Self {
        Self {
            content: content.into(),
            status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counts {
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoWriteResult {
    pub todos: Vec<TodoItem>,
    pub counts: Counts,
}

impl TodoWriteResult {
    pub fn render(&self) -> String {
        format!(
            "Updated todo list: {} pending, {} in progress, {} completed.",
            self.counts.pending, self.counts.in_progress, self.counts.completed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFailure {
    Args(String),
    ContextUnavailable(String),
    NotFound(String),
}

impl std::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Args(message) => write!(f, "invalid arguments: {message}"),
            Self::ContextUnavailable(message) => write!(f, "context unavailable: {message}"),
            Self::NotFound(name) => write!(f, "tool not found: {name}"),
        }
    }
}

impl std::error::Error for ToolFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTodo {
    pub content: String,
    pub status: String,
}

impl RawTodo {
    pub fn new(content: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            status: status.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoWriteArgs {
    pub todos: Vec<RawTodo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecution {
    pub agent: Option<String>,
}

impl ToolExecution {
    pub fn agent(name: impl Into<String>) -> Self {
        Self {
            agent: Some(name.into()),
        }
    }

    pub fn anonymous() -> Self {
        Self { agent: None }
    }
}

/// Port of upstream `toTodoList`.
pub fn to_todo_list(
    raw: &[RawTodo],
    allow_parallel: bool,
) -> std::result::Result<Vec<TodoItem>, ToolFailure> {
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

type ToolHandler = Arc<
    dyn Fn(
            TodoWriteArgs,
            ToolExecution,
        ) -> BoxFuture<'static, std::result::Result<TodoWriteResult, ToolFailure>>
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

/// Shared `ctx.tools` seam. A host keeps a clone and calls `invoke` to drive
/// the registered `todo_write` tool.
#[derive(Default, Clone)]
pub struct Tools {
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

    pub fn names(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("tools poisoned")
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    pub fn description(&self, name: &str) -> Option<String> {
        self.state
            .lock()
            .expect("tools poisoned")
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.description.clone())
    }

    pub async fn invoke(
        &self,
        name: &str,
        args: TodoWriteArgs,
        exec: ToolExecution,
    ) -> std::result::Result<TodoWriteResult, ToolFailure> {
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

    /// Synchronous variant for hosts whose event loop cannot await (for
    /// example a GPUI click handler). The todo handler itself never awaits.
    pub fn invoke_blocking(
        &self,
        name: &str,
        args: TodoWriteArgs,
        exec: ToolExecution,
    ) -> std::result::Result<TodoWriteResult, ToolFailure> {
        futures::executor::block_on(self.invoke(name, args, exec))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    TodoWrite { todos: Vec<TodoItem> },
    TurnStart,
}

/// Shared `ctx.sessionProjections` seam holding the `todos` projection.
#[derive(Default, Clone)]
pub struct SessionProjections {
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

    pub fn is_registered(&self, key: &str) -> bool {
        self.registered
            .lock()
            .expect("projections poisoned")
            .iter()
            .any(|current| current == key)
    }

    pub fn append(&self, event: SessionEvent) {
        let mut guard = self.todos.lock().expect("projection state poisoned");
        match event {
            SessionEvent::TodoWrite { todos } => *guard = Some(todos),
            SessionEvent::TurnStart => *guard = None,
        }
    }

    /// Current todo list as rendered by the `todos` projection.
    pub fn todos_view(&self) -> Option<Vec<TodoItem>> {
        self.todos
            .lock()
            .expect("projection state poisoned")
            .clone()
    }
}

fn execute_todo_write(
    args: TodoWriteArgs,
    exec: &ToolExecution,
    allow_parallel: bool,
    projections: &SessionProjections,
) -> std::result::Result<TodoWriteResult, ToolFailure> {
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

/// The `dsh-tool-todo` plugin. `inject = ['tools', 'sessionProjections']`.
pub struct ToolTodo {
    pub allow_parallel: bool,
}

#[async_trait]
impl Plugin for ToolTodo {
    fn name(&self) -> &str {
        "tool-todo"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "sessionProjections"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
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

/// Both seams a host needs to drive and observe the mounted tool.
#[derive(Clone)]
pub struct MountedTodo {
    pub tools: Tools,
    pub projections: SessionProjections,
}

/// Convenience: mount the port with both seams provided on `ctx`.
pub async fn mount(ctx: &Context, allow_parallel: bool) -> Result<MountedTodo> {
    let tools = Tools::default();
    let projections = SessionProjections::default();
    ctx.provide("tools", tools.clone()).await?;
    ctx.provide("sessionProjections", projections.clone())
        .await?;
    let fiber = crate::Fiber::mount_retained(ctx, Box::new(ToolTodo { allow_parallel })).await?;
    if fiber.state() != crate::Lifecycle::Active {
        return Err(Error::Other(format!(
            "tool-todo did not become active: {:?}",
            fiber.state()
        )));
    }
    Ok(MountedTodo { tools, projections })
}

/// Error helper for hosts that need an owned error from a tool failure.
pub fn tool_failure_error(failure: ToolFailure) -> Error {
    Error::Other(failure.to_string())
}
