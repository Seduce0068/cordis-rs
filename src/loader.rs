//! Minimal YAML composition loader for `cordis.yml`.
//!
//! Reads the documented subset: a top-level `entries:` sequence whose items
//! carry `id`, `plugin`, `config`, `disabled` and optional `inject` metadata.
//! Scalars, block/flow sequences and maps, comments, quotes and the `!!js`
//! tag are understood; `!!js $name` resolves as a variable reference and
//! `!!js <literal>` is just the literal. There is deliberately no general
//! JavaScript expression evaluator.
//!
//! Wheel-reinvention note: a maintained YAML crate would be the normal
//! choice (`serde_yaml` is deprecated and forwards to `noyalib`; `yaml-rust2`
//! is another live option). This PoC freezes `Cargo.toml`, so adding any
//! dependency is out of scope. The indentation parser below is ~200 lines
//! and intentionally limited to the shipped format. If entries ever gain
//! anchors, multi-document files or real expressions, replace it with a
//! maintained crate instead of growing it further.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use crate::config::{FieldSpec, ValidationError, Value, validate};
use crate::context::Context;
use crate::error::Error;
use crate::events::BoxFuture;
use crate::plugin::{Fiber, FunctionPlugin};

/// Failures produced while parsing or resolving a composition.
#[derive(Debug, Clone, PartialEq)]
pub enum LoaderError {
    Syntax {
        line: usize,
        message: String,
    },
    Structure(String),
    MissingPlugin {
        entry: String,
    },
    UnknownPlugin {
        plugin: String,
        entry: String,
    },
    InvalidDisabled {
        entry: String,
        got: &'static str,
    },
    UnresolvedReference {
        path: String,
        name: String,
    },
    InvalidExpression {
        path: String,
        expression: String,
        reason: String,
    },
    Validation(Vec<ValidationError>),
    Mount(String),
    TooDeep {
        depth: usize,
        max: usize,
    },
}

impl fmt::Display for LoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, message } => write!(f, "line {line}: {message}"),
            Self::Structure(message) => write!(f, "invalid composition: {message}"),
            Self::MissingPlugin { entry } => write!(f, "{entry} is missing `plugin`"),
            Self::UnknownPlugin { plugin, entry } => {
                write!(f, "{entry} references unregistered plugin `{plugin}`")
            }
            Self::InvalidDisabled { entry, got } => {
                write!(f, "{entry}.disabled expected boolean but got {got}")
            }
            Self::UnresolvedReference { path, name } => {
                write!(f, "{path} has unresolved reference ${name}")
            }
            Self::InvalidExpression {
                path,
                expression,
                reason,
            } => write!(
                f,
                "{path} has invalid !!js expression `{expression}`: {reason}"
            ),
            Self::Validation(errors) => {
                f.write_str("invalid config: ")?;
                for (index, error) in errors.iter().enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            }
            Self::Mount(message) => write!(f, "failed to mount fiber: {message}"),
            Self::TooDeep { depth, max } => {
                write!(f, "composition nesting depth {depth} exceeds limit {max}")
            }
        }
    }
}

impl std::error::Error for LoaderError {}

impl From<LoaderError> for Error {
    fn from(error: LoaderError) -> Self {
        Self::Other(error.to_string())
    }
}

/// Values available to `$name` references in config and `disabled` fields.
#[derive(Debug, Clone, Default)]
pub struct VarTable {
    values: BTreeMap<String, Value>,
    /// Base directory used to resolve DSH's `dshHomePath('...')` helper.
    home_path: Option<std::path::PathBuf>,
}

impl VarTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: Value) -> &mut Self {
        self.values.insert(name.into(), value);
        self
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name)
    }

    /// Set the base directory for `dshHomePath('...')` expressions.
    pub fn with_home_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.home_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Insert an `env.NAME` value without mutating the process environment.
    pub fn with_env_value(mut self, name: &str, value: impl Into<String>) -> Self {
        self.values
            .insert(format!("env.{name}"), Value::String(value.into()));
        self
    }

    /// Populate the table with `process.env.*`-style entries from the real
    /// process environment, so `!!js process.env.FOO` resolves in DSH-style
    /// compositions without evaluating JavaScript.
    pub fn from_env(mut self) -> Self {
        for (key, value) in std::env::vars() {
            self.values
                .entry(format!("env.{key}"))
                .or_insert_with(|| Value::String(value));
        }
        self
    }
}

/// One resolved `entries:` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub plugin: String,
    pub config: Value,
    pub disabled: bool,
    /// Dependency names from the entry, merged with `PluginFactory::inject`
    /// by [`Entry::to_plugin`] so they drive `Fiber` waiting too. Because the
    /// existing `Plugin::inject` returns `&[&'static str]`, runtime strings
    /// are interned once per process (see `intern_inject_name`).
    pub inject: Vec<String>,
}

/// A parsed `cordis.yml` composition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Composition {
    pub entries: Vec<Entry>,
}

impl Composition {
    /// Parse without variable substitutions.
    pub fn parse(yaml: &str) -> Result<Self, LoaderError> {
        Self::parse_with_vars(yaml, &VarTable::new())
    }

