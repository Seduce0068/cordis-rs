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
    use cordis_rs::plugin_ports::mount_tool_todo;

    let ctx = Context::root("prod-tool-todo");
    let mounted = mount_tool_todo(&ctx, true).await.unwrap();
    let _ = mounted;
    assert!(
        ctx.get::<cordis_rs::plugin_ports::TodoTools>("tools")
            .is_some()
    );
}
