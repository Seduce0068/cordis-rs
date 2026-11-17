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
    Composition, CompositionState, Entry, LoaderError, MountReport, PluginFactory, PluginRegistry,
    VarTable,
};
pub use plugin::{Fiber, FunctionPlugin, Plugin};

/// Reusable ports of official DSH plugin implementations.
pub mod plugin_ports {
    pub use crate::ports::command_compact::{
        CommandCompact, USAGE as COMMAND_COMPACT_USAGE, mount as mount_command_compact,
    };
    pub use crate::ports::command_feedback::{
        CommandFeedback, FEEDBACK_CATEGORIES, FeedbackRecord, USAGE as FEEDBACK_USAGE,
        feedback_events, mount as mount_command_feedback, record_feedback,
    };
    pub use crate::ports::command_goal::{
        CommandGoal, USAGE as GOAL_USAGE, mount as mount_command_goal,
    };
    pub use crate::ports::feedback_producer::{
        FeedbackProducer, ProducerOutcome, TurnStopping, TurnStoppingEvent,
        mount as mount_feedback_producer, produce, turn_stopping_event,
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
    pub use crate::ports::session_title_llm::{SessionTitleLlm, mount as mount_session_title_llm};
    pub use crate::ports::skill_filesystem::{
        SkillFilesystem, SkillFsConfig, discover as discover_skills,
        mount as mount_skill_filesystem,
    };
    pub use crate::ports::tool_bash::{
        BashArgs, DESCRIPTION as BASH_DESCRIPTION, ToolBash, ToolBashConfig, job_handle,
        mount as mount_tool_bash, parse_bash_args,
    };
    pub use crate::ports::tool_exit_plan_mode::{
        EXIT_PLAN_MODE, ExitPlanError, ToolExitPlanMode, mount as mount_tool_exit_plan_mode,
        validate_plan,
    };
    pub use crate::ports::tool_fs::{
        EditInput, READ_LIMIT, STREAM_MIN_SIZE, ToolFs, ToolFsConfig, mount as mount_tool_fs,
        parse_edit_args, parse_read_args, parse_write_args,
    };
    pub use crate::ports::tool_schedule::{ToolSchedule, mount as mount_tool_schedule};
    pub use crate::ports::tool_session_query::{
        ToolSessionQuery, ToolSessionQueryConfig, mount as mount_tool_session_query,
    };
    pub use crate::ports::tool_subagent::{
        ToolSubagent, ToolSubagentConfig, mount as mount_tool_subagent,
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
    pub use crate::services::agents::{
        AgentError, AgentFactory, AgentRecord, AgentRegistry, provide as provide_agents,
    };
    pub use crate::services::approval::{
        Answerer, ApprovalDecision, ApprovalError, ApprovalMode, ApprovalOutcome, ApprovalRequest,
        ApprovalService, provide as provide_approval, require_approval,
    };
    pub use crate::services::commands::{
        CommandDefinition, CommandHandler, CommandInvocation, CommandRegistry, CommandResult,
        provide as provide_commands,
    };
    pub use crate::services::compaction::{
        CompactionConfig, CompactionEngine, CompactionResult, CompactionTrigger,
        provide as provide_compaction,
    };
    pub use crate::services::fs::{
        FileSystem, FsEditOutcome, FsInfo, FsWriteOutcome, ReadWindow, WriteOperation,
        provide as provide_fs,
    };
    pub use crate::services::goals::{
        Goal, GoalError, GoalStatus, GoalStore, provide as provide_goals,
    };
    pub use crate::services::hook_protocol::{
        HookOutput, MergedDecision, MergedHookOutcome, is_match_all, matcher_diagnostic, matches,
        merge_hook_outputs,
    };
    pub use crate::services::jobs::{
        JobError, JobHandle, JobRegistry, JobStatus, JobView, provide as provide_jobs,
    };
    pub use crate::services::llm::{
        ChatMessage, LlmProvider, LlmRequest, LlmResponse, LlmService, StreamChunk, echo_provider,
        from_async_text, provide as provide_llm,
    };
    pub use crate::services::permission_presets::{
        ApprovalPolicy, PermissionPresets, PermissionState, Preset, PresetSpec, SandboxMode,
        provide as provide_permission_presets,
    };
    pub use crate::services::plan_mode::{PlanMode, PlanModeEvent, provide as provide_plan_mode};
    pub use crate::services::pruner::{
        PruneError, PrunedText, PrunerConfig, ToolResultPruner, provide as provide_pruner,
    };
    pub use crate::services::retry::{RetryDecision, RetryPolicy, provide as provide_retry};
    pub use crate::services::schedule::{
        ScheduleError, ScheduleRecord, ScheduleService, ScheduleStatus, provide as provide_schedule,
    };
    pub use crate::services::session_query::{
        SessionHit, SessionQueryEngine, provide as provide_session_query,
    };
    pub use crate::services::session_titles::{
        SessionTitles, TitleProviderConfig, frame_messages, provide as provide_session_titles,
        title_from_output, title_system_prompt,
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
    pub use crate::services::storage::{
        DomainSpec, MemoryBackend, StorageError, StorageHub, provide as provide_storage,
    };
    pub use crate::services::subagents::{
        SubagentError, SubagentRegistry, SubagentRequest, SubagentResult,
        provide as provide_subagents,
    };
    pub use crate::services::system_prompt::{
        AssembleContext, PromptAssembly, PromptError, PromptSection, SystemPrompt,
        provide as provide_system_prompt,
    };
    pub use crate::services::telemetry::{
        EventLogOptions, EventLogReporter, EventRecord, OTel, TelemetryError,
        provide as provide_otel,
    };
    pub use crate::services::token_meter::{
        TokenMeasurement, TokenMeter, estimate_message, estimate_text,
        provide as provide_token_meter,
    };
    pub use crate::services::tools::{
        ToolArgs, ToolDefinition, ToolError, ToolExecContext, ToolHandler, ToolNext, ToolOutput,
        ToolPipeline, ToolRegistry, invoke_blocking, provide as provide_tools,
    };
    pub use crate::services::working_directory::{
        WorkingDirectory, provide as provide_working_directory,
    };
    pub use crate::services::workspace::{
        WorkspaceError, WorkspaceRecord, WorkspaceRegistry, provide as provide_workspace,
    };
}