    /// Parse and resolve `$name` references against `vars`.
    pub fn parse_with_vars(yaml: &str, vars: &VarTable) -> Result<Self, LoaderError> {
        let limits = limits_for(yaml);
        let lines = tokenize(yaml)?;
        let root = if lines.is_empty() {
            Node::Map(Vec::new())
        } else {
            let mut index = 0;
            let indent = lines[0].indent;
            parse_node(&lines, &mut index, indent, limits, 1)?
        };

        let mut entries = Vec::new();
        match root {
            // A bare top-level sequence is the patch form used by DSH's
            // `cordis.yml` snapshot fixtures.
            Node::Seq(items) => {
                let mut flat = Vec::new();
                for item in items {
                    flatten_entry_nodes(item, &mut flat)?;
                }
                for (index, item) in flat.into_iter().enumerate() {
                    entries.push(parse_entry(item, index, vars, limits)?);
                }
            }
            Node::Map(pairs) => {
                for (key, node) in pairs {
                    if key != "entries" {
                        continue;
                    }
                    let mut flat = Vec::new();
                    for item in node.into_seq()? {
                        flatten_entry_nodes(item, &mut flat)?;
                    }
                    for (index, item) in flat.into_iter().enumerate() {
                        entries.push(parse_entry(item, index, vars, limits)?);
                    }
                }
            }
            other => {
                return Err(LoaderError::Structure(format!(
                    "expected a mapping or sequence but got {}",
                    other.kind()
                )));
            }
        }
        Ok(Self { entries })
    }

    /// Entries that should actually be mounted (`disabled: true` is skipped).
    pub fn enabled(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| !entry.disabled)
    }

    /// Validate each enabled entry's config and mount it as a fiber.
    pub async fn mount(
        &self,
        ctx: &Context,
        registry: &PluginRegistry,
    ) -> Result<Vec<Fiber>, LoaderError> {
        let mut fibers = Vec::new();
        for entry in self.enabled() {
            let plugin = entry.to_plugin(registry);
            let result = match plugin {
                Ok(plugin) => Fiber::mount(ctx, Box::new(plugin))
                    .await
                    .map_err(|error| LoaderError::Mount(format!("{}: {error}", entry.id))),
                Err(error) => Err(error),
            };
            match result {
                Ok(fiber) => fibers.push(fiber),
                Err(error) => {
                    // A partial mount must not leave earlier plugins running:
                    // unload in reverse order and surface the original error.
                    for fiber in fibers.drain(..).rev() {
                        let _ = fiber.unload().await;
                    }
                    return Err(error);
                }
            }
        }
        Ok(fibers)
    }

    /// Mount every entry whose plugin id is registered, skipping unknown ones.
    /// Useful for real DSH compositions that reference plugins a port has not
    /// implemented yet: partial support still boots the known subset.
    pub async fn mount_available(
        &self,
        ctx: &Context,
        registry: &PluginRegistry,
    ) -> Result<MountReport, LoaderError> {
        let mut report = MountReport::default();
        for entry in self.enabled() {
            match entry.to_plugin(registry) {
                Ok(plugin) => match Fiber::mount(ctx, Box::new(plugin)).await {
                    Ok(fiber) => report.mounted.push(fiber),
                    Err(error) => report
                        .failed
                        .push((entry.id.clone(), format!("mount: {error}"))),
                },
                Err(LoaderError::UnknownPlugin { plugin, .. }) => {
                    report.skipped_unknown.push((entry.id.clone(), plugin));
                }
                Err(error) => report.failed.push((entry.id.clone(), error.to_string())),
            }
        }
        Ok(report)
    }
}

/// Outcome of a partial composition mount.
#[derive(Default)]
pub struct MountReport {
    pub mounted: Vec<Fiber>,
    /// Entries whose plugin id is not in the registry.
    pub skipped_unknown: Vec<(String, String)>,
    /// Entries that had a factory but failed validation or mounting.
    pub failed: Vec<(String, String)>,
}

impl Entry {
    /// Turn a resolved entry into a `FunctionPlugin`, which `Fiber::mount`
    /// accepts directly. Config is validated here, before `apply` can run.
    /// Dependency names from YAML and from the factory are merged (deduplicated).
    pub fn to_plugin(&self, registry: &PluginRegistry) -> Result<FunctionPlugin, LoaderError> {
        let factory = registry
            .get(&self.plugin)
            .ok_or_else(|| LoaderError::UnknownPlugin {
                plugin: self.plugin.clone(),
                entry: self.id.clone(),
            })?;
        // A plugin without a declared schema receives its config verbatim;
        // a plugin with one receives only validated/declared fields + defaults.
        let config = if factory.fields.is_empty() {
            self.config.clone()
        } else {
            validate(&factory.fields, &self.config).map_err(LoaderError::Validation)?
        };
        let mut inject = factory.inject.clone();
        for name in &self.inject {
            let name = intern_inject_name(name);
            if !inject.contains(&name) {
                inject.push(name);
            }
        }

        let apply = factory.apply.clone();
        let plugin = FunctionPlugin::new(self.id.clone(), move |ctx| {
            let apply = apply.clone();
            let config = config.clone();
            apply(ctx, &config)
        });
        Ok(plugin.with_inject(inject))
    }
}

