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
