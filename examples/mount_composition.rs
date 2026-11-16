//! Demo: parse a `cordis.yml` composition and mount its plugins as fibers.
//!
//! Shows the full loader path: YAML -> Composition -> validated config ->
//! PluginFactory -> Fiber::mount. Only the plugin ids present in the
//! PluginRegistry are mounted; unknown ids are reported as errors.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cordis_rs::config::Value;
use cordis_rs::loader::{Composition, PluginFactory, PluginRegistry, VarTable};
use cordis_rs::{Context, Lifecycle};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let yaml = r#"
entries:
  - id: greet
    plugin: demo/greet
    config:
      language: ja
  - id: disabled-one
    plugin: demo/greet
    disabled: true
    config: {}
  - id: count
    plugin: demo/count
    config:
      start: 5
"#;
    let composition = Composition::parse_with_vars(yaml, &VarTable::new())?;
    assert_eq!(composition.entries.len(), 3);
    assert_eq!(composition.enabled().count(), 2);

    let mounted = Arc::new(AtomicUsize::new(0));
    let mut registry = PluginRegistry::new();

    let greet_calls = mounted.clone();
    registry.register(
        "demo/greet",
        PluginFactory::new(move |_ctx, config| {
            let calls = greet_calls.clone();
            let language = match config {
                Value::Object(map) => match map.get("language") {
                    Some(Value::String(text)) => text.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                println!("demo/greet applied (language = {language})");
                Ok(())
            }
        }),
    );

    let count_calls = mounted.clone();
    registry.register(
        "demo/count",
        PluginFactory::new(move |_ctx, _config| {
            let calls = count_calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                println!("demo/count applied");
                Ok(())
            }
        }),
    );

    let ctx = Context::root("mount-composition");
    let fibers = composition.mount(&ctx, &registry).await?;
    assert_eq!(fibers.len(), 2, "disabled row must be skipped");
    assert!(
        fibers
            .iter()
            .all(|fiber| fiber.state() == Lifecycle::Active)
    );
    assert_eq!(mounted.load(Ordering::SeqCst), 2);

    println!("mounted {} fibers from the composition", fibers.len());
    Ok(())
}
