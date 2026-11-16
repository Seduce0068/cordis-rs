//! Schemastery-like config validation for plugin `config` blocks.
//!
//! This is intentionally not a Schemastery clone. It covers the three checks
//! the composition loader needs — required fields, primitive/container types
//! (including array element types), and inclusive numeric ranges — using
//! declarative [`FieldSpec`]s and a single [`validate`] pass. Every failure
//! carries a dotted field path so callers can report `$.targets` style errors
//! like the upstream Cordis tutorial.
//!
//! Wheel-reinvention note: `Cargo.toml` is frozen for this PoC, so no schema
//! crate (Schemastery, schemars, a Standard Schema binding) may be added.
//! The validator below is small and deliberately scoped to the documented
//! config shapes; swapping in a real schema engine stays local to the
//! loader's `validate` call site.

use std::collections::BTreeMap;
use std::fmt;

/// Runtime value of a config field after loader resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
    /// A `$name` reference emitted by the loader parser. The loader resolves
    /// these before validation; finding one here means the caller skipped it.
    Reference(String),
}

impl Value {
    /// Human-readable type name used in type-mismatch errors.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Integer(_) => "integer",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
            Self::Reference(_) => "reference",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Self::Object(values) => Some(values),
            _ => None,
        }
    }

    /// An empty config object, handy for plugins without a schema.
    pub fn object() -> Self {
        Self::Object(BTreeMap::new())
    }
}

/// Expected type of a config field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    String,
    Integer,
    Number,
    Bool,
    Array,
    Object,
}

impl FieldType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Bool => "boolean",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Declarative schema for one field (the Schemastery-shaped part of this PoC).
#[derive(Debug, Clone)]
pub struct FieldSpec {
    pub name: String,
    pub ty: FieldType,
    pub required: bool,
    /// Inclusive lower bound for `Integer` / `Number` fields.
    pub min: Option<f64>,
    /// Inclusive upper bound for `Integer` / `Number` fields.
    pub max: Option<f64>,
    pub default: Option<Value>,
    /// Element schema for `FieldType::Array` fields.
    pub element: Option<Box<FieldSpec>>,
    /// Field schemas for `FieldType::Object` fields.
    pub fields: Vec<FieldSpec>,
}

impl FieldSpec {
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        Self {
            name: name.into(),
            ty,
            required: true,
            min: None,
            max: None,
            default: None,
            element: None,
            fields: Vec::new(),
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Supplying a default also makes the field optional.
    pub fn with_default(mut self, value: Value) -> Self {
        self.default = Some(value);
        self.required = false;
        self
    }

    /// Inclusive `[min, max]` range for numeric fields.
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    pub fn min(mut self, min: f64) -> Self {
        self.min = Some(min);
        self
    }

    pub fn max(mut self, max: f64) -> Self {
        self.max = Some(max);
        self
    }

    pub fn elements(mut self, element: FieldSpec) -> Self {
        self.element = Some(Box::new(element));
        self
    }

    pub fn fields(mut self, fields: Vec<FieldSpec>) -> Self {
        self.fields = fields;
        self
    }
}

/// Why a field failed validation.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationReason {
    Missing,
    TypeMismatch {
        expected: FieldType,
        actual: &'static str,
    },
    OutOfRange {
        min: Option<f64>,
        max: Option<f64>,
        actual: f64,
    },
    UnresolvedReference(String),
}

