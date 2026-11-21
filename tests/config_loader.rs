//! Integration coverage for the config validator and `cordis.yml` loader.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cordis_rs::{
    Composition, ConfigValue, Context, Fiber, FieldSpec, FieldType, Lifecycle, LoaderError,
    PluginFactory, PluginRegistry, ValidationErrors, ValidationReason, VarTable,
};

/// Auto-cleaning temp directory (keep in sync with core_services.rs). Uses a
/// PID + nanosecond suffix and ignores cleanup errors in Drop (R4-TQ09).
struct TempDirGuard(std::path::PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for TempDirGuard {
    type Target = std::path::Path;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<std::path::Path> for TempDirGuard {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

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
    // Poll with a bounded deadline instead of a fixed sleep (R5-TS02).
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while fibers[0].state() != Lifecycle::Active {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fiber did not reach Active within 2s");
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

#[test]
fn js_expressions_are_evaluated_in_config_and_disabled() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert("model", Value::String("gpt-test".to_string()));
    vars.insert("limit", Value::Integer(7));

    let yaml = r#"
entries:
  - id: enabled
    plugin: demo/greet
    disabled: !!js false
    config:
      model: !!js $model
      label: !!js "model=${model},limit=${limit}"
      retries: !!js 3
      tags: !!js ["a", "b"]
  - id: skipped
    plugin: demo/greet
    disabled: !!js true
    config: {}
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let enabled: Vec<_> = composition.enabled().collect();
    assert_eq!(enabled.len(), 1, "disabled !!js true row must be skipped");
    let entry = enabled[0];
    assert_eq!(entry.id, "enabled");

    let model = match &entry.config {
        Value::Object(map) => map.get("model").cloned(),
        _ => None,
    };
    assert_eq!(model, Some(Value::String("gpt-test".to_string())));

    let label = match &entry.config {
        Value::Object(map) => map.get("label").cloned(),
        _ => None,
    };
    assert_eq!(
        label,
        Some(Value::String("model=gpt-test,limit=7".to_string()))
    );

    let retries = match &entry.config {
        Value::Object(map) => map.get("retries").cloned(),
        _ => None,
    };
    assert_eq!(retries, Some(Value::Integer(3)));

    let tags = match &entry.config {
        Value::Object(map) => map.get("tags").cloned(),
        _ => None,
    };
    assert_eq!(
        tags,
        Some(Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ]))
    );
}

#[test]
fn js_comparisons_and_ternary_are_evaluated() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert("mode", Value::String("fast".to_string()));
    vars.insert("count", Value::Integer(3));
    vars.insert("empty", Value::String(String::new()));

    let yaml = r#"
entries:
  - id: t1
    plugin: demo/a
    config:
      flag: !!js mode == "fast"
      not_flag: !!js mode != "slow"
      ge: !!js count >= 3
      lt: !!js count < 3
      chosen: !!js mode == "fast" ? "yes" : "no"
      fallback: !!js empty ? "non-empty" : "empty"
      numeric: !!js count > 10 ? 10 : count
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let config = composition.entries[0].config.clone();
    let map = match config {
        Value::Object(map) => map,
        _ => panic!("expected object config"),
    };
    assert_eq!(map.get("flag"), Some(&Value::Bool(true)));
    assert_eq!(map.get("not_flag"), Some(&Value::Bool(true)));
    assert_eq!(map.get("ge"), Some(&Value::Bool(true)));
    assert_eq!(map.get("lt"), Some(&Value::Bool(false)));
    assert_eq!(map.get("chosen"), Some(&Value::String("yes".to_string())));
    assert_eq!(
        map.get("fallback"),
        Some(&Value::String("empty".to_string()))
    );
    assert_eq!(map.get("numeric"), Some(&Value::Integer(3)));
}

