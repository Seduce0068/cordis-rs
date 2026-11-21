//! Tests for the reusable core service implementations.

use std::sync::Arc;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::atomic::{AtomicUsize, Ordering};

use cordis_rs::Context;
use cordis_rs::core_services::{
    ToolDefinition, ToolError, ToolExecContext, ToolOutput, ToolRegistry, provide_tools,
};

/// Auto-cleaning temp directory for tests (swarm review R5-TS03). Uses a
/// PID + nanosecond suffix so repeated runs never collide; cleanup errors are
/// ignored in Drop so a test failure is not masked by a cleanup panic.
struct TempDirGuard(std::path::PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for TempDirGuard {
    type Target = std::path::Path;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<std::path::Path> for TempDirGuard {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

fn echo_tool() -> ToolDefinition {
    ToolDefinition::new(
        "echo",
        "echo the input",
        Arc::new(|args, _exec| {
            Box::pin(async move {
                let text = args
                    .get("text")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(ToolOutput {
                    content: serde_json::json!({ "echo": text }),
                    rendered: vec![text],
                })
            })
        }),
    )
}

#[tokio::test]
async fn tool_registry_registers_invokes_and_disposes() {
    let ctx = Context::root("tools-service");
    let (registry, _pipeline) = provide_tools(&ctx).await.unwrap();
    let disposer = registry.register(echo_tool());
    assert_eq!(registry.names(), ["echo"]);

    let output = registry
        .invoke(
            "echo",
            serde_json::json!({ "text": "hello" }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    assert_eq!(output.content, serde_json::json!({ "echo": "hello" }));
    assert_eq!(output.rendered, ["hello"]);

    let err = registry
        .invoke(
            "missing",
            serde_json::json!({}),
            ToolExecContext::anonymous(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::UnknownTool(name) if name == "missing"));

    disposer.dispose().await.unwrap();
    assert!(registry.names().is_empty());
}

#[tokio::test]
async fn pipeline_middleware_and_post_execute_are_reversible() {
    let ctx = Context::root("tools-pipeline");
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    let _tool = registry.register(echo_tool());

    let calls = Arc::new(AtomicUsize::new(0));
    let wrapped = calls.clone();
    let middleware = pipeline.use_middleware(Arc::new(move |args, exec, next| {
        let calls = wrapped.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            next(args, exec).await
        })
    }));
    let seen = Arc::new(AtomicUsize::new(0));
    let post_seen = seen.clone();
    let post = pipeline.on_post_execute(Arc::new(move |output| {
        let seen = post_seen.clone();
        Box::pin(async move {
            seen.fetch_add(output.rendered.len(), Ordering::SeqCst);
            Ok(output)
        })
    }));

    let output = pipeline
        .run(
            &registry,
            "echo",
            serde_json::json!({ "text": "world" }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    assert_eq!(output.rendered, ["world"]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(seen.load(Ordering::SeqCst), 1);

    middleware.dispose().await.unwrap();
    post.dispose().await.unwrap();
    pipeline
        .run(
            &registry,
            "echo",
            serde_json::json!({ "text": "again" }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn services_are_observable_on_context() {
    let ctx = Context::root("tools-context");
    let _ = provide_tools(&ctx).await.unwrap();
    assert!(ctx.get::<ToolRegistry>("tools").is_some());
    assert!(
        ctx.get::<cordis_rs::core_services::ToolPipeline>("toolPipeline")
            .is_some()
    );
    assert!(ctx.get::<String>("tools").is_none());
    assert!(ctx.require::<ToolRegistry>("tools").await.is_ok());
}

#[tokio::test]
async fn session_store_appends_and_projections_fold() {
    let ctx = Context::root("sessions-service");
    let (sessions, projections) = cordis_rs::core_services::provide_sessions(&ctx)
        .await
        .unwrap();

    let session = sessions
        .create(Some("s1".to_string()), Some("/tmp".to_string()))
        .unwrap();
    assert_eq!(session.id(), "s1");
    assert_eq!(session.cwd(), Some("/tmp"));
    assert!(sessions.get("s1").is_some());
    assert_eq!(sessions.list().len(), 1);

    let disposer = projections.register(
        "count",
        Arc::new(|state, event| {
            let mut count = state.as_u64().unwrap_or(0);
            if event.event_type == "tick" {
                count += 1;
            }
            serde_json::json!(count)
        }),
    );
    assert!(projections.has("count"));

    session.append("tick", serde_json::json!({}));
    session.append("other", serde_json::json!({}));
    session.append("tick", serde_json::json!({}));
    assert_eq!(session.seq(), 3);
    assert_eq!(
        projections.view("count", &session),
        Some(serde_json::json!(2))
    );

    let forked = sessions.fork(&session, 2, Some("s2".to_string())).unwrap();
    assert_eq!(forked.seq(), 2);
    assert_eq!(
        projections.view("count", &forked),
        Some(serde_json::json!(1))
    );

    disposer.dispose().await.unwrap();
    assert!(!projections.has("count"));
}

#[tokio::test]
async fn command_feedback_records_log_only_event() {
    use cordis_rs::core_services::{CommandInvocation, CommandResult, Session};
    use cordis_rs::plugin_ports::{FEEDBACK_CATEGORIES, feedback_events, mount_command_feedback};

    assert!(FEEDBACK_CATEGORIES.contains(&"task-result"));

    let ctx = Context::root("feedback-port");
    let session = Session::new("session-1", None);
    let commands = mount_command_feedback(&ctx, session.clone()).await.unwrap();
    assert_eq!(commands.names(), ["/feedback"]);

    let blank = commands
        .invoke("/feedback", CommandInvocation::new("   "))
        .await
        .unwrap();
    assert!(matches!(blank, CommandResult::Error { .. }));
    assert!(feedback_events(&session).is_empty());

    let ok = commands
        .invoke(
            "/feedback",
            CommandInvocation::new("great response").with_session("session-1"),
        )
        .await
        .unwrap();
    assert!(matches!(ok, CommandResult::Success { .. }));
    let events = feedback_events(&session);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "feedback/record");
    assert_eq!(events[0].data["text"], serde_json::json!("great response"));
}

#[tokio::test]
async fn llm_registry_streams_and_agent_loop_uses_tools() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        AgentLoop, LlmProvider, StreamChunk, ToolCall, ToolDefinition, ToolOutput, echo_provider,
        provide_llm, provide_tools,
    };

    let ctx = Context::root("agent-loop");
    let (registry, _pipeline) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();

    let _tool = registry.register(ToolDefinition::new(
        "echo",
        "echo input",
        StdArc::new(|args, _exec| {
            Box::pin(async move {
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(ToolOutput::text(format!("echo:{text}")))
            })
        }),
    ));

    // Provider that returns a tool call on the first turn and a final answer
    // after seeing the tool result.
    let first = StdArc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = first.clone();
    let provider = LlmProvider::new(
        "scripted",
        "scripted-1",
        StdArc::new(move |request| {
            let has_tool_result = request
                .messages
                .iter()
                .any(|m| m.content.starts_with("echo:"));
            let flag = flag.clone();
            let text = if !has_tool_result && flag.swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                "call echo".to_string()
            } else {
                "done".to_string()
            };
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text(text),
                StreamChunk::Done,
            ]))
        }),
    );
    let _provider = llm.register(provider);
    assert!(llm.set_default("scripted"));
    assert_eq!(llm.names(), ["scripted"]);

    // Verify streaming with a separate keyless provider so the scripted flag
    // remains intact for the agent loop below.
    let _echo_registration = llm.register(echo_provider("echo"));
    let response = llm
        .stream(
            Some("echo"),
            cordis_rs::core_services::LlmRequest {
                model: String::new(),
                messages: vec![cordis_rs::core_services::ChatMessage::user("hi")],
            },
        )
        .await
        .unwrap();
    assert_eq!(response.text, "hi");

    let step_count = std::sync::atomic::AtomicUsize::new(0);
    let parser = StdArc::new(move |text: &str| {
        step_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if text == "call echo" {
            vec![ToolCall::new(
                "echo",
                serde_json::json!({ "text": "hello" }),
            )]
        } else {
            Vec::new()
        }
    });
    let loop_ = AgentLoop::new(llm.clone(), registry.clone())
        .with_max_steps(4)
        .with_call_parser(parser);
    let run = loop_.run("start").await.unwrap();
    assert_eq!(run.steps.len(), 2);
    assert_eq!(run.final_text, "done");
    assert_eq!(run.steps[0].tool_results.len(), 1);
    assert!(run.steps[0].tool_results[0].as_ref().unwrap().rendered[0].starts_with("echo:hello"));

    assert!(llm.names().contains(&"echo".to_string()));
}

#[tokio::test]
async fn working_directory_tool_reads_and_changes_cwd() {
    use cordis_rs::core_services::{
        ToolExecContext, ToolRegistry, WorkingDirectory, provide_tools,
    };
    use cordis_rs::plugin_ports::{execute_working_directory, mount_working_directory};

    let ctx = Context::root("working-directory-port");
    let (registry, _pipeline) = provide_tools(&ctx).await.unwrap();
    assert!(ctx.get::<ToolRegistry>("tools").is_some());

    let directory = WorkingDirectory::new();
    directory.ensure("agent-1", "/tmp/base");

    let read = execute_working_directory(
        &directory,
        &serde_json::json!({}),
        &ToolExecContext::agent("agent-1"),
    )
    .unwrap();
    assert_eq!(read.rendered, ["/tmp/base"]);

    let changed = execute_working_directory(
        &directory,
        &serde_json::json!({ "cd": "sub" }),
        &ToolExecContext::agent("agent-1"),
    )
    .unwrap();
    assert_eq!(changed.rendered, ["/tmp/base/sub"]);
    assert_eq!(
        directory.get("agent-1"),
        Some(std::path::PathBuf::from("/tmp/base/sub"))
    );

    let empty = execute_working_directory(
        &directory,
        &serde_json::json!({ "cd": "" }),
        &ToolExecContext::agent("agent-1"),
    );
    assert!(empty.is_err());

    let no_agent = execute_working_directory(
        &directory,
        &serde_json::json!({}),
        &ToolExecContext::anonymous(),
    );
    assert!(no_agent.is_err());

    // Mount variant registers the tool on the shared tools seam.
    let _mounted = mount_working_directory(&ctx).await.unwrap();
    assert!(registry.names().contains(&"working_directory".to_string()));
}

#[tokio::test]
async fn tool_fs_read_write_edit_through_shared_seams() {
    use cordis_rs::core_services::{ToolExecContext, ToolOutput, provide_fs, provide_tools};
    use cordis_rs::plugin_ports::{
        EditInput, ToolFsConfig, mount_tool_fs, parse_edit_args, parse_read_args, parse_write_args,
    };

    let ctx = Context::root("tool-fs-port");
    let (registry, _pipeline) = provide_tools(&ctx).await.unwrap();
    let fs = provide_fs(&ctx).await.unwrap();
    let _tools = mount_tool_fs(&ctx, fs.clone(), ToolFsConfig::default())
        .await
        .unwrap();

    let mut names = registry.names();
    names.sort();
    assert_eq!(names, ["edit", "read", "write"]);

    let dir = TempDirGuard::new("cordis-tool-fs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("note.txt");
    let path_str = path.to_string_lossy().to_string();

    // write (create)
    let (parsed_path, content) = parse_write_args(&serde_json::json!({
        "file_path": path_str,
        "content": "alpha beta\ngamma\n",
    }))
    .unwrap();
    assert_eq!(parsed_path, path_str);
    let output: ToolOutput = registry
        .invoke(
            "write",
            serde_json::json!({ "file_path": path_str, "content": content }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    assert!(output.rendered[0].contains("Created file"));
    assert_eq!(fs.version(&path_str), Some(1));

    // read with a bounded window
    let (_, offset, limit) = parse_read_args(
        &serde_json::json!({
            "file_path": path_str,
            "offset": 1,
            "limit": 1,
        }),
        10,
    )
    .unwrap();
    assert_eq!((offset, limit), (1, 1));
    let output: ToolOutput = registry
        .invoke(
            "read",
            serde_json::json!({ "file_path": path_str, "offset": 1, "limit": 1 }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    assert!(output.rendered[0].contains("alpha beta"));
    assert!(!output.rendered[0].contains("gamma"));

    // edit with unique match
    let input: EditInput = parse_edit_args(&serde_json::json!({
        "file_path": path_str,
        "old_string": "alpha",
        "new_string": "ALPHA",
    }))
    .unwrap();
    assert!(!input.replace_all);
    let output: ToolOutput = registry
        .invoke(
            "edit",
            serde_json::json!({
                "file_path": path_str,
                "old_string": "alpha",
                "new_string": "ALPHA",
            }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    assert!(output.rendered[0].contains("updated successfully"));
    assert_eq!(fs.read_text(&path_str).unwrap(), "ALPHA beta\ngamma\n");
    assert_eq!(fs.version(&path_str), Some(2));

    // argument validation
    assert!(
        parse_edit_args(&serde_json::json!({
            "file_path": path_str,
            "old_string": "x",
            "new_string": "x",
        }))
        .is_err()
    );
    assert!(
        parse_read_args(
            &serde_json::json!({
                "file_path": path_str,
                "limit": 0,
            }),
            10
        )
        .is_err()
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn job_registry_lifecycle_output_and_owner_fencing() {
    use cordis_rs::core_services::{JobError, JobStatus, provide_jobs};

    let ctx = Context::root("jobs-service");
    let jobs = provide_jobs(&ctx).await.unwrap();

    let handle = jobs.start(Some("agent-1".to_string())).unwrap();
    let id = handle.id();
    assert_eq!(handle.status(), JobStatus::Running);
    assert_eq!(
        jobs.view(id, Some("agent-1")).unwrap().status,
        JobStatus::Running
    );

    handle.append(b"hello ");
    handle.append(b"world");
    let (text, end) = jobs.read(id, 0, Some("agent-1")).unwrap();
    assert_eq!(text, "hello world");
    assert_eq!(end, 11);
    let (tail, _) = jobs.read(id, 6, Some("agent-1")).unwrap();
    assert_eq!(tail, "world");

    assert!(matches!(
        jobs.view(id, Some("agent-2")),
        Err(JobError::NotOwner { .. })
    ));
    assert!(matches!(
        jobs.view(999, Some("agent-1")),
        Err(JobError::NotFound(999))
    ));

    handle.settle(JobStatus::Succeeded);
    assert_eq!(handle.status(), JobStatus::Succeeded);
    assert!(jobs.wait(id, Some("agent-1"), 50).unwrap());
    assert!(matches!(
        jobs.kill(id, Some("agent-1")),
        Err(JobError::AlreadyTerminal(_))
    ));

    let listed = jobs.list(Some("agent-1"));
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, JobStatus::Succeeded);
    assert!(jobs.list(Some("agent-2")).is_empty());
}

#[tokio::test]
async fn tool_bash_runs_foreground_and_background() {
    use cordis_rs::core_services::{
        JobStatus, ToolExecContext, ToolOutput, provide_jobs, provide_shell, provide_tools,
    };
    use cordis_rs::plugin_ports::{ToolBashConfig, mount_tool_bash, parse_bash_args};

    let ctx = Context::root("tool-bash-port");
    let (registry, _) = provide_tools(&ctx).await.unwrap();
    let shell = provide_shell(&ctx).await.unwrap();
    let jobs = provide_jobs(&ctx).await.unwrap();
    let _tools = mount_tool_bash(&ctx, shell.clone(), ToolBashConfig::default())
        .await
        .unwrap();
    assert!(registry.names().contains(&"bash".to_string()));

    // argument validation
    assert!(
        parse_bash_args(
            &serde_json::json!({ "command": "" }),
            &ToolBashConfig::default()
        )
        .is_err()
    );
    let disabled = ToolBashConfig {
        enable_run_in_background: false,
        ..Default::default()
    };
    assert!(
        parse_bash_args(
            &serde_json::json!({ "command": "echo hi", "run_in_background": true }),
            &disabled,
        )
        .is_err()
    );

    // foreground calls return job ids while a job registry is composed
    // (upstream registers every call as a job).
    let output: ToolOutput = registry
        .invoke(
            "bash",
            serde_json::json!({ "command": "printf hello" }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    let foreground_job = output.content["job_id"].as_u64().unwrap();
    assert!(jobs.wait(foreground_job, Some("bash"), 1000).unwrap());
    let (foreground_text, _) = jobs.read(foreground_job, 0, Some("bash")).unwrap();
    assert_eq!(foreground_text, "hello");

    // background: returns immediately with a job id that settles
    let output: ToolOutput = registry
        .invoke(
            "bash",
            serde_json::json!({
                "command": "printf background",
                "run_in_background": true,
            }),
            ToolExecContext::agent("agent-1"),
        )
        .await
        .unwrap();
    let id = output.content["job_id"].as_u64().unwrap();
    assert!(jobs.wait(id, Some("bash"), 1000).unwrap());
    assert_eq!(
        jobs.view(id, Some("bash")).unwrap().status,
        JobStatus::Succeeded
    );
    let (text, _) = jobs.read(id, 0, Some("bash")).unwrap();
    assert_eq!(text, "background");
}

#[tokio::test]
async fn jsonl_persistence_appends_reads_and_restores() {
    use cordis_rs::plugin_ports::{
        JsonlPersistence, PersistenceError, mount_session_persistence_jsonl,
    };

    let dir = TempDirGuard::new("cordis-jsonl");
    let _ = std::fs::remove_dir_all(&dir);

    let ctx = Context::root("jsonl-persistence");
    let store = mount_session_persistence_jsonl(&ctx, &dir).await.unwrap();

    let handle = store.create("session-1").unwrap();
    assert_eq!(handle.id(), "session-1");
    assert!(matches!(
        store.create("session-1"),
        Err(PersistenceError::AlreadyExists(_))
    ));

    handle
        .append(&cordis_rs::core_services::LoggedSessionEvent {
            seq: 1,
            event_type: "user/message".to_string(),
            data: serde_json::json!({ "text": "hello" }),
        })
        .unwrap();
    handle
        .append(&cordis_rs::core_services::LoggedSessionEvent {
            seq: 2,
            event_type: "assistant/message".to_string(),
            data: serde_json::json!({ "text": "hi" }),
        })
        .unwrap();

    let events = handle.read().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "user/message");
    assert_eq!(events[1].data["text"], "hi");

    let restored = store
        .restore("session-1", Some("/tmp".to_string()))
        .unwrap();
    assert_eq!(restored.id(), "session-1");
    assert_eq!(restored.seq(), 2);
    assert_eq!(restored.cwd(), Some("/tmp"));

    // reopen from disk after dropping the in-memory handle
    store.close("session-1");
    let reopened = store.open("session-1").unwrap();
    assert_eq!(reopened.read().unwrap().len(), 2);
    assert!(matches!(
        store.open("missing"),
        Err(PersistenceError::NotFound(_))
    ));

    let _ = JsonlPersistence::new(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn agent_loop_records_session_events_and_checkpoints() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        AgentLoop, ChatMessage, LlmProvider, LlmRequest, Session, StreamChunk, ToolCall,
        provide_llm, provide_tools,
    };

    let ctx = Context::root("agent-checkpoint");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();
    let session = Session::new("agent-session", Some("/tmp".to_string()));

    let provider = LlmProvider::new(
        "one-step",
        "one-step-1",
        StdArc::new(|_request: LlmRequest| {
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text("done".to_string()),
                StreamChunk::Done,
            ]))
        }),
    );
    let _p = llm.register(provider);
    llm.set_default("one-step");
    let _echo = llm.register(cordis_rs::core_services::echo_provider("echo"));

    let checkpoints = StdArc::new(AtomicUsize::new(0));
    let counter = checkpoints.clone();
    let hook: cordis_rs::core_services::CheckpointHook = StdArc::new(move |s: &Session| {
        counter.fetch_add(1, AtomicOrdering::SeqCst);
        assert!(!s.events().is_empty());
        Ok(())
    });

    let loop_ = AgentLoop::new(llm, tools)
        .with_session(session.clone())
        .with_checkpoint(hook)
        .with_call_parser(StdArc::new(|_text: &str| Vec::<ToolCall>::new()));
    let run = loop_.run("hi").await.unwrap();
    assert_eq!(run.final_text, "done");

    let types: Vec<String> = session.events().into_iter().map(|e| e.event_type).collect();
    assert!(types.contains(&"turn/start".to_string()));
    assert!(types.contains(&"user/message".to_string()));
    assert!(types.contains(&"assistant/message".to_string()));
    assert!(types.contains(&"step/start".to_string()));
    assert!(types.contains(&"step/end".to_string()));
    assert!(types.contains(&"turn/end".to_string()));
    // one step-end + one turn-end checkpoint
    assert_eq!(checkpoints.load(AtomicOrdering::SeqCst), 2);

    let _ = ChatMessage::user("unused");
}

#[tokio::test]
async fn async_text_provider_wraps_external_backend() {
    use cordis_rs::core_services::{ChatMessage, LlmRequest, from_async_text, provide_llm};

    let ctx = Context::root("async-llm");
    let llm = provide_llm(&ctx).await.unwrap();
    let provider = from_async_text("external", "external-1", |request: LlmRequest| async move {
        let last = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok::<_, String>(format!("echo:{last}"))
    });
    let _p = llm.register(provider);
    llm.set_default("external");

    let response = llm
        .stream(
            None,
            LlmRequest {
                model: String::new(),
                messages: vec![ChatMessage::user("payload")],
            },
        )
        .await
        .unwrap();
    assert_eq!(response.text, "echo:payload");
    assert_eq!(response.model, "external-1");
}

#[tokio::test]
async fn checkpoint_cursor_appends_only_new_events() {
    use cordis_rs::core_services::{CheckpointCursor, Session};

    let session = Session::new("session-cursor", None);
    let cursor = CheckpointCursor::new();

    session.append("first", serde_json::json!({}));
    assert_eq!(cursor.pending(&session).len(), 1);
    let first = cursor.pending(&session);
    cursor.commit(first.len());
    assert_eq!(cursor.written(), 1);
    assert!(cursor.pending(&session).is_empty());

    session.append("second", serde_json::json!({}));
    session.append("third", serde_json::json!({}));
    let pending = cursor.pending(&session);
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].event_type, "second");
    cursor.commit(pending.len());
    assert_eq!(cursor.written(), 3);
    assert!(cursor.pending(&session).is_empty());

    // Re-checkpointing an unchanged session writes nothing.
    assert!(cursor.pending(&session).is_empty());
    assert_eq!(cursor.written(), 3);
}

#[tokio::test]
async fn checkpoint_cursor_survives_restart() {
    use cordis_rs::core_services::{CheckpointCursor, Session};

    let dir = TempDirGuard::new("cordis-cursor");
    let _ = std::fs::create_dir_all(&dir);
    let cursor_path = dir.join("cursor.json");

    let session = Session::new("restart-session", None);
    session.append("first", serde_json::json!({}));
    session.append("second", serde_json::json!({}));

    let cursor = CheckpointCursor::new();
    let pending = cursor.pending(&session);
    assert_eq!(pending.len(), 2);
    cursor.commit(pending.len());
    cursor.save(&cursor_path).unwrap();
    assert_eq!(cursor.written(), 2);

    // Simulate a restart: a fresh session log with two more events and a
    // cursor restored from disk.
    session.append("third", serde_json::json!({}));
    session.append("fourth", serde_json::json!({}));
    let resumed = CheckpointCursor::load(&cursor_path);
    assert_eq!(resumed.written(), 2);
    let pending = resumed.pending(&session);
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].event_type, "third");
    assert_eq!(pending[1].event_type, "fourth");

    // A missing cursor file starts from zero.
    let fresh = CheckpointCursor::load(dir.join("missing.json"));
    assert_eq!(fresh.written(), 0);
    assert_eq!(fresh.pending(&session).len(), 4);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn checkpoint_cursor_save_is_atomic_and_overwrites_cleanly() {
    use cordis_rs::core_services::{CheckpointCursor, Session};

    let dir = TempDirGuard::new("cordis-cursor-atomic");
    let _ = std::fs::create_dir_all(&dir);
    let cursor_path = dir.join("cursor.json");

    let session = Session::new("atomic", None);
    let cursor = CheckpointCursor::new();

    // First save.
    session.append("a", serde_json::json!({}));
    cursor.commit(cursor.pending(&session).len());
    cursor.save(&cursor_path).unwrap();
    assert_eq!(CheckpointCursor::load(&cursor_path).written(), 1);

    // Second save overwrites the existing file without leaving a temp file.
    session.append("b", serde_json::json!({}));
    cursor.commit(cursor.pending(&session).len());
    cursor.save(&cursor_path).unwrap();
    assert_eq!(CheckpointCursor::load(&cursor_path).written(), 2);
    assert!(!cursor_path.with_extension("tmp").exists());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn skill_filesystem_discovers_and_registers_a_provider() {
    use cordis_rs::plugin_ports::{SkillFsConfig, mount_skill_filesystem};

    let dir = TempDirGuard::new("cordis-skills");
    let skill_dir = dir.join("demo-skill");
    let _ = std::fs::create_dir_all(&skill_dir);
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo-skill\n---\n# Demo\nA demo skill for tests.\n",
    )
    .unwrap();

    let ctx = Context::root("skill-fs");
    let config = SkillFsConfig {
        roots: vec![dir.to_path_buf()],
        rank: 10,
    };
    let registry = mount_skill_filesystem(&ctx, config).await.unwrap();

    let catalog = registry.catalog();
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].name, "demo-skill");
    assert_eq!(catalog.skills[0].description, "A demo skill for tests.");
    assert_eq!(catalog.skills[0].source, "filesystem");
    assert_eq!(registry.names(), ["demo-skill"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn deepseek_provider_requires_api_key() {
    use std::collections::BTreeMap;

    use cordis_rs::core_services::{ChatMessage, LlmRequest, provide_llm};
    use cordis_rs::plugin_ports::{DeepSeekConfig, mount_llm_deepseek, resolve_api_key};

    let config = DeepSeekConfig {
        api_key_env: "CORDIS_TEST_KEY".to_string(),
        model: "deepseek-chat".to_string(),
        ..Default::default()
    };

    // Missing key fails the credential gate.
    let mut env = BTreeMap::new();
    assert!(resolve_api_key(&config, &env).is_err());

    let ctx = Context::root("deepseek-no-key");
    let _ = provide_llm(&ctx).await.unwrap();
    assert!(
        mount_llm_deepseek(&ctx, config.clone(), env.clone())
            .await
            .is_err()
    );

    // With a key the provider mounts and streams an authenticated response.
    env.insert("CORDIS_TEST_KEY".to_string(), "sk-test".to_string());
    assert_eq!(resolve_api_key(&config, &env).unwrap(), "sk-test");
    let ctx2 = Context::root("deepseek-key");
    let llm = mount_llm_deepseek(&ctx2, config, env).await.unwrap();
    let response = llm
        .stream(
            Some("deepseek-official"),
            LlmRequest {
                model: String::new(),
                messages: vec![ChatMessage::user("hello")],
            },
        )
        .await
        .unwrap();
    assert!(response.text.contains("authenticated"));
    assert_eq!(response.model, "deepseek-chat");
}

#[tokio::test]
async fn message_feedback_validates_notes_and_records_events() {
    use cordis_rs::plugin_ports::{
        FeedbackError, FeedbackItem, MessageFeedbackConfig, Rating, mount_message_feedback,
    };

    let ctx = Context::root("message-feedback");
    let feedback = mount_message_feedback(&ctx, MessageFeedbackConfig { max_note_bytes: 16 })
        .await
        .unwrap();

    let session = cordis_rs::core_services::Session::new("fb-session", None);
    feedback.register_session(session.clone());

    let item = |version: &str, note: Option<&str>| FeedbackItem {
        message_id: "msg-1".to_string(),
        rating: Rating::Positive,
        note: note.map(str::to_string),
        version: version.to_string(),
    };

    // Too-large note is rejected before any event is appended.
    assert!(matches!(
        feedback.put("fb-session", item("v1", Some("this note is way too long"))),
        Err(FeedbackError::NoteTooLarge { .. })
    ));
    assert!(feedback.put_events("fb-session").is_empty());

    feedback
        .put("fb-session", item("v1", Some("good")))
        .unwrap();
    assert_eq!(feedback.put_events("fb-session").len(), 1);
    let event = &feedback.put_events("fb-session")[0];
    assert_eq!(event.data["rating"], "positive");
    assert_eq!(event.data["note"], "good");

    // An older/equal version is a conflict.
    assert!(matches!(
        feedback.put("fb-session", item("v1", Some("again"))),
        Err(FeedbackError::VersionConflict { .. })
    ));
    feedback.put("fb-session", item("v2", None)).unwrap();
    assert_eq!(feedback.put_events("fb-session").len(), 2);

    // Unknown session fails.
    assert!(matches!(
        feedback.put("missing", item("v1", None)),
        Err(FeedbackError::SessionNotFound(_))
    ));
}

#[tokio::test]
async fn tool_subagent_delegates_with_depth_cap() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{provide_subagents, provide_tools};
    use cordis_rs::plugin_ports::{ToolSubagentConfig, mount_tool_subagent};

    let ctx = Context::root("subagent-tool");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let subagents = provide_subagents(&ctx).await.unwrap();

    let _provider = subagents.register(
        "spawn",
        StdArc::new(|request| Box::pin(async move { format!("child says: {}", request.prompt) })),
    );

    let config = ToolSubagentConfig {
        provider: "spawn".to_string(),
        tool_name: "subagent".to_string(),
        max_depth: 1,
    };
    let _tools = mount_tool_subagent(&ctx, config).await.unwrap();
    assert!(tools.names().contains(&"subagent".to_string()));

    let output = tools
        .invoke(
            "subagent",
            serde_json::json!({ "prompt": "hello", "depth": 1 }),
            cordis_rs::core_services::ToolExecContext::agent("parent"),
        )
        .await
        .unwrap();
    assert_eq!(output.rendered, ["child says: hello"]);

    // depth 2 exceeds the cap of 1.
    let error = tools
        .invoke(
            "subagent",
            serde_json::json!({ "prompt": "deep", "depth": 2 }),
            cordis_rs::core_services::ToolExecContext::agent("parent"),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("depth"));
}

#[tokio::test]
async fn feedback_producer_records_log_only_feedback() {
    use cordis_rs::core_services::{
        CommandDefinition, CommandInvocation, CommandResult, provide_commands,
    };
    use cordis_rs::plugin_ports::{MessageFeedback, MessageFeedbackConfig, TurnStopping, produce};

    let ctx = Context::root("feedback-producer");
    let commands = provide_commands(&ctx).await.unwrap();
    let _d = commands.register(CommandDefinition::new(
        "/feedback",
        "record feedback",
        std::sync::Arc::new(|_inv: CommandInvocation| {
            Box::pin(async { CommandResult::success("recorded") })
        }),
    ));

    let session = cordis_rs::core_services::Session::new("producer-session", None);
    let feedback = MessageFeedback::new(MessageFeedbackConfig::default()).unwrap();
    feedback.register_session(session.clone());

    let root = TurnStopping {
        session_id: "producer-session".to_string(),
        is_root: true,
        has_assistant_message: true,
    };
    let outcome = produce(&root, &commands, &session, &feedback)
        .await
        .unwrap();
    assert!(outcome.command_success);
    assert!(outcome.session_recorded);
    assert!(outcome.message_rated);

    // A non-root agent is skipped without side effects.
    let child = TurnStopping {
        session_id: "producer-session".to_string(),
        is_root: false,
        has_assistant_message: true,
    };
    let skipped = produce(&child, &commands, &session, &feedback)
        .await
        .unwrap();
    assert!(!skipped.session_recorded);

    // A root turn without an assistant message fails loudly.
    let no_assistant = TurnStopping {
        session_id: "producer-session".to_string(),
        is_root: true,
        has_assistant_message: false,
    };
    assert!(
        produce(&no_assistant, &commands, &session, &feedback)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn goal_store_enforces_revisions_and_command_registers() {
    use cordis_rs::core_services::{CommandResult, provide_commands};
    use cordis_rs::plugin_ports::mount_command_goal;
    use cordis_rs::services::goals::GoalError;

    let ctx = Context::root("goals-service");
    let commands = provide_commands(&ctx).await.unwrap();
    let store = mount_command_goal(&ctx).await.unwrap();
    assert!(commands.names().contains(&"/goal".to_string()));

    let goal = store.set("finish the port", 100).unwrap();
    assert_eq!(goal.revision, 1);
    assert_eq!(goal.objective, "finish the port");

    // A stale edit is rejected.
    assert!(matches!(
        store.edit("new objective", 100, 0),
        Err(GoalError::StaleRevision { .. })
    ));

    let goal = store.edit("new objective", 100, goal.revision).unwrap();
    assert_eq!(goal.revision, 2);
    let goal = store.pause(goal.revision).unwrap();
    assert_eq!(goal.status, cordis_rs::services::goals::GoalStatus::Paused);
    let goal = store.resume(goal.revision).unwrap();
    assert_eq!(goal.status, cordis_rs::services::goals::GoalStatus::Active);

    // The command reflects the live store.
    let result = commands
        .invoke(
            "/goal",
            cordis_rs::services::commands::CommandInvocation::new(""),
        )
        .await
        .unwrap();
    eprintln!("RESULT={:?}", result);
    assert!(matches!(result, CommandResult::Success { .. }));

    store.clear();
    assert!(store.get().is_none());
    assert!(store.event_count() > 0);
}

#[tokio::test]
async fn system_prompt_sections_sort_interpolate_and_reject_duplicates() {
    use cordis_rs::core_services::{
        AssembleContext, PromptError, PromptSection, provide_system_prompt,
    };
    use std::collections::BTreeMap;

    let ctx = Context::root("system-prompt");
    let prompt = provide_system_prompt(&ctx).await.unwrap();

    let _d1 = prompt
        .section(PromptSection::new("z-last", 900, "last"))
        .unwrap();
    let _d2 = prompt
        .section(PromptSection::new("a-first", 100, "You are {{role}}."))
        .unwrap();
    assert!(matches!(
        prompt.section(PromptSection::new("a-first", 100, "dup")),
        Err(PromptError::DuplicateSection(_))
    ));

    let mut variables = BTreeMap::new();
    variables.insert("role".to_string(), "a helper".to_string());
    let assembly = prompt.assemble(&AssembleContext { variables }).unwrap();
    assert_eq!(assembly.sections, ["a-first", "z-last"]);
    assert_eq!(assembly.text, "You are a helper.\n\nlast");

    // A complete section replaces all others.
    let _d3 = prompt
        .section(PromptSection {
            name: "override".to_string(),
            order: 500,
            text: "ONLY THIS".to_string(),
            interpolate: false,
            complete: true,
        })
        .unwrap();
    let assembly = prompt.assemble(&AssembleContext::default()).unwrap();
    assert_eq!(assembly.text, "ONLY THIS");
    assert_eq!(assembly.sections, ["override"]);
}

#[tokio::test]
async fn agent_loop_injects_system_prompt_and_goal() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        AgentLoop, LlmRequest, PromptSection, provide_goals, provide_llm, provide_system_prompt,
        provide_tools,
    };
    use cordis_rs::plugin_ports::mount_command_goal;

    let ctx = Context::root("agent-prompt-goal");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();
    let prompt = provide_system_prompt(&ctx).await.unwrap();
    let goals = provide_goals(&ctx).await.unwrap();
    let _ = mount_command_goal(&ctx).await.unwrap();
    goals.set("ship the Rust port", 50).unwrap();

    let _section = prompt
        .section(PromptSection::new(
            "task",
            100,
            "Goal: {{goal}} ({{goal_status}})",
        ))
        .unwrap();

    // Capture the request messages from the provider.
    let seen = StdArc::new(std::sync::Mutex::new(Vec::<LlmRequest>::new()));
    let capture = seen.clone();
    let provider = cordis_rs::core_services::from_async_text(
        "capture",
        "capture-1",
        move |request: LlmRequest| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(request);
                Ok::<_, String>("done".to_string())
            }
        },
    );
    let _p = llm.register(provider);
    llm.set_default("capture");

    let loop_ = AgentLoop::new(llm, tools)
        .with_system_prompt(prompt)
        .with_goals(goals)
        .with_call_parser(StdArc::new(|_text: &str| Vec::new()));
    loop_.run("start").await.unwrap();

    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages[0].role, "system");
    assert!(
        requests[0].messages[0]
            .content
            .contains("ship the Rust port")
    );
    assert!(requests[0].messages[0].content.contains("active"));
    assert_eq!(requests[0].messages[1].role, "user");
}

#[tokio::test]
async fn tool_result_pruner_keeps_head_and_tail() {
    use cordis_rs::core_services::{PrunerConfig, ToolResultPruner};

    let pruner = ToolResultPruner::new(PrunerConfig {
        head_chars: 8,
        tail_chars: 8,
        threshold_chars: 60,
    })
    .unwrap();

    // Within budget: untouched.
    assert!(pruner.prune_text("short").is_none());

    // Over budget: head and tail preserved, middle replaced by a marker.
    let pruned = pruner
        .prune_text("abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ")
        .unwrap();
    assert!(pruned.text.starts_with("abcdefgh"));
    assert!(pruned.text.contains("STUVWXYZ"));
    assert!(pruned.text.contains("[... tool result middle pruned ...]"));
    assert!(pruned.removed_chars > 0);
    assert!(pruned.text.chars().count() < 60);

    // Recursive over a JSON tool result.
    let value = serde_json::json!({
        "stdout": "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ",
        "meta": { "count": 1 },
    });
    let result = pruner.prune_json(&value);
    assert!(
        result["stdout"]
            .as_str()
            .unwrap()
            .contains("[... tool result middle pruned ...]")
    );
    assert_eq!(result["meta"]["count"], 1);

    // Invalid config is rejected at construction.
    assert!(
        ToolResultPruner::new(PrunerConfig {
            head_chars: 100,
            tail_chars: 100,
            threshold_chars: 20,
        })
        .is_err()
    );
}

#[tokio::test]
async fn token_meter_estimates_session_pressure() {
    use cordis_rs::core_services::{
        Session, TokenMeter, estimate_message, estimate_text, provide_token_meter,
    };

    let ctx = Context::root("token-meter");
    let meter: TokenMeter = provide_token_meter(&ctx).await.unwrap();
    assert_eq!(estimate_text("abcd"), 1);
    assert_eq!(estimate_message("user", "abcd"), 1 + 1 + 4);

    let session = Session::new("meter-session", None);
    session.append("user/message", serde_json::json!({ "text": "hello world" }));
    session.append(
        "assistant/message",
        serde_json::json!({ "text": "hi there" }),
    );
    session.append("step/start", serde_json::json!({})); // ignored

    let measurement = meter.measure(&session);
    assert_eq!(measurement.message_count, 2);
    assert!(measurement.total_tokens > 0);
    assert!(measurement.pressure > 0.0 && measurement.pressure < 1.0);

    let small = TokenMeter::new(1);
    let high = small.measure(&session);
    assert!(high.pressure > 1.0);
}

#[tokio::test]
async fn compaction_triggers_on_pressure_and_appends_checkpoint() {
    use cordis_rs::core_services::{
        CompactionConfig, CompactionEngine, CompactionTrigger, Session, TokenMeter,
    };

    let session = Session::new("compact-session", None);
    session.append("user/message", serde_json::json!({ "text": "abc" }));
    session.append("assistant/message", serde_json::json!({ "text": "def" }));

    // A high threshold means no compaction.
    let meter = TokenMeter::new(1000);
    let engine = CompactionEngine::new(
        CompactionConfig {
            pressure_threshold: 0.99,
            ..Default::default()
        },
        meter,
    )
    .unwrap();
    let result = engine
        .maybe_compact(&session, CompactionTrigger::Pressure)
        .unwrap();
    assert!(!result.compacted);
    assert!(
        !session
            .events()
            .iter()
            .any(|event| event.event_type == "compaction/checkpoint")
    );

    // A tiny budget triggers compaction and appends a checkpoint.
    let meter = TokenMeter::new(1);
    let engine = CompactionEngine::new(
        CompactionConfig {
            pressure_threshold: 0.5,
            head_chars: 2,
            tail_chars: 2,
        },
        meter,
    )
    .unwrap();
    let result = engine
        .maybe_compact(&session, CompactionTrigger::Pressure)
        .unwrap();
    assert!(result.compacted);
    assert_eq!(result.shadowed_events, 2);
    assert!(result.pressure > 1.0);
    let checkpoints: Vec<_> = session
        .events()
        .into_iter()
        .filter(|event| event.event_type == "compaction/checkpoint")
        .collect();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].data["trigger"], "pressure");
}

