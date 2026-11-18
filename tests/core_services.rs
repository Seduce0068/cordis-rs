//! Tests for the reusable core service implementations.

use std::sync::Arc;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::atomic::{AtomicUsize, Ordering};

use cordis_rs::Context;
use cordis_rs::core_services::{
    ToolDefinition, ToolError, ToolExecContext, ToolOutput, ToolRegistry, provide_tools,
};

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

    let dir = std::env::temp_dir().join(format!("cordis-tool-fs-{}", std::process::id()));
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

    let dir = std::env::temp_dir().join(format!("cordis-jsonl-{}", std::process::id()));
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

    let dir = std::env::temp_dir().join(format!("cordis-cursor-{}", std::process::id()));
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

    let dir = std::env::temp_dir().join(format!("cordis-cursor-atomic-{}", std::process::id()));
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

    let dir = std::env::temp_dir().join(format!("cordis-skills-{}", std::process::id()));
    let skill_dir = dir.join("demo-skill");
    let _ = std::fs::create_dir_all(&skill_dir);
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo-skill\n---\n# Demo\nA demo skill for tests.\n",
    )
    .unwrap();

    let ctx = Context::root("skill-fs");
    let config = SkillFsConfig {
        roots: vec![dir.clone()],
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

    let dir = std::env::temp_dir().join(format!("agent-instructions-{}", std::process::id()));
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

    let dir = std::env::temp_dir().join(format!("file-search-{}", std::process::id()));
    let _ = std::fs::create_dir_all(dir.join("src"));
    let _ = std::fs::create_dir_all(dir.join("node_modules"));
    std::fs::write(dir.join("src/main.rs"), "").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "").unwrap();
    std::fs::write(dir.join("node_modules/ignored.js"), "").unwrap();

    let search = WorkspaceFileSearch::new(dir.clone(), Default::default()).unwrap();
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
    assert!(WorkspaceFileSearch::new(&dir, config).is_err());

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
) -> cordis_rs::context::Disposer
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
