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
pub use loader::{
    Composition, Entry, LoaderError, MountReport, PluginFactory, PluginRegistry, VarTable,
};
pub use plugin::{Fiber, FunctionPlugin, Plugin};

/// Reusable ports of official DSH plugin implementations.
pub mod plugin_ports {
    pub use crate::ports::command_feedback::{
        CommandFeedback, FEEDBACK_CATEGORIES, FeedbackRecord, USAGE as FEEDBACK_USAGE,
        feedback_events, mount as mount_command_feedback, record_feedback,
    };
    pub use crate::ports::llm_deepseek::{
        DeepSeekConfig, LlmDeepSeek, PROVIDER as DEEPSEEK_PROVIDER, mount as mount_llm_deepseek,
        resolve_api_key,
    };
    pub use crate::ports::message_feedback::{
        FeedbackError, FeedbackItem, MessageFeedback, MessageFeedbackConfig, MessageFeedbackPlugin,
        Rating, mount as mount_message_feedback,
    };
    pub use crate::ports::session_persistence_jsonl::{
        JsonlPersistence, JsonlSessionHandle, PersistedEvent, PersistenceError,
        mount as mount_session_persistence_jsonl,
    };
    pub use crate::ports::skill_filesystem::{
        SkillFilesystem, SkillFsConfig, discover as discover_skills,
        mount as mount_skill_filesystem,
    };
    pub use crate::ports::tool_bash::{
        BashArgs, DESCRIPTION as BASH_DESCRIPTION, ToolBash, ToolBashConfig, job_handle,
        mount as mount_tool_bash, parse_bash_args,
    };
    pub use crate::ports::tool_fs::{
        EditInput, READ_LIMIT, STREAM_MIN_SIZE, ToolFs, ToolFsConfig, mount as mount_tool_fs,
        parse_edit_args, parse_read_args, parse_write_args,
    };
    pub use crate::ports::tool_todo::{
        Counts, MountedTodo, RawTodo, SessionEvent, SessionProjections, TodoItem, TodoStatus,
        TodoWriteArgs, TodoWriteResult, ToolExecution, ToolFailure, ToolTodo, Tools, describe,
        mount, to_todo_list,
    };
    pub use crate::ports::tool_working_directory::{
        DESCRIPTION as WORKING_DIRECTORY_DESCRIPTION, ToolWorkingDirectory,
        execute as execute_working_directory, mount as mount_working_directory,
    };
    pub use crate::services::working_directory::WorkingDirectory;
}

/// Core service implementations.
pub mod core_services {
    pub use crate::services::agent_loop::{
        AgentLoop, AgentRun, AgentStep, CallParser, CheckpointHook, ToolCall,
    };
    pub use crate::services::commands::{
        CommandDefinition, CommandHandler, CommandInvocation, CommandRegistry, CommandResult,
        provide as provide_commands,
    };
    pub use crate::services::fs::{
        FileSystem, FsEditOutcome, FsInfo, FsWriteOutcome, ReadWindow, WriteOperation,
        provide as provide_fs,
    };
    pub use crate::services::jobs::{
        JobError, JobHandle, JobRegistry, JobStatus, JobView, provide as provide_jobs,
    };
    pub use crate::services::llm::{
        ChatMessage, LlmProvider, LlmRequest, LlmResponse, LlmService, StreamChunk, echo_provider,
        from_async_text, provide as provide_llm,
    };
    pub use crate::services::sessions::{
        CheckpointCursor, ProjectionFold, Session, SessionEvent as LoggedSessionEvent,
        SessionHeader, SessionProjectionRegistry, SessionStore, provide as provide_sessions,
    };
    pub use crate::services::shell::{
        Shell, ShellError, ShellRequest, ShellResult, ShellSpec, provide as provide_shell,
    };
    pub use crate::services::skills::{
        Skill, SkillCatalog, SkillProvider, SkillRegistry, provide as provide_skills,
    };
    pub use crate::services::tools::{
        ToolArgs, ToolDefinition, ToolError, ToolExecContext, ToolHandler, ToolNext, ToolOutput,
        ToolPipeline, ToolRegistry, invoke_blocking, provide as provide_tools,
    };
    pub use crate::services::working_directory::{
        WorkingDirectory, provide as provide_working_directory,
    };
}
