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
pub struct VarTable(BTreeMap<String, Value>);

impl VarTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: Value) -> &mut Self {
        self.0.insert(name.into(), value);
        self
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
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
        for (key, node) in root.into_map()? {
            if key != "entries" {
                continue;
            }
            for (index, item) in node.into_seq()?.into_iter().enumerate() {
                entries.push(parse_entry(item, index, vars, limits)?);
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
            "plugin" => plugin = Some(expect_scalar(&node, &path)?),
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

    // Bare reference.
    if let Some(name) = reference_name(expression) {
        return vars
            .get(name)
            .cloned()
            .ok_or_else(|| LoaderError::UnresolvedReference {
                path: path.to_string(),
                name: name.to_string(),
            });
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
    if expression.starts_with('"') && expression.ends_with('"') && expression.len() >= 2 {
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
