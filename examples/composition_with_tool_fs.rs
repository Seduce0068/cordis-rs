//! Demo: a `cordis.yml` composition mounts the official tool-fs port.
//!
//! The YAML config selects which tools are enabled; the PluginFactory body
//! registers the matching handlers on the shared `tools` seam. Then a file is
//! written and read back through the mounted tools.

use cordis_rs::Context;
use cordis_rs::config::Value;
use cordis_rs::core_services::{ToolExecContext, ToolOutput, provide_fs, provide_tools};
use cordis_rs::loader::{Composition, PluginFactory, PluginRegistry, VarTable};
use cordis_rs::plugin_ports::{ToolFsConfig, mount_tool_fs};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let yaml = r#"
entries:
  - id: fs-tools
    plugin: dsh-tool-fs
    config:
      read: true
      write: true
      edit: false
"#;
    let composition = Composition::parse_with_vars(yaml, &VarTable::new())?;
    assert_eq!(composition.entries.len(), 1);

    let ctx = Context::root("composition-tool-fs");
    let (tools, _) = provide_tools(&ctx).await?;
    let fs = provide_fs(&ctx).await?;

    let mut registry = PluginRegistry::new();
    let expected_fs = fs.clone();
    registry.register(
        "dsh-tool-fs",
        PluginFactory::new(move |ctx, config| {
            let fs = expected_fs.clone();
            let ctx = ctx.clone();
            let mut tool_fs_config = ToolFsConfig::default();
            if let Value::Object(map) = config {
                if let Some(Value::Bool(value)) = map.get("read") {
                    tool_fs_config.read = *value;
                }
                if let Some(Value::Bool(value)) = map.get("write") {
                    tool_fs_config.write = *value;
                }
                if let Some(Value::Bool(value)) = map.get("edit") {
                    tool_fs_config.edit = *value;
                }
            }
            async move {
                // Mount the port; its effect-scoped disposers keep the tools
                // registered while the fiber is alive.
                let mounted = mount_tool_fs(&ctx, fs, tool_fs_config).await?;
                let _ = mounted;
                Ok(())
            }
        }),
    );

    let fibers = composition.mount(&ctx, &registry).await?;
    assert_eq!(fibers.len(), 1);
    println!("mounted: {}", fibers[0].name());

    let mut names = tools.names();
    names.sort();
    println!("tools: {names:?}");
    assert_eq!(names, ["read", "write"], "edit must be disabled by config");

    let path = std::env::temp_dir().join(format!(
        "cordis-composition-tool-fs-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let path_str = path.to_string_lossy().to_string();
    let written: ToolOutput = tools
        .invoke(
            "write",
            serde_json::json!({ "file_path": path_str, "content": "from composition\n" }),
            ToolExecContext::agent("demo"),
        )
        .await?;
    println!("{}", written.rendered[0]);

    let read: ToolOutput = tools
        .invoke(
            "read",
            serde_json::json!({ "file_path": path_str }),
            ToolExecContext::agent("demo"),
        )
        .await?;
    assert!(read.rendered[0].contains("from composition"));
    std::fs::remove_file(path)?;

    println!("composition-driven tool-fs verified");
    Ok(())
}