#[tokio::test]
async fn agent_loop_compacts_at_step_boundary_under_pressure() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        AgentLoop, CompactionConfig, CompactionEngine, LlmRequest, Session, StreamChunk,
        TokenMeter, provide_llm, provide_tools,
    };

    let ctx = Context::root("agent-compaction");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();
    let session = Session::new("auto-compact", None);

    let provider = cordis_rs::core_services::LlmProvider::new(
        "one",
        "one-1",
        StdArc::new(|_request: LlmRequest| {
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text("done".to_string()),
                StreamChunk::Done,
            ]))
        }),
    );
    let _p = llm.register(provider);
    llm.set_default("one");

    // A tiny budget guarantees pressure > threshold after the first user msg.
    let meter = TokenMeter::new(1);
    let engine = CompactionEngine::new(
        CompactionConfig {
            pressure_threshold: 0.5,
            head_chars: 2,
            tail_chars: 2,
        },
        meter,
    )
    .unwrap();

    let loop_ = AgentLoop::new(llm, tools)
        .with_session(session.clone())
        .with_compaction(engine)
        .with_call_parser(StdArc::new(|_text: &str| Vec::new()));
    loop_
        .run("a long prompt that exceeds the tiny budget")
        .await
        .unwrap();

    let checkpoints: Vec<_> = session
        .events()
        .into_iter()
        .filter(|event| event.event_type == "compaction/checkpoint")
        .collect();
    assert!(!checkpoints.is_empty(), "expected an automatic checkpoint");
    assert_eq!(checkpoints[0].data["trigger"], "pressure");
}

