//! A Rust reimplementation of Cordis, the plugin runtime underneath DSH.
//!
//! Scope of this PoC: service registry, typed events with all five dispatch
//! modes, reversible effects, fibers with notification-driven reload, plus a
//! scoped config validator and a minimal `cordis.yml` composition loader.
//! Full Schemastery semantics and file-watching HMR remain out of scope.

pub mod config;
pub mod context;
pub mod error;
pub mod events;
pub mod loader;
pub mod plugin;
pub mod ports;
pub mod services;

pub use config::{
    FieldSpec, FieldType, ValidationError, ValidationErrors, ValidationReason,
    Value as ConfigValue, validate as validate_config,
};
pub use context::{Context, Disposer, EffectScope, Lifecycle};
pub use error::{Error, Result};
pub use events::Event;
pub use loader::{Composition, Entry, LoaderError, PluginFactory, PluginRegistry, VarTable};
pub use plugin::{Fiber, FunctionPlugin, Plugin};

/// Reusable ports of official DSH plugin implementations.
pub mod plugin_ports {
    pub use crate::ports::command_feedback::{
        CommandFeedback, FEEDBACK_CATEGORIES, FeedbackRecord, USAGE as FEEDBACK_USAGE,
        feedback_events, mount as mount_command_feedback, record_feedback,
    };
    pub use crate::ports::tool_todo::{
        Counts, MountedTodo, RawTodo, SessionEvent, SessionProjections, TodoItem, TodoStatus,
        TodoWriteArgs, TodoWriteResult, ToolExecution, ToolFailure, ToolTodo, Tools, describe,
        mount, to_todo_list,
    };
    pub use crate::ports::tool_working_directory::{
        DESCRIPTION as WORKING_DIRECTORY_DESCRIPTION, ToolWorkingDirectory, WorkingDirectory,
        execute as execute_working_directory, mount as mount_working_directory,
    };
}

/// Core service implementations.
pub mod core_services {
    pub use crate::services::agent_loop::{AgentLoop, AgentRun, AgentStep, CallParser, ToolCall};
    pub use crate::services::commands::{
        CommandDefinition, CommandHandler, CommandInvocation, CommandRegistry, CommandResult,
        provide as provide_commands,
    };
    pub use crate::services::llm::{
        ChatMessage, LlmProvider, LlmRequest, LlmResponse, LlmService, StreamChunk, echo_provider,
        provide as provide_llm,
    };
    pub use crate::services::sessions::{
        ProjectionFold, Session, SessionEvent as LoggedSessionEvent, SessionHeader,
        SessionProjectionRegistry, SessionStore, provide as provide_sessions,
    };
    pub use crate::services::tools::{
        ToolArgs, ToolDefinition, ToolError, ToolExecContext, ToolHandler, ToolNext, ToolOutput,
        ToolPipeline, ToolRegistry, invoke_blocking, provide as provide_tools,
    };
}