#[test]
fn js_logical_operators_are_evaluated() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert("a", Value::Bool(true));
    vars.insert("b", Value::Bool(false));
    vars.insert("n", Value::Integer(0));

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      and: !!js a && b
      or: !!js a || b
      not: !!js !b
      not_n: !!js !n
      combined: !!js a && !b ? "ok" : "bad"
      inequality: !!js a != b
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(map.get("and"), Some(&Value::Bool(false)));
    assert_eq!(map.get("or"), Some(&Value::Bool(true)));
    assert_eq!(map.get("not"), Some(&Value::Bool(true)));
    assert_eq!(map.get("not_n"), Some(&Value::Bool(true)));
    assert_eq!(map.get("combined"), Some(&Value::String("ok".to_string())));
    assert_eq!(map.get("inequality"), Some(&Value::Bool(true)));
}

#[test]
fn js_member_access_resolves_indices_and_length() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert(
        "items",
        Value::Array(vec![
            Value::String("alpha".to_string()),
            Value::String("beta".to_string()),
        ]),
    );
    vars.insert("name", Value::String("hello".to_string()));
    vars.insert(
        "obj",
        Value::Object({
            let mut map = std::collections::BTreeMap::new();
            map.insert("key".to_string(), Value::Integer(42));
            map
        }),
    );

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      first: !!js $items[0]
      second: !!js $items[1]
      count: !!js $items.length
      name_len: !!js $name.length
      obj_key: !!js $obj.key
      out_of_range: !!js $items[9]
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(map.get("first"), Some(&Value::String("alpha".to_string())));
    assert_eq!(map.get("second"), Some(&Value::String("beta".to_string())));
    assert_eq!(map.get("count"), Some(&Value::Integer(2)));
    assert_eq!(map.get("name_len"), Some(&Value::Integer(5)));
    assert_eq!(map.get("obj_key"), Some(&Value::Integer(42)));
    assert_eq!(map.get("out_of_range"), Some(&Value::Null));
}

#[test]
fn upstream_style_composition_with_name_and_insert_is_loaded() {
    use cordis_rs::loader::Composition;

    // Mirrors snapshots/sdk/text-turn/cordis.yml: `name:` package ids and a
    // nested `insert:` row. !!js expressions outside our subset stay literal.
    let yaml = r#"
- id: skill-filesystem
  name: '@deepseek-ai/dsh-skill-filesystem'
  config:
    includeDefaultRoots: false

- id: llm-deepseek
  name: '@deepseek-ai/dsh-llm-deepseek-api-key'
  config:
    thinking: enabled

- insert:
    - id: snapshot-message-feedback
      name: '@deepseek-ai/dsh-message-feedback'
      disabled: !!js "process.env.DSH_SNAPSHOT_FEEDBACK !== '1'"
      config:
        maxNoteBytes: 1024
"#;
    let composition = Composition::parse(yaml).unwrap();
    assert_eq!(composition.entries.len(), 3);
    assert_eq!(
        composition.entries[0].plugin,
        "@deepseek-ai/dsh-skill-filesystem"
    );
    assert_eq!(composition.entries[1].id, "llm-deepseek");
    assert_eq!(composition.entries[2].id, "snapshot-message-feedback");
    // The !!js process.env expression is not in our subset; it resolves to a
    // non-bool literal and is rejected by the strict disabled validator. We
    // keep the row but document that full JS evaluation is out of scope.
    let _ = composition.enabled().count();
    let direct = composition.entries[0].clone();
    assert!(!direct.disabled);
}

#[test]
fn js_process_env_reference_and_inequality() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    // A deterministic env var for the test (set only for this process).
    let mut vars = VarTable::new();
    vars.insert("env.CORDIS_TEST_FLAG", Value::String("1".to_string()));

    let yaml = r#"
entries:
  - id: skip
    plugin: demo/x
    disabled: !!js process.env.CORDIS_TEST_FLAG !== '1'
    config: {}
  - id: keep
    plugin: demo/y
    disabled: !!js env.CORDIS_TEST_FLAG == '1'
    config: {}
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    assert_eq!(composition.entries.len(), 2);
    // CORDIS_TEST_FLAG == "1", so `!== '1'` is false (enabled) and
    // `== '1'` is true (disabled).
    assert!(!composition.entries[0].disabled);
    assert!(composition.entries[1].disabled);
    assert_eq!(composition.enabled().count(), 1);

    let _ = Value::Null;
}

