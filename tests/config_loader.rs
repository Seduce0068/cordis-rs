//! Integration coverage for the config validator and `cordis.yml` loader.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cordis_rs::{
    Composition, ConfigValue, Context, Fiber, FieldSpec, FieldType, Lifecycle, LoaderError,
    PluginFactory, PluginRegistry, ValidationErrors, ValidationReason, VarTable,
};

fn schema() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("greeting", FieldType::String)
            .with_default(ConfigValue::String("Hello".into())),
        FieldSpec::new("targets", FieldType::Array)
            .elements(FieldSpec::new("target", FieldType::String)),
        FieldSpec::new("retries", FieldType::Integer)
            .optional()
            .range(1.0, 5.0),
        FieldSpec::new("debug", FieldType::Bool).optional(),
    ]
}

fn raw_object(entries: Vec<(&str, ConfigValue)>) -> ConfigValue {
    ConfigValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

#[test]
fn validator_detects_missing_and_type_mismatch_with_field_names() {
    let raw = raw_object(vec![(
        "targets",
        ConfigValue::String("not-an-array".into()),
    )]);
    let errors = cordis_rs::validate_config(&schema(), &raw).unwrap_err();
    let paths: Vec<_> = errors.iter().map(|error| error.path.as_str()).collect();
    assert!(paths.contains(&"targets"));
    assert!(errors.iter().any(|error| {
        matches!(
            error.reason,
            ValidationReason::TypeMismatch {
                expected: FieldType::Array,
                ..
            }
        )
    }));
    assert!(errors.iter().all(|error| !error.path.is_empty()));
}

#[test]
fn validator_detects_range_and_element_type_errors() {
    let raw = raw_object(vec![
        (
            "targets",
            ConfigValue::Array(vec![
                ConfigValue::String("ok".into()),
                ConfigValue::Integer(7),
            ]),
        ),
        ("retries", ConfigValue::Integer(9)),
    ]);
    let errors = cordis_rs::validate_config(&schema(), &raw).unwrap_err();
    assert!(errors.iter().any(|error| error.path == "targets[1]"));
    assert!(errors.iter().any(|error| error.path == "retries"));
    assert!(
        errors
            .iter()
            .any(|error| matches!(error.reason, ValidationReason::OutOfRange { .. }))
    );
    assert!(ValidationErrors(errors).to_string().contains("retries"));
}

#[test]
fn validator_applies_defaults_and_keeps_valid_values() {
    let raw = raw_object(vec![(
        "targets",
        ConfigValue::Array(vec![ConfigValue::String("alpha".into())]),
    )]);
    let validated = cordis_rs::validate_config(&schema(), &raw).unwrap();
    let map = validated.as_object().unwrap();
    assert_eq!(map["greeting"], ConfigValue::String("Hello".into()));
    assert_eq!(map["targets"], raw.as_object().unwrap()["targets"]);
    assert!(!map.contains_key("retries"));
}

#[test]
fn loader_parses_entries_and_resolves_references() {
    let yaml = r#"
# a leading comment
entries:
  - id: greeter
    plugin: demo/greet
    disabled: false
    config:
      targets: ['alpha', "beta"] # inline comment
      retries: !!js $limit
      debug: true
  - id: skipped
    plugin: demo/greet
    disabled: true
    config:
      targets: [nope]
"#;
    let mut vars = VarTable::new();
    vars.insert("limit", ConfigValue::Integer(3));
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    assert_eq!(composition.entries.len(), 2);
    assert_eq!(composition.entries[0].id, "greeter");
    assert_eq!(composition.entries[0].plugin, "demo/greet");
    assert_eq!(
        composition.entries[0].config.as_object().unwrap()["retries"],
        ConfigValue::Integer(3)
    );
    assert!(composition.entries[1].disabled);
    assert_eq!(composition.enabled().count(), 1);
}

#[tokio::test]
async fn loader_skips_disabled_and_mounts_enabled_entry_through_fiber() {
    let yaml = r#"
entries:
  - id: active
    plugin: demo/greet
    config:
      targets: [a, b]
      retries: 2
  - id: dormant
    plugin: demo/greet
    disabled: true
    config:
      targets: [c]
  - id: literal
    plugin: demo/greet
    config:
      targets: [$not-a-ref]
"#;
    let composition = Composition::parse(yaml).unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let body_seen = seen.clone();
    let mut registry = PluginRegistry::new();
    registry.register(
        "demo/greet",
        PluginFactory::new(move |_ctx, config| {
            let seen = body_seen.clone();
            let targets = config.as_object().unwrap()["targets"].clone();
            async move {
                seen.lock().unwrap().push(targets);
                Ok(())
            }
        })
        .with_schema(schema()),
    );

    let ctx = Context::root("loader");
    let fibers = composition.mount(&ctx, &registry).await.unwrap();
    assert_eq!(fibers.len(), 2, "disabled row must be skipped");
    assert!(
        fibers
            .iter()
            .all(|fiber| fiber.state() == Lifecycle::Active)
    );

    let expected = [
        ConfigValue::Array(vec![
            ConfigValue::String("a".into()),
            ConfigValue::String("b".into()),
        ]),
        ConfigValue::Array(vec![ConfigValue::String("$not-a-ref".into())]),
    ];
    assert_eq!(*seen.lock().unwrap(), expected);
    assert!(fibers.iter().any(|fiber: &Fiber| fiber.name() == "literal"));
    assert!(!fibers.iter().any(|fiber: &Fiber| fiber.name() == "dormant"));
}

#[test]
fn loader_rejects_unresolved_reference_and_bad_disabled() {
    let yaml =
        "entries:\n  - id: bad\n    plugin: demo/greet\n    config:\n      retries: $missing\n";
    let error = Composition::parse(yaml).unwrap_err();
    assert!(matches!(
        error,
        LoaderError::UnresolvedReference { ref name, .. } if name == "missing"
    ));

    let yaml = "entries:\n  - id: bad\n    plugin: demo/greet\n    disabled: yes\n";
    let error = Composition::parse(yaml).unwrap_err();
    assert!(matches!(error, LoaderError::InvalidDisabled { .. }));
    assert!(error.to_string().contains("disabled"));
}

#[tokio::test]
async fn loader_surfaces_validation_errors_before_plugin_runs() {
    let yaml = "entries:\n  - id: bad\n    plugin: demo/greet\n    config:\n      targets: nope\n";
    let composition = Composition::parse(yaml).unwrap();
    let mut registry = PluginRegistry::new();
    let ran = Arc::new(AtomicBool::new(false));
    let body_ran = ran.clone();
    registry.register(
        "demo/greet",
        PluginFactory::new(move |_ctx, _config| {
            let ran = body_ran.clone();
            async move {
                ran.store(true, Ordering::SeqCst);
                Ok(())
            }
        })
        .with_schema(schema()),
    );
    let ctx = Context::root("invalid-config");
    let error = composition
        .mount(&ctx, &registry)
        .await
        .err()
        .expect("mount must fail on invalid config");
    assert!(matches!(error, LoaderError::Validation(_)));
    assert!(error.to_string().contains("targets"));
    assert!(
        !ran.load(Ordering::SeqCst),
        "plugin must never start on bad config"
    );
}

#[tokio::test]
async fn loader_passes_raw_config_to_plugins_without_a_schema() {
    let yaml = "entries:\n  - id: raw\n    plugin: demo/raw\n    config:\n      anything: 42\n";
    let composition = Composition::parse(yaml).unwrap();
    let mut registry = PluginRegistry::new();
    let seen = Arc::new(std::sync::Mutex::new(None));
    let body_seen = seen.clone();
    registry.register(
        "demo/raw",
        PluginFactory::new(move |_ctx, config| {
            let seen = body_seen.clone();
            let config = config.clone();
            async move {
                *seen.lock().unwrap() = Some(config);
                Ok(())
            }
        }),
    );
    let ctx = Context::root("raw-config");
    let fibers = composition.mount(&ctx, &registry).await.unwrap();
    assert_eq!(fibers.len(), 1);
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some(raw_object(vec![("anything", ConfigValue::Integer(42))]))
    );
}