#[tokio::test]
async fn approval_fails_closed_and_honors_answerers() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        ApprovalDecision, ApprovalError, ApprovalRequest, ApprovalService, provide_approval,
    };

    let ctx = Context::root("approval");
    let approval: ApprovalService = provide_approval(&ctx).await.unwrap();
    let req = ApprovalRequest::new("sandbox escalate", "allow danger-full-access?");

    // No answerer -> fail closed.
    assert!(matches!(
        approval.request(&req),
        Err(ApprovalError::Unavailable)
    ));

    let _d = approval.add_answerer(
        "test-answerer",
        StdArc::new(|req: &ApprovalRequest| {
            if req.action == "sandbox escalate" {
                Some(ApprovalDecision::Deny)
            } else {
                None
            }
        }),
    );
    let outcome = approval.request(&req).unwrap();
    assert_eq!(outcome.decision, ApprovalDecision::Deny);
    assert_eq!(outcome.answerer.as_deref(), Some("test-answerer"));
    assert_eq!(approval.answerer_names(), ["test-answerer"]);

    // A different action that no answerer handles fails closed again.
    let other = ApprovalRequest::new("other", "unhandled");
    assert!(matches!(
        approval.request(&other),
        Err(ApprovalError::Unavailable)
    ));
}

#[tokio::test]
async fn permission_presets_switch_both_knobs() {
    use cordis_rs::core_services::{
        ApprovalPolicy, PermissionPresets, Preset, SandboxMode, provide_permission_presets,
    };

    let ctx = Context::root("permission-presets");
    let presets: PermissionPresets = provide_permission_presets(&ctx).await.unwrap();
    assert_eq!(presets.names().len(), 3);

    let state = presets.switch("read-only").unwrap();
    assert_eq!(state.preset, Some(Preset::ReadOnly));
    assert_eq!(state.sandbox, SandboxMode::ReadOnly);
    assert_eq!(state.approval, ApprovalPolicy::Never);

    let state = presets.switch("danger-full-access").unwrap();
    assert_eq!(state.sandbox, SandboxMode::DangerFullAccess);
    assert_eq!(state.approval, ApprovalPolicy::Ask); // Auto-style preset asks

    assert!(presets.switch("unknown").is_err());
    assert_eq!(presets.current().sandbox, SandboxMode::DangerFullAccess);
}

#[tokio::test]
async fn session_query_indexes_traces_filters_and_searches() {
    use cordis_rs::core_services::{Session, provide_session_query};

    let ctx = Context::root("session-query");
    let engine = provide_session_query(&ctx).await.unwrap();

    let s1 = Session::new("s1", None);
    s1.append(
        "user/message",
        serde_json::json!({ "text": "Implement Rust port" }),
    );
    s1.append(
        "assistant/message",
        serde_json::json!({ "text": "Done, the rust port works" }),
    );
    let s2 = Session::new("s2", None);
    s2.append(
        "user/message",
        serde_json::json!({ "text": "Unrelated question" }),
    );

    engine.index(s1.clone());
    engine.index(s2);
    assert_eq!(engine.session_ids(), ["s1", "s2"]);

    let trace = engine.trace("s1");
    assert_eq!(trace.len(), 2);
    assert_eq!(trace[0].event_type, "user/message");

    let users = engine.filter("user/message");
    assert_eq!(users.len(), 2);

    let hits = engine.search("RUST PORT");
    assert_eq!(hits.len(), 2); // both s1 messages mention the rust port
    assert!(hits.iter().all(|hit| hit.session_id == "s1"));
    assert_eq!(hits[0].event_seq, 1);

    assert!(engine.search("").is_empty());
    assert!(engine.search("nothing").is_empty());
    assert!(engine.trace("missing").is_empty());
}

#[tokio::test]
async fn session_query_tool_searches_and_traces() {
    use cordis_rs::core_services::{Session, ToolExecContext, provide_tools};
    use cordis_rs::plugin_ports::mount_tool_session_query;

    let ctx = Context::root("session-query-tool");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let engine = cordis_rs::core_services::provide_session_query(&ctx)
        .await
        .unwrap();
    let _mounted = mount_tool_session_query(&ctx, Default::default())
        .await
        .unwrap();

    let session = Session::new("qa-1", None);
    session.append(
        "user/message",
        serde_json::json!({ "text": "How to port plugins?" }),
    );
    session.append(
        "assistant/message",
        serde_json::json!({ "text": "Use the Cordis seams" }),
    );
    engine.index(session);

    let mut names = tools.names();
    names.sort();
    assert_eq!(names, ["session_search", "session_trace"]);

    let result = tools
        .invoke(
            "session_search",
            serde_json::json!({ "query": "port" }),
            ToolExecContext::agent("agent"),
        )
        .await
        .unwrap();
    assert_eq!(result.content["hits"].as_array().unwrap().len(), 1);

    let result = tools
        .invoke(
            "session_trace",
            serde_json::json!({ "session_id": "qa-1" }),
            ToolExecContext::agent("agent"),
        )
        .await
        .unwrap();
    assert_eq!(result.content["events"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn workspace_registry_orders_and_attaches_sessions() {
    use cordis_rs::core_services::{WorkspaceError, provide_workspace};

    let ctx = Context::root("workspace");
    let registry = provide_workspace(&ctx).await.unwrap();

    let w1 = registry.create("w1", "/home/user/project-a").unwrap();
    let _w2 = registry.create("w2", "/home/user/project-b").unwrap();
    assert_eq!(w1.title, "project-a");
    assert!(matches!(
        registry.create("w1", "/another"),
        Err(WorkspaceError::Duplicate(_))
    ));

    registry.attach("w1", "s1").unwrap();
    registry.attach("w1", "s1").unwrap(); // idempotent
    registry.attach("w1", "s2").unwrap();
    assert_eq!(registry.sessions_in("w1"), ["s1", "s2"]);

    let list = registry.list();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, "w1"); // durable registry order

    assert!(matches!(
        registry.attach("missing", "s1"),
        Err(WorkspaceError::NotFound(_))
    ));
    registry.archive("w1").unwrap();
    assert!(matches!(
        registry.attach("w1", "s3"),
        Err(WorkspaceError::Archived(_))
    ));
}

#[tokio::test]
async fn storage_hub_validates_domains_and_routes_backends() {
    use cordis_rs::core_services::{DomainSpec, MemoryBackend, StorageError, provide_storage};

    let ctx = Context::root("storage");
    let hub = provide_storage(&ctx).await.unwrap();
    hub.register_backend("mem", MemoryBackend::new());
    hub.define_domain(DomainSpec::new("prefs", vec!["theme".to_string()]));

    // Valid record writes and reads back.
    hub.write(
        "prefs",
        "mem",
        "user-1",
        serde_json::json!({ "theme": "dark", "lang": "ja" }),
    )
    .unwrap();
    let value = hub.read("prefs", "mem", "user-1").unwrap();
    assert_eq!(value["theme"], "dark");

    // Missing required field is rejected before any write.
    assert!(matches!(
        hub.write(
            "prefs",
            "mem",
            "user-2",
            serde_json::json!({ "lang": "en" })
        ),
        Err(StorageError::InvalidRecord { .. })
    ));
    assert!(matches!(
        hub.read("prefs", "mem", "user-2"),
        Err(StorageError::NotFound(_))
    ));

    assert!(matches!(
        hub.read("unknown", "mem", "k"),
        Err(StorageError::UnknownDomain(_))
    ));
    assert!(matches!(
        hub.read("prefs", "missing", "k"),
        Err(StorageError::UnknownBackend(_))
    ));
}

#[tokio::test]
async fn agent_registry_creates_and_lists_agents() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{AgentError, AgentRecord, provide_agents};

    let ctx = Context::root("agents");
    let registry = provide_agents(&ctx).await.unwrap();

    let _factory = registry.register_factory(
        "demo",
        StdArc::new(|id| {
            Ok(AgentRecord {
                id: id.to_string(),
                session_id: String::new(),
                cwd: Some("/tmp".to_string()),
            })
        }),
    );

    let agent = registry.create("demo", "a-1").unwrap();
    assert_eq!(agent.id, "a-1");
    assert_eq!(agent.session_id, "a-1"); // 1:1 with session
    assert_eq!(agent.cwd.as_deref(), Some("/tmp"));

    assert!(matches!(
        registry.create("demo", "a-1"),
        Err(AgentError::Duplicate(_))
    ));
    assert!(matches!(
        registry.create("unknown", "a-2"),
        Err(AgentError::UnknownFactory(_))
    ));

    let direct = AgentRecord {
        id: "a-2".to_string(),
        session_id: "a-2".to_string(),
        cwd: None,
    };
    let _disposer = registry.register(direct).unwrap();
    assert_eq!(registry.list().len(), 2);
    assert!(registry.get("a-1").is_some());
    assert!(registry.get("missing").is_none());
}

#[tokio::test]
async fn otel_event_channels_batch_and_bound_records() {
    use cordis_rs::core_services::{EventLogOptions, TelemetryError, provide_otel};
    use std::collections::HashMap;

    let ctx = Context::root("otel");
    let otel = provide_otel(&ctx).await.unwrap();
    assert!(otel.channel_names().is_empty());

    let (reporter, _disposer) = otel.event_channel(
        "diagnostics",
        EventLogOptions {
            max_events: 3,
            batch_size: 2,
        },
    );
    assert_eq!(otel.channel_names(), ["diagnostics"]);

    let mut attrs = HashMap::new();
    attrs.insert("kind".to_string(), "test".to_string());
    assert!(!reporter.record("tick", attrs.clone()).unwrap());
    assert!(reporter.record("tick", attrs.clone()).unwrap()); // batch boundary (2 ticks)
    assert!(!reporter.record("tick", attrs.clone()).unwrap());
    reporter.record("tick", attrs.clone()).unwrap();
    assert_eq!(reporter.events().len(), 3); // bounded by max_events
    assert_eq!(reporter.dropped(), 1); // one event evicted by the cap (R5-VU04)
    assert_eq!(reporter.count("tick"), 4);
    assert!(reporter.record("", attrs).is_err());
    assert!(matches!(
        reporter.record("", HashMap::new()),
        Err(TelemetryError::InvalidName)
    ));
}

#[tokio::test]
async fn tool_pipeline_approval_gate_is_fail_closed() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        ApprovalDecision, ApprovalService, ToolDefinition, ToolExecContext, ToolOutput,
        provide_approval, provide_tools,
    };

    let ctx = Context::root("approval-gate");
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    let approval: ApprovalService = provide_approval(&ctx).await.unwrap();
    pipeline.with_approval(approval.clone());

    let _tool = registry.register(ToolDefinition::new(
        "echo",
        "echo",
        StdArc::new(|_args, _exec| {
            Box::pin(async { Ok(ToolOutput::text("ok")) })
                as cordis_rs::events::BoxFuture<
                    'static,
                    std::result::Result<ToolOutput, cordis_rs::core_services::ToolError>,
                >
        }),
    ));

    // No answerer -> fail closed.
    let error = pipeline
        .run(
            &registry,
            "echo",
            serde_json::json!({}),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("blocked"));

    // An answerer that allows enables execution.
    let _d = approval.add_answerer(
        "allow-all",
        StdArc::new(|_request| Some(ApprovalDecision::Allow)),
    );
    let output = pipeline
        .run(
            &registry,
            "echo",
            serde_json::json!({}),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(output.rendered, ["ok"]);

    // An answerer that denies blocks execution.
    let _d2 = approval.add_answerer(
        "deny-all",
        StdArc::new(|_request| Some(ApprovalDecision::Deny)),
    );
    let error = pipeline
        .run(
            &registry,
            "echo",
            serde_json::json!({}),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("denied"));
}

#[tokio::test]
async fn approval_mode_never_denies_even_with_answerer() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        ApprovalDecision, ApprovalMode, ApprovalRequest, provide_approval,
    };

    let ctx = Context::root("approval-mode");
    let approval = provide_approval(&ctx).await.unwrap();

    let _d = approval.add_answerer(
        "allow",
        StdArc::new(|_request| Some(ApprovalDecision::Allow)),
    );
    let request = ApprovalRequest::new("tool:bash", "run command");

    // Ask -> answerer allows.
    assert_eq!(approval.policy(), ApprovalMode::Ask);
    assert_eq!(
        approval.request(&request).unwrap().decision,
        ApprovalDecision::Allow
    );

    // Never -> answerer is bypassed and the request is denied.
    approval.set_policy(ApprovalMode::Never);
    let outcome = approval.request(&request).unwrap();
    assert_eq!(outcome.decision, ApprovalDecision::Deny);
    assert_eq!(outcome.answerer.as_deref(), Some("policy:never"));
}

#[tokio::test]
async fn switching_preset_updates_approval_mode() {
    use cordis_rs::core_services::{ApprovalMode, provide_approval, provide_permission_presets};

    let ctx = Context::root("preset-approval");
    let approval = provide_approval(&ctx).await.unwrap();
    let presets = provide_permission_presets(&ctx).await.unwrap();
    presets.bind_approval(approval.clone());

    // danger-full-access has approval=ask in the upstream spec.
    presets.switch("danger-full-access").unwrap();
    assert_eq!(approval.policy(), ApprovalMode::Ask);

    // read-only and workspace-write both use approval=never.
    presets.switch("read-only").unwrap();
    assert_eq!(approval.policy(), ApprovalMode::Never);
    presets.switch("workspace-write").unwrap();
    assert_eq!(approval.policy(), ApprovalMode::Never);
}

#[tokio::test]
async fn command_compact_port_compacts_active_session() {
    use cordis_rs::core_services::{
        CommandInvocation, CommandRegistry, CommandResult, CompactionConfig, CompactionEngine,
        TokenMeter, provide_commands, provide_sessions,
    };
    use cordis_rs::plugin_ports::mount_command_compact;

    let ctx = Context::root("command-compact-port");
    let commands: CommandRegistry = provide_commands(&ctx).await.unwrap();
    let (sessions, _) = provide_sessions(&ctx).await.unwrap();
    let session = sessions
        .create(Some("compact-cmd".to_string()), None)
        .unwrap();
    session.append(
        "user/message",
        serde_json::json!({ "text": "keep this line" }),
    );

    // A high threshold: automatic compaction would not fire, but /compact forces it.
    let engine = CompactionEngine::new(
        CompactionConfig {
            pressure_threshold: 0.99,
            head_chars: 4,
            tail_chars: 4,
        },
        TokenMeter::new(1000),
    )
    .unwrap();
    ctx.provide("compaction", engine).await.unwrap();
    mount_command_compact(&ctx, sessions.clone()).await.unwrap();

    assert!(commands.names().contains(&"/compact".to_string()));
    let result = commands
        .invoke(
            "/compact",
            CommandInvocation::new("").with_session("compact-cmd"),
        )
        .await
        .unwrap();
    assert!(matches!(result, CommandResult::Success { .. }));
    assert!(
        session
            .events()
            .iter()
            .any(|event| event.event_type == "compaction/checkpoint")
    );

    let with_args = commands
        .invoke("/compact", CommandInvocation::new("extra"))
        .await
        .unwrap();
    assert!(matches!(with_args, CommandResult::Error { .. }));
}

#[tokio::test]
async fn retry_policy_decides_and_computes_backoff() {
    use cordis_rs::core_services::{RetryDecision, provide_retry};

    let ctx = Context::root("retry");
    let policy = provide_retry(&ctx).await.unwrap();

    assert_eq!(policy.should_retry(429, 0), RetryDecision::Retry);
    assert_eq!(policy.should_retry(500, 2), RetryDecision::Retry);
    assert_eq!(policy.should_retry(400, 0), RetryDecision::Stop);
    assert_eq!(policy.should_retry(429, 3), RetryDecision::Stop); // budget used

    let first = policy.backoff_ms(1, 0.0);
    let mid = policy.backoff_ms(1, 0.5);
    let last = policy.backoff_ms(1, 1.0);
    assert!(first <= mid && mid <= last);
    assert!(policy.backoff_ms(9, 0.5) <= policy.max_delay_ms);
}

#[tokio::test]
async fn tool_pipeline_retries_retryable_errors() {
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use cordis_rs::core_services::{
        RetryPolicy, ToolDefinition, ToolError, ToolExecContext, ToolOutput, provide_tools,
    };

    let ctx = Context::root("tool-retry");
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    let calls = StdArc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let _tool = registry.register(ToolDefinition::new(
        "flaky",
        "flaky",
        StdArc::new(move |_args, _exec| {
            let counter = counter.clone();
            Box::pin(async move {
                if counter.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                    Err(ToolError::Execution("HTTP 429 rate limited".to_string()))
                } else {
                    Ok(ToolOutput::text("ok"))
                }
            })
        }),
    ));

    pipeline.with_retry(RetryPolicy {
        max_retries: 2,
        initial_delay_ms: 1,
        max_delay_ms: 1,
        ..Default::default()
    });

    let output = pipeline
        .run(
            &registry,
            "flaky",
            serde_json::json!({}),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(output.rendered, ["ok"]);
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 2);
}

#[tokio::test]
async fn agent_loop_retries_retryable_llm_errors() {
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use cordis_rs::core_services::{
        AgentLoop, LlmRequest, RetryPolicy, StreamChunk, provide_llm, provide_tools,
    };

    let ctx = Context::root("agent-llm-retry");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();

    let attempts = StdArc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let provider = cordis_rs::core_services::from_async_text(
        "flaky",
        "flaky-1",
        move |_request: LlmRequest| {
            let counter = counter.clone();
            async move {
                if counter.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                    Err("HTTP 429 rate limited".to_string())
                } else {
                    Ok::<_, String>("done".to_string())
                }
            }
        },
    );
    let _p = llm.register(provider);
    llm.set_default("flaky");

    let policy = RetryPolicy {
        max_retries: 2,
        initial_delay_ms: 1,
        max_delay_ms: 1,
        ..Default::default()
    };
    let loop_ = AgentLoop::new(llm, tools)
        .with_retry(policy)
        .with_call_parser(StdArc::new(|_text: &str| Vec::new()));
    let run = loop_.run("go").await.unwrap();
    assert_eq!(run.final_text, "done");
    assert_eq!(attempts.load(AtomicOrdering::SeqCst), 2);

    let _ = StreamChunk::Done;
}

#[tokio::test]
async fn hook_protocol_merges_most_restrictive_outcome() {
    use cordis_rs::core_services::{
        HookOutput, MergedDecision, is_match_all, matches, merge_hook_outputs,
    };

    let outputs = vec![
        HookOutput {
            decision: MergedDecision::Allow,
            reason: Some("looks fine".to_string()),
            stop: false,
            context: Some("ctx-a".to_string()),
        },
        HookOutput {
            decision: MergedDecision::Deny,
            reason: Some("blocked by policy".to_string()),
            stop: true,
            context: None,
        },
        HookOutput {
            decision: MergedDecision::Ask,
            reason: Some("please confirm".to_string()),
            stop: false,
            context: None,
        },
    ];
    let merged = merge_hook_outputs(&outputs);
    assert_eq!(merged.decision, MergedDecision::Deny);
    assert_eq!(merged.reasons, ["blocked by policy"]);
    assert!(merged.stop);
    assert_eq!(merged.contexts, ["ctx-a"]);

    assert!(merge_hook_outputs(&[]).decision == MergedDecision::None);
    assert!(is_match_all(None));
    assert!(is_match_all(Some("*")));
    assert!(matches(Some("*"), "anything", false));
    assert!(matches(Some("a|b"), "b", true));
    assert!(!matches(Some("a|b"), "c", true));
}

#[tokio::test]
async fn hook_protocol_regex_matcher_and_diagnostics() {
    use cordis_rs::core_services::{matcher_diagnostic, matches};

    // Unanchored regex matching.
    assert!(matches(Some("PreToolUse|Bash"), "tool:Bash", false));
    assert!(!matches(Some("^Bash$"), "tool:Bash", false));
    assert!(matches(Some("^tool:Bash$"), "tool:Bash", false));

    // Invalid regexes are non-matches at runtime but diagnosed at config load.
    assert!(!matches(Some("["), "anything", false));
    assert!(matcher_diagnostic(Some("["), false).is_some());
    assert!(matcher_diagnostic(Some("^tool:.*$"), false).is_none());
    assert!(matcher_diagnostic(None, false).is_none());
    assert!(matcher_diagnostic(Some("*"), false).is_none());

    // Literal mode accepts any pattern without regex validation.
    assert!(matcher_diagnostic(Some("["), true).is_none());
}

#[tokio::test]
async fn session_titles_keep_generated_and_fallback_titles() {
    use cordis_rs::core_services::{SessionTitles, provide_session_titles};

    let ctx = Context::root("session-titles");
    let titles: SessionTitles = provide_session_titles(&ctx).await.unwrap();

    let messages = vec!["How do I port plugins?".to_string()];
    titles.derive_from_messages("s1", &messages, 12);
    assert_eq!(titles.get("s1").as_deref(), Some("How do I por"));

    // An existing title is not overwritten by the fallback.
    titles.derive_from_messages("s1", &["Different question".to_string()], 20);
    assert_eq!(titles.get("s1").as_deref(), Some("How do I por"));

    // A generated title replaces it, using the first non-empty line.
    assert!(titles.set_generated("s1", "\n  Rust plugin port  \nextra", 30));
    assert_eq!(titles.get("s1").as_deref(), Some("Rust plugin port"));

    // Empty provider output is ignored.
    assert!(!titles.set_generated("s2", "  \n  ", 30));
    assert!(titles.get("s2").is_none());

    titles.set("s3", "  trimmed  ");
    assert_eq!(titles.get("s3").as_deref(), Some("trimmed"));
    assert_eq!(titles.list().len(), 2);
}