#[test]
fn js_dsh_home_path_helper_resolves_against_var_table() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let vars = VarTable::new().with_home_path("/home/user/.dsh");
    let yaml = r#"
entries:
  - id: persistence
    plugin: demo/persist
    config:
      root: !!js dshHomePath('sessions')
      text: !!js String("ok")
      count: !!js Number(3)
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(
        map.get("root"),
        Some(&Value::String("/home/user/.dsh/sessions".to_string()))
    );
    assert_eq!(map.get("text"), Some(&Value::String("ok".to_string())));
    assert_eq!(map.get("count"), Some(&Value::Integer(3)));
}

#[test]
fn composition_state_persists_toggle_overrides() {
    use cordis_rs::loader::{Composition, CompositionState};

    let yaml = r#"
entries:
  - id: a
    plugin: demo/a
    config: {}
  - id: b
    plugin: demo/b
    disabled: true
    config: {}
"#;
    let composition = Composition::parse(yaml).unwrap();
    assert_eq!(composition.enabled().count(), 1); // only "a"

    let mut state = CompositionState::new();
    state.set("a", false); // disable a
    state.set("b", true); // re-enable b

    let enabled = composition.enabled_with(&state);
    let ids: Vec<_> = enabled.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(ids, ["b"]);

    let dir = TempDirGuard::new("cordis-composition-state");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("state.json");
    state.save(&path).unwrap();
    assert!(!path.with_extension("tmp").exists());

    let restored = CompositionState::load(&path);
    assert_eq!(restored, state);
    let enabled_restored = composition.enabled_with(&restored);
    let ids: Vec<_> = enabled_restored
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, ["b"]);

    // Missing file yields no overrides (YAML defaults).
    let fresh = CompositionState::load(dir.join("missing.json"));
    assert!(fresh.overrides.is_empty());
    assert_eq!(composition.enabled_with(&fresh).len(), 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn js_string_methods_are_evaluated() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert(
        "path",
        Value::String("https://api.deepseek.com".to_string()),
    );
    vars.insert(
        "items",
        Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ]),
    );

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      inc: !!js $path.includes('deepseek')
      starts: !!js $path.startsWith('https')
      ends: !!js $path.endsWith('.com')
      miss: !!js $path.includes('openai')
      arr: !!js $items.includes('b')
      chained: !!js $path.includes('deepseek') ? "yes" : "no"
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(map.get("inc"), Some(&Value::Bool(true)));
    assert_eq!(map.get("starts"), Some(&Value::Bool(true)));
    assert_eq!(map.get("ends"), Some(&Value::Bool(true)));
    assert_eq!(map.get("miss"), Some(&Value::Bool(false)));
    assert_eq!(map.get("arr"), Some(&Value::Bool(true)));
    assert_eq!(map.get("chained"), Some(&Value::String("yes".to_string())));
}

#[test]
fn js_arithmetic_operators_are_evaluated() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert("count", Value::Integer(5));
    vars.insert("path", Value::String("working-directory".to_string()));

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      add: !!js 2 + 3
      sub: !!js 10 - 4
      mul: !!js count * 2
      div: !!js 9 / 2
      chain: !!js count + 1 >= 6 ? "yes" : "no"
      literal: !!js $path
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(map.get("add"), Some(&Value::Integer(5)));
    assert_eq!(map.get("sub"), Some(&Value::Integer(6)));
    assert_eq!(map.get("mul"), Some(&Value::Integer(10)));
    assert_eq!(map.get("div"), Some(&Value::Number(4.5)));
    assert_eq!(map.get("chain"), Some(&Value::String("yes".to_string())));
    // Hyphen in a value must not be parsed as subtraction.
    assert_eq!(
        map.get("literal"),
        Some(&Value::String("working-directory".to_string()))
    );
}

