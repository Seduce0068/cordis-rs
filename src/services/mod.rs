//! Reusable implementations of DSH core services (ctx.tools, ctx.sessions, ...).

pub mod tools;

pub mod sessions;

pub mod commands;

pub mod llm;

pub mod agent_loop;

pub mod fs;

pub mod jobs;

pub mod shell;

pub mod working_directory;

pub mod skills;

pub mod subagents;

pub mod goals;

pub mod system_prompt;

pub mod pruner;

pub mod token_meter;

pub mod compaction;

pub mod approval;

pub mod permission_presets;

pub mod session_query;

pub mod workspace;

pub mod storage;

pub mod agents;

pub mod telemetry;

pub mod retry;

pub mod hook_protocol;

pub mod session_titles;

pub mod schedule;

pub mod plan_mode;

pub mod credentials;

pub mod timeout_policy;

pub mod plugin_inventory;

pub mod time_context;

pub mod session_reference;

pub mod file_reference;

pub mod file_search;

pub mod tmux_context;