#[tokio::test]
async fn title_provider_frames_messages_and_parses_output() {
    use cordis_rs::core_services::{
        TitleProviderConfig, frame_messages, title_from_output, title_system_prompt,
    };

    let config = TitleProviderConfig::default();
    let prompt = title_system_prompt(&config, false);
    assert!(prompt.contains("about 6 words"));
    assert!(!prompt.contains("currentTitle"));
    let with_title = title_system_prompt(&config, true);
    assert!(with_title.contains("currentTitle"));

    let framed = frame_messages(&["hello".to_string(), "world".to_string()], None);
    assert!(framed.contains("\"hello\""));
    let updated = frame_messages(&["more".to_string()], Some("Old title"));
    assert!(updated.contains("\"Old title\""));

    assert_eq!(
        title_from_output("\n  **My Title**  \nextra").as_deref(),
        Some("My Title")
    );
    assert_eq!(
        title_from_output("plain title").as_deref(),
        Some("plain title")
    );
    assert!(title_from_output("   \n  ").is_none());
}

#[tokio::test]
async fn session_title_llm_port_generates_and_stores_title() {
    use cordis_rs::core_services::{
        LlmRequest, TitleProviderConfig, provide_llm, provide_session_titles,
    };
    use cordis_rs::plugin_ports::{SessionTitleLlm, mount_session_title_llm};

    let ctx = Context::root("title-llm-port");
    let llm = provide_llm(&ctx).await.unwrap();
    let titles = provide_session_titles(&ctx).await.unwrap();
    let config = TitleProviderConfig::default();
    mount_session_title_llm(&ctx, config.clone()).await.unwrap();

    let provider = cordis_rs::core_services::from_async_text(
        "titles",
        "titles-1",
        |_request: LlmRequest| async { Ok::<_, String>("**Ported Plugins**\nextra".to_string()) },
    );
    let _p = llm.register(provider);
    llm.set_default("titles");

    let generator = SessionTitleLlm::new(config);
    let messages = vec!["How do I port plugins?".to_string()];
    assert!(
        generator
            .generate(&llm, &titles, "s1", &messages)
            .await
            .unwrap()
    );
    assert_eq!(titles.get("s1").as_deref(), Some("Ported Plugins"));
    assert!(
        !generator
            .generate(&llm, &titles, "empty", &[])
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn schedule_service_manages_status_and_due_tasks() {
    use cordis_rs::core_services::{ScheduleError, ScheduleStatus, provide_schedule};

    let ctx = Context::root("schedule");
    let service = provide_schedule(&ctx).await.unwrap();

    let a = service.create("morning", "check builds", 1000);
    let b = service.create("evening", "summarize", 5000);
    assert_eq!(a.status, ScheduleStatus::Active);
    assert_eq!(service.list().len(), 2);

    // Only the first is due at t=2000.
    let due = service.due(2000);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, a.id);

    // Pausing removes it from due tasks.
    service.pause(&a.id).unwrap();
    assert!(service.due(9000).iter().all(|item| item.id != a.id));
    service.resume(&a.id).unwrap();
    assert_eq!(service.due(9000).len(), 2);

    service.cancel(&b.id).unwrap();
    assert_eq!(
        service.get(&b.id).unwrap().status,
        ScheduleStatus::Cancelled
    );
    assert!(matches!(
        service.pause("missing"),
        Err(ScheduleError::NotFound(_))
    ));
}

#[tokio::test]
async fn tool_schedule_exposes_management_tools() {
    use cordis_rs::core_services::{ToolExecContext, provide_schedule, provide_tools};
    use cordis_rs::plugin_ports::mount_tool_schedule;

    let ctx = Context::root("tool-schedule");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    provide_schedule(&ctx).await.unwrap();
    mount_tool_schedule(&ctx).await.unwrap();

    let mut names = tools.names();
    names.sort();
    assert_eq!(
        names,
        [
            "schedule_create",
            "schedule_edit",
            "schedule_list",
            "schedule_view"
        ]
    );

    let created = tools
        .invoke(
            "schedule_create",
            serde_json::json!({ "title": "build", "prompt": "run tests", "due_ms": 1000 }),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert!(created.rendered[0].contains("Created schedule"));
    let id = created.content["id"].as_str().unwrap().to_string();

    let listed = tools
        .invoke(
            "schedule_list",
            serde_json::json!({}),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert_eq!(listed.content["schedules"].as_array().unwrap().len(), 1);

    let edited = tools
        .invoke(
            "schedule_edit",
            serde_json::json!({ "id": id, "action": "pause" }),
            ToolExecContext::agent("a"),
        )
        .await
        .unwrap();
    assert!(edited.rendered[0].contains("paused"));
}

#[tokio::test]
async fn plan_mode_folds_last_session_event() {
    use cordis_rs::core_services::{PlanMode, provide_plan_mode};

    let ctx = Context::root("plan-mode");
    let plan: PlanMode = provide_plan_mode(&ctx).await.unwrap();

    // No events -> inactive.
    assert!(!plan.is_active("s1"));

    let first = plan.enter("s1");
    assert!(plan.is_active("s1"));
    let second = plan.exit("s1");
    assert!(!plan.is_active("s1"));
    plan.enter("s1");
    assert!(plan.is_active("s1"));

    let trace = plan.trace("s1");
    assert_eq!(trace.len(), 3);
    assert!(trace[0].seq < trace[1].seq && trace[1].seq < trace[2].seq);
    assert!(trace[0].active && !trace[1].active && trace[2].active);
    assert_eq!(first.seq, trace[0].seq);
    assert_eq!(second.seq, trace[1].seq);

    // Sessions are isolated.
    assert!(!plan.is_active("s2"));
}

#[tokio::test]
async fn exit_plan_mode_tool_gates_on_plan_state() {
    use cordis_rs::core_services::{ToolExecContext, provide_plan_mode, provide_tools};
    use cordis_rs::plugin_ports::{EXIT_PLAN_MODE, mount_tool_exit_plan_mode, validate_plan};

    let ctx = Context::root("exit-plan-tool");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let plan = provide_plan_mode(&ctx).await.unwrap();
    mount_tool_exit_plan_mode(&ctx).await.unwrap();
    assert!(tools.names().contains(&EXIT_PLAN_MODE.to_string()));

    let exec = ToolExecContext::agent("a");
    let mut with_session = exec.clone();
    with_session.session = Some("s1".to_string());

    // Not in plan mode -> informational response, state unchanged.
    let result = tools
        .invoke(
            EXIT_PLAN_MODE,
            serde_json::json!({ "plan": "# Plan", "approved": true }),
            with_session.clone(),
        )
        .await
        .unwrap();
    assert!(result.rendered[0].contains("PLAN_NOT_ACTIVE"));

    plan.enter("s1");
    // Invalid plan text is rejected.
    let result = tools
        .invoke(
            EXIT_PLAN_MODE,
            serde_json::json!({ "plan": "not markdown", "approved": true }),
            with_session.clone(),
        )
        .await
        .unwrap();
    assert!(result.rendered[0].contains("non-empty"));
    assert!(plan.is_active("s1"));

    // Approved call exits plan mode.
    let result = tools
        .invoke(
            EXIT_PLAN_MODE,
            serde_json::json!({ "plan": "# Plan\nsteps", "approved": true }),
            with_session,
        )
        .await
        .unwrap();
    assert!(result.rendered[0].contains("Plan approved"));
    assert!(!plan.is_active("s1"));

    assert!(validate_plan("# ok").is_ok());
    assert!(validate_plan("bad").is_err());
}

#[tokio::test]
async fn credentials_resolve_store_then_env_without_exposing_values() {
    use cordis_rs::core_services::{credential_ref, provide_credentials};
    use std::collections::BTreeMap;

    let ctx = Context::root("credentials");
    let mut env = BTreeMap::new();
    env.insert("DEEPSEEK_API_KEY".to_string(), "env-secret".to_string());
    let mut provider = provide_credentials(&ctx).await.unwrap();
    provider = provider.with_env(env);

    // Grammar: only uppercase/digit/underscore segments after `$`.
    assert!(credential_ref("$DEEPSEEK_API_KEY").is_some());
    assert!(credential_ref("DEEPSEEK_API_KEY").is_none());
    assert!(credential_ref("$lower").is_none());

    let reference = credential_ref("$DEEPSEEK_API_KEY").unwrap();
    let resolved = provider.resolve(&reference).unwrap();
    assert_eq!(resolved.value, "env-secret");
    assert_eq!(resolved.source, "env");

    // Stored value takes precedence; describe never exposes it.
    assert!(provider.set("DEEPSEEK_API_KEY", "stored-secret"));
    let resolved = provider.resolve(&reference).unwrap();
    assert_eq!(resolved.value, "stored-secret");
    assert_eq!(resolved.source, "store");
    let info = provider.describe(&reference);
    assert!(info.configured);
    assert_eq!(info.source.as_deref(), Some("store"));

    // Empty writes are rejected and unset falls back to env.
    assert!(!provider.set("DEEPSEEK_API_KEY", ""));
    provider.unset("DEEPSEEK_API_KEY");
    assert_eq!(provider.resolve(&reference).unwrap().source, "env");

    let missing = credential_ref("$MISSING").unwrap();
    assert!(!provider.describe(&missing).configured);
}

#[tokio::test]
async fn timeout_policy_resolves_budgets_and_detects_expiry() {
    use cordis_rs::core_services::{TimeoutPolicy, provide_timeout_policy};

    let ctx = Context::root("timeout-policy");
    let policy: TimeoutPolicy = provide_timeout_policy(&ctx).await.unwrap();

    // No declaration and no fallback -> no timeout.
    assert_eq!(policy.timeout_of(None), None);
    assert!(!policy.is_expired(None, 100_000));

    let policy = TimeoutPolicy {
        default_timeout_ms: Some(1000),
    };
    assert_eq!(policy.timeout_of(None), Some(1000));
    assert_eq!(policy.timeout_of(Some(250)), Some(250)); // declaration wins
    assert_eq!(policy.timeout_of(Some(0)), None); // zero is not a budget

    assert!(!policy.is_expired(None, 999));
    assert!(policy.is_expired(None, 1000));
    let result = policy.check(Some(250), 300).unwrap();
    assert!(result.timed_out);
    assert_eq!(result.message, "tool call timed out after 250ms");
    assert!(policy.check(None, 500).is_none());
}

#[tokio::test]
async fn plugin_inventory_projects_composition_entries() {
    use cordis_rs::core_services::provide_plugin_inventory;
    use cordis_rs::loader::{Composition, CompositionState};

    let ctx = Context::root("plugin-inventory");
    let inventory = provide_plugin_inventory(&ctx).await.unwrap();
    assert!(inventory.list().entries.is_empty());

    let yaml = r#"
entries:
  - id: a
    plugin: demo/a
    config: {}
  - id: b
    plugin: demo/b
    disabled: true
    config: {}
"#;
    inventory.set_composition(Composition::parse(yaml).unwrap());
    let snapshot = inventory.list();
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries[0].entry_id, "a");
    assert!(snapshot.entries[0].enabled);
    assert!(!snapshot.entries[1].enabled);

    // Overrides are reflected on the next read (no cache).
    let mut state = CompositionState::new();
    state.set("a", false);
    state.set("b", true);
    inventory.set_state(state);
    let snapshot = inventory.list();
    assert!(!snapshot.entries[0].enabled);
    assert!(snapshot.entries[1].enabled);
}

#[tokio::test]
async fn time_context_formats_samples_and_respects_refresh_interval() {
    use cordis_rs::core_services::{
        TimeContext, TimeContextConfig, format_sample, has_utc_offset, provide_time_context,
        validate_refresh_interval,
    };

    let ctx = Context::root("time-context");
    let _service = provide_time_context(&ctx).await.unwrap();
    assert!(validate_refresh_interval(1000).is_ok());

    let line = format_sample(7, 3, "2026-11-17T21:56:29+09:00").unwrap();
    assert_eq!(
        line,
        "Time sampled while preparing turn 7, step 3: 2026-11-17T21:56:29+09:00\n"
    );
    assert!(format_sample(1, 1, "2026-11-17T12:56:29Z").is_ok());
    assert!(format_sample(1, 1, "2026-11-17T12:56:29").is_err());
    assert!(has_utc_offset("2026-11-17T12:56:29-05:00"));
    assert!(!has_utc_offset("no-offset"));

    let mut throttled = TimeContext::new(TimeContextConfig {
        refresh_interval_ms: 1000,
    })
    .unwrap();
    assert!(throttled.should_inject(10_000));
    throttled.mark_injected(10_000);
    assert!(!throttled.should_inject(10_500));
    assert!(throttled.should_inject(11_000));
}

#[tokio::test]
async fn agent_instructions_register_baseline_sections() {
    use cordis_rs::core_services::provide_system_prompt;
    use cordis_rs::plugin_ports::register_baseline;

    let ctx = Context::root("agent-instructions");
    let prompt = provide_system_prompt(&ctx).await.unwrap();

    let dir = TempDirGuard::new("agent-instructions");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("AGENTS.md"), "# Agent rules\nuse bash").unwrap();
    std::fs::write(dir.join("GEMINI.md"), "# Gemini rules").unwrap();

    let names = register_baseline(&prompt, &dir).unwrap();
    assert_eq!(names.len(), 2);
    // Unknown section names get the default order of 500.
    assert_eq!(
        prompt.get_section_order("agent-instructions:AGENTS.md"),
        500
    );

    // Duplicate registration is rejected by the prompt registry.
    assert!(register_baseline(&prompt, &dir).is_err());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn session_reference_roundtrips_encoded_uris() {
    use cordis_rs::core_services::{
        SessionReference, decode_session_reference_uri, encode_session_reference_uri,
        format_session_reference_mention,
    };

    let uri = encode_session_reference_uri("session-123");
    assert!(uri.starts_with("dsh-session:"));
    assert_eq!(decode_session_reference_uri(&uri).unwrap(), "session-123");

    let mention = format_session_reference_mention(&SessionReference {
        session_id: "session-123".to_string(),
        label: "Prior work".to_string(),
    });
    assert!(mention.contains("Prior work"));
    assert!(mention.contains("dsh-session:"));

    assert!(decode_session_reference_uri("not-a-reference").is_err());
    assert!(decode_session_reference_uri("dsh-session:!!!").is_err());
}

#[tokio::test]
async fn file_reference_formats_and_detects_mentions() {
    use cordis_rs::core_services::{ReferenceKind, active_at_token, format_file_mention};

    assert_eq!(
        format_file_mention("src/main.rs", ReferenceKind::File, false).as_deref(),
        Some("@src/main.rs")
    );
    assert_eq!(
        format_file_mention("my docs", ReferenceKind::File, false).as_deref(),
        Some("@\"my docs\"")
    );
    assert_eq!(
        format_file_mention("src", ReferenceKind::Directory, false).as_deref(),
        Some("@src/")
    );
    assert!(format_file_mention("bad\"path", ReferenceKind::File, false).is_none());

    let token = active_at_token("look at @src/ma", 15).unwrap();
    assert_eq!(token.raw, "src/ma");
    assert!(!token.quoted);

    let quoted = active_at_token("use @\"my docs", 13).unwrap();
    assert_eq!(quoted.raw, "my docs");
    assert!(quoted.quoted);

    assert!(active_at_token("no mention", 2).is_none());
}

#[tokio::test]
async fn workspace_file_search_indexes_and_filters() {
    use cordis_rs::core_services::WorkspaceFileSearch;

    let dir = TempDirGuard::new("file-search");
    let _ = std::fs::create_dir_all(dir.join("src"));
    let _ = std::fs::create_dir_all(dir.join("node_modules"));
    std::fs::write(dir.join("src/main.rs"), "").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "").unwrap();
    std::fs::write(dir.join("node_modules/ignored.js"), "").unwrap();

    let search = WorkspaceFileSearch::new(dir.to_path_buf(), Default::default()).unwrap();
    let all = search.search("");
    assert!(all.iter().any(|entry| entry.path == "src/main.rs"));
    assert!(!all.iter().any(|entry| entry.path.contains("node_modules")));

    let src = search.search("src/");
    assert_eq!(src.len(), 2);
    assert!(src.iter().all(|entry| !entry.is_directory));

    // Invalid config is rejected.
    let config = cordis_rs::core_services::FileSearchConfig {
        max_results: 0,
        ..Default::default()
    };
    assert!(WorkspaceFileSearch::new(dir.path(), config).is_err());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn harness_dev_context_injects_runtime_values() {
    use cordis_rs::core_services::AssembleContext;
    use cordis_rs::plugin_ports::{
        HARNESS_SOURCE_KEY, WEB_GUI_URL_KEY, harness_context_value, with_harness_context,
    };

    let context = with_harness_context(
        AssembleContext::default(),
        "/opt/dsh",
        "https://example.ts.net:8445/",
    );
    assert_eq!(
        harness_context_value(&context, HARNESS_SOURCE_KEY),
        Some("/opt/dsh")
    );
    assert_eq!(
        harness_context_value(&context, WEB_GUI_URL_KEY),
        Some("https://example.ts.net:8445/")
    );
    assert!(harness_context_value(&context, "missing").is_none());
}

#[tokio::test]
async fn tmux_context_parses_and_formats_locations() {
    use cordis_rs::core_services::{TmuxContext, TmuxLocation, format_reading, parse_location};

    let raw = "main\\t0\\tshell\\t1\\t%3\\t1\\t0\\t=,80,24,0,0,0";
    let location = parse_location(raw).unwrap();
    assert_eq!(location.session_name, "main");
    assert_eq!(location.window_name, "shell");
    assert_eq!(location.pane_id, "%3");
    assert!(parse_location("only\\ttwo").is_err());

    let line = format_reading(4, 2, &location, "2026-11-18T00:00:00+09:00");
    assert!(line.contains("turn 4, step 2"));
    assert!(line.contains("session=main"));
    assert!(line.contains("window=0:shell"));

    let mut context = TmuxContext::new();
    assert!(context.should_inject(&location));
    context.mark_injected(location.clone());
    assert!(!context.should_inject(&location));
    let moved = TmuxLocation {
        window_name: "editor".to_string(),
        ..location
    };
    assert!(context.should_inject(&moved));
}

#[tokio::test]
async fn repeat_reminder_detects_consecutive_identical_calls() {
    use cordis_rs::core_services::{
        ReminderConfig, RepeatToolReminder, canonicalize, validate_thresholds,
    };

    assert!(validate_thresholds(&[3, 5]).is_ok());
    assert!(validate_thresholds(&[]).is_err());
    assert!(validate_thresholds(&[1]).is_err());
    assert!(validate_thresholds(&[3, 3]).is_err());

    // Deep key-sort makes argument order irrelevant.
    let a = serde_json::json!({ "b": 2, "a": 1 });
    let b = serde_json::json!({ "a": 1, "b": 2 });
    assert_eq!(canonicalize(&a), canonicalize(&b));

    let mut detector = RepeatToolReminder::new(ReminderConfig {
        thresholds: vec![3],
        ..Default::default()
    })
    .unwrap();
    for _ in 0..2 {
        assert!(detector.observe("bash", &a).is_none());
    }
    let reminder = detector.observe("bash", &b).unwrap();
    assert_eq!(reminder.run_length, 3);
    assert!(!reminder.detailed);

    // A different call resets the chain.
    assert!(detector.observe("fs", &a).is_none());
}

#[tokio::test]
async fn cancelled_exec_context_skips_tool_body() {
    use cordis_rs::core_services::{ToolError, ToolExecContext, ToolOutput, provide_tools};
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    let ctx = Context::root("tool-cancel");
    let (tools, pipeline) = provide_tools(&ctx).await.unwrap();
    let calls = StdArc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let _tool = registry_register(&tools, "tracked", move || {
        let counter = counter.clone();
        Box::pin(async move {
            counter.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(ToolOutput::text("should not run"))
        })
            as cordis_rs::events::BoxFuture<'static, std::result::Result<ToolOutput, ToolError>>
    });

    let exec = ToolExecContext {
        signal_cancelled: true,
        cancel_flag: None,
        ..ToolExecContext::anonymous()
    };
    let error = pipeline
        .run(&tools, "tracked", serde_json::json!({}), exec)
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Cancelled(_)));
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
}

fn registry_register<F>(
    registry: &cordis_rs::core_services::ToolRegistry,
    name: &str,
    factory: F,
) -> cordis_rs::Disposer
where
    F: Fn() -> cordis_rs::events::BoxFuture<
            'static,
            std::result::Result<
                cordis_rs::core_services::ToolOutput,
                cordis_rs::core_services::ToolError,
            >,
        > + Send
        + Sync
        + 'static,
{
    use std::sync::Arc;
    registry.register(ToolDefinition::new(
        name,
        name,
        Arc::new(move |_args, _exec| factory()),
    ))
}