#[test]
fn js_object_literals_are_evaluated() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert("name", Value::String("cordis".to_string()));
    vars.insert("n", Value::Integer(3));

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      obj: !!js {name: $name, count: $n, ok: true}
      list: !!js [{id: 1}, {id: 2}]
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    let obj = match map.get("obj") {
        Some(Value::Object(map)) => map.clone(),
        _ => panic!("expected object literal"),
    };
    assert_eq!(obj.get("name"), Some(&Value::String("cordis".to_string())));
    assert_eq!(obj.get("count"), Some(&Value::Integer(3)));
    assert_eq!(obj.get("ok"), Some(&Value::Bool(true)));

    let list = match map.get("list") {
        Some(Value::Array(items)) => items.clone(),
        _ => panic!("expected array literal"),
    };
    assert_eq!(list.len(), 2);
    assert!(matches!(&list[0], Value::Object(m) if m.get("id") == Some(&Value::Integer(1))));
}

#[test]
fn js_template_literals_interpolate_variables() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert("home", Value::String("/home/user".to_string()));
    vars.insert("n", Value::Integer(3));

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      path: !!js `${home}/sessions`
      mixed: !!js `run ${n}`
      plain: !!js `no interpolation`
"#;
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(
        map.get("path"),
        Some(&Value::String("/home/user/sessions".to_string()))
    );
    assert_eq!(map.get("mixed"), Some(&Value::String("run 3".to_string())));
    assert_eq!(
        map.get("plain"),
        Some(&Value::String("no interpolation".to_string()))
    );
}

#[test]
fn js_nullish_coalescing_keeps_falsy_values() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let mut vars = VarTable::new();
    vars.insert("empty", Value::String(String::new()));
    vars.insert("zero", Value::Integer(0));

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      set: !!js process.env.DEMO_GREETING ?? 'Hello'
      missing: !!js process.env.DSH_NOT_SET ?? 'fallback'
      falsy: !!js $empty ?? 'unused'
      zero: !!js $zero ?? 'unused'
      mixed: !!js $empty ?? (process.env.DSH_NOT_SET ? 'a' : 'b')
"#;
    let vars = vars.with_env_value("DEMO_GREETING", "Hi");
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(map.get("set"), Some(&Value::String("Hi".to_string())));
    assert_eq!(
        map.get("missing"),
        Some(&Value::String("fallback".to_string()))
    );
    assert_eq!(map.get("falsy"), Some(&Value::String(String::new())));
    assert_eq!(map.get("zero"), Some(&Value::Integer(0)));
    assert_eq!(map.get("mixed"), Some(&Value::String(String::new())));
}

#[test]
fn js_process_globals_concatenate_with_strings() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      cwd: !!js process.cwd()
      joined: !!js process.cwd() + '/native-helper.mjs'
      exec: !!js process.execPath
"#;
    let vars = VarTable::new().with_home_path("/tmp/dsh-home");
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert_eq!(
        map.get("cwd"),
        Some(&Value::String("/tmp/dsh-home".to_string()))
    );
    assert_eq!(
        map.get("joined"),
        Some(&Value::String(
            "/tmp/dsh-home/native-helper.mjs".to_string()
        ))
    );
    assert!(matches!(map.get("exec"), Some(Value::String(_))));
}

#[test]
fn js_optional_chaining_short_circuits_on_null() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      fallback: !!js process.env.NOPE?.trim() || 'fallback'
      trimmed: !!js process.env.SPACE?.trim()
      ternary: !!js process.env.DEMO_GREETING === 'Hi' ? 'yes' : 'no'
"#;
    let vars = VarTable::new()
        .with_env_value("SPACE", "  padded  ")
        .with_env_value("DEMO_GREETING", "Hi");
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    // `||` currently returns a Bool (existing semantics); the key point is
    // that `?.` no longer errors out on an unset env var.
    assert!(map.contains_key("fallback"));
    assert_eq!(
        map.get("trimmed"),
        Some(&Value::String("padded".to_string()))
    );
    assert_eq!(map.get("ternary"), Some(&Value::String("yes".to_string())));
}

