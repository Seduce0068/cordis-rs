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
pub use events::{EmitErrors, Event, FallibleEmitListener};
pub use loader::{
    Composition, CompositionState, Entry, LoaderError, MountReport, PluginFactory, PluginRegistry,
    VarTable,
};
pub use plugin::{Fiber, FunctionPlugin, Plugin};

/// Reusable ports of official DSH plugin implementations.
pub mod plugin_ports {
    pub use crate::ports::agent_instructions::{register_baseline, register_file};
    pub use crate::ports::command_compact::{
        CommandCompact, USAGE as COMMAND_COMPACT_USAGE, mount as mount_command_compact,
    };
    pub use crate::ports::command_feedback::{
        CommandFeedback, FEEDBACK_CATEGORIES, FeedbackRecord, USAGE as FEEDBACK_USAGE,
        feedback_events, mount as mount_command_feedback, record_feedback,
    };
    pub use crate::ports::command_goal::{
        CommandGoal, DEFAULT_MAX_GOAL_ROUNDS, USAGE as GOAL_USAGE, mount as mount_command_goal,
    };
    pub use crate::ports::compaction_image_offload::{
        CompactionImageOffload, Decision, EVENT_TYPE, IMAGE_OFFLOAD_PROJECTION_KEY,
        IMAGE_OFFLOAD_REQUIRED_CODE, OffloadTarget, decide,
        mount as mount_compaction_image_offload, offload_payload, select_oldest,
        select_oldest_after, validate_payload,
    };
    pub use crate::ports::compaction_tool_result_pruner::{
        CompactionToolResultPruner, PruneSessionResult, REPLACEMENT_EVENT, SERVICE_NAME,
        SHADOW_PRICE_EVENT, measure_content, mount as mount_compaction_tool_result_pruner,
        provide as provide_tool_result_pruner, prune_content, prune_session,
    };
    pub use crate::ports::feedback_producer::{
        FeedbackProducer, ProducerOutcome, TurnStopping, TurnStoppingEvent,
        mount as mount_feedback_producer, produce, turn_stopping_event,
    };
    pub use crate::ports::fs_observation_policy::{
        FsObservationPolicy, FsObservationPolicyPlugin, Observation, PolicyError, WriteIntent,
        provide as provide_fs_observation_policy,
    };
    pub use crate::ports::fs_patch::{
        ApplyHunksResult, FsPatchLibrary, HunkApplyFailure, PatchHunk, PatchOperation,
        PatchParseError, apply_hunks, parse_patch,
    };
    pub use crate::ports::harness_dev_context::{
        HARNESS_SOURCE_KEY, HARNESS_SOURCE_PROMPT, WEB_GUI_URL_KEY, WEB_SURFACE_PROMPT,
        harness_context_value, with_harness_context,
    };
    pub use crate::ports::hooks_claude_code::{
        AUDIT_INVOKED, AUDIT_RESULT, DEFAULT_TIMEOUT_MS, HookEffect, HookEntry, HookEvent,
        HooksClaudeCodePlugin, HooksConfig, audit_pair, map_outcome,
        mount as mount_hooks_claude_code, parse_config, select_hooks,
    };
    pub use crate::ports::llm_deepseek::{
        CredentialError, DEFAULT_API_KEY_ENV, DEFAULT_BASE_URL, DeepSeekConfig, LlmDeepSeek,
        PROVIDER as DEEPSEEK_PROVIDER, mount as mount_llm_deepseek, resolve_api_key,
    };
    pub use crate::ports::mcp_resources::{
        LIST_RESOURCES, LIST_TEMPLATES, MCP_RESOURCES_SERVICE, McpResourceRegistry,
        McpResourcesPlugin, READ_RESOURCE, ResourceProvider, mount as mount_mcp_resources,
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
    pub use crate::ports::time_context::{
        DEFAULT_REFRESH_INTERVAL_MS, PROJECTION_KEY, PreStepArgs, PreStepDecision,
        TimeContextPlugin, TimeContextPluginConfig, mount as mount_time_context, sample_message,
    };
    pub use crate::ports::timeout_policy::{
        POLICY_NAME, TOOL_TIMEOUT_CODE, TimeoutPolicyPlugin, TimeoutPolicyPluginConfig,
        evaluate as evaluate_tool_timeout, mount as mount_timeout_policy, timeout_output,
    };
    pub use crate::ports::tmux_context::{
        TMUX_DEFAULT_REFRESH_INTERVAL_MS, TMUX_PROJECTION_KEY, TmuxContextPlugin,
        TmuxContextPluginConfig, mount as mount_tmux_context, sample_reading, should_sample,
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
    pub use crate::ports::tool_goal::{
        BLOCKED_AFTER_CONSECUTIVE_ROUNDS, GoalAuthority, ToolGoal, ToolGoalConfig,
        mount as mount_tool_goal,
    };
    pub use crate::ports::tool_jobs::{
        DEFAULT_WAIT_TIMEOUT_MS, INLINE_OUTPUT_BYTES, MAX_WAIT_TIMEOUT_MS, ToolJobs,
        mount as mount_tool_jobs,
    };
    pub use crate::ports::tool_schedule::{ToolSchedule, mount as mount_tool_schedule};
    pub use crate::ports::tool_session_list::{
        DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, Page, ToolSessionList,
        mount as mount_tool_session_list, resolve_page,
    };
    pub use crate::ports::tool_session_query::{
        ToolSessionQuery, ToolSessionQueryConfig, mount as mount_tool_session_query,
    };
    pub use crate::ports::tool_str_replace_editor::{
        ToolStrReplaceEditor, mount as mount_tool_str_replace_editor,
    };
    pub use crate::ports::tool_subagent::{
        ToolSubagent, ToolSubagentConfig, mount as mount_tool_subagent,
    };
    pub use crate::ports::tool_todo::{
        Counts, MountedTodo, RawTodo, SessionEvent, SessionProjections, TodoItem, TodoStatus,
        TodoWriteArgs, TodoWriteResult, ToolExecution, ToolFailure, ToolTodo, Tools, describe,
        mount as mount_tool_todo, to_todo_list, tool_failure_error,
    };
    pub use crate::ports::tool_working_directory::{
        DESCRIPTION as WORKING_DIRECTORY_DESCRIPTION, ToolWorkingDirectory,
        execute as execute_working_directory, mount as mount_working_directory,
    };
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
    pub use crate::services::credentials::{
        CredentialInfo, CredentialProvider, CredentialRef, ResolvedCredential, credential_ref,
        provide as provide_credentials,
    };
    pub use crate::services::file_reference::{
        ActiveAtToken, FileReferences, ReferenceKind, active_at_token, format_file_mention,
        provide as provide_file_references,
    };
    pub use crate::services::file_search::{
        DEFAULT_EXCLUDED_DIRECTORIES, DEFAULT_MAX_ENTRIES, DEFAULT_MAX_RESULTS,
        FileSearchCandidate, FileSearchConfig, WorkspaceFileSearch, provide as provide_file_search,
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
    pub use crate::services::plugin_inventory::{
        PluginEntryView, PluginInventory, PluginInventorySnapshot,
        provide as provide_plugin_inventory,
    };
    pub use crate::services::pruner::{
        PruneError, PrunedText, PrunerConfig, ToolResultPruner, provide as provide_pruner,
    };
    pub use crate::services::repeat_reminder::{
        DEFAULT_THRESHOLDS, Reminder, ReminderConfig, RepeatToolReminder, canonicalize,
        provide as provide_repeat_reminder, sort_json_value, validate_thresholds,
    };
    pub use crate::services::retry::{RetryDecision, RetryPolicy, provide as provide_retry};
    pub use crate::services::schedule::{
        ScheduleError, ScheduleRecord, ScheduleService, ScheduleStatus, provide as provide_schedule,
    };
    pub use crate::services::session_query::{
        SessionHit, SessionQueryEngine, provide as provide_session_query,
    };
    pub use crate::services::session_reference::{
        SESSION_REFERENCE_SCHEME, SessionReference, SessionReferences,
        decode_session_reference_uri, encode_session_reference_uri,
        format_session_reference_mention, provide as provide_session_references,
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
    pub use crate::services::time_context::{
        TimeContext, TimeContextConfig, format_sample, has_utc_offset,
        provide as provide_time_context, validate_refresh_interval,
    };
    pub use crate::services::timeout_policy::{
        TimeoutPolicy, TimeoutResult, provide as provide_timeout_policy, tool_timeout_result,
    };
    pub use crate::services::tmux_context::{
        READING_PREFIX, TmuxContext, TmuxLocation, format_reading, is_changed, parse_location,
        provide as provide_tmux_context,
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