#[tokio::test]
async fn loader_supports_block_sequences_and_unknown_plugin_errors() {
    let yaml = r#"
entries:
  - id: blocky
    plugin: demo/greet
    config:
      targets:
        - one
        - two
      retries: 1
"#;
    let composition = Composition::parse(yaml).unwrap();
    let mut registry = PluginRegistry::new();
    registry.register(
        "demo/greet",
        PluginFactory::new(|_ctx, _config| async { Ok(()) }).with_schema(schema()),
    );
    let ctx = Context::root("block-seq");
    let fibers = composition.mount(&ctx, &registry).await.unwrap();
    assert_eq!(fibers.len(), 1);
    let fiber: Fiber = fibers.into_iter().next().unwrap();
    assert_eq!(fiber.name(), "blocky");

    let yaml = "entries:\n  - id: ghost\n    plugin: not-registered\n";
    let composition = Composition::parse(yaml).unwrap();
    let error = composition
        .mount(&ctx, &registry)
        .await
        .err()
        .expect("unknown plugin must fail");
    assert!(matches!(error, LoaderError::UnknownPlugin { .. }));
    assert!(error.to_string().contains("not-registered"));
}

#[tokio::test]
async fn mount_failure_unloads_already_mounted_fibers() {
    let yaml =
        "entries:\n  - id: first\n    plugin: demo/ok\n  - id: broken\n    plugin: demo/bad\n";
    let composition = Composition::parse(yaml).unwrap();
    let mut registry = PluginRegistry::new();
    let disposed = Arc::new(AtomicBool::new(false));
    let body_disposed = disposed.clone();
    registry.register(
        "demo/ok",
        PluginFactory::new(move |ctx, _config| {
            let ctx = ctx.clone();
            let disposed = body_disposed.clone();
            async move {
                ctx.effect(move |scope| {
                    let disposed = disposed.clone();
                    async move {
                        scope.on_dispose(move || {
                            let disposed = disposed.clone();
                            async move {
                                disposed.store(true, Ordering::SeqCst);
                                Ok(())
                            }
                        });
                        Ok(())
                    }
                })
                .await
            }
        }),
    );
    registry.register(
        "demo/bad",
        PluginFactory::new(|_ctx, _config| async { Err(cordis_rs::Error::Other("boom".into())) }),
    );

    let ctx = Context::root("rollback");
    let result = composition.mount(&ctx, &registry).await;
    let error = result.err().expect("second plugin must fail the mount");
    assert!(matches!(error, LoaderError::Mount(_)));
    assert!(
        disposed.load(Ordering::SeqCst),
        "first fiber must be unloaded"
    );
}

