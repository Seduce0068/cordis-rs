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
