//! Pure Rust port of `@deepseek-ai/dsh-commands/builtin`.
//!
//! Source: `packages/interaction/commands/src/builtin.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Maps built-in command capabilities to definition IDs and resolves en/zh
//! tokens against command descriptors (exact name wins over alias).

/// A built-in command capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinCommandName {
    Goal,
    Plan,
    Feedback,
    Compact,
    Permission,
    Export,
}

impl BuiltinCommandName {
    pub const ALL: [Self; 6] = [
        Self::Goal,
        Self::Plan,
        Self::Feedback,
        Self::Compact,
        Self::Permission,
        Self::Export,
    ];

    pub fn definition_id(self) -> &'static str {
        match self {
            Self::Goal => "@deepseek-ai/dsh-command-goal",
            Self::Plan => "@deepseek-ai/dsh-plan-mode",
            Self::Feedback => "@deepseek-ai/dsh-command-feedback",
            Self::Compact => "@deepseek-ai/dsh-command-compact",
            Self::Permission => "@deepseek-ai/dsh-permission-presets",
            Self::Export => "@deepseek-ai/dsh-session-log-export",
        }
    }

    pub fn en(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Plan => "plan",
            Self::Feedback => "feedback",
            Self::Compact => "compact",
            Self::Permission => "permission",
            Self::Export => "export",
        }
    }

    pub fn zh(self) -> &'static str {
        match self {
            Self::Goal => "目标",
            Self::Plan => "计划",
            Self::Feedback => "反馈",
            Self::Compact => "压缩",
            Self::Permission => "权限",
            Self::Export => "导出",
        }
    }

    /// English and Chinese tokens for this capability.
    pub fn tokens(self) -> [&'static str; 2] {
        [self.en(), self.zh()]
    }
}

/// Minimal command descriptor (mirrors upstream `CommandDescriptor`).
#[derive(Debug, Clone, PartialEq)]
pub struct CommandDescriptor {
    pub definition_id: Option<String>,
    pub name: String,
    pub description: String,
}

/// Normalize a command name: lowercase, trim whitespace, drop leading `/`.
fn normalize_name(value: &str) -> String {
    value.trim().trim_start_matches('/').to_ascii_lowercase()
}

/// Identify a builtin capability from a definition id.
pub fn builtin_command_name(definition_id: &str) -> Option<BuiltinCommandName> {
    BuiltinCommandName::ALL
        .into_iter()
        .find(|name| name.definition_id() == definition_id)
}

/// Resolve a token against descriptors. Exact name match (normalized, without
/// leading slash) wins; otherwise alias tokens map to a definition id.
pub fn resolve_command(
    token: &str,
    descriptors: &[CommandDescriptor],
) -> Option<CommandDescriptor> {
    let needle = normalize_name(token);
    // Exact name first (preserves scoped shadowing).
    if let Some(found) = descriptors
        .iter()
        .find(|descriptor| normalize_name(&descriptor.name) == needle)
    {
        return Some(found.clone());
    }
    // Alias lookup: zh and en tokens map to a definition id.
    let alias_id = BuiltinCommandName::ALL
        .into_iter()
        .find(|name| needle == name.zh() || needle == name.en())
        .map(|name| name.definition_id());
    let id = alias_id?;
    descriptors
        .iter()
        .find(|descriptor| descriptor.definition_id.as_deref() == Some(id))
        .cloned()
}