#[tokio::test]
async fn yaml_inject_delays_fiber_until_dependency_is_provided() {
    let yaml = "entries:\n  - id: needs-input\n    plugin: demo/greet\n    inject: [input]\n";
    let composition = Composition::parse(yaml).unwrap();
    let ran = Arc::new(AtomicBool::new(false));
    let body_ran = ran.clone();
    let mut registry = PluginRegistry::new();
    registry.register(
        "demo/greet",
        PluginFactory::new(move |_ctx, _config| {
            let ran = body_ran.clone();
            async move {
                ran.store(true, Ordering::SeqCst);
                Ok(())
            }
        }),
    );

    let ctx = Context::root("yaml-inject");
    let fibers = composition.mount(&ctx, &registry).await.unwrap();
    assert_eq!(fibers.len(), 1);
    assert_eq!(fibers[0].state(), Lifecycle::Pending);
    assert!(!ran.load(Ordering::SeqCst));

    ctx.provide("input", 1u32).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(fibers[0].state(), Lifecycle::Active);
    assert!(ran.load(Ordering::SeqCst));
}

#[test]
fn recursion_depth_is_bounded_by_input_size() {
    let nested = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let yaml = format!(
        "entries:\n  - id: deep\n    plugin: demo/deep\n    config:\n      nested: {nested}\n"
    );
    let error = Composition::parse(&yaml).unwrap_err();
    assert!(matches!(error, LoaderError::TooDeep { .. }));

    let reasonable =
        "entries:\n  - id: ok\n    plugin: demo/deep\n    config:\n      nested: [[[1]]]\n";
    assert!(Composition::parse(reasonable).is_ok());
}

#[test]
fn dash_line_mapping_keys_align_with_the_first_key_column() {
    // Regression for key_indent: `config:` opens an inline mapping on the
    // dash line; `id` / `plugin` below are siblings at the same column, not
    // children of `config`.
    let yaml = "entries:\n  - config:\n    id: a\n    plugin: demo/greet\n";
    let composition = Composition::parse(yaml).unwrap();
    assert_eq!(composition.entries.len(), 1);
    let entry = &composition.entries[0];
    assert_eq!(entry.id, "a");
    assert_eq!(entry.plugin, "demo/greet");
    assert_eq!(entry.config, ConfigValue::object());
}
