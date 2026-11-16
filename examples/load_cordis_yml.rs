//! Demo: load a real DSH `cordis.yml` snapshot through the Rust loader.
//!
//! Uses `examples/fixtures/text-turn.cordis.yml`, copied from
//! `snapshots/sdk/text-turn/cordis.yml` in deepseek-ai/deepseek-harness (MIT).
//! Verifies that upstream patch syntax (top-level sequence, `name:`, nested
//! `insert:`) and `!!js` expressions (dshHomePath, process.env ternaries)
//! resolve without a JavaScript engine.

use cordis_rs::config::Value;
use cordis_rs::loader::{Composition, VarTable};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let yaml = include_str!("fixtures/text-turn.cordis.yml");

    let mut vars = VarTable::new().with_home_path("/home/user/.dsh");
    // DSH_SNAPSHOT is not set in this demo; DSH_SNAPSHOT_FEEDBACK is enabled.
    vars.insert("env.DSH_SNAPSHOT_FEEDBACK", Value::String("1".to_string()));

    let composition = Composition::parse_with_vars(yaml, &vars)?;

    println!("entries: {}", composition.entries.len());
    for entry in &composition.entries {
        println!(
            "  {} -> {} (disabled: {})",
            entry.id, entry.plugin, entry.disabled
        );
    }

    assert_eq!(composition.entries.len(), 6);
    assert_eq!(
        composition.entries[0].plugin,
        "@deepseek-ai/dsh-skill-filesystem"
    );
    assert_eq!(composition.entries[2].id, "session-persistence-jsonl");

    // `root: !!js dshHomePath('sessions')` resolves to an absolute path.
    if let Value::Object(map) = &composition.entries[2].config {
        assert_eq!(
            map.get("root"),
            Some(&Value::String("/home/user/.dsh/sessions".to_string()))
        );
        // `compression` is a ternary over process.env; DSH_SNAPSHOT is absent
        // so it resolves to 'zstd'.
        assert_eq!(
            map.get("compression"),
            Some(&Value::String("zstd".to_string()))
        );
    } else {
        return Err("expected object config".into());
    }

    // Feedback rows are enabled because DSH_SNAPSHOT_FEEDBACK == '1' makes
    // `!== '1'` false.
    assert!(!composition.entries[3].disabled);
    assert!(!composition.entries[4].disabled);
    assert_eq!(composition.enabled().count(), 6);

    println!("all entries resolved and validated");
    Ok(())
}