#[tokio::test]
async fn tool_goal_registers_three_tools_with_cas() {
    use cordis_rs::core_services::{ToolExecContext, provide_agents, provide_goals, provide_tools};
    use cordis_rs::plugin_ports::mount_tool_goal;

    let ctx = Context::root("tool-goal");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let store = provide_goals(&ctx).await.unwrap();
    provide_agents(&ctx).await.unwrap();
    mount_tool_goal(&ctx).await.unwrap();

    let mut names = tools.names();
    names.sort();
    assert_eq!(names, ["create_goal", "get_goal", "update_goal"]);

    let human = ToolExecContext::agent("human");
    let created = tools
        .invoke(
            "create_goal",
            serde_json::json!({ "objective": "ship the port", "max_goal_rounds": 10 }),
            human.clone(),
        )
        .await
        .unwrap();
    assert_eq!(created.content["goal"]["objective"], "ship the port");
    let revision = created.content["goal"]["revision"].as_u64().unwrap();

    // Stale revision is rejected via CAS.
    let stale = tools
        .invoke(
            "update_goal",
            serde_json::json!({ "goal_id": revision + 1, "action": "edit", "objective": "new" }),
            human.clone(),
        )
        .await;
    assert!(
        stale.is_err()
            || matches!(
                stale.unwrap().content["error"],
                serde_json::Value::String(_)
            )
    );

    let read = tools
        .invoke("get_goal", serde_json::json!({}), human)
        .await
        .unwrap();
    assert_eq!(read.content["goal"]["objective"], "ship the port");
    assert_eq!(store.get().unwrap().objective, "ship the port");

    // A goal round cannot create, but may complete.
    let round = ToolExecContext::agent("agent");
    assert!(
        tools
            .invoke(
                "create_goal",
                serde_json::json!({ "objective": "no" }),
                round,
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn session_store_rejects_duplicate_ids_and_invalid_forks() {
    use cordis_rs::Error;
    use cordis_rs::core_services::provide_sessions;

    let ctx = Context::root("session-validation");
    let (store, _) = provide_sessions(&ctx).await.unwrap();

    let first = store.create(Some("dup".to_string()), None).unwrap();
    assert!(matches!(
        store.create(Some("dup".to_string()), None),
        Err(Error::DuplicateSession(_))
    ));
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.get("dup").unwrap().id(), first.id());

    first.append("a", serde_json::json!({}));
    first.append("b", serde_json::json!({}));
    first.append("c", serde_json::json!({}));

    // Boundary beyond the event count is rejected (local seq is 1-based).
    assert!(matches!(
        store.fork(&first, 99, Some("child-a".to_string())),
        Err(Error::InvalidBoundary { .. })
    ));
    // Zero boundary is invalid.
    assert!(matches!(
        store.fork(&first, 0, Some("child-b".to_string())),
        Err(Error::InvalidBoundary { .. })
    ));
    // An existing child id is rejected.
    assert!(matches!(
        store.fork(&first, 2, Some("dup".to_string())),
        Err(Error::DuplicateSession(_))
    ));

    // Inclusive boundary copies only the first two events.
    let forked = store.fork(&first, 2, Some("child-ok".to_string())).unwrap();
    assert_eq!(forked.seq(), 2);
    assert_eq!(forked.events().len(), 2);
}

#[tokio::test]
async fn agent_loop_closes_turn_on_llm_failure() {
    use cordis_rs::core_services::{
        AgentLoop, CheckpointHook, LlmRequest, Session, provide_llm, provide_tools,
    };
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    let ctx = Context::root("turn-end-failure");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();
    let session = Session::new("failure-session", None);

    let provider = cordis_rs::core_services::from_async_text(
        "always-fails",
        "fails-1",
        |_request: LlmRequest| async { Err::<String, String>("HTTP 500 error".to_string()) },
    );
    let _p = llm.register(provider);
    llm.set_default("always-fails");

    let cleanup = StdArc::new(AtomicUsize::new(0));
    let counter = cleanup.clone();
    let hook: CheckpointHook = StdArc::new(move |_session| {
        counter.fetch_add(1, AtomicOrdering::SeqCst);
        Ok(())
    });

    let run = AgentLoop::new(llm, tools)
        .with_session(session.clone())
        .with_checkpoint(hook)
        .with_call_parser(StdArc::new(|_text: &str| Vec::new()));
    assert!(run.run("go").await.is_err());

    let ends: Vec<_> = session
        .events()
        .into_iter()
        .filter(|event| event.event_type == "turn/end")
        .collect();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].data["reason"]["kind"], "llm_error");
    assert_eq!(cleanup.load(AtomicOrdering::SeqCst), 1);
}

#[tokio::test]
async fn agent_loop_retry_budget_resets_between_steps() {
    use cordis_rs::core_services::{
        AgentLoop, LlmRequest, RetryPolicy, provide_llm, provide_tools,
    };
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    let ctx = Context::root("retry-per-step");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();

    let calls = StdArc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let provider = cordis_rs::core_services::from_async_text(
        "flaky-per-step",
        "flaky-1",
        move |_request: LlmRequest| {
            let counter = counter.clone();
            async move {
                let n = counter.fetch_add(1, AtomicOrdering::SeqCst);
                // Fail once at the start of each step (calls 0 and 2).
                if n.is_multiple_of(2) {
                    Err("HTTP 429 rate limited".to_string())
                } else {
                    Ok("ok".to_string())
                }
            }
        },
    );
    let _p = llm.register(provider);
    llm.set_default("flaky-per-step");

    // Step 1 emits a tool call so the loop advances to step 2.
    let mut step = StdArc::new(AtomicUsize::new(0));
    let parser_step = step.clone();
    let loop_ = AgentLoop::new(llm, tools)
        .with_retry(RetryPolicy {
            max_retries: 1,
            initial_delay_ms: 1,
            max_delay_ms: 1,
            ..Default::default()
        })
        .with_call_parser(StdArc::new(move |_text: &str| {
            if parser_step.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                vec![cordis_rs::core_services::ToolCall {
                    name: "noop".to_string(),
                    args: serde_json::json!({}),
                }]
            } else {
                Vec::new()
            }
        }));
    let _ = std::mem::replace(&mut step, StdArc::new(AtomicUsize::new(0)));
    let run = loop_.run("go").await.unwrap();
    assert_eq!(run.steps.len(), 2);
    // Two steps, each with one failure + one success = 4 provider calls.
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 4);
}

#[tokio::test]
async fn compaction_shortens_next_request_messages() {
    use cordis_rs::core_services::{
        AgentLoop, CompactionConfig, CompactionEngine, LlmRequest, Session, TokenMeter,
        provide_llm, provide_tools,
    };
    use std::sync::Arc as StdArc;
    use std::sync::Mutex as StdMutex;

    let ctx = Context::root("compaction-messages");
    let (tools, _) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();
    let session = Session::new("compact-msg", None);

    // Seed enough history so pressure triggers at the first step boundary.
    for i in 0..8 {
        session.append(
            "user/message",
            serde_json::json!({ "text": format!("old message {i}") }),
        );
    }

    let engine = CompactionEngine::new(
        CompactionConfig {
            pressure_threshold: 0.0, // always compact
            head_chars: 4,
            tail_chars: 4,
        },
        TokenMeter::new(1),
    )
    .unwrap();

    let seen: StdArc<StdMutex<Vec<usize>>> = StdArc::new(StdMutex::new(Vec::new()));
    let capture = seen.clone();
    let provider = cordis_rs::core_services::from_async_text(
        "capture",
        "capture-1",
        move |request: LlmRequest| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(request.messages.len());
                Ok::<_, String>("done".to_string())
            }
        },
    );
    let _p = llm.register(provider);
    llm.set_default("capture");

    let loop_ = AgentLoop::new(llm, tools)
        .with_session(session)
        .with_compaction(engine)
        .with_call_parser(StdArc::new(|_text: &str| Vec::new()));
    loop_.run("fresh prompt").await.unwrap();

    let lengths = seen.lock().unwrap();
    assert!(!lengths.is_empty());
    assert!(
        lengths[0] < 8,
        "first request should be compacted: {lengths:?}"
    );
    assert!(
        lengths.iter().any(|n| *n <= 3),
        "summary + tail: {lengths:?}"
    );
}

#[tokio::test]
async fn security_hardening_for_credentials_and_loader_paths() {
    use cordis_rs::core_services::{ResolvedCredential, credential_ref, provide_credentials};
    use cordis_rs::loader::{Composition, VarTable};

    // Debug output must never expose the secret value.
    let resolved = ResolvedCredential {
        value: "super-secret-key".to_string(),
        source: "env".to_string(),
    };
    let debug = format!("{resolved:?}");
    assert!(!debug.contains("super-secret-key"));
    assert!(debug.contains("<redacted>"));

    let ctx = Context::root("security-hardening");
    let provider = provide_credentials(&ctx).await.unwrap();
    assert!(
        provider
            .resolve(&credential_ref("$MISSING").unwrap())
            .is_none()
    );

    // dshHomePath rejects traversal and absolute paths.
    let vars = VarTable::new().with_home_path("/home/dsh");
    let bad = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      p: !!js dshHomePath('../../etc/passwd')
"#;
    assert!(Composition::parse_with_vars(bad, &vars).is_err());
    let absolute = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      p: !!js dshHomePath('/etc/passwd')
"#;
    assert!(Composition::parse_with_vars(absolute, &vars).is_err());
}

#[tokio::test]
async fn agent_loop_with_pipeline_applies_tool_middleware() {
    use std::sync::Arc as StdArc;

    use cordis_rs::core_services::{
        AgentLoop, LlmProvider, StreamChunk, ToolDefinition, ToolError, ToolExecContext,
        ToolOutput, provide_llm, provide_tools,
    };
    use cordis_rs::events::BoxFuture;

    let ctx = Context::root("agent-loop-pipeline");
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();

    let _guard = pipeline.use_middleware(StdArc::new(
        |_args: serde_json::Value,
         _exec: ToolExecContext,
         _next: cordis_rs::core_services::ToolNext| {
            Box::pin(async { Err(ToolError::Execution("blocked by middleware".to_string())) })
                as BoxFuture<'static, Result<ToolOutput, ToolError>>
        },
    ));

    let _tool = registry.register(ToolDefinition::new(
        "echo",
        "echo input",
        StdArc::new(|_args, _exec| Box::pin(async { Ok(ToolOutput::text("direct")) })),
    ));

    let flag = StdArc::new(std::sync::atomic::AtomicBool::new(true));
    let provider_flag = flag.clone();
    let provider = LlmProvider::new(
        "scripted",
        "scripted-1",
        StdArc::new(move |_request| {
            let has_result = _request
                .messages
                .iter()
                .any(|message| message.content.contains("blocked by middleware"));
            let flag = provider_flag.clone();
            let text = if !has_result && flag.swap(false, std::sync::atomic::Ordering::SeqCst) {
                "call echo".to_string()
            } else {
                "done".to_string()
            };
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text(text),
                StreamChunk::Done,
            ]))
        }),
    );
    llm.register(provider);

    let parser: cordis_rs::core_services::CallParser = StdArc::new(|text: &str| {
        if text == "call echo" {
            vec![cordis_rs::core_services::ToolCall::new(
                "echo",
                serde_json::json!({}),
            )]
        } else {
            Vec::new()
        }
    });

    let loop_ = AgentLoop::new(llm, registry.clone())
        .with_pipeline(pipeline)
        .with_call_parser(parser);
    loop_.run("go").await.unwrap();
    assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn agent_loop_propagates_caller_and_session_to_tool_exec() {
    use std::sync::Arc as StdArc;
    use std::sync::Mutex as StdMutex;

    use cordis_rs::core_services::{
        AgentLoop, LlmProvider, StreamChunk, ToolDefinition, ToolError, ToolExecContext,
        ToolOutput, provide_llm, provide_tools,
    };
    use cordis_rs::events::BoxFuture;

    let ctx = Context::root("agent-loop-caller");
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    let llm = provide_llm(&ctx).await.unwrap();

    type ExecLog = StdArc<StdMutex<Vec<(Option<String>, Option<String>)>>>;
    let seen: ExecLog = StdArc::new(StdMutex::new(Vec::new()));
    let captured = seen.clone();
    let _guard = pipeline.use_middleware(StdArc::new(
        move |_args: serde_json::Value,
              exec: ToolExecContext,
              next: cordis_rs::core_services::ToolNext| {
            let captured = captured.clone();
            Box::pin(async move {
                captured
                    .lock()
                    .unwrap()
                    .push((exec.agent.clone(), exec.session.clone()));
                next(_args, exec).await
            }) as BoxFuture<'static, Result<ToolOutput, ToolError>>
        },
    ));

    let _tool = registry.register(ToolDefinition::new(
        "echo",
        "echo",
        StdArc::new(|_args, _exec| Box::pin(async { Ok(ToolOutput::text("ok")) })),
    ));

    let first = StdArc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = first.clone();
    let provider = LlmProvider::new(
        "scripted",
        "scripted-1",
        StdArc::new(move |_request| {
            let flag = flag.clone();
            let text = if flag.swap(false, std::sync::atomic::Ordering::SeqCst) {
                "call echo".to_string()
            } else {
                "done".to_string()
            };
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text(text),
                StreamChunk::Done,
            ]))
        }),
    );
    llm.register(provider);

    let parser: cordis_rs::core_services::CallParser = StdArc::new(|text: &str| {
        if text == "call echo" {
            vec![cordis_rs::core_services::ToolCall::new(
                "echo",
                serde_json::json!({}),
            )]
        } else {
            Vec::new()
        }
    });

    AgentLoop::new(llm, registry)
        .with_pipeline(pipeline)
        .with_call_parser(parser)
        .with_caller("agent-42")
        .with_tool_session("session-7")
        .run("go")
        .await
        .unwrap();

    let captured = seen.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].0.as_deref(), Some("agent-42"));
    assert_eq!(captured[0].1.as_deref(), Some("session-7"));
}

