//! Tests of the PRODUCTION official port mount APIs.
//!
//! `tests/official_plugin_port.rs` and `tests/official_plugin_ports.rs` are
//! local contract reproductions with in-test stubs (swarm review R5-TS01).
//! This file exercises the real `src/ports/*::mount` functions so a broken
//! production port cannot hide behind passing stub tests.

use cordis_rs::Context;

#[tokio::test]
async fn production_command_goal_mount_registers_goal_store() {
    use cordis_rs::plugin_ports::mount_command_goal;

    let ctx = Context::root("prod-command-goal");
    // The port waits for its injected dependencies; provide them first.
    cordis_rs::services::agents::provide(&ctx).await.unwrap();
    let store = mount_command_goal(&ctx).await.unwrap();
    let _ = store;
    assert!(
        ctx.get::<cordis_rs::services::goals::GoalStore>("goals")
            .is_some()
    );
}

#[tokio::test]
async fn production_command_compact_mount_registers_command() {
    use cordis_rs::plugin_ports::mount_command_compact;

    let ctx = Context::root("prod-command-compact");
    cordis_rs::services::commands::provide(&ctx).await.unwrap();
    cordis_rs::services::compaction::provide(&ctx, cordis_rs::core_services::TokenMeter::new(100))
        .await
        .unwrap();
    let sessions = cordis_rs::services::sessions::SessionStore::default();
    let registry = mount_command_compact(&ctx, sessions).await.unwrap();
    let names = registry.names();
    assert!(
        names.iter().any(|name| name == "/compact"),
        "production port must register the /compact command; got {names:?}"
    );
}

#[tokio::test]
async fn production_tool_todo_mount_registers_tools() {
    use cordis_rs::plugin_ports::{TODO_TOOLS_SERVICE_NAME, mount_tool_todo};

    let ctx = Context::root("prod-tool-todo");
    let mounted = mount_tool_todo(&ctx, true).await.unwrap();
    let _ = mounted;
    assert!(
        ctx.get::<cordis_rs::plugin_ports::TodoTools>(TODO_TOOLS_SERVICE_NAME)
            .is_some()
    );
}

#[tokio::test]
async fn tool_registry_and_todo_tools_coexist_on_one_context() {
    use cordis_rs::plugin_ports::{TODO_TOOLS_SERVICE_NAME, mount_tool_todo};
    use cordis_rs::services::tools::provide as provide_tools;

    let ctx = Context::root("coexist-tools");

    // Both orders must work: the two services use distinct names (R1-AR01).
    provide_tools(&ctx).await.unwrap();
    mount_tool_todo(&ctx, false).await.unwrap();
    assert!(
        ctx.get::<cordis_rs::services::tools::ToolRegistry>("tools")
            .is_some()
    );
    assert!(
        ctx.get::<cordis_rs::plugin_ports::TodoTools>(TODO_TOOLS_SERVICE_NAME)
            .is_some()
    );

    let ctx2 = Context::root("coexist-tools-reverse");
    mount_tool_todo(&ctx2, false).await.unwrap();
    provide_tools(&ctx2).await.unwrap();
    assert!(
        ctx2.get::<cordis_rs::services::tools::ToolRegistry>("tools")
            .is_some()
    );
    assert!(
        ctx2.get::<cordis_rs::plugin_ports::TodoTools>(TODO_TOOLS_SERVICE_NAME)
            .is_some()
    );
}

#[tokio::test]
async fn todo_projections_coexist_with_core_session_projections() {
    use cordis_rs::core_services::provide_sessions;
    use cordis_rs::plugin_ports::{TODO_PROJECTIONS_SERVICE_NAME, mount_tool_todo};

    let ctx = Context::root("coexist-projections");
    // Core registry and the tool-todo projection use distinct service names
    // (swarm review R2-AR04), so both mounts succeed in either order.
    provide_sessions(&ctx).await.unwrap();
    mount_tool_todo(&ctx, false).await.unwrap();
    assert!(
        ctx.get::<cordis_rs::plugin_ports::TodoTools>(
            cordis_rs::plugin_ports::TODO_TOOLS_SERVICE_NAME
        )
        .is_some()
    );
    assert!(ctx.get_untyped(TODO_PROJECTIONS_SERVICE_NAME).is_some());
    assert!(ctx.get_untyped("sessionProjections").is_some());

    let ctx2 = Context::root("coexist-projections-reverse");
    mount_tool_todo(&ctx2, false).await.unwrap();
    provide_sessions(&ctx2).await.unwrap();
    assert!(ctx2.get_untyped(TODO_PROJECTIONS_SERVICE_NAME).is_some());
    assert!(ctx2.get_untyped("sessionProjections").is_some());
}

#[tokio::test]
async fn production_tool_fs_mount_registers_tools() {
    use cordis_rs::core_services::{FileSystem, provide_tools};
    use cordis_rs::plugin_ports::{ToolFsConfig, mount_tool_fs};

    let ctx = Context::root("prod-tool-fs");
    provide_tools(&ctx).await.unwrap();
    let fs = FileSystem::default();
    let registry = mount_tool_fs(&ctx, fs, ToolFsConfig::default())
        .await
        .unwrap();
    let names = registry.names();
    assert!(
        names.iter().any(|name| name == "read") && names.iter().any(|name| name == "write"),
        "production tool-fs must register read/write; got {names:?}"
    );
}

#[tokio::test]
async fn production_tool_bash_mount_registers_command() {
    use cordis_rs::core_services::{Shell, provide_tools};
    use cordis_rs::plugin_ports::{ToolBashConfig, mount_tool_bash};

    let ctx = Context::root("prod-tool-bash");
    provide_tools(&ctx).await.unwrap();
    let shell = Shell::default();
    let registry = mount_tool_bash(&ctx, shell, ToolBashConfig::default())
        .await
        .unwrap();
    assert!(
        registry.names().iter().any(|name| name == "bash"),
        "production tool-bash must register the bash tool"
    );
}

#[tokio::test]
async fn production_session_persistence_mount_registers_store() {
    use cordis_rs::plugin_ports::mount_session_persistence_jsonl;

    let base = std::env::temp_dir().join(format!(
        "prod-session-persistence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let ctx = Context::root("prod-session-persistence");
    let store = mount_session_persistence_jsonl(&ctx, &base).await.unwrap();
    store.create("s1").unwrap();
    assert!(ctx.get_untyped("sessionPersistence").is_some());
    let _ = std::fs::remove_dir_all(&base);
}