/// Give a runtime dependency name the `'static` lifetime required by
/// `Plugin::inject() -> &[&'static str]`, without `unsafe`. Each distinct name
/// is leaked at most once per process, so repeated mounts/HMR reloads with the
/// same names do not grow memory without bound.
fn intern_inject_name(name: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<BTreeSet<&'static str>>> = OnceLock::new();
    let names = INTERNED.get_or_init(|| Mutex::new(BTreeSet::new()));
    let mut names = names.lock().expect("inject name intern pool poisoned");
    if let Some(interned) = names.get(name) {
        return interned;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.insert(leaked);
    leaked
}

/// The plugin body registered for a plugin id.
pub type ApplyFn = dyn Fn(&Context, &Value) -> BoxFuture<'static, crate::Result<()>> + Send + Sync;

/// A plugin id's schema, dependencies and apply body.
#[derive(Clone)]
pub struct PluginFactory {
    fields: Vec<FieldSpec>,
    inject: Vec<&'static str>,
    apply: Arc<ApplyFn>,
}

impl PluginFactory {
    pub fn new<F, Fut>(apply: F) -> Self
    where
        F: Fn(&Context, &Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<()>> + Send + 'static,
    {
        Self {
            fields: Vec::new(),
            inject: Vec::new(),
            apply: Arc::new(move |ctx, config| {
                Box::pin(apply(ctx, config)) as BoxFuture<'static, crate::Result<()>>
            }),
        }
    }

    /// Config schema applied before the plugin body runs.
    pub fn with_schema(mut self, fields: Vec<FieldSpec>) -> Self {
        self.fields = fields;
        self
    }

    /// Services that must exist before the plugin body runs.
    pub fn with_inject(mut self, inject: Vec<&'static str>) -> Self {
        self.inject = inject;
        self
    }

    pub fn fields(&self) -> &[FieldSpec] {
        &self.fields
    }

    pub fn inject(&self) -> &[&'static str] {
        &self.inject
    }
}

/// Maps the YAML `plugin:` string to a [`PluginFactory`].
#[derive(Default)]
pub struct PluginRegistry {
    factories: BTreeMap<String, PluginFactory>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, id: impl Into<String>, factory: PluginFactory) -> &mut Self {
        self.factories.insert(id.into(), factory);
        self
    }

    pub fn get(&self, id: &str) -> Option<&PluginFactory> {
        self.factories.get(id)
    }
}

// ---------------------------------------------------------------------------
// Lightweight YAML subset parser (internal).
// ---------------------------------------------------------------------------

/// Recursion budget derived from the input size. A larger document may nest
/// deeper, but the cap keeps pathological input from exhausting the stack.
#[derive(Debug, Clone, Copy)]
struct Limits {
    max_depth: usize,
}

fn limits_for(input: &str) -> Limits {
    Limits {
        max_depth: (input.len() / 64 + 8).min(128),
    }
}

fn check_depth(limits: Limits, depth: usize) -> Result<(), LoaderError> {
    if depth > limits.max_depth {
        Err(LoaderError::TooDeep {
            depth,
            max: limits.max_depth,
        })
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct Line {
    indent: usize,
    text: String,
    number: usize,
}

#[derive(Debug, Clone)]
struct Scalar {
    raw: String,
    /// Quoted scalars keep `$name` as a literal string.
    quoted: bool,
    /// Set when the source used the `!!js` tag.
    tagged: bool,
}

#[derive(Debug, Clone)]
enum Node {
    Scalar(Scalar),
    Seq(Vec<Node>),
    Map(Vec<(String, Node)>),
}

impl Node {
    fn null() -> Self {
        Self::Scalar(Scalar {
            raw: String::new(),
            quoted: false,
            tagged: false,
        })
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Scalar(_) => "scalar",
            Self::Seq(_) => "sequence",
            Self::Map(_) => "mapping",
        }
    }

    fn into_map(self) -> Result<Vec<(String, Node)>, LoaderError> {
        match self {
            Self::Map(map) => Ok(map),
            other => Err(LoaderError::Structure(format!(
                "expected a mapping but got {}",
                other.kind()
            ))),
        }
    }

    fn into_seq(self) -> Result<Vec<Node>, LoaderError> {
        match self {
            Self::Seq(seq) => Ok(seq),
            other => Err(LoaderError::Structure(format!(
                "`entries` must be a sequence but got {}",
                other.kind()
            ))),
        }
    }
}

fn tokenize(input: &str) -> Result<Vec<Line>, LoaderError> {
    let mut lines = Vec::new();
    for (index, raw) in input.lines().enumerate() {
        let trimmed_end = strip_comment(raw).trim_end();
        if trimmed_end.trim().is_empty() {
            continue;
        }
        let indent = trimmed_end.len() - trimmed_end.trim_start().len();
        if trimmed_end[..indent].contains('\t') {
            return Err(LoaderError::Syntax {
                line: index + 1,
                message: "tabs are not valid indentation".to_string(),
            });
        }
        lines.push(Line {
            indent,
            text: trimmed_end.trim_start().to_string(),
            number: index + 1,
        });
    }
    Ok(lines)
}

/// Remove a `#` comment that starts at line level or after whitespace,
/// ignoring `#` inside single or double quotes.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut previous_was_space = true;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' if !in_double => {
                if in_single && index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                in_single = !in_single;
            }
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single && !in_double && previous_was_space => return &line[..index],
            _ => {}
        }
        previous_was_space = bytes[index].is_ascii_whitespace();
        index += 1;
    }
    line
}

