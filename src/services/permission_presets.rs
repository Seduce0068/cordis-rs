//! `ctx.permissionPresets`: user-facing permission presets.
//!
//! Upstream: `packages/interaction/permission-presets/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! A preset bundles the independent sandbox-mode and approval-policy knobs.
//! Switching writes both knobs and records the chosen preset as user intent.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPolicy {
    Never,
    Ask,
}

impl ApprovalPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Ask => "ask",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "read-only" => Some(Self::ReadOnly),
            "workspace-write" => Some(Self::WorkspaceWrite),
            "danger-full-access" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }

    /// The knob bundle this preset writes through.
    pub fn bundle(self) -> (SandboxMode, ApprovalPolicy) {
        match self {
            Self::ReadOnly => (SandboxMode::ReadOnly, ApprovalPolicy::Never),
            Self::WorkspaceWrite => (SandboxMode::WorkspaceWrite, ApprovalPolicy::Never),
            Self::DangerFullAccess => (SandboxMode::DangerFullAccess, ApprovalPolicy::Ask),
        }
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PermissionState {
    pub preset: Option<Preset>,
    pub sandbox: SandboxMode,
    pub approval: ApprovalPolicy,
}

#[derive(Debug, Clone)]
pub struct PresetSpec {
    pub sandbox: SandboxMode,
    pub approval: ApprovalPolicy,
    pub label: Option<String>,
    pub description: Option<String>,
}

struct RegistryState {
    presets: BTreeMap<String, PresetSpec>,
    state: PermissionState,
    /// When set, switching a preset also updates the approval gate policy.
    approval: Option<crate::services::approval::ApprovalService>,
}

/// The permission preset registry.
#[derive(Clone)]
pub struct PermissionPresets {
    state: Arc<RwLock<RegistryState>>,
}

impl Default for PermissionPresets {
    fn default() -> Self {
        let mut presets = BTreeMap::new();
        for preset in [
            Preset::ReadOnly,
            Preset::WorkspaceWrite,
            Preset::DangerFullAccess,
        ] {
            let (sandbox, approval) = preset.bundle();
            presets.insert(
                preset.as_str().to_string(),
                PresetSpec {
                    sandbox,
                    approval,
                    label: None,
                    description: None,
                },
            );
        }
        Self {
            state: Arc::new(RwLock::new(RegistryState {
                presets,
                state: PermissionState {
                    preset: None,
                    sandbox: SandboxMode::WorkspaceWrite,
                    approval: ApprovalPolicy::Never,
                },
                approval: None,
            })),
        }
    }
}

impl PermissionPresets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind an approval service so switching presets updates its mode.
    pub fn bind_approval(&self, approval: crate::services::approval::ApprovalService) {
        self.state
            .write()
            .expect("permission presets poisoned")
            .approval = Some(approval);
    }

    /// Switch to a preset, writing both knobs. Unknown presets are rejected.
    pub fn switch(&self, name: &str) -> std::result::Result<PermissionState, Error> {
        let mut guard = self.state.write().expect("permission presets poisoned");
        let spec = guard
            .presets
            .get(name)
            .cloned()
            .ok_or_else(|| Error::Other(format!("unknown permission preset: {name}")))?;
        let preset = Preset::parse(name)
            .ok_or_else(|| Error::Other(format!("preset has no canonical identity: {name}")))?;
        guard.state = PermissionState {
            preset: Some(preset),
            sandbox: spec.sandbox,
            approval: spec.approval,
        };
        let snapshot = guard.state.clone();
        let approval = guard.approval.clone();
        drop(guard);
        if let Some(approval) = approval {
            use crate::services::approval::ApprovalMode;
            approval.set_policy(match snapshot.approval {
                ApprovalPolicy::Ask => ApprovalMode::Ask,
                ApprovalPolicy::Never => ApprovalMode::Never,
            });
        }
        Ok(snapshot)
    }

    /// Register a custom preset (e.g. the Auto review preset).
    pub fn register(&self, name: impl Into<String>, spec: PresetSpec) {
        self.state
            .write()
            .expect("permission presets poisoned")
            .presets
            .insert(name.into(), spec);
    }

    pub fn current(&self) -> PermissionState {
        self.state
            .read()
            .expect("permission presets poisoned")
            .state
            .clone()
    }

    pub fn names(&self) -> Vec<String> {
        self.state
            .read()
            .expect("permission presets poisoned")
            .presets
            .keys()
            .cloned()
            .collect()
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<PermissionPresets> {
    let presets = PermissionPresets::new();
    ctx.provide("permissionPresets", presets.clone()).await?;
    Ok(presets)
}