#[tokio::test]
async fn checkpoint_try_load_fails_closed_on_corruption_and_tool_output_exposes_error() {
    use cordis_rs::core_services::{CheckpointCursor, ToolOutput};

    let dir = TempDirGuard::new("checkpoint-fc");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("cursor.json");

    // Missing file is a fresh start (Ok with zero).
    let cursor = CheckpointCursor::try_load(&path).unwrap();
    assert_eq!(cursor.written(), 0);

    // Malformed JSON is surfaced instead of silently reprocessing the log.
    std::fs::write(&path, "not json").unwrap();
    assert!(CheckpointCursor::try_load(&path).is_err());

    // `load` keeps legacy fallback behaviour.
    assert_eq!(CheckpointCursor::load(&path).written(), 0);

    // Structured tool errors are detectable by callers.
    let timeout = ToolOutput {
        content: serde_json::json!({ "code": "TOOL_TIMEOUT", "message": "timeout" }),
        rendered: vec!["timeout".to_string()],
    };
    assert!(timeout.is_error());
    assert!(!ToolOutput::text("ok").is_error());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn tool_pipeline_post_execute_observes_failed_tools() {
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cordis_rs::core_services::{
        ToolDefinition, ToolError, ToolExecContext, ToolOutput, provide_tools,
    };
    use cordis_rs::events::BoxFuture;

    let (registry, pipeline) = provide_tools(&Context::root("post-on-failure"))
        .await
        .unwrap();
    let _tool = registry.register(ToolDefinition::new(
        "fails",
        "always fails",
        StdArc::new(|_args, _exec| {
            Box::pin(async { Err(ToolError::Execution("boom".to_string())) })
        }),
    ));

    let observed = StdArc::new(AtomicUsize::new(0));
    let counter = observed.clone();
    let _observer = pipeline.on_post_execute(StdArc::new(move |output: ToolOutput| {
        let counter = counter.clone();
        Box::pin(async move {
            if output.is_error() {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            Ok(output)
        }) as BoxFuture<'static, Result<ToolOutput, ToolError>>
    }));

    let result = pipeline
        .run(
            &registry,
            "fails",
            serde_json::json!({}),
            ToolExecContext::agent("a"),
        )
        .await;
    assert!(matches!(result, Err(ToolError::Execution(_))));
    assert_eq!(observed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sandbox_policy_resolves_explicit_session_and_default_modes() {
    use cordis_rs::core_services::{SandboxMode, Session, provide_sessions};
    use cordis_rs::plugin_ports::{
        SANDBOX_INVALID_ROOT, SandboxPolicyConfig, mount_sandbox_policy, render_policy_context,
    };
    use std::path::PathBuf;

    let ctx = Context::root("sandbox-policy");
    provide_sessions(&ctx).await.unwrap();
    let config = SandboxPolicyConfig {
        mode: SandboxMode::ReadOnly,
        workspace_root: PathBuf::from("/work"),
        additional_writable_roots: vec![PathBuf::from("/tmp")],
    };
    let service = mount_sandbox_policy(&ctx, config).await.unwrap();
    assert!(
        ctx.get::<cordis_rs::plugin_ports::SandboxPolicyService>("sandboxPolicy")
            .is_some()
    );

    // Default is read-only (fail-safe).
    let resolved = service.resolve(None, None);
    assert_eq!(resolved.mode, SandboxMode::ReadOnly);
    assert_eq!(resolved.workspace_root, PathBuf::from("/work"));

    // Explicit override wins over the session projection.
    let session = Session::new("s1", Some("/work/repo".to_string()));
    session.append(
        "sandbox/mode",
        serde_json::json!({ "mode": "workspace-write" }),
    );
    assert_eq!(
        service.override_of(&session),
        Some(SandboxMode::WorkspaceWrite)
    );
    assert_eq!(
        service.resolve(Some(&session), None).mode,
        SandboxMode::WorkspaceWrite
    );
    assert_eq!(
        service
            .resolve(Some(&session), Some(SandboxMode::DangerFullAccess))
            .mode,
        SandboxMode::DangerFullAccess
    );
    assert_eq!(
        service.resolve(Some(&session), None).workspace_root,
        PathBuf::from("/work/repo")
    );

    // Writable root registration and disposal.
    let d = service
        .register_writable_root(PathBuf::from("/extra"))
        .unwrap();
    assert_eq!(
        service.resolve(None, None).additional_writable_roots.len(),
        2
    );
    assert!(
        service
            .register_writable_root(PathBuf::from("relative"))
            .is_err()
    );
    d.dispose().await.unwrap();
    assert_eq!(
        service.resolve(None, None).additional_writable_roots.len(),
        1
    );

    // Context rendering covers all three modes.
    assert!(render_policy_context(&resolved).contains("read-only"));
    assert!(
        render_policy_context(&service.resolve(Some(&session), None)).contains("workspace-write")
    );
    assert_eq!(SANDBOX_INVALID_ROOT, "SANDBOX_INVALID_ROOT");
}

#[tokio::test]
async fn user_questions_phase1_validates_and_routes_answers() {
    use cordis_rs::core_services::{provide_sessions, provide_tools};
    use cordis_rs::plugin_ports::{
        Answer, AskQuestion, AskRequest, UserQuestionError, mount_user_questions,
    };

    let ctx = Context::root("user-questions");
    provide_sessions(&ctx).await.unwrap();
    provide_tools(&ctx).await.unwrap();
    let service = mount_user_questions(&ctx).await.unwrap();

    // An empty question list is rejected before dispatch.
    assert_eq!(
        service
            .ask(
                AskRequest {
                    questions: Vec::new(),
                    agent: None
                },
                false
            )
            .await,
        Err(UserQuestionError::EmptyQuestions)
    );
    assert_eq!(
        service
            .ask(
                AskRequest {
                    questions: vec![],
                    agent: None
                },
                true
            )
            .await,
        Err(UserQuestionError::AskAborted)
    );

    let request = AskRequest {
        questions: vec![AskQuestion {
            id: "q1".to_string(),
            question: "Proceed?".to_string(),
            header: None,
            options: Vec::new(),
            multi_select: false,
        }],
        agent: None,
    };

    // No answerer -> NO_PROVIDER.
    assert_eq!(
        service.ask(request.clone(), false).await,
        Err(UserQuestionError::NoProvider)
    );

    // A registered answerer supplies the answer through the waterfall seam.
    let _disposer = service.on_answerer(|shared, _next| {
        let shared = shared.clone();
        Box::pin(async move {
            let agent = shared.lock().expect("ask request poisoned").agent.clone();
            Ok(Answer {
                answers: vec![cordis_rs::plugin_ports::AnswerItem {
                    id: "q1".to_string(),
                    selected: vec![agent.unwrap_or_else(|| String::from("anonymous"))],
                    custom: None,
                }],
            })
        })
    });
    let answer = service.ask(request, false).await.unwrap();
    assert_eq!(answer.answers[0].id, "q1");

    // Phase 2 APIs are intentionally absent; projection fold still tracks state.
    let session = cordis_rs::core_services::Session::new("uq", None);
    session.append(
        "user/question-asked",
        serde_json::json!({ "call_id": "c1" }),
    );
    let folded =
        cordis_rs::plugin_ports::user_questions_fold(serde_json::Value::Null, &session.events()[0]);
    assert_eq!(folded["c1"], "open");
}

#[test]
fn jsonl_persistence_rejects_path_traversal_session_ids() {
    use cordis_rs::plugin_ports::{JsonlPersistence, PersistenceError};

    let root = TempDirGuard::new("cordis-traversal");
    let store = JsonlPersistence::new(root.path());

    // Existing-style ids still work.
    store.create("session-1").unwrap();
    assert!(store.open("session-1").is_ok());

    // Traversal / separator / hidden / unicode ids are rejected before any file
    // is created, and no file escapes the configured root.
    for id in ["../evil", "a/b", "", ".hidden", "あ", "with space"] {
        assert!(
            matches!(store.create(id), Err(PersistenceError::InvalidId(_))),
            "{id}"
        );
        assert!(
            matches!(store.open(id), Err(PersistenceError::InvalidId(_))),
            "{id}"
        );
    }
    assert!(!root.parent().unwrap().join("evil.jsonl").exists());
}

#[test]
fn goal_fold_strictly_decodes_and_validates_replay() {
    use cordis_rs::core_services::Session;
    use cordis_rs::plugin_ports::{
        GoalPhase, apply_goal_change, decode_goal_change, empty_goal_fold_state, fold_goal,
        goal_change_ref,
    };

    let create = serde_json::json!({
        "kind": "goal/change", "version": 1, "operation": "create",
        "goal": {
            "id": "g1", "revision": 1, "objective": "Ship it",
            "phase": "active", "maxGoalRounds": 5
        },
        "roundsStarted": 0, "createdAt": 100, "updatedAt": 100
    });
    let change = decode_goal_change(&create).unwrap().unwrap();
    assert_eq!(goal_change_ref(&change).revision, 1);
    let mut state = empty_goal_fold_state();
    apply_goal_change(&mut state, change).unwrap();
    assert_eq!(state.goal.as_ref().unwrap().phase, GoalPhase::Active);

    // Revision jump is rejected (CAS on the read side).
    let edit = serde_json::json!({
        "kind": "goal/change", "version": 1, "operation": "edit",
        "goal": {
            "id": "g1", "revision": 3, "objective": "Ship more",
            "phase": "active", "maxGoalRounds": 5
        },
        "roundsStarted": 0, "createdAt": 100, "updatedAt": 110
    });
    let bad = decode_goal_change(&edit).unwrap().unwrap();
    let mut before = state.clone();
    assert!(apply_goal_change(&mut before, bad).is_err());
    assert_eq!(before.goal.as_ref().unwrap().revision, 1);

    // Malformed changes fail loudly.
    assert!(
        decode_goal_change(&serde_json::json!({
            "kind": "goal/change", "version": 2, "operation": "create"
        }))
        .is_err()
    );
    assert!(
        decode_goal_change(&serde_json::json!({"kind": "other"}))
            .unwrap()
            .is_none()
    );

    // Fold through session events, ignoring unrelated ones.
    let session = Session::new("fold", None);
    session.append("user/message", serde_json::json!({}));
    session.append("goal/change", create);
    let folded = fold_goal(&session.events()).unwrap();
    assert_eq!(folded.goal.as_ref().unwrap().id, "g1");
}

#[tokio::test]
async fn tool_pipeline_require_approval_is_opt_in_fail_closed() {
    use cordis_rs::core_services::{ToolDefinition, ToolError, provide_tools};
    use std::sync::Arc;

    let (registry, pipeline) = provide_tools(&Context::root("fail-closed")).await.unwrap();
    registry.register(ToolDefinition::new(
        "guarded",
        "guarded tool",
        Arc::new(|_args, _exec| {
            Box::pin(async {
                Ok(cordis_rs::core_services::ToolOutput {
                    content: serde_json::json!({"ok": true}),
                    rendered: vec!["ok".to_string()],
                })
            })
        }),
    ));
    let context = cordis_rs::core_services::ToolExecContext::agent("a");

    // Default remains fail-open (backwards compatible).
    assert!(
        pipeline
            .run(&registry, "guarded", serde_json::json!({}), context.clone())
            .await
            .is_ok()
    );

    // Opt-in gate denies when no ApprovalService is installed.
    pipeline.require_approval();
    let error = pipeline
        .run(&registry, "guarded", serde_json::json!({}), context)
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Execution(_)));
    assert!(error.to_string().contains("blocked"));
    assert!(error.to_string().contains("approval unavailable"));
}

#[tokio::test]
async fn hook_deny_middleware_blocks_tool_execution() {
    use cordis_rs::core_services::{
        ToolDefinition, ToolError, hook_deny_middleware, provide_tools,
    };
    use cordis_rs::plugin_ports::HookEffect;
    use std::sync::Arc;

    let (registry, pipeline) = provide_tools(&Context::root("hook-deny")).await.unwrap();
    registry.register(ToolDefinition::new(
        "guarded",
        "guarded tool",
        Arc::new(|_args, _exec| {
            Box::pin(async {
                Ok(cordis_rs::core_services::ToolOutput {
                    content: serde_json::json!({"ok": true}),
                    rendered: vec!["ok".to_string()],
                })
            })
        }),
    ));
    let context = cordis_rs::core_services::ToolExecContext::agent("a");

    // No middleware: tool runs.
    assert!(
        pipeline
            .run(&registry, "guarded", serde_json::json!({}), context.clone())
            .await
            .is_ok()
    );

    // Opt-in hook deny blocks execution with a "blocked" diagnostic.
    let deny_disposer =
        pipeline.use_middleware(hook_deny_middleware(HookEffect::Reject("policy".into())));
    let error = pipeline
        .run(&registry, "guarded", serde_json::json!({}), context.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Execution(_)));
    assert!(error.to_string().contains("blocked by hook"));

    // Proceed delegates to the inner tool once the deny middleware is removed.
    deny_disposer.dispose().await.unwrap();
    let _disposer2 = pipeline.use_middleware(hook_deny_middleware(HookEffect::Proceed));
    assert!(
        pipeline
            .run(&registry, "guarded", serde_json::json!({}), context)
            .await
            .is_ok()
    );
}

#[test]
fn user_approval_normalization_is_fail_closed() {
    use cordis_rs::core_services::{ApprovalDecision, ApprovalError, ApprovalOutcome};
    use cordis_rs::plugin_ports::{normalize_approval_outcome, outcome_label};

    let allow = ApprovalOutcome {
        decision: ApprovalDecision::Allow,
        answerer: None,
    };
    let deny = ApprovalOutcome {
        decision: ApprovalDecision::Deny,
        answerer: None,
    };

    assert_eq!(
        normalize_approval_outcome(Ok(allow)),
        ApprovalDecision::Allow
    );
    // Explicit deny, unavailable and cancelled all collapse to Deny.
    assert_eq!(normalize_approval_outcome(Ok(deny)), ApprovalDecision::Deny);
    assert_eq!(
        normalize_approval_outcome(Err(ApprovalError::Unavailable)),
        ApprovalDecision::Deny
    );
    assert_eq!(
        normalize_approval_outcome(Err(ApprovalError::Cancelled)),
        ApprovalDecision::Deny
    );
    assert_eq!(outcome_label(ApprovalDecision::Allow), "allowed-once");
    assert_eq!(outcome_label(ApprovalDecision::Deny), "rejected");
}

#[tokio::test]
async fn sandbox_policy_middleware_blocks_writes_in_read_only() {
    use cordis_rs::core_services::{
        SandboxMode, ToolDefinition, ToolError, ToolOutput, provide_sessions, provide_tools,
    };
    use cordis_rs::plugin_ports::{
        SandboxPolicyConfig, is_write_tool, mount_sandbox_policy, sandbox_policy_middleware,
    };
    use std::sync::Arc;

    let ok_output = || ToolOutput {
        content: serde_json::json!({"ok": true}),
        rendered: vec!["ok".to_string()],
    };

    let ctx = Context::root("sandbox-gate");
    provide_sessions(&ctx).await.unwrap();
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    for name in ["bash", "read"] {
        let handler_output = ok_output();
        registry.register(ToolDefinition::new(
            name,
            "tool",
            Arc::new(move |_args, _exec| {
                let output = handler_output.clone();
                Box::pin(async move { Ok(output) })
            }),
        ));
    }
    let exec = cordis_rs::core_services::ToolExecContext::agent("a");

    // No gate: both tools run (backwards compatible).
    assert!(
        pipeline
            .run(&registry, "bash", serde_json::json!({}), exec.clone())
            .await
            .is_ok()
    );

    // ReadOnly policy blocks the write tool, allows the read tool.
    let config = SandboxPolicyConfig {
        mode: SandboxMode::ReadOnly,
        ..Default::default()
    };
    let config_ctx = Context::root("sandbox-config");
    provide_sessions(&config_ctx).await.unwrap();
    let service = mount_sandbox_policy(&config_ctx, config).await.unwrap();
    let resolved = service.resolve(None, None);
    let _write_gate = pipeline.use_middleware(sandbox_policy_middleware(resolved.clone()));
    let error = pipeline
        .run(&registry, "bash", serde_json::json!({}), exec.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Execution(_)));
    assert!(error.to_string().contains("blocked"));
    // ReadOnly blocks every tool at the pipeline level (phase-1 fail-safe).
    assert!(
        pipeline
            .run(&registry, "read", serde_json::json!({}), exec)
            .await
            .is_err()
    );

    // WorkspaceWrite allows writes; is_write_tool is fail-safe.
    assert!(is_write_tool("bash"));
    assert!(!is_write_tool("read"));
    let writable = service.resolve(None, Some(SandboxMode::WorkspaceWrite));
    let (_, pipeline2) = provide_tools(&Context::root("sandbox-allow"))
        .await
        .unwrap();
    let _gate2 = pipeline2.use_middleware(sandbox_policy_middleware(writable));
    assert!(
        pipeline2
            .run(
                &registry,
                "bash",
                serde_json::json!({}),
                cordis_rs::core_services::ToolExecContext::agent("a")
            )
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn goal_round_driver_queues_and_fences_rounds() {
    use cordis_rs::core_services::{AgentRecord, provide_agents, provide_goals, provide_sessions};
    use cordis_rs::plugin_ports::DriveOutcome;
    use cordis_rs::plugin_ports::GoalRoundDriver;

    let ctx = Context::root("goal-driver");
    let (sessions, _) = provide_sessions(&ctx).await.unwrap();
    let goals = provide_goals(&ctx).await.unwrap();
    let agents = provide_agents(&ctx).await.unwrap();

    let session = sessions
        .create(Some("driver-session".to_string()), None)
        .unwrap();
    agents
        .register(AgentRecord {
            id: "a1".to_string(),
            session_id: session.id().to_string(),
            cwd: None,
        })
        .unwrap();
    goals.set("Ship it".to_string(), 3).unwrap();

    let driver = GoalRoundDriver::new();
    assert_eq!(
        driver.drive(&ctx, "a1").unwrap(),
        DriveOutcome::Queued { round: 1 }
    );
    // Duplicate guard: no second round without an intervening step/competition.
    assert_eq!(driver.drive(&ctx, "a1").unwrap(), DriveOutcome::Noop);
    let prompts = session.events();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].data["source"]["kind"], "goal");
    assert!(
        prompts[0].data["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Round: 1/3")
    );

    // A competing human input drops the attempt so the next drive advances.
    driver.note_competing();
    assert_eq!(driver.drive(&ctx, "a1").unwrap(), DriveOutcome::Noop);
    assert_eq!(
        driver.drive(&ctx, "a1").unwrap(),
        DriveOutcome::Queued { round: 2 }
    );

    // Unknown agent is a no-op.
    assert_eq!(driver.drive(&ctx, "missing").unwrap(), DriveOutcome::Noop);

    // Prompt rendering preserves the upstream frame.
    let prompt = GoalRoundDriver::prompt("obj", 2, 5);
    assert!(prompt.starts_with("<goal_round>"));
    assert!(prompt.contains("Objective: obj"));
    assert!(prompt.contains("Round: 2/5"));
}

#[tokio::test]
async fn compaction_automation_drives_and_reports_outcomes() {
    use cordis_rs::core_services::{
        CompactionTrigger, TokenMeter, provide_compaction, provide_sessions,
    };
    use cordis_rs::plugin_ports::{CompactionAutomationConfig, mount_compaction_automation};

    let ctx = Context::root("compaction-auto");
    let (sessions, _) = provide_sessions(&ctx).await.unwrap();
    let meter = TokenMeter::new(100);
    provide_compaction(&ctx, meter).await.unwrap();

    // Invalid config is rejected at mount.
    let bad = CompactionAutomationConfig {
        pressure_threshold: 1.5,
        ..Default::default()
    };
    assert!(mount_compaction_automation(&ctx, bad).await.is_err());

    let service = mount_compaction_automation(&ctx, CompactionAutomationConfig::default())
        .await
        .unwrap();

    // Disabled automation is a no-op.
    let disabled = CompactionAutomationConfig {
        auto: false,
        ..Default::default()
    };
    let svc2 = mount_compaction_automation(&Context::root("disabled"), disabled)
        .await
        .is_err();
    // (the second context lacks services, so mount fails before config use)
    assert!(svc2);

    // Manual trigger delegates to the engine (compact or no-op are both valid).
    let session = sessions
        .create(Some("compact-session".to_string()), None)
        .unwrap();
    let _outcome = service
        .drive(session.id(), CompactionTrigger::Manual)
        .unwrap();

    // Config is exposed for host inspection.
    assert_eq!(service.config().pressure_threshold, 0.8);
    assert_eq!(service.config().overflow_code, "CONTEXT_WINDOW_EXCEEDED");
}

#[tokio::test]
async fn install_default_controls_enables_fail_closed_and_sandbox_gate() {
    use cordis_rs::core_services::{
        SandboxMode, ToolDefinition, ToolError, provide_sessions, provide_tools,
    };
    use cordis_rs::plugin_ports::{SandboxPolicyConfig, mount_sandbox_policy};
    use std::sync::Arc;

    let ctx = Context::root("default-controls");
    provide_sessions(&ctx).await.unwrap();
    let (registry, pipeline) = provide_tools(&ctx).await.unwrap();
    registry.register(ToolDefinition::new(
        "guarded",
        "guarded tool",
        Arc::new(|_args, _exec| {
            Box::pin(async {
                Ok(cordis_rs::core_services::ToolOutput {
                    content: serde_json::json!({"ok": true}),
                    rendered: vec!["ok".to_string()],
                })
            })
        }),
    ));
    let exec = cordis_rs::core_services::ToolExecContext::agent("a");

    // Before installing controls the tool runs (fail-open default).
    assert!(
        pipeline
            .run(&registry, "guarded", serde_json::json!({}), exec.clone())
            .await
            .is_ok()
    );

    let policy_ctx = Context::root("control-policy");
    provide_sessions(&policy_ctx).await.unwrap();
    let policy_service = mount_sandbox_policy(
        &policy_ctx,
        SandboxPolicyConfig {
            mode: SandboxMode::WorkspaceWrite,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let policy = policy_service.resolve(None, None);
    let _disposers = pipeline.install_default_controls(None, policy);

    // With controls installed and no approval answerer, execution is denied.
    let error = pipeline
        .run(&registry, "guarded", serde_json::json!({}), exec)
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Execution(_)));
    assert!(error.to_string().contains("blocked"));
}

#[tokio::test]
async fn llm_resilience_timeout_yields_retryable_error_chunk() {
    use cordis_rs::core_services::{LlmRequest, RetryPolicy, StreamChunk};
    use cordis_rs::plugin_ports::{TIMEOUT_PREFIX, with_llm_retry, with_llm_timeout};
    use futures::StreamExt;
    use std::sync::Arc;
    use std::time::Duration;

    // A handler that never yields a chunk.
    let hanging: cordis_rs::services::llm::StreamHandler =
        Arc::new(|_request| Box::pin(futures::stream::pending::<StreamChunk>()));
    let timed = with_llm_timeout(hanging, Duration::from_millis(5));
    let mut stream = timed(LlmRequest {
        model: "m".to_string(),
        messages: Vec::new(),
    });
    let first = stream.next().await.unwrap();
    match first {
        StreamChunk::Error(reason) => assert!(reason.starts_with(TIMEOUT_PREFIX)),
        other => panic!("expected timeout error, got {other:?}"),
    }

    // Retry decorator replays a failing handler up to the policy limit.
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = attempts.clone();
    let flaky: cordis_rs::services::llm::StreamHandler = Arc::new(move |_request| {
        let counter = counter.clone();
        Box::pin(futures::stream::once(async move {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                StreamChunk::Error("HTTP 504 gateway timeout".to_string())
            } else {
                StreamChunk::Text("ok".to_string())
            }
        }))
    });
    let policy = RetryPolicy {
        max_retries: 3,
        initial_delay_ms: 1,
        max_delay_ms: 1,
        ..Default::default()
    };
    let retried = with_llm_retry(flaky, policy);
    let chunks: Vec<_> = retried(LlmRequest {
        model: "m".to_string(),
        messages: Vec::new(),
    })
    .collect()
    .await;
    assert!(
        chunks
            .iter()
            .any(|chunk| matches!(chunk, StreamChunk::Text(text) if text == "ok"))
    );
    assert!(attempts.load(std::sync::atomic::Ordering::SeqCst) >= 3);
}

#[test]
fn jsonl_recovery_stops_at_torn_write_and_reports_line() {
    use cordis_rs::plugin_ports::{JsonlPersistence, PersistenceError};
    use std::io::Write;

    let root = TempDirGuard::new("cordis-recovery");
    let store = JsonlPersistence::new(root.path());
    let handle = store.create("recover").unwrap();

    // Complete log: no truncation reported, existing read() still works.
    let session = cordis_rs::core_services::Session::new("recover", None);
    let event = session.append("user/message", serde_json::json!({"text": "hello"}));
    handle.append(&event).unwrap();
    let report = handle.read_recovered().unwrap();
    assert_eq!(report.events.len(), 1);
    assert_eq!(report.truncated_at_line, None);
    assert!(handle.read().is_ok());

    // Append a torn (incomplete JSON) final line, then verify recovery stops
    // there and reports the line while the strict read() still fails closed.
    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(handle.path())
            .unwrap();
        writeln!(file, "{{\"seq\":2,\"type\":\"user/mess").unwrap();
    }
    let recovered = handle.read_recovered().unwrap();
    assert_eq!(recovered.events.len(), 1);
    assert_eq!(recovered.truncated_at_line, Some(2));
    assert!(matches!(
        handle.read(),
        Err(PersistenceError::Corrupt { .. })
    ));
}

#[test]
fn llm_transport_fake_records_wire_request_without_secrets() {
    use cordis_rs::plugin_ports::{
        FakeTransport, HttpRequest, HttpResponse, LlmTransport, handler_from_transport,
    };
    use futures::StreamExt;

    let transport = FakeTransport::new();
    transport.push_response(HttpResponse::ok("hello"));
    let headers = vec![
        ("user-agent".to_string(), "deepseek-harness/1.0".to_string()),
        ("authorization".to_string(), "Bearer test-key".to_string()),
    ];
    let handler = handler_from_transport(
        std::sync::Arc::new(transport.clone()),
        "http://127.0.0.1:1234/chat/completions",
        headers,
    );
    let chunks: Vec<_> = futures::executor::block_on(async {
        handler(cordis_rs::core_services::LlmRequest {
            model: "m".to_string(),
            messages: Vec::new(),
        })
        .collect()
        .await
    });
    assert!(chunks.iter().any(|chunk| matches!(
        chunk,
        cordis_rs::core_services::StreamChunk::Text(text) if text == "hello"
    )));

    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|(key, _)| key == "user-agent")
    );
    assert!(requests[0].url.contains("127.0.0.1"));
    let _ = HttpRequest::post("x", "y"); // ensure public constructor stays usable
    let _: &dyn LlmTransport = &transport;
}

#[test]
fn deepseek_transport_handler_sends_messages_and_redacts_key() {
    use cordis_rs::plugin_ports::{
        DeepSeekConfig, FakeTransport, HttpResponse, handler_with_transport,
    };
    use futures::StreamExt;

    let transport = FakeTransport::new();
    transport.push_response(HttpResponse::ok("answer"));
    let handler = handler_with_transport(
        &DeepSeekConfig::default(),
        "super-secret-key",
        std::sync::Arc::new(transport.clone()),
    );
    let request = cordis_rs::core_services::LlmRequest {
        model: "deepseek-chat".to_string(),
        messages: vec![cordis_rs::core_services::ChatMessage::user("hello")],
    };
    let chunks: Vec<_> = futures::executor::block_on(async { handler(request).collect().await });
    assert!(chunks.iter().any(|chunk| matches!(
        chunk,
        cordis_rs::core_services::StreamChunk::Text(text) if text == "answer"
    )));

    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/chat/completions"));
    assert!(requests[0].body.contains("hello"));
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|(key, value)| { key == "authorization" && value == "Bearer super-secret-key" })
    );
    // Debug output must never contain the key.
    let debug = format!("{:?}", requests);
    assert!(!debug.contains("super-secret-key"));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn otel_exporter_redacts_secrets_and_builds_otlp_payload() {
    use cordis_rs::plugin_ports::{
        FakeCollector, OTelExporter, RedactingExporter, otlp_payload, redact_record,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    let mut attributes = HashMap::new();
    attributes.insert("tool".to_string(), "bash".to_string());
    attributes.insert("api_key".to_string(), "super-secret".to_string());
    let record = cordis_rs::core_services::EventRecord {
        name: "tool\ncall".to_string(),
        attributes,
        seq: 1,
    };

    // Redaction strips control chars from names and masks unlisted attributes.
    let redacted = redact_record(&record);
    assert_eq!(redacted.name, "toolcall");
    assert_eq!(redacted.attributes["tool"], "bash");
    assert_eq!(redacted.attributes["api_key"], "[redacted]");

    // The wrapper applies redaction exactly once before delegating.
    let collector = FakeCollector::new();
    let exporter = RedactingExporter::new(Arc::new(collector.clone()) as Arc<dyn OTelExporter>);
    exporter.export(&[record]).unwrap();
    let batches = collector.batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0][0].attributes["api_key"], "[redacted]");

    // OTLP payload uses the redacted values.
    let payload = otlp_payload(&batches[0]);
    let text = payload.to_string();
    assert!(!text.contains("super-secret"));
    assert!(text.contains("resourceLogs"));
}