/// Split `key: value` at the first colon outside quotes and flow brackets.
fn split_key_value(text: &str) -> Option<(String, &str)> {
    let bytes = text.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth = 0_i32;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' if !in_double => {
                if in_single && index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                in_single = !in_single;
            }
            b'"' if !in_single => in_double = !in_double,
            b'[' | b'{' if !in_single && !in_double => depth += 1,
            b']' | b'}' if !in_single && !in_double => depth -= 1,
            b':' if !in_single && !in_double && depth == 0 => {
                let rest = &bytes[index + 1..];
                if rest.is_empty() || rest[0].is_ascii_whitespace() {
                    let key = text[..index].trim();
                    if key.is_empty() {
                        return None;
                    }
                    return Some((unquote(key).0, text[index + 1..].trim()));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Split a flow collection on a separator, respecting quotes and nesting.
fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let bytes = text.as_bytes();
    let separator = separator as u8;
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut depth = 0_i32;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' if !in_double => {
                if in_single && index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                in_single = !in_single;
            }
            b'"' if !in_single => in_double = !in_double,
            b'[' | b'{' if !in_single && !in_double => depth += 1,
            b']' | b'}' if !in_single && !in_double => depth -= 1,
            byte if byte == separator && !in_single && !in_double && depth == 0 => {
                parts.push(text[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    parts.push(text[start..].trim().to_string());
    parts
}

/// Remove quotes and record whether the source was quoted.
fn unquote(text: &str) -> (String, bool) {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
        (text[1..text.len() - 1].replace("''", "'"), true)
    } else if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        (unescape_double(&text[1..text.len() - 1]), true)
    } else {
        (text.to_string(), false)
    }
}

fn unescape_double(inner: &str) -> String {
    let mut output = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some('r') => output.push('\r'),
                Some('"') => output.push('"'),
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn scalar_from_text(text: &str) -> Scalar {
    let (raw, quoted) = unquote(text);
    Scalar {
        raw,
        quoted,
        tagged: false,
    }
}

fn parse_inline(raw: &str, limits: Limits, depth: usize) -> Result<Node, LoaderError> {
    check_depth(limits, depth)?;
    let mut text = raw.trim();
    let mut tagged = false;
    if let Some(rest) = text.strip_prefix("!!js") {
        tagged = true;
        text = rest.trim();
    }

    if text.starts_with('[') && text.ends_with(']') {
        let inner = &text[1..text.len() - 1];
        let mut items = Vec::new();
        for part in split_top_level(inner, ',') {
            if !part.is_empty() {
                items.push(parse_inline(&part, limits, depth + 1)?);
            }
        }
        Ok(Node::Seq(items))
    } else if text.starts_with('{') && text.ends_with('}') {
        let inner = &text[1..text.len() - 1];
        let mut map = Vec::new();
        for part in split_top_level(inner, ',') {
            if part.is_empty() {
                continue;
            }
            let (key, value) = split_key_value(&part).ok_or_else(|| {
                LoaderError::Structure(format!("invalid inline mapping entry `{part}`"))
            })?;
            map.push((key, parse_inline(value, limits, depth + 1)?));
        }
        Ok(Node::Map(map))
    } else {
        let mut scalar = scalar_from_text(text);
        scalar.tagged = tagged;
        Ok(Node::Scalar(scalar))
    }
}

fn parse_node(
    lines: &[Line],
    index: &mut usize,
    indent: usize,
    limits: Limits,
    depth: usize,
) -> Result<Node, LoaderError> {
    check_depth(limits, depth)?;
    if is_dash(&lines[*index].text) {
        parse_seq(lines, index, indent, limits, depth)
    } else {
        parse_map(lines, index, indent, limits, depth)
    }
}

fn is_dash(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

fn parse_map(
    lines: &[Line],
    index: &mut usize,
    indent: usize,
    limits: Limits,
    depth: usize,
) -> Result<Node, LoaderError> {
    let mut map = Vec::new();
    while *index < lines.len() && lines[*index].indent == indent && !is_dash(&lines[*index].text) {
        let text = lines[*index].text.clone();
        let (key, raw_value) = split_key_value(&text).ok_or_else(|| LoaderError::Syntax {
            line: lines[*index].number,
            message: format!("expected `key: value` but got `{text}`"),
        })?;
        *index += 1;
        let value = parse_value_after_key(lines, index, indent, raw_value, limits, depth + 1)?;
        map.push((key, value));
    }
    Ok(Node::Map(map))
}

fn parse_seq(
    lines: &[Line],
    index: &mut usize,
    indent: usize,
    limits: Limits,
    depth: usize,
) -> Result<Node, LoaderError> {
    let mut items = Vec::new();
    while *index < lines.len() && lines[*index].indent == indent && is_dash(&lines[*index].text) {
        let text = lines[*index].text.clone();
        let rest = text[1..].trim_start();
        // The first mapping key starts after `- ` (the dash, its following
        // spaces), not at the dash column. Undercounting makes a nested
        // block look like a sibling and swallows `id` / `plugin` into config.
        let key_indent = indent + text.len() - rest.len();
        *index += 1;

        if rest.is_empty() {
            items.push(parse_nested_or_null(
                lines,
                index,
                indent,
                limits,
                depth + 1,
            )?);
        } else if let Some((key, raw_value)) = split_key_value(rest) {
            // A sequence item can start a mapping on the dash line itself:
            // `- id: greeter` followed by `  plugin: ...`.
            let value =
                parse_value_after_key(lines, index, key_indent, raw_value, limits, depth + 1)?;
            let mut map = vec![(key, value)];
            while *index < lines.len()
                && lines[*index].indent > indent
                && !is_dash(&lines[*index].text)
            {
                let sibling_indent = lines[*index].indent;
                let sibling = lines[*index].text.clone();
                let (key, raw_value) =
                    split_key_value(&sibling).ok_or_else(|| LoaderError::Syntax {
                        line: lines[*index].number,
                        message: format!("expected `key: value` but got `{sibling}`"),
                    })?;
                *index += 1;
                map.push((
                    key,
                    parse_value_after_key(
                        lines,
                        index,
                        sibling_indent,
                        raw_value,
                        limits,
                        depth + 1,
                    )?,
                ));
            }
            items.push(Node::Map(map));
        } else {
            items.push(parse_inline(rest, limits, depth + 1)?);
        }
    }
    Ok(Node::Seq(items))
}

fn parse_nested_or_null(
    lines: &[Line],
    index: &mut usize,
    indent: usize,
    limits: Limits,
    depth: usize,
) -> Result<Node, LoaderError> {
    if *index < lines.len() && lines[*index].indent > indent {
        let child_indent = lines[*index].indent;
        parse_node(lines, index, child_indent, limits, depth)
    } else {
        Ok(Node::null())
    }
}

fn parse_value_after_key(
    lines: &[Line],
    index: &mut usize,
    indent: usize,
    raw_value: &str,
    limits: Limits,
    depth: usize,
) -> Result<Node, LoaderError> {
    if raw_value.is_empty() {
        parse_nested_or_null(lines, index, indent, limits, depth)
    } else {
        parse_inline(raw_value, limits, depth)
    }
}

/// Evaluate known DSH helper calls. Returns `Ok(None)` for other forms.
fn evaluate_helper_call(
    expression: &str,
    vars: &VarTable,
    path: &str,
) -> Result<Option<Value>, LoaderError> {
    let Some(open) = expression.find('(') else {
        return Ok(None);
    };
    if !expression.ends_with(')') {
        return Ok(None);
    }
    let function = expression[..open].trim();
    let argument = expression[open + 1..expression.len() - 1].trim();
    let argument = argument
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            argument
                .strip_prefix('\'')
                .and_then(|v| v.strip_suffix('\''))
        })
        .unwrap_or(argument);

    match function {
        "dshHomePath" => {
            let Some(home) = &vars.home_path else {
                return Err(LoaderError::InvalidExpression {
                    path: path.to_string(),
                    expression: expression.to_string(),
                    reason: "dshHomePath requires VarTable::with_home_path".to_string(),
                });
            };
            Ok(Some(Value::String(
                home.join(argument).to_string_lossy().to_string(),
            )))
        }
        "String" => Ok(Some(Value::String(argument.to_string()))),
        "Number" => argument
            .parse::<i64>()
            .map(|value| Some(Value::Integer(value)))
            .map_err(|_| LoaderError::InvalidExpression {
                path: path.to_string(),
                expression: expression.to_string(),
                reason: format!("Number({argument:?}) is not an integer"),
            }),
        _ => Ok(None),
    }
}

/// Flatten upstream's nested `insert:` rows into a single entry list.
fn flatten_entry_nodes(node: Node, output: &mut Vec<Node>) -> Result<(), LoaderError> {
    match node {
        Node::Seq(items) => {
            for item in items {
                flatten_entry_nodes(item, output)?;
            }
            Ok(())
        }
        Node::Map(entries) => {
            let mut inserted = Vec::new();
            let mut normal = Vec::new();
            for (key, child) in entries {
                if key == "insert" {
                    inserted.push(child);
                } else {
                    normal.push((key, child));
                }
            }
            for child in inserted {
                flatten_entry_nodes(child, output)?;
            }
            if !normal.is_empty() {
                output.push(Node::Map(normal));
            }
            Ok(())
        }
        other => {
            output.push(other);
            Ok(())
        }
    }
}

fn parse_entry(
    node: Node,
    position: usize,
    vars: &VarTable,
    limits: Limits,
) -> Result<Entry, LoaderError> {
    let label = format!("entries[{position}]");
    let mut id = None;
    let mut plugin = None;
    let mut config = Value::object();
    let mut disabled = false;
    let mut inject = Vec::new();

    for (key, node) in node.into_map()? {
        let path = format!("{label}.{key}");
        match key.as_str() {
            "id" => id = Some(expect_scalar(&node, &path)?),
            // Upstream compositions use `name:` for the plugin package id;
            // `plugin:` is accepted as a compatible alias.
            "plugin" | "name" => plugin = Some(expect_scalar(&node, &path)?),
            "config" => {
                let value = node_to_value(&node);
                let value = if value == Value::Null {
                    Value::object()
                } else {
                    value
                };
                config = resolve_value(value, vars, &path, limits, 1)?;
            }
            "disabled" => {
                let value = resolve_value(node_to_value(&node), vars, &path, limits, 1)?;
                disabled = match value {
                    Value::Bool(flag) => flag,
                    other => {
                        return Err(LoaderError::InvalidDisabled {
                            entry: label.clone(),
                            got: other.type_name(),
                        });
                    }
                };
            }
            "inject" => {
                let Value::Array(items) = node_to_value(&node) else {
                    return Err(LoaderError::Structure(format!(
                        "{path} must be a sequence of strings"
                    )));
                };
                for item in items {
                    let Value::String(name) = item else {
                        return Err(LoaderError::Structure(format!(
                            "{path} must be a sequence of strings"
                        )));
                    };
                    inject.push(name);
                }
            }
            _ => {}
        }
    }

    if !matches!(config, Value::Object(_)) {
        return Err(LoaderError::Structure(format!(
            "{label}.config must be a mapping"
        )));
    }
    let plugin = plugin.ok_or(LoaderError::MissingPlugin { entry: label })?;
    Ok(Entry {
        id: id.unwrap_or_else(|| plugin.clone()),
        plugin,
        config,
        disabled,
        inject,
    })
}

fn expect_scalar(node: &Node, path: &str) -> Result<String, LoaderError> {
    match node {
        Node::Scalar(scalar) => Ok(scalar.raw.clone()),
        other => Err(LoaderError::Structure(format!(
            "{path} must be a scalar but got {}",
            other.kind()
        ))),
    }
}

fn node_to_value(node: &Node) -> Value {
    match node {
        Node::Scalar(scalar) => scalar_to_value(scalar),
        Node::Seq(items) => Value::Array(items.iter().map(node_to_value).collect()),
        Node::Map(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), node_to_value(value)))
                .collect(),
        ),
    }
}

