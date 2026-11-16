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
    SkillFsConfig, ToolFsConfig, mount_session_persistence_jsonl, mount_skill_filesystem,
    mount_tool_fs,
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
    println!(
        "skipped {} unported entries: {:?}",
        report.skipped_unknown.len(),
        report
            .skipped_unknown
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
    );
    assert!(report.failed.is_empty());
    assert!(
        !report.skipped_unknown.is_empty(),
        "fixture has unported plugins"
    );

    let _ = std::fs::remove_dir_all("/tmp/cordis-real");
    println!("partial real composition mounted successfully");
    Ok(())
}