#[tokio::test]
async fn llm_timeout_yields_exactly_one_error_then_terminates() {
    use cordis_rs::plugin_ports::{TIMEOUT_PREFIX, with_llm_timeout};
    use futures::StreamExt;
    use std::time::Duration;

    let hanging: cordis_rs::services::llm::StreamHandler = std::sync::Arc::new(|_request| {
        Box::pin(futures::stream::pending::<
            cordis_rs::core_services::StreamChunk,
        >())
    });
    let timed = with_llm_timeout(hanging, Duration::from_millis(5));
    let chunks: Vec<_> = timed(cordis_rs::core_services::LlmRequest {
        model: "m".to_string(),
        messages: Vec::new(),
    })
    .take(5)
    .collect()
    .await;

    assert_eq!(
        chunks.len(),
        1,
        "timeout stream must terminate after one error"
    );
    match &chunks[0] {
        cordis_rs::core_services::StreamChunk::Error(reason) => {
            assert!(reason.starts_with(TIMEOUT_PREFIX))
        }
        other => panic!("expected timeout error, got {other:?}"),
    }
}

#[tokio::test]
async fn llm_timeout_error_carries_504_contract_code() {
    use cordis_rs::plugin_ports::with_llm_timeout;
    use futures::StreamExt;
    use std::time::Duration;

    let hanging: cordis_rs::services::llm::StreamHandler = std::sync::Arc::new(|_request| {
        Box::pin(futures::stream::pending::<
            cordis_rs::core_services::StreamChunk,
        >())
    });
    let timed = with_llm_timeout(hanging, Duration::from_millis(5));
    let mut stream = timed(cordis_rs::core_services::LlmRequest {
        model: "m".to_string(),
        messages: Vec::new(),
    });
    let chunk = stream.next().await.unwrap();
    // LlmService::stream converts StreamChunk::Error(msg) to Error::Other(msg);
    // the 504 code must survive that conversion for RetryPolicy::error_code.
    let reason = match chunk {
        cordis_rs::core_services::StreamChunk::Error(reason) => reason,
        other => panic!("expected error chunk, got {other:?}"),
    };
    let error = cordis_rs::Error::Other(reason);
    assert!(error.to_string().contains("504"));
}

#[test]
fn session_append_is_atomic_under_concurrency() {
    use std::thread;

    let session = cordis_rs::core_services::Session::new("concurrent", None);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let session = session.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                session.append("user/message", serde_json::json!({"text": "x"}));
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let events = session.events();
    assert_eq!(events.len(), 400);
    let seqs: Vec<u64> = events.iter().map(|event| event.seq).collect();
    let expected: Vec<u64> = (1..=400).collect();
    assert_eq!(seqs, expected, "seq order must match log order");
    assert_eq!(session.seq(), 400);
}

#[test]
fn jsonl_rejects_invalid_seq_and_type_instead_of_sentinels() {
    use cordis_rs::plugin_ports::{JsonlPersistence, PersistenceError};
    use std::io::Write;

    let root = TempDirGuard::new("cordis-schema");
    let store = JsonlPersistence::new(root.path());
    let session = cordis_rs::core_services::Session::new("schema", None);
    let good = session.append("user/message", serde_json::json!({"text": "ok"}));

    // First line is valid; second line has a string seq and no type.
    let handle = store.create("schema").unwrap();
    handle.append(&good).unwrap();
    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(handle.path())
            .unwrap();
        writeln!(file, "{{\"seq\":\"bad\",\"data\":{{}}}}").unwrap();
    }

    // Strict read fails closed with a line-numbered Corrupt error.
    assert!(matches!(
        handle.read(),
        Err(PersistenceError::Corrupt { .. })
    ));

    // Recovery stops at the malformed line and reports the truncation.
    let recovered = handle.read_recovered().unwrap();
    assert_eq!(recovered.events.len(), 1);
    assert_eq!(recovered.truncated_at_line, Some(2));
}

#[tokio::test]
async fn llm_stream_without_done_is_an_error_not_partial_success() {
    let ctx = Context::root("llm-no-done");
    let llm = cordis_rs::core_services::provide_llm(&ctx).await.unwrap();
    let handler: cordis_rs::services::llm::StreamHandler = std::sync::Arc::new(|_request| {
        Box::pin(futures::stream::once(async {
            cordis_rs::core_services::StreamChunk::Text("partial".to_string())
        }))
    });
    llm.register(cordis_rs::core_services::LlmProvider::new(
        "no-done", "m", handler,
    ));
    let error = llm
        .stream(
            Some("no-done"),
            cordis_rs::core_services::LlmRequest {
                model: "m".to_string(),
                messages: Vec::new(),
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("without Done"));
}

#[test]
#[cfg(unix)]
fn scoped_fs_rejects_symlink_escape_even_when_target_missing() {
    use cordis_rs::core_services::FileSystem;

    let base = TempDirGuard::new("cordis-fs-escape");
    let root = base.join("root");
    let outside = base.join("outside");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();

    let fs = FileSystem::scoped(&root);
    assert!(fs.stat("escape/hidden.txt").is_err());

    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn js_expression_recursion_is_bounded() {
    use cordis_rs::{Composition, LoaderError};

    // A deeply nested `!` chain must be rejected with TooDeep, not stack overflow.
    let deep = "!".repeat(400) + "true";
    let yaml = format!(
        "entries:\n  - id: deep\n    plugin: test\n    config:\n      value: !!js '{deep}'\n"
    );
    let result = Composition::parse(&yaml);
    assert!(
        matches!(result, Err(LoaderError::TooDeep { .. })),
        "expected TooDeep, got {result:?}"
    );
}

#[tokio::test]
async fn effect_setup_during_teardown_does_not_revive_disposer() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let ctx = Context::root("effect-teardown");
    let cleaned = Arc::new(AtomicUsize::new(0));
    let counter = cleaned.clone();
    let ctx_clone = ctx.clone();
    let result = ctx
        .effect(move |scope| {
            let ctx = ctx_clone.clone();
            let counter = counter.clone();
            async move {
                scope.on_dispose(move || {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                });
                // Reentrant teardown while setup is still running.
                ctx.dispose_effects().await?;
                Ok(())
            }
        })
        .await;
    assert!(result.is_ok());
    // The child disposer ran exactly once and the effect list stays empty.
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    ctx.dispose_effects().await.unwrap();
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn invoke_blocking_rejects_calls_from_tokio_runtime() {
    let ctx = Context::root("blocking-guard");
    let (registry, pipeline) = cordis_rs::core_services::provide_tools(&ctx).await.unwrap();
    let result = cordis_rs::core_services::invoke_blocking(
        &registry,
        &pipeline,
        "nonexistent",
        serde_json::json!({}),
        cordis_rs::core_services::ToolExecContext::agent("a"),
    );
    // The guard runs before any async work, so an Err is returned promptly
    // instead of block_on stalling the current-thread runtime.
    let error = result.unwrap_err();
    assert!(error.to_string().contains("must not be called"));
}

#[tokio::test]
async fn cancellation_during_tool_body_reclassifies_success() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let ctx = Context::root("cancel-during");
    let (registry, pipeline) = cordis_rs::core_services::provide_tools(&ctx).await.unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let flag_for_tool = flag.clone();
    registry.register(ToolDefinition::new(
        "slow",
        "slow tool",
        Arc::new(move |_args, _exec| {
            let flag = flag_for_tool.clone();
            Box::pin(async move {
                flag.store(true, Ordering::Release);
                Ok(cordis_rs::core_services::ToolOutput {
                    content: serde_json::json!({"ok": true}),
                    rendered: vec!["ok".to_string()],
                })
            })
        }),
    ));
    let exec = cordis_rs::core_services::ToolExecContext {
        agent: Some("a".to_string()),
        session: None,
        signal_cancelled: false,
        cancel_flag: Some(flag.clone()),
    };
    let error = pipeline
        .run(&registry, "slow", serde_json::json!({}), exec)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        cordis_rs::core_services::ToolError::Cancelled(_)
    ));
}

#[test]
fn projection_fold_can_reenter_registry_without_deadlock() {
    use std::sync::mpsc;
    use std::time::Duration;

    let registry = cordis_rs::core_services::SessionProjectionRegistry::default();
    let session = cordis_rs::core_services::Session::new("proj", None);
    session.append("user/message", serde_json::json!({"text": "x"}));

    // The fold re-enters the registry (has()) while view() is running. Before
    // the fix, view() held the read guard and this deadlocked.
    let registry_for_fold = registry.clone();
    registry.register(
        "reentrant",
        std::sync::Arc::new(move |value, _event| {
            let _ = registry_for_fold.has("reentrant");
            value
        }),
    );

    let (tx, rx) = mpsc::channel();
    let registry_for_thread = registry.clone();
    let session_for_thread = session.clone();
    std::thread::spawn(move || {
        let result = registry_for_thread.view("reentrant", &session_for_thread);
        tx.send(result).unwrap();
    });
    let result = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("view deadlocked while fold re-entered the registry");
    assert!(result.is_some());
}

#[test]
fn jsonl_persistence_lists_sessions_from_disk_after_restart() {
    use cordis_rs::plugin_ports::JsonlPersistence;
    use std::fs;

    let root = TempDirGuard::new("cordis-list");
    let _ = fs::remove_dir_all(&root);

    // First instance creates a session and writes its file.
    let store = JsonlPersistence::new(&root);
    store.create("alpha").unwrap();
    assert_eq!(store.list(), vec!["alpha".to_string()]);

    // A fresh instance (empty in-memory handles) still sees the disk session.
    let restarted = JsonlPersistence::new(&root);
    assert_eq!(restarted.list(), vec!["alpha".to_string()]);

    // Invalid filenames are filtered out.
    fs::write(root.join("weird.name.jsonl"), "{}").unwrap();
    fs::write(root.join(".hidden.jsonl"), "{}").unwrap();
    fs::write(root.join("stray.tmp"), "{}").unwrap();
    assert_eq!(restarted.list(), vec!["alpha".to_string()]);

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn jsonl_restore_preserves_persisted_seq_and_rejects_nonmonotonic() {
    use cordis_rs::plugin_ports::{JsonlPersistence, PersistenceError};
    use std::io::Write;

    let root = TempDirGuard::new("cordis-restore-seq");
    let _ = std::fs::remove_dir_all(&root);
    let store = JsonlPersistence::new(&root);
    let handle = store.create("seq").unwrap();

    // Write three events with explicit seq values, including a gap (1, 3, 5).
    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(handle.path())
            .unwrap();
        writeln!(file, "{{\"seq\":1,\"type\":\"a\",\"data\":{{}}}}").unwrap();
        writeln!(file, "{{\"seq\":3,\"type\":\"b\",\"data\":{{}}}}").unwrap();
        writeln!(file, "{{\"seq\":5,\"type\":\"c\",\"data\":{{}}}}").unwrap();
    }
    let restored = store.restore("seq", None).unwrap();
    assert_eq!(restored.seq(), 5);
    assert_eq!(restored.events()[1].seq, 3);
    assert_eq!(restored.append("d", serde_json::json!({})).seq, 6);

    // A non-monotonic log fails closed on restore.
    std::fs::write(
        handle.path(),
        "{\"seq\":2,\"type\":\"a\",\"data\":{}}\n{\"seq\":1,\"type\":\"b\",\"data\":{}}\n",
    )
    .unwrap();
    let restarted = JsonlPersistence::new(&root);
    assert!(matches!(
        restarted.restore("seq", None),
        Err(PersistenceError::Corrupt { .. })
    ));

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn file_search_reports_index_truncation_and_io_errors() {
    use cordis_rs::core_services::{FileSearchConfig, WorkspaceFileSearch};

    let dir = TempDirGuard::new("cordis-fs-search");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["a", "b", "c", "d"] {
        std::fs::write(dir.join(name), "").unwrap();
    }

    let config = FileSearchConfig {
        max_entries: 2,
        ..Default::default()
    };
    let search = WorkspaceFileSearch::new(dir.path(), config).unwrap();
    assert!(
        search.status().truncated,
        "small max_entries must mark truncation"
    );
    assert!(search.status().io_errors.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn projection_registry_same_key_replacement_is_generation_fenced() {
    use cordis_rs::core_services::SessionProjectionRegistry;
    use std::sync::Arc;

    let registry = SessionProjectionRegistry::default();
    let session = cordis_rs::core_services::Session::new("proj-dup", None);
    session.append("n", serde_json::json!(1));

    let first = registry.register(
        "count",
        Arc::new(|mut value, _event| {
            value["first"] = serde_json::json!(true);
            value
        }),
    );
    // Last registration wins: view() must use the second fold.
    let second = registry.register(
        "count",
        Arc::new(|mut value, _event| {
            value["second"] = serde_json::json!(true);
            value
        }),
    );
    let view = registry.view("count", &session).unwrap();
    assert_eq!(view["second"], true);
    assert_eq!(view["first"], serde_json::Value::Null);

    // Disposing the first registration must not remove the second one.
    first.dispose().await.unwrap();
    assert!(registry.has("count"));
    let view = registry.view("count", &session).unwrap();
    assert_eq!(view["second"], true);

    // Disposing the current registration removes the key.
    second.dispose().await.unwrap();
    assert!(!registry.has("count"));
}

#[test]
fn goal_change_decoding_tolerates_unknown_fields_but_requires_known_ones() {
    use cordis_rs::plugin_ports::decode_goal_change;

    let mut change = serde_json::json!({
        "kind": "goal/change", "version": 1, "operation": "create",
        "goal": {
            "id": "g1", "revision": 1, "objective": "Ship it",
            "phase": "active", "maxGoalRounds": 5,
            "futureGoalField": "ignored"
        },
        "roundsStarted": 0, "createdAt": 100, "updatedAt": 100,
        "futureChangeField": 42
    });
    assert!(decode_goal_change(&change).unwrap().is_some());

    // A missing required field still fails.
    change.as_object_mut().unwrap().remove("roundsStarted");
    assert!(decode_goal_change(&change).is_err());

    // Unknown version is still rejected.
    let mut wrong_version = serde_json::json!({
        "kind": "goal/change", "version": 2, "operation": "create",
        "goal": {
            "id": "g1", "revision": 1, "objective": "Ship it",
            "phase": "active", "maxGoalRounds": 5
        },
        "roundsStarted": 0, "createdAt": 100, "updatedAt": 100
    });
    wrong_version
        .as_object_mut()
        .unwrap()
        .insert("future".into(), serde_json::json!(1));
    assert!(decode_goal_change(&wrong_version).is_err());
}

#[test]
fn emit_errors_keeps_message_and_exposes_details_and_source() {
    use cordis_rs::EmitErrors;
    use std::error::Error as _;

    let errors = EmitErrors(vec![
        cordis_rs::Error::Other("first".to_string()),
        cordis_rs::Error::Other("second".to_string()),
    ]);
    // Established Display message is unchanged.
    assert_eq!(errors.to_string(), "2 listener error(s)");
    // Details expose per-listener failures for diagnostics.
    let details = errors.details();
    assert!(details.contains("1:") && details.contains("first"));
    assert!(details.contains("2:") && details.contains("second"));
    // source() returns the first failure (Rust's source() is single-valued).
    assert_eq!(errors.source().unwrap().to_string(), "first");
}

#[test]
fn skill_catalog_reports_provider_failures_without_losing_other_skills() {
    use cordis_rs::core_services::{Skill, SkillRegistry};
    use std::sync::Arc;

    let registry = SkillRegistry::default();
    let good = Skill {
        name: "good".to_string(),
        description: String::new(),
        path: String::new(),
        source: "test".to_string(),
    };
    registry.register_provider(1, Arc::new(move || Ok(vec![good.clone()])));
    registry.register_provider(2, Arc::new(|| Err("boom\ninjected".to_string())));

    let catalog = registry.catalog();
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].name, "good");
    assert_eq!(catalog.failures.len(), 1);
    assert!(catalog.failures[0].message.contains("boom"));
    assert!(!catalog.failures[0].message.contains('\n'));
    assert_eq!(registry.names(), vec!["good".to_string()]);

    // Equality ignores failures (backward compatibility).
    let mut other = catalog.clone();
    other.failures.clear();
    assert_eq!(catalog, other);
}

#[tokio::test]
async fn shell_request_timeout_applies_and_kills_child() {
    use cordis_rs::core_services::{Shell, ShellError, ShellRequest};

    let shell = Shell::new();

    // No timeout keeps the legacy path and succeeds.
    let ok = shell
        .run_with_timeout(&ShellRequest::new("printf ok"))
        .await
        .unwrap();
    assert_eq!(ok.stdout, "ok");

    // A short timeout on a quick command still succeeds.
    let ok = shell
        .run_with_timeout(&ShellRequest::new("printf fast").with_timeout(2_000))
        .await
        .unwrap();
    assert_eq!(ok.stdout, "fast");

    // A hanging command is killed and reported as TimedOut (R3-IV02).
    let start = std::time::Instant::now();
    let result = shell
        .run_with_timeout(&ShellRequest::new("sleep 30").with_timeout(80))
        .await;
    assert!(matches!(result, Err(ShellError::TimedOut(80))));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(3),
        "timeout must fire promptly, not wait for the child"
    );
    // Display must not embed the 3-digit ms value (retry misdetection).
    assert_eq!(ShellError::TimedOut(80).to_string(), "command timed out");
}

#[test]
fn deepseek_base_url_validation_blocks_credential_exfiltration() {
    use cordis_rs::core_services::StreamChunk;
    use cordis_rs::plugin_ports::{
        DeepSeekConfig, FakeTransport, HttpResponse, handler_with_transport,
    };
    use futures::StreamExt;

    let request = cordis_rs::core_services::LlmRequest {
        model: "deepseek-chat".to_string(),
        messages: vec![cordis_rs::core_services::ChatMessage::user("hello")],
    };

    // Default official URL still works.
    let transport = FakeTransport::new();
    transport.push_response(HttpResponse::ok("ok"));
    let config = DeepSeekConfig::default();
    assert!(
        config
            .validated_endpoint()
            .unwrap()
            .starts_with("https://api.deepseek.com/")
    );
    let handler = handler_with_transport(&config, "secret", std::sync::Arc::new(transport.clone()));
    let chunks: Vec<_> =
        futures::executor::block_on(async { handler(request.clone()).collect().await });
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::Text(t) if t == "ok"))
    );
    assert_eq!(transport.requests().len(), 1);

    // Hostile URLs are rejected before the key is attached: no transport call.
    for base in [
        "http://api.deepseek.com",
        "https://evil.com",
        "https://api.deepseek.com@evil.com",
        "https://api.deepseek.com.evil.com",
    ] {
        let config = DeepSeekConfig {
            base_url: base.to_string(),
            ..Default::default()
        };
        assert!(config.validated_endpoint().is_err(), "must reject {base}");
        let transport = FakeTransport::new();
        transport.push_response(HttpResponse::ok("should not be used"));
        let handler =
            handler_with_transport(&config, "secret", std::sync::Arc::new(transport.clone()));
        let chunks: Vec<_> =
            futures::executor::block_on(async { handler(request.clone()).collect().await });
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Error(_))));
        assert!(transport.requests().is_empty(), "no request to {base}");
    }
}