fn scalar_to_value(scalar: &Scalar) -> Value {
    if scalar.tagged {
        // `!!js` marks an expression. `$name` stays a direct reference;
        // `${name}` inside a larger string is interpolated at resolve time.
        return Value::Reference(format!("expr:{}", scalar.raw));
    }
    if let Some(name) = (!scalar.quoted)
        .then(|| reference_name(&scalar.raw))
        .flatten()
    {
        return Value::Reference(name.to_string());
    }
    match scalar.raw.as_str() {
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        "null" | "Null" | "NULL" | "~" | "" => Value::Null,
        raw => {
            if let Ok(number) = raw.parse::<i64>() {
                Value::Integer(number)
            } else if let Ok(number) = raw.parse::<f64>() {
                Value::Number(number)
            } else {
                Value::String(raw.to_string())
            }
        }
    }
}

/// Evaluate a `!!js` expression. Supported forms mirror the subset of
/// `cordis-plugin-include` that composition files use: a bare `$name`,
/// `${name}` interpolation inside a larger string, JSON-like literals
/// (`true` / `false` / `null` / numbers / quoted strings), and a flow array
/// of those. Anything else stays a literal string.
fn evaluate_expression(
    expression: &str,
    vars: &VarTable,
    path: &str,
) -> Result<Value, LoaderError> {
    let expression = expression.trim();

    // Ternary: `condition ? when_true : when_false`. The condition is
    // evaluated first, so references and comparisons work on both sides.
    if let Some((condition, rest)) = split_top_level_once(expression, '?') {
        let (when_true, when_false) =
            split_top_level_once(rest, ':').ok_or_else(|| LoaderError::InvalidExpression {
                path: path.to_string(),
                expression: expression.to_string(),
                reason: "ternary needs `:`".to_string(),
            })?;
        let truthy = expression_truthy(&evaluate_expression(condition, vars, path)?);
        let selected = if truthy { when_true } else { when_false };
        return evaluate_expression(selected.trim(), vars, path);
    }

    // Logical OR: lowest precedence among the logical operators.
    if let Some((left, right)) = split_once_top_level(expression, "||") {
        let left_truthy = expression_truthy(&evaluate_expression(left.trim(), vars, path)?);
        let right_truthy = expression_truthy(&evaluate_expression(right.trim(), vars, path)?);
        return Ok(Value::Bool(left_truthy || right_truthy));
    }

    // Logical AND.
    if let Some((left, right)) = split_once_top_level(expression, "&&") {
        let left_truthy = expression_truthy(&evaluate_expression(left.trim(), vars, path)?);
        let right_truthy = expression_truthy(&evaluate_expression(right.trim(), vars, path)?);
        return Ok(Value::Bool(left_truthy && right_truthy));
    }

    // Comparisons: `==`, `!=`, `>=`, `<=`, `>`, `<`. Operands are evaluated
    // as expressions and compared structurally; the result is a Bool.
    for operator in ["===", "!==", "==", "!=", ">=", "<=", ">", "<"] {
        if let Some((left, right)) = split_once_top_level(expression, operator) {
            let left_value = evaluate_expression(left.trim(), vars, path)?;
            let right_value = evaluate_expression(right.trim(), vars, path)?;
            return Ok(Value::Bool(compare_values(
                &left_value,
                operator,
                &right_value,
            )?));
        }
    }

    // Logical NOT (prefix). Placed after comparisons so `!=` is consumed first.
    if let Some(inner) = expression.strip_prefix('!') {
        let truthy = expression_truthy(&evaluate_expression(inner.trim(), vars, path)?);
        return Ok(Value::Bool(!truthy));
    }

    // DSH helper calls: `dshHomePath('sessions')` resolves against the
    // VarTable's configured home path. Other helpers are left as literals.
    if let Some(result) = evaluate_helper_call(expression, vars, path)? {
        return Ok(result);
    }

    // Member access: `$name[0]` (array index) or `$name.length` / `[0].length`.
    // Applied left-to-right so chains such as `$items[1].length` work.
    if let Some(result) = evaluate_member_access(expression, vars, path)? {
        return Ok(result);
    }

    // `undefined` is a JS literal, not a variable reference.
    if expression == "undefined" {
        return Ok(Value::Null);
    }

    // Bare reference: `$name` or a plain identifier inside `!!js`.
    let bare_name = reference_name(expression).or_else(|| {
        (!expression.is_empty()
            && expression
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            && expression
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic()))
        .then_some(expression)
    });
    if let Some(name) = bare_name {
        if let Some(value) = vars.get(name) {
            return Ok(value.clone());
        }
        // `$name` must resolve; a plain identifier that is not a variable is
        // kept as a literal string (e.g. `fast`).
        if name.starts_with('$') || name.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_') {
            return Err(LoaderError::UnresolvedReference {
                path: path.to_string(),
                name: name.to_string(),
            });
        }
    }

    // `${name}` interpolation: replace every occurrence with a textual form.
    if expression.contains("${") {
        let mut rendered = String::new();
        let mut rest = expression;
        while let Some(start) = rest.find("${") {
            rendered.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after
                .find('}')
                .ok_or_else(|| LoaderError::InvalidExpression {
                    path: path.to_string(),
                    expression: expression.to_string(),
                    reason: "unterminated ${...}".to_string(),
                })?;
            let name = &after[..end];
            let value = vars
                .get(name)
                .ok_or_else(|| LoaderError::UnresolvedReference {
                    path: path.to_string(),
                    name: name.to_string(),
                })?;
            rendered.push_str(&value_to_text(value));
            rest = &after[end + 1..];
        }
        rendered.push_str(rest);
        return Ok(Value::String(rendered));
    }

    // Literal forms.
    match expression {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        "null" | "" => return Ok(Value::Null),
        _ => {}
    }
    if let Ok(number) = expression.parse::<i64>() {
        return Ok(Value::Integer(number));
    }
    if let Ok(number) = expression.parse::<f64>() {
        return Ok(Value::Number(number));
    }
    // Upstream wraps JS expressions in quotes in YAML, e.g.
    // `!!js "process.env.FLAG !== '1'"`. When the quoted body contains a
    // recognizable operator or env reference, evaluate the inner expression.
    if expression.starts_with('"') && expression.ends_with('"') && expression.len() >= 2 {
        let inner = &expression[1..expression.len() - 1];
        let looks_like_expression = inner.contains("process.env")
            || inner.contains("$")
            || inner.contains("==")
            || inner.contains("!=")
            || inner.contains("&&")
            || inner.contains("||");
        if looks_like_expression {
            return evaluate_expression(inner, vars, path);
        }
    }
    if expression.starts_with('"') && expression.ends_with('"') && expression.len() >= 2 {
        return Ok(Value::String(
            expression[1..expression.len() - 1].to_string(),
        ));
    }
    if expression.starts_with('\'') && expression.ends_with('\'') && expression.len() >= 2 {
        return Ok(Value::String(
            expression[1..expression.len() - 1].to_string(),
        ));
    }
    if expression.starts_with('[') && expression.ends_with(']') {
        let inner = &expression[1..expression.len() - 1];
        let mut items = Vec::new();
        for part in split_flow(inner) {
            items.push(evaluate_expression(part.trim(), vars, path)?);
        }
        return Ok(Value::Array(items));
    }
    Ok(Value::String(expression.to_string()))
}

