//! Tests for the reusable core service implementations.

use std::sync::Arc;
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

    let session = sessions.create(Some("s1".to_string()), Some("/tmp".to_string()));
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

    let forked = sessions.fork(&session, 2, Some("s2".to_string()));
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
    use cordis_rs::core_services::{ToolExecContext, ToolRegistry, provide_tools};
    use cordis_rs::plugin_ports::{
        WorkingDirectory, execute_working_directory, mount_working_directory,
    };

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
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

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