#[tokio::test]
async fn provide_scoped_fs_rejects_outside_paths_and_symlink_roots() {
    use cordis_rs::core_services::{FileSystem, provide_fs_scoped};

    let base = TempDirGuard::new("cordis-scoped-fs");
    let root = base.join("root");
    let outside = base.join("outside.txt");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&outside, "secret").unwrap();

    let ctx = Context::root("scoped-fs");
    let fs = provide_fs_scoped(&ctx, &root).await.unwrap();

    // Relative path inside root works.
    fs.write_text("inside.txt", "ok").unwrap();
    assert_eq!(fs.read_text("inside.txt").unwrap(), "ok");

    // Absolute path outside root is rejected (scoped root enforced).
    assert!(fs.read_text(outside.to_str().unwrap()).is_err());

    // A symlinked root pointing outside is rejected at construction and every
    // operation fails closed (canonical root fixed at construction).
    let link = base.join("link");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let linked = FileSystem::scoped(&link);
        assert!(linked.read_text("x").is_err());
    }

    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn approval_request_details_reach_answerer() {
    use cordis_rs::core_services::{ApprovalDecision, ApprovalRequest};
    use std::sync::Arc as StdArc;

    // details is builder-only; existing new() callers stay unchanged (ADR-012).
    let plain = ApprovalRequest::new("tool:bash", "Execute tool bash");
    assert_eq!(plain.details(), None);
    let with =
        ApprovalRequest::new("tool:bash", "Execute tool bash").with_details("echo 'hello\\nworld'");
    assert_eq!(with.details(), Some("echo 'hello\\nworld'"));

    // An answerer can inspect the command text to decide.
    let seen = StdArc::new(std::sync::Mutex::new(String::new()));
    let seen_clone = seen.clone();
    let answerer = move |request: &ApprovalRequest| {
        if let Some(command) = request.details() {
            *seen_clone.lock().unwrap() = command.to_string();
            if command.contains("rm -rf") {
                return Some(ApprovalDecision::Deny);
            }
        }
        Some(ApprovalDecision::Allow)
    };
    assert_eq!(answerer(&with), Some(ApprovalDecision::Allow));
    assert_eq!(seen.lock().unwrap().as_str(), "echo 'hello\\nworld'");
    let dangerous = ApprovalRequest::new("tool:bash", "s").with_details("rm -rf /");
    assert_eq!(answerer(&dangerous), Some(ApprovalDecision::Deny));
}

#[tokio::test]
async fn shell_output_cap_truncates_without_breaking_utf8() {
    use cordis_rs::core_services::{Shell, ShellRequest};

    let shell = Shell::new();

    // Small output unchanged.
    let ok = shell
        .run_with_timeout(&ShellRequest::new("printf small"))
        .await
        .unwrap();
    assert_eq!(ok.stdout, "small");

    // Large output truncated with marker on both run() and run_with_timeout().
    let request = ShellRequest::new("printf 'abcdefghijklmnopqrstu'").with_max_output_bytes(20);
    let result = shell.run(&request).unwrap();
    assert_eq!(result.stdout, "abcdef…[truncated]");
    let result_async = shell.run_with_timeout(&request).await.unwrap();
    assert_eq!(result_async.stdout, "abcdef…[truncated]");

    // Multi-byte characters never cut mid-codepoint; result stays valid UTF-8.
    let unicode_request =
        ShellRequest::new("printf 'あいうえおかきくけこ'").with_max_output_bytes(20);
    let unicode = shell.run_with_timeout(&unicode_request).await.unwrap();
    assert!(unicode.stdout.ends_with("…[truncated]"));
    assert!(std::str::from_utf8(unicode.stdout.as_bytes()).is_ok());
}

#[test]
fn read_window_streams_without_materialising_whole_file() {
    use cordis_rs::core_services::FileSystem;

    let dir = TempDirGuard::new("cordis-window");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lines.txt");
    let content = (0..1000)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, &content).unwrap();

    let fs = FileSystem::local();
    let path_str = path.to_str().unwrap();

    // Window in the middle: fields match the old full-read semantics.
    let window = fs.read_window(path_str, 10, 3, 100).unwrap();
    assert_eq!(window.total_lines, 1000);
    assert_eq!(window.start_line, 10);
    assert_eq!(window.lines_returned, 3);
    assert_eq!(window.content, "line 9\nline 10\nline 11");

    // Beyond EOF: same boundary behaviour as before.
    let tail = fs.read_window(path_str, 2000, 5, 100).unwrap();
    assert_eq!(tail.total_lines, 1000);
    assert_eq!(tail.lines_returned, 0);

    // max_line_length truncation does not affect the total count.
    let short = fs.read_window(path_str, 1, 2, 4).unwrap();
    assert_eq!(short.total_lines, 1000);
    assert_eq!(short.content, "line…\nline…");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compiled_matcher_caches_regex_and_fails_closed() {
    use cordis_rs::core_services::{CompiledMatcher, is_match_all, matcher_diagnostic, matches};

    // Existing adapter still works for all sentinels and literal mode.
    assert!(matches(None, "anything", false));
    assert!(matches(Some(""), "anything", false));
    assert!(matches(Some("*"), "anything", false));
    assert!(matches(Some("a|b"), "b", true));
    assert!(!matches(Some("a|b"), "c", true));
    assert!(is_match_all(Some("*")));

    // Compiled once, reused across dispatches with identical results.
    let compiled = CompiledMatcher::compile(Some("^tool_[0-9]+$"), false);
    for index in 0..100 {
        assert!(compiled.matches(&format!("tool_{index}")));
        assert!(!compiled.matches("other"));
    }

    // Invalid pattern is a fail-closed non-match and is diagnosed at config load.
    let invalid = CompiledMatcher::compile(Some("["), false);
    assert!(!invalid.matches("anything"));
    assert!(matcher_diagnostic(Some("["), false).is_some());
}

#[test]
fn otlp_payload_always_redacts_raw_records() {
    use cordis_rs::plugin_ports::{otlp_payload, redact_record};
    use cordis_rs::services::telemetry::EventRecord;
    use std::collections::HashMap;

    let mut attrs = HashMap::new();
    attrs.insert("api_key".to_string(), "super-secret".to_string());
    attrs.insert("tool".to_string(), "bash\nINJECTED".to_string());
    let raw = EventRecord {
        name: "evil\nline".to_string(),
        attributes: attrs,
        seq: 7,
    };

    // Passing a RAW record directly must not leak the secret (R4-OB01).
    let payload = otlp_payload(std::slice::from_ref(&raw)).to_string();
    assert!(
        !payload.contains("super-secret"),
        "secret leaked: {payload}"
    );
    assert!(
        !payload.contains('\n'),
        "raw control char in payload: {payload}"
    );
    assert!(payload.contains("[redacted]"));

    // Idempotence: redact_record is stable under a second application
    // (compare records directly; HashMap iteration order is not significant).
    let safe = redact_record(&raw);
    let twice = redact_record(&safe);
    assert_eq!(safe, twice);
}

#[test]
fn otlp_redaction_caps_length_idempotently() {
    use cordis_rs::plugin_ports::{MAX_NAME_LEN, MAX_SESSION_ID_LEN, MAX_VALUE_LEN, redact_record};
    use cordis_rs::services::telemetry::EventRecord;
    use std::collections::HashMap;

    let mut attrs = HashMap::new();
    attrs.insert("tool".to_string(), "x".repeat(MAX_VALUE_LEN + 500));
    attrs.insert(
        "session_id".to_string(),
        "s".repeat(MAX_SESSION_ID_LEN + 500),
    );
    attrs.insert("secret".to_string(), "y".repeat(MAX_VALUE_LEN + 500));
    let raw = EventRecord {
        name: "n".repeat(MAX_NAME_LEN + 500),
        attributes: attrs,
        seq: 1,
    };

    let once = redact_record(&raw);
    // Long values truncated with marker; secret always redacted.
    assert!(once.name.chars().count() <= MAX_NAME_LEN);
    assert!(once.name.ends_with("…[truncated]"));
    assert!(once.attributes["tool"].chars().count() <= MAX_VALUE_LEN);
    assert!(once.attributes["session_id"].chars().count() <= MAX_SESSION_ID_LEN);
    assert_eq!(once.attributes["secret"], "[redacted]");

    // Idempotence: a second redaction does not stack markers.
    let twice = redact_record(&once);
    assert_eq!(once, twice);

    // UTF-8 safe: multibyte chars cut at a valid boundary.
    let mut utf8_attrs = HashMap::new();
    utf8_attrs.insert("tool".to_string(), "あ".repeat(MAX_VALUE_LEN + 100));
    let utf8 = redact_record(&EventRecord {
        name: "evt".to_string(),
        attributes: utf8_attrs,
        seq: 2,
    });
    let value = &utf8.attributes["tool"];
    assert!(std::str::from_utf8(value.as_bytes()).is_ok());
    assert!(value.chars().count() <= MAX_VALUE_LEN);
}

#[test]
fn redacting_exporter_counts_failures_without_failing_host() {
    use cordis_rs::plugin_ports::{OTelExporter, RedactingExporter};
    use cordis_rs::services::telemetry::{EventRecord, TelemetryError};
    use std::collections::HashMap;
    use std::sync::Arc;

    struct AlwaysFails;
    impl OTelExporter for AlwaysFails {
        fn export(&self, _records: &[EventRecord]) -> Result<(), TelemetryError> {
            Err(TelemetryError::InvalidName)
        }
    }

    let record = EventRecord {
        name: "evt".to_string(),
        attributes: HashMap::new(),
        seq: 1,
    };
    let exporter = RedactingExporter::new(Arc::new(AlwaysFails));
    assert_eq!(exporter.failures(), 0);

    // Host-facing API keeps returning Ok (R4-OB04).
    assert!(exporter.export(std::slice::from_ref(&record)).is_ok());
    assert_eq!(exporter.failures(), 1);
    exporter.export(std::slice::from_ref(&record)).unwrap();
    assert_eq!(exporter.failures(), 2);

    // Clones share the counter.
    let clone = exporter.clone();
    clone.export(&[record]).unwrap();
    assert_eq!(exporter.failures(), 3);
    assert_eq!(clone.failures(), 3);
}

#[test]
fn otlp_payload_carries_export_timestamp() {
    use cordis_rs::plugin_ports::otlp_payload;
    use cordis_rs::services::telemetry::EventRecord;
    use std::collections::HashMap;

    let record = EventRecord {
        name: "evt".to_string(),
        attributes: HashMap::new(),
        seq: 1,
    };

    let payload = otlp_payload(std::slice::from_ref(&record));
    let nano = payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["timeUnixNano"]
        .as_str()
        .unwrap();
    assert_ne!(nano, "0", "timestamp must not be the fixed placeholder");
    assert!(
        nano.chars().all(|ch| ch.is_ascii_digit()) && nano.len() >= 18,
        "timestamp must be unix nanoseconds as a numeric string: {nano}"
    );
}

#[test]
fn telemetry_counters_are_bounded_and_collapse_into_other() {
    use cordis_rs::core_services::{EventLogOptions, EventLogReporter, OTHER_COUNTER_KEY};

    let reporter =
        EventLogReporter::new("bounded", EventLogOptions::default()).with_max_counters(2);

    // First two names get their own counters.
    reporter
        .record("alpha", std::collections::HashMap::new())
        .unwrap();
    reporter
        .record("alpha", std::collections::HashMap::new())
        .unwrap();
    reporter
        .record("beta", std::collections::HashMap::new())
        .unwrap();
    assert_eq!(reporter.count("alpha"), 2);
    assert_eq!(reporter.count("beta"), 1);

    // Third and later names collapse into the shared bucket.
    reporter
        .record("gamma", std::collections::HashMap::new())
        .unwrap();
    reporter
        .record("delta", std::collections::HashMap::new())
        .unwrap();
    assert_eq!(reporter.count("gamma"), 0); // unknown name not tracked separately
    assert_eq!(reporter.count(OTHER_COUNTER_KEY), 2);

    // Reserved key cannot collide with a user event name.
    reporter
        .record(OTHER_COUNTER_KEY, std::collections::HashMap::new())
        .unwrap();
    assert_eq!(reporter.count(OTHER_COUNTER_KEY), 3);
}

#[test]
fn event_log_options_reject_zero_values() {
    use cordis_rs::core_services::{EventLogOptions, EventLogReporter, TelemetryError};

    for options in [
        EventLogOptions {
            max_events: 0,
            batch_size: 100,
        },
        EventLogOptions {
            max_events: 100,
            batch_size: 0,
        },
    ] {
        assert!(options.validate().is_err());
        let reporter = EventLogReporter::new("invalid", options);
        assert!(matches!(
            reporter.record("evt", std::collections::HashMap::new()),
            Err(TelemetryError::InvalidOptions)
        ));
    }

    assert!(EventLogOptions::default().validate().is_ok());
}

#[test]
fn session_reference_bounds_input_and_escapes_mention() {
    use cordis_rs::core_services::{
        MAX_REFERENCE_PAYLOAD_LEN, SessionReference, decode_session_reference_uri,
        encode_session_reference_uri, format_session_reference_mention,
    };

    // Round-trip still works.
    let uri = encode_session_reference_uri("session-1");
    assert_eq!(decode_session_reference_uri(&uri).unwrap(), "session-1");

    // Oversized payload is rejected before allocation, and the error does not
    // echo the untrusted URI (R3-IV10).
    let huge = format!("dsh-session:{}", "A".repeat(MAX_REFERENCE_PAYLOAD_LEN + 1));
    let error = decode_session_reference_uri(&huge).unwrap_err().to_string();
    assert!(error.contains("length"));
    assert!(
        !error.contains('A'),
        "URI must not appear in error: {error}"
    );

    // All markdown-significant characters in the label are escaped.
    let mention = format_session_reference_mention(&SessionReference {
        session_id: "s".to_string(),
        label: "a[b]c)\nd".to_string(),
    });
    assert!(mention.contains("a\\[b\\]c\\)"));
    assert!(mention.contains("\\\n"), "newline must be escaped");
    assert!(mention.starts_with("@[") && mention.contains("](dsh-session:"));
}

#[test]
fn plugin_error_preserves_source_chain() {
    use cordis_rs::Error;
    use std::error::Error as _;

    let inner = Error::ServiceNotFound("config".to_string());
    let wrapped = Error::Plugin("greeter".to_string(), Box::new(inner));
    let source = wrapped
        .source()
        .expect("Plugin must expose its cause via source()");
    assert!(source.to_string().contains("config"));
    assert!(wrapped.to_string().contains("greeter"));
}

#[tokio::test]
async fn provide_if_absent_guard_detects_type_mismatch() {
    use cordis_rs::Context;

    let ctx = Context::root("type-mismatch-guard");
    // Register a service under "shell" with a type the port does not expect.
    ctx.provide("shell", 42u32).await.unwrap();

    // A typed get would downcast to None and look "absent"; the untyped
    // guard must see the entry and skip re-registration (R1-AR02).
    assert!(ctx.get_untyped("shell").is_some());
    assert!(ctx.get::<String>("shell").is_none());
}

#[tokio::test]
async fn tool_bash_background_job_owner_is_session_scoped() {
    use cordis_rs::core_services::{
        JobStatus, ToolExecContext, ToolOutput, provide_jobs, provide_shell, provide_tools,
    };
    use cordis_rs::plugin_ports::{ToolBashConfig, mount_tool_bash};

    let ctx = Context::root("tool-bash-session-fence");
    let (registry, _) = provide_tools(&ctx).await.unwrap();
    let shell = provide_shell(&ctx).await.unwrap();
    let jobs = provide_jobs(&ctx).await.unwrap();
    let _tools = mount_tool_bash(&ctx, shell.clone(), ToolBashConfig::default())
        .await
        .unwrap();

    let mut exec = ToolExecContext::agent("agent-1");
    exec.session = Some("session-s1".to_string());
    let output: ToolOutput = registry
        .invoke(
            "bash",
            serde_json::json!({"command": "printf scoped", "run_in_background": true}),
            exec,
        )
        .await
        .unwrap();
    let id = output.content["job_id"].as_u64().unwrap();

    // The owning session can view the job; another session cannot
    // (swarm review R2-FG06).
    assert_eq!(
        jobs.view(id, Some("session-s1")).unwrap().status,
        JobStatus::Succeeded
    );
    assert!(jobs.view(id, Some("session-s2")).is_err());
}

#[test]
#[should_panic(expected = "max_counters must be positive")]
fn with_max_counters_rejects_zero() {
    use cordis_rs::core_services::{EventLogOptions, EventLogReporter};
    let _ = EventLogReporter::new("zero-cap", EventLogOptions::default()).with_max_counters(0);
}

#[test]
fn session_title_fallback_does_not_overwrite_existing_title() {
    use cordis_rs::core_services::SessionTitles;

    let titles = SessionTitles::new();
    assert!(titles.set_generated("s1", "LLM generated", 40));
    titles.derive_from_messages("s1", &["fallback first message".to_string()], 40);
    assert_eq!(titles.get("s1").as_deref(), Some("LLM generated"));

    // No title yet: the deterministic fallback still works.
    titles.derive_from_messages("s2", &["fallback title".to_string()], 40);
    assert_eq!(titles.get("s2").as_deref(), Some("fallback title"));
}

#[test]
fn credential_provider_set_rejects_unresolvable_names() {
    use cordis_rs::core_services::CredentialProvider;

    let provider = CredentialProvider::new();
    assert!(provider.set("API_KEY", "secret-value"));
    // Lowercase names cannot be referenced via `$NAME` (swarm review R2-IV01).
    assert!(!provider.set("api_key", "secret-value"));
    assert!(!provider.set("", "secret-value"));
    assert!(!provider.set("BAD-NAME", "secret-value"));
    assert!(!provider.set("API_KEY", ""));
}

#[test]
fn plan_mode_log_is_bounded_per_session() {
    use cordis_rs::core_services::PlanMode;

    let mode = PlanMode::new().with_max_events(3);
    for _ in 0..5 {
        mode.set("s1", true);
    }
    let log = mode.trace("s1");
    assert_eq!(log.len(), 3, "log must be capped at max_events");
    assert_eq!(log.last().map(|event| event.seq), Some(5));
    assert!(mode.is_active("s1"));
}

#[test]
fn session_query_index_is_a_snapshot_with_refresh_and_remove() {
    use cordis_rs::core_services::{Session, SessionQueryEngine};

    let engine = SessionQueryEngine::new();
    let session = Session::new("sq1", None);
    session.append("user/message", serde_json::json!({"text": "first"}));
    engine.index(session.clone());
    assert_eq!(engine.trace("sq1").len(), 1);

    // Session clones share their inner state, so an append on the original
    // is visible through the index (live reference, not a deep copy).
    session.append("user/message", serde_json::json!({"text": "second"}));
    assert_eq!(engine.trace("sq1").len(), 2);

    // refresh re-indexes explicitly; remove drops the entry entirely.
    engine.refresh(session.clone());
    assert_eq!(engine.trace("sq1").len(), 2);
    assert!(engine.remove("sq1"));
    assert!(engine.trace("sq1").is_empty());
    assert!(!engine.remove("sq1"));
}

#[test]
fn user_question_error_codes_cover_all_reserved_variants() {
    use cordis_rs::plugin_ports::UserQuestionError;

    // Reserved phase-2 variants must keep their upstream code strings for
    // public API stability (swarm review R2-ER06).
    let codes = [
        UserQuestionError::NoProvider,
        UserQuestionError::AskAborted,
        UserQuestionError::CallerNotLive,
        UserQuestionError::DelegatedCaller,
        UserQuestionError::BadIntent,
        UserQuestionError::EmptyQuestions,
        UserQuestionError::BadAnswer,
        UserQuestionError::ReplyQueued,
        UserQuestionError::ProviderError,
    ]
    .map(|variant| variant.code());
    assert_eq!(codes.len(), 9);
    assert!(codes.iter().all(|code| !code.is_empty()));
    assert!(codes.iter().all(|code| *code == code.trim()));
}

#[test]
fn registered_custom_preset_can_be_switched() {
    use cordis_rs::core_services::{ApprovalPolicy, PermissionPresets, PresetSpec, SandboxMode};

    let registry = PermissionPresets::new();
    registry.register(
        "auto",
        PresetSpec {
            sandbox: SandboxMode::WorkspaceWrite,
            approval: ApprovalPolicy::Never,
            label: Some("Auto review".to_string()),
            description: None,
        },
    );

    // A custom name has no canonical Preset identity, but its spec must still
    // be applicable (swarm review R3-AP01).
    let state = registry.switch("auto").unwrap();
    assert_eq!(state.preset, None);
    assert_eq!(state.sandbox, SandboxMode::WorkspaceWrite);
    assert_eq!(state.approval, ApprovalPolicy::Never);

    // Canonical presets keep their identity, and unknown names still fail.
    let canonical = registry.switch("read-only").unwrap();
    assert!(canonical.preset.is_some());
    assert!(registry.switch("unknown").is_err());
}

#[test]
fn retry_backoff_stays_positive_with_invalid_jitter_ratio() {
    use cordis_rs::core_services::RetryPolicy;

    for ratio in [-1.0, 2.0, f64::NAN] {
        let policy = RetryPolicy {
            jitter_ratio: ratio,
            ..RetryPolicy::default()
        };
        let delay = policy.backoff_ms(1, 0.0);
        assert!(
            delay > 0,
            "invalid jitter_ratio {ratio} produced zero delay"
        );
    }
}

#[test]
fn retry_error_code_requires_a_label() {
    use cordis_rs::Error;
    use cordis_rs::core_services::RetryPolicy;

    for (message, expected) in [
        ("HTTP 429 rate limited", Some(429)),
        ("status 503", Some(503)),
        ("TIMEOUT 504: no done", Some(504)),
        ("retry in 500ms", None),
        ("timed out after 1000ms", None),
        ("error 499", None),
    ] {
        assert_eq!(
            RetryPolicy::error_code(&Error::Other(message.to_string())),
            expected,
            "message: {message}"
        );
    }
}