/// A single failure. `path` always contains the offending field name.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    pub path: String,
    pub reason: ValidationReason,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            ValidationReason::Missing => write!(f, "{} is required", self.path),
            ValidationReason::TypeMismatch { expected, actual } => {
                write!(f, "{} expected {expected} but got {actual}", self.path)
            }
            ValidationReason::OutOfRange { min, max, actual } => match (min, max) {
                (Some(min), Some(max)) => {
                    write!(
                        f,
                        "{} out of range [{min}, {max}] (got {actual})",
                        self.path
                    )
                }
                (Some(min), None) => write!(f, "{} must be >= {min} (got {actual})", self.path),
                (None, Some(max)) => write!(f, "{} must be <= {max} (got {actual})", self.path),
                (None, None) => write!(f, "{} out of range (got {actual})", self.path),
            },
            ValidationReason::UnresolvedReference(name) => {
                write!(f, "{} has unresolved reference ${name}", self.path)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// All failures from one validation pass.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationErrors(pub Vec<ValidationError>);

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

impl From<ValidationErrors> for crate::Error {
    fn from(errors: ValidationErrors) -> Self {
        Self::Other(errors.to_string())
    }
}

/// Validate `raw` against `fields`, applying defaults for absent optional
/// fields. On success returns the complete, declared-fields-only object.
/// On failure every [`ValidationError::path`] includes the field name.
pub fn validate(
    fields: &[FieldSpec],
    raw: &Value,
) -> std::result::Result<Value, Vec<ValidationError>> {
    let object = match raw {
        Value::Object(map) => map,
        Value::Reference(name) => {
            return Err(vec![ValidationError {
                path: "$".to_string(),
                reason: ValidationReason::UnresolvedReference(name.clone()),
            }]);
        }
        other => {
            return Err(vec![ValidationError {
                path: "$".to_string(),
                reason: ValidationReason::TypeMismatch {
                    expected: FieldType::Object,
                    actual: other.type_name(),
                },
            }]);
        }
    };

    let mut errors = Vec::new();
    let mut result = BTreeMap::new();
    for spec in fields {
        let path = spec.name.clone();
        match object.get(&spec.name) {
            // YAML `null` and an absent key have the same meaning here.
            None | Some(Value::Null) => {
                if let Some(default) = &spec.default {
                    result.insert(spec.name.clone(), default.clone());
                } else if spec.required {
                    errors.push(ValidationError {
                        path,
                        reason: ValidationReason::Missing,
                    });
                }
            }
            Some(value) => match validate_value(spec, value, &path) {
                Ok(validated) => {
                    result.insert(spec.name.clone(), validated);
                }
                Err(mut found) => errors.append(&mut found),
            },
        }
    }

    if errors.is_empty() {
        Ok(Value::Object(result))
    } else {
        Err(errors)
    }
}

fn validate_value(
    spec: &FieldSpec,
    value: &Value,
    path: &str,
) -> std::result::Result<Value, Vec<ValidationError>> {
    if let Value::Reference(name) = value {
        return Err(vec![ValidationError {
            path: path.to_string(),
            reason: ValidationReason::UnresolvedReference(name.clone()),
        }]);
    }

    let type_ok = match spec.ty {
        FieldType::String => matches!(value, Value::String(_)),
        FieldType::Integer => matches!(value, Value::Integer(_)),
        FieldType::Number => matches!(value, Value::Integer(_) | Value::Number(_)),
        FieldType::Bool => matches!(value, Value::Bool(_)),
        FieldType::Array => matches!(value, Value::Array(_)),
        FieldType::Object => matches!(value, Value::Object(_)),
    };
    if !type_ok {
        return Err(vec![ValidationError {
            path: path.to_string(),
            reason: ValidationReason::TypeMismatch {
                expected: spec.ty,
                actual: value.type_name(),
            },
        }]);
    }

    let mut errors = Vec::new();
    if spec.min.is_some() || spec.max.is_some() {
        let number = match value {
            Value::Integer(number) => Some(*number as f64),
            Value::Number(number) => Some(*number),
            _ => None,
        };
        if let Some(number) = number
            && (spec.min.is_some_and(|min| number < min)
                || spec.max.is_some_and(|max| number > max))
        {
            errors.push(ValidationError {
                path: path.to_string(),
                reason: ValidationReason::OutOfRange {
                    min: spec.min,
                    max: spec.max,
                    actual: number,
                },
            });
        }
    }

    let mut result = value.clone();
    if let (FieldType::Array, Some(element), Value::Array(items)) =
        (spec.ty, spec.element.as_deref(), value)
    {
        let mut validated_items = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let child_path = format!("{path}[{index}]");
            match validate_value(element, item, &child_path) {
                Ok(validated) => validated_items.push(validated),
                Err(mut found) => errors.append(&mut found),
            }
        }
        result = Value::Array(validated_items);
    } else if spec.ty == FieldType::Object && !spec.fields.is_empty() {
        match validate(&spec.fields, value) {
            Ok(validated) => result = validated,
            Err(mut found) => errors.append(&mut found),
        }
    }

    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}