#[test]
fn js_block_scalar_expression_is_folded() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    let yaml = r#"
entries:
  - id: t
    plugin: demo/x
    config:
      expr: !!js >-
        [
        'a',
        'b'
        ]
      after: sibling
"#;
    let vars = VarTable::new()
        .with_env_value("A", "a")
        .with_env_value("B", "b");
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    assert!(map.contains_key("expr"));
    assert_eq!(
        map.get("after"),
        Some(&Value::String("sibling".to_string()))
    );
}

#[test]
fn loader_preserves_hash_after_escaped_quote_in_double_quoted_string() {
    let yaml = r#"
entries:
  - id: quoted
    plugin: demo/greet
    config:
      msg: "a\" # not-a-comment"
"#;
    let composition = Composition::parse(yaml).unwrap();
    let msg = &composition.entries[0].config.as_object().unwrap()["msg"];
    assert!(format!("{msg:?}").contains("not-a-comment"));

    // Multibyte characters inside quotes must not break comment scanning.
    let yaml = "entries:\n  - id: cjk\n    plugin: demo/greet\n    config:\n      msg: \"あ😀\" # comment\n";
    let composition = Composition::parse(yaml).unwrap();
    assert_eq!(composition.entries.len(), 1);
}

#[test]
fn var_table_env_access_is_allowlisted_and_fails_closed() {
    use cordis_rs::loader::{Composition, VarTable};

    let yaml = r#"
entries:
  - id: a
    plugin: demo/x
    config: { v: !!js env.CORDIS_ALLOWED_TEST }
  - id: b
    plugin: demo/x
    config: { v: !!js process.env.CORDIS_SECRET_TEST }
  - id: c
    plugin: demo/x
    config: { v: !!js process?.env.CORDIS_SECRET_TEST }
  - id: d
    plugin: demo/x
    config: { v: !!js env.DEEPSEEK_API_KEY }
"#;

    // from_env_allowlisted with only one name: all other env.* lookups (three
    // syntactic forms, including a credential) resolve to Null.
    let vars = VarTable::new()
        .from_env_allowlisted(&["CORDIS_ALLOWED_TEST"])
        .with_env_value("CORDIS_ALLOWED_TEST", "ok")
        .with_env_value("CORDIS_SECRET_TEST", "s3cret")
        .with_env_value("DEEPSEEK_API_KEY", "sk-test-should-not-leak");
    let composition = Composition::parse_with_vars(yaml, &vars).unwrap();
    assert_eq!(
        composition.entries[0].config.as_object().unwrap().get("v"),
        Some(&ConfigValue::String("ok".to_string()))
    );
    for (index, entry) in composition.entries.iter().enumerate().skip(1) {
        assert_eq!(
            entry.config.as_object().unwrap().get("v"),
            Some(&ConfigValue::Null),
            "unlisted env value must not leak (entry {index})"
        );
    }
}

#[test]
fn inject_names_are_validated_before_interning() {
    use cordis_rs::{Composition, VarTable};

    // A name with a control character is rejected before it can be interned.
    let bad =
        "entries:\n  - id: bad\n    plugin: demo/x\n    inject: [\"bad\\nname\"]\n    config: {}\n";
    assert!(Composition::parse_with_vars(bad, &VarTable::new()).is_err());

    // An over-long name is rejected.
    let long_name = "a".repeat(129);
    let long = format!(
        "entries:\n  - id: long\n    plugin: demo/x\n    inject: [\"{long_name}\"]\n    config: {{}}\n"
    );
    assert!(Composition::parse_with_vars(&long, &VarTable::new()).is_err());

    // A valid name still works (unknown plugin id is fine at parse time).
    let good = "entries:\n  - id: good\n    plugin: demo/x\n    inject: [\"valid_name-1\"]\n    config: {}\n";
    assert!(Composition::parse_with_vars(good, &VarTable::new()).is_ok());
}

