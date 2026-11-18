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
pub use events::{EmitErrors, Event, EventDecodeError, EventOptions, FallibleEmitListener};
pub use loader::{
    Composition, CompositionState, Entry, LoaderError, MountReport, PluginFactory, PluginRegistry,
    VarTable,
};
pub use plugin::{Fiber, FunctionPlugin, Plugin};

/// Reusable ports of official DSH plugin implementations.
pub mod plugin_ports {
    pub use crate::ports::agent_instructions::register_baseline;
    pub use crate::ports::attachment_admission::{
        AttachmentError, AttachmentErrorCode, DecodedImage, EncodedFile, EncodedImage,
        decode_canonical_base64, decode_file, decode_images,
    };
    pub use crate::ports::code_language::{
        extension_for_path, language_for_path, read_lang_hint_for_path,
    };
    pub use crate::ports::command_builtins::{
        BuiltinCommandName, CommandDescriptor, builtin_command_name, resolve_command,
    };
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
    pub use crate::ports::command_parse::{ParsedCommand, parse_command};
    pub use crate::ports::compaction_automation::{
        AutomationOutcome, CompactionAutomation, CompactionAutomationConfig,
        CompactionAutomationPlugin, mount as mount_compaction_automation,
    };
    pub use crate::ports::compaction_checkpoint::{
        COMPACT_CHECKPOINT_KIND, CompactionCheckpointSource, checkpoint_source_json,
        compact_checkpoint_source, is_compact_checkpoint_source,
    };
    #[deprecated(note = "use `ImageOffloadDecision` instead")]
    pub use crate::ports::compaction_image_offload::Decision;
    #[deprecated(note = "use `IMAGE_OFFLOAD_EVENT_TYPE` instead")]
    pub use crate::ports::compaction_image_offload::EVENT_TYPE;
    pub use crate::ports::compaction_image_offload::{
        CompactionImageOffload, Decision as ImageOffloadDecision,
        EVENT_TYPE as IMAGE_OFFLOAD_EVENT_TYPE, IMAGE_OFFLOAD_PROJECTION_KEY,
        IMAGE_OFFLOAD_REQUIRED_CODE, OffloadTarget, decide,
        mount as mount_compaction_image_offload, offload_payload, select_oldest,
        select_oldest_after, validate_payload,
    };
    #[deprecated(note = "use `TOOL_RESULT_PRUNER_SERVICE_NAME` instead")]
    pub use crate::ports::compaction_tool_result_pruner::SERVICE_NAME;
    pub use crate::ports::compaction_tool_result_pruner::{
        CompactionToolResultPruner, PruneSessionResult, REPLACEMENT_EVENT,
        SERVICE_NAME as TOOL_RESULT_PRUNER_SERVICE_NAME, SHADOW_PRICE_EVENT, measure_content,
        mount as mount_compaction_tool_result_pruner, provide as provide_tool_result_pruner,
        prune_content, prune_session,
    };
    pub use crate::ports::deepseek_llm_api_extensions::{
        AcceptCallback, DEEPSEEK_EXTENSIONS_SERVICE, DeepSeekLlmApiExtensionRegistry,
        DeepSeekLlmApiExtensions, ExtensionProvider, PreparedExtensions, PreparedField,
        mount as mount_deepseek_llm_api_extensions,
    };
    pub use crate::ports::duration::{Decomposed, Duration};
    pub use crate::ports::feedback_producer::{
        FeedbackProducer, ProducerOutcome, TurnStopping, TurnStoppingEvent,
        mount as mount_feedback_producer, produce, turn_stopping_event,
    };
    pub use crate::ports::fs_errno::{FsError, FsErrorCode, fs_error_from_errno};
    pub use crate::ports::fs_observation_policy::{
        FsObservationPolicy, FsObservationPolicyPlugin, Observation, PolicyError, WriteIntent,
        provide as provide_fs_observation_policy,
    };
    pub use crate::ports::fs_patch::{
        ApplyHunksResult, FsPatchLibrary, HunkApplyFailure, PatchHunk, PatchOperation,
        PatchParseError, apply_hunks, parse_patch,
    };
    pub use crate::ports::goal_fold::{
        GOAL_CHANGE_VERSION, GoalBlockReason, GoalChange, GoalFoldState, GoalPhase, GoalRef,
        GoalSnapshot, apply_goal_change, apply_goal_event, decode_goal_change,
        empty_goal_fold_state, fold_goal, goal_change_ref,
    };
    pub use crate::ports::goal_round_driver::{
        DriveOutcome, GoalRoundDriver, GoalRoundDriverPlugin, mount as mount_goal_round_driver,
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
    pub use crate::ports::hooks_codex::{
        HooksCodex, codex_audit_pair, codex_effect, codex_outcome, load_codex_config,
        mount as mount_hooks_codex, select_tool_hooks, tool_payload,
    };
    pub use crate::ports::llm_attribution::{
        AppIdentity, attribution_headers, is_product_token, user_agent,
    };
    pub use crate::ports::llm_call_config::{LlmCallConfig, call_config_eq};
    pub use crate::ports::llm_deepseek::{
        CredentialError, DEFAULT_API_KEY_ENV, DEFAULT_BASE_URL, DeepSeekConfig, LlmDeepSeek,
        PROVIDER as DEEPSEEK_PROVIDER, mount as mount_llm_deepseek, resolve_api_key,
    };
    pub use crate::ports::llm_message::{
        CONTEXT_SUMMARY_MAX_CHARS, bound_context_summary, extract_text_content,
    };
    pub use crate::ports::llm_resilience::{TIMEOUT_PREFIX, with_llm_retry, with_llm_timeout};
    pub use crate::ports::mcp_resources::{
        LIST_RESOURCES, LIST_TEMPLATES, MCP_RESOURCES_SERVICE, McpResourceRegistry,
        McpResourcesPlugin, READ_RESOURCE, ResourceProvider, ScopeKey,
        mount as mount_mcp_resources,
    };
    pub use crate::ports::mcp_templates::{
        MCP_CREDENTIAL_NOT_FOUND, TemplateSegment, assert_template_syntax, credential_refs_in,
        resolve_template_string,
    };
    pub use crate::ports::message_feedback::{
        FeedbackError, FeedbackItem, MessageFeedback, MessageFeedbackConfig, MessageFeedbackPlugin,
        Rating, mount as mount_message_feedback,
    };
    pub use crate::ports::native_tool_executor::{
        DEFAULT_MAX_PARALLEL, ExecutionOutcome, ExecutorSelector, NativeToolExecutor,
        NativeToolExecutorPlugin, SERVICE_NAME as NATIVE_EXECUTOR_SERVICE, ToolCallSpec,
        ToolSchema, mount as mount_native_tool_executor,
    };
    pub use crate::ports::sandbox_policy::{
        PROJECTION_KEY as SANDBOX_PROJECTION_KEY, ResolvedPolicy, SANDBOX_INVALID_ROOT,
        SandboxPolicyConfig, SandboxPolicyService, is_valid_path, is_write_tool,
        mount as mount_sandbox_policy, render_policy_context, sandbox_mode_fold,
        sandbox_policy_middleware,
    };
    pub use crate::ports::session_filename::{
        parse_session_format_log_filename, session_format_log_filename, session_format_version,
    };
    pub use crate::ports::session_log_deepseek::{
        FIELD_NAME, SessionLogConfig, SessionLogDeepSeek, WATERMARK_EVENT, accepted_seq,
        fitting_prefix, mount as mount_session_log_deepseek, prepare_field,
    };
    pub use crate::ports::session_persistence_jsonl::RecoveryReport;
    pub use crate::ports::session_persistence_jsonl::{
        JsonlPersistence, JsonlSessionHandle, PersistedEvent, PersistenceError,
        mount as mount_session_persistence_jsonl,
    };
    pub use crate::ports::session_stats::{
        SessionStats, fold as session_stats_fold, mount as mount_session_stats,
        view as session_stats_view,
    };
    pub use crate::ports::session_title_first_prompt_llm::{
        FirstPromptTitleConfig, SessionTitleFirstPromptLlm, first_title_line,
        mount as mount_session_title_first_prompt_llm, strip_emphasis,
    };
    pub use crate::ports::session_title_llm::{SessionTitleLlm, mount as mount_session_title_llm};
    pub use crate::ports::session_turn_outline::{
        PROMPT_PREVIEW_LIMIT, RESPONSE_PREVIEW_LIMIT, SessionTurnOutline,
        TURN_OUTLINE_PROJECTION_KEY, fold, mount as mount_session_turn_outline,
    };
    pub use crate::ports::skill_filesystem::{
        SkillFilesystem, SkillFsConfig, discover as discover_skills,
        mount as mount_skill_filesystem,
    };
    pub use crate::ports::text_search::{
        SearchKeys, TextSearchIndex, normalize as normalize_search,
    };
    #[deprecated(note = "use `TIME_CONTEXT_PROJECTION_KEY` instead")]
    pub use crate::ports::time_context::PROJECTION_KEY;
    pub use crate::ports::time_context::{
        DEFAULT_REFRESH_INTERVAL_MS, PROJECTION_KEY as TIME_CONTEXT_PROJECTION_KEY, PreStepArgs,
        PreStepDecision, TimeContextPlugin, TimeContextPluginConfig, mount as mount_time_context,
        sample_message,
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
    pub use crate::ports::tool_pairing::{
        CutBalance, ToolPairingError, all_cuts, balanced_after, balanced_before,
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
    #[deprecated(note = "use `TodoWriteCounts` instead")]
    pub use crate::ports::tool_todo::Counts;
    #[deprecated(note = "use `TodoSessionEvent` instead")]
    pub use crate::ports::tool_todo::SessionEvent;
    #[deprecated(note = "use `TodoTools` instead")]
    pub use crate::ports::tool_todo::Tools;
    pub use crate::ports::tool_todo::{
        Counts as TodoWriteCounts, MountedTodo, RawTodo, SessionEvent as TodoSessionEvent,
        SessionProjections, TodoItem, TodoStatus, TodoWriteArgs, TodoWriteResult, ToolExecution,
        ToolFailure, ToolTodo, Tools as TodoTools, describe, mount as mount_tool_todo,
        to_todo_list, tool_failure_error,
    };
    pub use crate::ports::tool_working_directory::{
        DESCRIPTION as WORKING_DIRECTORY_DESCRIPTION, ToolWorkingDirectory,
        execute as execute_working_directory, mount as mount_working_directory,
    };
    pub use crate::ports::user_approval::{
        APPROVAL_OUTCOMES, APPROVAL_POLICIES, ASK_SENTENCE, NEVER_SENTENCE,
        normalize_approval_outcome, outcome_label,
    };
    pub use crate::ports::user_questions::{
        Answer, AnswerItem, AskQuestion, AskRequest, QuestionOption, TOOL_NAME, UserQuestionError,
        UserQuestionService, mount as mount_user_questions, user_questions_fold,
    };
    pub use crate::ports::workspace_path::{
        abbreviate_home_path, file_address_for, is_absolute_workspace_path, is_windows_style_path,
        path_parts_of, relativize_to_cwd, resolve_workspace_path, validate_segment,
        workspace_title_of,
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
        ToolPipeline, ToolRegistry, hook_deny_middleware, invoke_blocking,
        provide as provide_tools,
    };
    pub use crate::services::working_directory::{
        WorkingDirectory, provide as provide_working_directory,
    };
    pub use crate::services::workspace::{
        WorkspaceError, WorkspaceRecord, WorkspaceRegistry, provide as provide_workspace,
    };
}