/// Split at the first top-level occurrence of `separator`, ignoring quoted
/// strings and nested brackets. Used for ternaries and comparisons.
fn split_once_top_level<'a>(input: &'a str, separator: &str) -> Option<(&'a str, &'a str)> {
    let bytes = input.as_bytes();
    let separator_bytes = separator.as_bytes();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        match ch {
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
            }
            '[' | '{' | '(' if quote.is_none() => depth += 1,
            ']' | '}' | ')' if quote.is_none() => depth = depth.saturating_sub(1),
            _ => {}
        }
        if quote.is_none() && depth == 0 && bytes[index..].starts_with(separator_bytes) {
            return Some((&input[..index], &input[index + separator.len()..]));
        }
        index += 1;
    }
    None
}

fn split_top_level_once(input: &str, separator: char) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (index, ch) in input.char_indices() {
        match ch {
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
            }
            '[' | '{' | '(' if quote.is_none() => depth += 1,
            ']' | '}' | ')' if quote.is_none() => depth = depth.saturating_sub(1),
            c if c == separator && quote.is_none() && depth == 0 => {
                return Some((&input[..index], &input[index + c.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

fn expression_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Integer(value) => *value != 0,
        Value::Number(value) => *value != 0.0,
        Value::String(value) => !value.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        Value::Reference(name) => !name.is_empty(),
    }
}

fn compare_values(left: &Value, operator: &str, right: &Value) -> Result<bool, LoaderError> {
    let equal = left == right;
    match operator {
        "===" | "==" => Ok(equal),
        "!==" | "!=" => Ok(!equal),
        ">" | "<" | ">=" | "<=" => {
            let ordering = match (left, right) {
                (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
                (Value::Number(a), Value::Number(b)) => a
                    .partial_cmp(b)
                    .ok_or_else(|| LoaderError::Structure("non-comparable numbers".to_string()))?,
                (Value::String(a), Value::String(b)) => a.cmp(b),
                _ => {
                    return Err(LoaderError::Structure(format!(
                        "cannot compare {left:?} and {right:?}"
                    )));
                }
            };
            Ok(match operator {
                ">" => ordering.is_gt(),
                "<" => ordering.is_lt(),
                ">=" => ordering.is_ge(),
                _ => ordering.is_le(),
            })
        }
        _ => Err(LoaderError::Structure(format!(
            "unknown operator {operator}"
        ))),
    }
}

/// Evaluate `$name[0]`, `$name.length`, `$name.0` and chains thereof.
/// Returns `Ok(None)` when the expression has no member access at all.
fn evaluate_member_access(
    expression: &str,
    vars: &VarTable,
    path: &str,
) -> Result<Option<Value>, LoaderError> {
    // Split the leading identifier from the rest of the chain.
    let bytes = expression.as_bytes();
    let mut end = 0usize;
    while end < bytes.len() {
        let ch = bytes[end] as char;
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || (ch == '$' && end == 0) {
            end += 1;
        } else {
            break;
        }
    }
    if end == 0 || end == expression.len() {
        return Ok(None);
    }
    let head = &expression[..end];
    let tail = &expression[end..];
    if !tail.starts_with('[') && !tail.starts_with('.') {
        return Ok(None);
    }

    let name = head.strip_prefix('$').unwrap_or(head);
    let mut value = vars
        .get(name)
        .cloned()
        .unwrap_or(Value::String(head.to_string()));

    // DSH-style `process.env.FOO` / `env.FOO` lookups resolve against the
    // VarTable's `env.FOO` entries (populated by `VarTable::from_env`).
    if (head == "process" && tail.starts_with(".env.")) || head == "env" {
        let env_name = if head == "process" {
            &tail[".env.".len()..]
        } else {
            &tail[1..]
        };
        let key = format!("env.{env_name}");
        return Ok(Some(vars.get(&key).cloned().unwrap_or(Value::Null)));
    }

    let mut rest = tail;
    while !rest.is_empty() {
        if let Some(after_open) = rest.strip_prefix('[') {
            let close = after_open
                .find(']')
                .ok_or_else(|| LoaderError::InvalidExpression {
                    path: path.to_string(),
                    expression: expression.to_string(),
                    reason: "unterminated [".to_string(),
                })?;
            let index_text = after_open[..close].trim();
            let index: usize = index_text
                .parse()
                .map_err(|_| LoaderError::InvalidExpression {
                    path: path.to_string(),
                    expression: expression.to_string(),
                    reason: format!("invalid array index {index_text:?}"),
                })?;
            value = match value {
                Value::Array(items) => items.get(index).cloned().unwrap_or(Value::Null),
                Value::Object(map) => map.get(index_text).cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            rest = &after_open[close + 1..];
        } else if let Some(after_dot) = rest.strip_prefix('.') {
            let mut prop_end = 0usize;
            for ch in after_dot.chars() {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    prop_end += ch.len_utf8();
                } else {
                    break;
                }
            }
            if prop_end == 0 {
                return Err(LoaderError::InvalidExpression {
                    path: path.to_string(),
                    expression: expression.to_string(),
                    reason: "expected property after '.'".to_string(),
                });
            }
            let property = &after_dot[..prop_end];
            value = match (&value, property) {
                (Value::String(text), "length") => Value::Integer(text.chars().count() as i64),
                (Value::Array(items), "length") => Value::Integer(items.len() as i64),
                (Value::Object(map), _) => map.get(property).cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            rest = &after_dot[prop_end..];
        } else {
            break;
        }
    }
    Ok(Some(value))
}

fn split_flow(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut quote: Option<char> = None;
    for (index, ch) in input.char_indices() {
        match ch {
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
            }
            '[' | '{' if quote.is_none() => depth += 1,
            ']' | '}' if quote.is_none() => depth = depth.saturating_sub(1),
            ',' if quote.is_none() && depth == 0 => {
                parts.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if !input[start..].trim().is_empty() {
        parts.push(&input[start..]);
    }
    parts
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Integer(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Reference(name) => format!("${name}"),
        Value::Array(_) | Value::Object(_) => "".to_string(),
    }
}

/// `$name` in unquoted or `!!js`-tagged scalars is a variable reference.
fn reference_name(raw: &str) -> Option<&str> {
    let name = raw.strip_prefix('$')?;
    if !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        Some(name)
    } else {
        None
    }
}

fn resolve_value(
    value: Value,
    vars: &VarTable,
    path: &str,
    limits: Limits,
    depth: usize,
) -> Result<Value, LoaderError> {
    check_depth(limits, depth)?;
    match value {
        Value::Reference(name) => {
            if let Some(expression) = name.strip_prefix("expr:") {
                return evaluate_expression(expression, vars, path);
            }
            vars.get(&name)
                .cloned()
                .ok_or_else(|| LoaderError::UnresolvedReference {
                    path: path.to_string(),
                    name,
                })
        }
        Value::Array(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                resolved.push(resolve_value(
                    item,
                    vars,
                    &format!("{path}[{index}]"),
                    limits,
                    depth + 1,
                )?);
            }
            Ok(Value::Array(resolved))
        }
        Value::Object(map) => {
            let mut resolved = BTreeMap::new();
            for (key, item) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                resolved.insert(
                    key,
                    resolve_value(item, vars, &child_path, limits, depth + 1)?,
                );
            }
            Ok(Value::Object(resolved))
        }
        other => Ok(other),
    }
}