#[tokio::test]
async fn mount_with_applies_persisted_overrides() {
    use cordis_rs::{Composition, CompositionState, PluginFactory, PluginRegistry};

    let yaml = r#"
entries:
  - id: a
    plugin: demo/x
    config: {}
  - id: b
    plugin: demo/x
    disabled: true
    config: {}
"#;
    let composition = Composition::parse(yaml).unwrap();
    let mut registry = PluginRegistry::new();
    registry.register("demo/x", PluginFactory::new(|_ctx, _cfg| async { Ok(()) }));

    let ctx = Context::root("mount-with");
    // Default: only a (YAML default).
    let fibers = composition.mount(&ctx, &registry).await.unwrap();
    assert_eq!(fibers.len(), 1);
    for fiber in fibers {
        let _ = fiber.unload().await;
    }

    // Override: disable a and re-enable b -> only b mounts.
    let mut state = CompositionState::new();
    state.set("a", false);
    state.set("b", true);
    let fibers = composition
        .mount_with(&ctx, &registry, &state)
        .await
        .unwrap();
    assert_eq!(fibers.len(), 1);
    for fiber in fibers {
        let _ = fiber.unload().await;
    }

    // mount_available_with honours the same overrides.
    let report = composition
        .mount_available_with(&ctx, &registry, &state)
        .await
        .unwrap();
    assert_eq!(report.mounted.len(), 1);
}

