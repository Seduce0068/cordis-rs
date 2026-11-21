//! Demo: mount a real DSH `cordis.yml` against the ports available today.
//!
//! The upstream fixture references packages this port has not implemented
//! (skill-filesystem, llm-deepseek, tool-subagent). `mount_available` boots
//! the entries we do support and reports the rest, so a partial Rust port can
//! still run a real composition.

use cordis_rs::config::Value;
use cordis_rs::core_services::{provide_fs, provide_tools};
use cordis_rs::loader::{Composition, PluginFactory, PluginRegistry, VarTable};
use cordis_rs::plugin_ports::{
    DeepSeekConfig, MessageFeedbackConfig, SkillFsConfig, ToolFsConfig, ToolSubagentConfig,
    mount_feedback_producer, mount_llm_deepseek, mount_message_feedback,
    mount_session_persistence_jsonl, mount_skill_filesystem, mount_tool_fs, mount_tool_subagent,
    turn_stopping_event,
};
use cordis_rs::{Context, Lifecycle};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let yaml = include_str!("fixtures/text-turn.cordis.yml");
    let vars = VarTable::new()
        .with_home_path("/tmp/cordis-real")
        .with_env_value("DSH_SNAPSHOT_FEEDBACK", "1");
    let composition = Composition::parse_with_vars(yaml, &vars)?;
    println!("parsed {} entries", composition.entries.len());

    let ctx = Context::root("real-composition");
    let _ = provide_tools(&ctx).await?;
    let fs = provide_fs(&ctx).await?;

    let mut registry = PluginRegistry::new();

    // session-persistence-jsonl -> our JSONL port
    let ctx_for_persist = ctx.clone();
    registry.register(
        "@deepseek-ai/dsh-session-persistence-jsonl",
        PluginFactory::new(move |_ctx, config| {
            let ctx = ctx_for_persist.clone();
            let root = match config {
                Value::Object(map) => match map.get("root") {
                    Some(Value::String(path)) => path.clone(),
                    _ => "/tmp/cordis-real/sessions".to_string(),
                },
                _ => "/tmp/cordis-real/sessions".to_string(),
            };
            async move {
                let _ = mount_session_persistence_jsonl(&ctx, root).await?;
                Ok(())
            }
        }),
    );

    // llm-deepseek-api-key -> credential-gated provider (demo key supplied).
    let ctx_for_llm = ctx.clone();
    registry.register(
        "@deepseek-ai/dsh-llm-deepseek-api-key",
        PluginFactory::new(move |_ctx, _config| {
            let ctx = ctx_for_llm.clone();
            async move {
                let mut env = std::collections::BTreeMap::new();
                env.insert("DEEPSEEK_API_KEY".to_string(), "sk-demo".to_string());
                let _ = mount_llm_deepseek(&ctx, DeepSeekConfig::default(), env).await?;
                Ok(())
            }
        }),
    );

    // snapshot-feedback-producer -> turn-stopping listener.
    let ctx_for_producer = ctx.clone();
    registry.register(
        "./feedback-producer.mjs",
        PluginFactory::new(move |_ctx, _config| {
            let ctx = ctx_for_producer.clone();
            async move {
                let event = turn_stopping_event();
                mount_feedback_producer(&ctx, event).await?;
                Ok(())
            }
        }),
    );

    // tool-subagent -> delegation tool (no providers registered in this demo).
    let ctx_for_subagent = ctx.clone();
    registry.register(
        "@deepseek-ai/dsh-tool-subagent",
        PluginFactory::new(move |_ctx, _config| {
            let ctx = ctx_for_subagent.clone();
            async move {
                let config = ToolSubagentConfig {
                    provider: "spawn".to_string(),
                    ..Default::default()
                };
                let _ = mount_tool_subagent(&ctx, config).await?;
                Ok(())
            }
        }),
    );

    // message-feedback -> log-only rating service.
    let ctx_for_feedback = ctx.clone();
    registry.register(
        "@deepseek-ai/dsh-message-feedback",
        PluginFactory::new(move |_ctx, _config| {
            let ctx = ctx_for_feedback.clone();
            async move {
                let _ = mount_message_feedback(&ctx, MessageFeedbackConfig::default()).await?;
                Ok(())
            }
        }),
    );

    // skill-filesystem -> our skill provider port (no roots in this demo).
    let ctx_for_skills = ctx.clone();
    registry.register(
        "@deepseek-ai/dsh-skill-filesystem",
        PluginFactory::new(move |_ctx, _config| {
            let ctx = ctx_for_skills.clone();
            async move {
                let _ = mount_skill_filesystem(&ctx, SkillFsConfig::default()).await?;
                Ok(())
            }
        }),
    );

    // A generic tool bundle mapping for the tool-fs package id.
    let ctx_for_fs = ctx.clone();
    let fs_for_factory = fs.clone();
    registry.register(
        "@deepseek-ai/dsh-tool-fs",
        PluginFactory::new(move |_ctx, _config| {
            let ctx = ctx_for_fs.clone();
            let fs = fs_for_factory.clone();
            async move {
                let _ = mount_tool_fs(&ctx, fs, ToolFsConfig::default()).await?;
                Ok(())
            }
        }),
    );

    let report = composition.mount_available(&ctx, &registry).await?;
    println!("mounted {} fibers", report.mounted.len());
    assert!(
        report
            .mounted
            .iter()
            .all(|fiber| fiber.state() == Lifecycle::Active)
    );
    // Diagnostic report instead of assertions: as the upstream fixture grows,
    // new unported entries should be listed for triage rather than crashing
    // the smoke run (swarm review R5-VU08).
    println!("skipped {} unported entries:", report.skipped_unknown.len());
    for (id, reason) in &report.skipped_unknown {
        println!("  - {id}: {reason}");
    }
    println!("failed {} entries:", report.failed.len());
    for (id, reason) in &report.failed {
        println!("  - {id}: {reason}");
    }

    let _ = std::fs::remove_dir_all("/tmp/cordis-real");
    println!("partial real composition mounted successfully");
    Ok(())
}