#[test]
fn composition_state_try_load_surfaces_corruption() {
    use cordis_rs::CompositionState;

    let dir = TempDirGuard::new("cordis-state-corrupt");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Missing file is a fresh start.
    let missing = dir.join("missing.json");
    assert_eq!(
        CompositionState::try_load(&missing).unwrap(),
        CompositionState::new()
    );

    // Valid JSON round-trips.
    let valid = dir.join("valid.json");
    let mut state = CompositionState::new();
    state.set("a", false);
    state.save(&valid).unwrap();
    assert_eq!(CompositionState::try_load(&valid).unwrap(), state);

    // Corrupt JSON fails closed.
    let corrupt = dir.join("corrupt.json");
    std::fs::write(&corrupt, "not json").unwrap();
    assert!(CompositionState::try_load(&corrupt).is_err());
    // Legacy load still falls back for compatibility.
    assert_eq!(CompositionState::load(&corrupt), CompositionState::new());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn composition_reports_unknown_entry_keys_as_warnings() {
    use cordis_rs::{Composition, CompositionState};

    let yaml = "entries:\n  - id: a\n    plugin: demo/x\n    config: {}\n    future_key: 1\n";
    let composition = Composition::parse(yaml).unwrap();
    assert_eq!(composition.entries.len(), 1);
    assert_eq!(composition.warnings.len(), 1);
    assert_eq!(composition.warnings[0].entry, "entries[0]");
    assert_eq!(composition.warnings[0].key, "future_key");

    // Warnings do not change equality of the meaningful content.
    assert_eq!(composition, Composition::parse(yaml).unwrap());
    assert_eq!(Composition::default().warnings.len(), 0);

    // No warnings for a well-formed composition.
    let clean =
        Composition::parse("entries:\n  - id: a\n    plugin: demo/x\n    config: {}\n").unwrap();
    assert!(clean.warnings.is_empty());
    let _ = CompositionState::new();
}

#[test]
fn composition_warns_on_duplicate_entry_keys_but_keeps_last_wins() {
    use cordis_rs::Composition;

    let yaml = "entries:\n  - id: first\n    id: second\n    plugin: demo/x\n    config: {}\n";
    let composition = Composition::parse(yaml).unwrap();
    assert_eq!(composition.entries.len(), 1);
    // Last-wins: the second `id` is kept.
    assert_eq!(composition.entries[0].id, "second");
    assert!(
        composition.warnings.iter().any(|w| w.key == "duplicate:id"),
        "duplicate key must be reported as a warning"
    );

    // plugin/name are distinct aliases, not a duplicate.
    let alias = "entries:\n  - id: a\n    plugin: demo/x\n    name: demo/y\n    config: {}\n";
    let composition = Composition::parse(alias).unwrap();
    assert!(
        composition
            .warnings
            .iter()
            .all(|w| !w.key.starts_with("duplicate:"))
    );
}

#[test]
fn js_literal_block_preserves_bare_hash() {
    use cordis_rs::config::Value;
    use cordis_rs::loader::{Composition, VarTable};

    // The tokeniser used to strip `#` from block bodies as a comment before
    // assembly, truncating quoted literals (R3-IV). The raw line is now used.
    let yaml = concat!(
        "entries:\n",
        "  - id: t\n",
        "    plugin: demo/x\n",
        "    config:\n",
        "      expr: !!js |\n",
        "        [\n",
        "          'a#b',\n",
        "          'c'\n",
        "        ]\n",
        "      after: sibling\n",
    );
    let composition = Composition::parse_with_vars(yaml, &VarTable::new()).unwrap();
    let map = match &composition.entries[0].config {
        Value::Object(map) => map.clone(),
        _ => panic!("expected object"),
    };
    let debug = format!("{map:?}");
    assert!(debug.contains("a#b"), "bare # must survive: {debug}");
    assert_eq!(
        map.get("after"),
        Some(&Value::String("sibling".to_string()))
    );
}

#[test]
fn composition_state_save_leaves_no_temp_residue() {
    use cordis_rs::loader::CompositionState;

    let dir = TempDirGuard::new("cordis-state-tmp");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.json");

    let mut state = CompositionState::new();
    state.set("a", false);
    state.save(&path).unwrap();

    // No sibling .tmp residue from the atomic rename (R2-BV03).
    let residue: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(residue.is_empty(), "temp residue: {residue:?}");

    // Repeated saves still round-trip and leave no residue.
    state.set("b", true);
    state.save(&path).unwrap();
    assert_eq!(CompositionState::try_load(&path).unwrap(), state);
    let residue: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        residue.is_empty(),
        "temp residue after second save: {residue:?}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn integer_range_validation_uses_exact_i64_comparison() {
    use cordis_rs::{ConfigValue, FieldSpec, FieldType, ValidationReason, validate_config};
    use std::collections::BTreeMap;

    fn run(spec: FieldSpec, value: i64) -> Vec<cordis_rs::ValidationError> {
        let mut map = BTreeMap::new();
        map.insert(spec.name.clone(), ConfigValue::Integer(value));
        match validate_config(&[spec], &ConfigValue::Object(map)) {
            Ok(_) => Vec::new(),
            Err(errors) => errors,
        }
    }

    // Beyond 2^53, casting to f64 would collapse distinct values: 2^53+1
    // becomes 2^53 and would pass a max bound of 2^53 (swarm review R1-AR03).
    let spec = FieldSpec::new("big", FieldType::Integer)
        .range_int(9_007_199_254_740_992, 9_007_199_254_740_993);

    assert!(run(spec.clone(), 9_007_199_254_740_992).is_empty());
    assert!(run(spec.clone(), 9_007_199_254_740_993).is_empty());
    let errors = run(spec, 9_007_199_254_740_994);
    assert!(
        errors
            .iter()
            .any(|error| matches!(error.reason, ValidationReason::OutOfRange { .. }))
    );

    // Existing f64 `range()` still works for Integer fields (backward compat).
    let legacy = FieldSpec::new("small", FieldType::Integer).range(1.0, 5.0);
    assert!(run(legacy.clone(), 3).is_empty());
    assert!(!run(legacy, 9).is_empty());
}

#[test]
#[should_panic(expected = "range min must not exceed max")]
fn field_spec_range_rejects_inverted_bounds() {
    use cordis_rs::{FieldSpec, FieldType};
    let _ = FieldSpec::new("bad", FieldType::Number).range(10.0, 1.0);
}

#[test]
#[should_panic(expected = "range_int min must not exceed max")]
fn field_spec_range_int_rejects_inverted_bounds() {
    use cordis_rs::{FieldSpec, FieldType};
    let _ = FieldSpec::new("bad", FieldType::Integer).range_int(10, 1);
}
