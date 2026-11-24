//! `ctx.credentials`: credential-reference resolution seam.
//!
//! Upstream: `packages/credentials/credentials/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Settings carry references (`$ENV_NAME`) to secrets; the provider resolves
//! them per call from a writable store first, then ambient environment.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::Result;

/// Validate an environment-variable name: first char must be ASCII uppercase
/// or `_`, remaining chars may also be digits (swarm review R10-IV04).
fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase() || ch == '_')
        && chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

/// A reference to an environment-variable name (`$NAME`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRef {
    pub name: String,
}

/// Parse `$NAME`; names start with an ASCII uppercase letter or `_`, then
/// may contain uppercase letters, digits and underscores (swarm review
/// R10-IV04).
pub fn credential_ref(value: &str) -> Option<CredentialRef> {
    let name = value.strip_prefix('$')?;
    if !is_valid_env_name(name) {
        return None;
    }
    Some(CredentialRef {
        name: name.to_string(),
    })
}

#[derive(Clone, PartialEq)]
pub struct ResolvedCredential {
    pub value: String,
    pub source: String,
}

impl std::fmt::Debug for ResolvedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedCredential")
            .field("value", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialInfo {
    pub configured: bool,
    pub source: Option<String>,
}

#[derive(Default)]
struct CredentialState {
    store: BTreeMap<String, String>,
}

#[derive(Default, Clone)]
pub struct CredentialProvider {
    state: Arc<RwLock<CredentialState>>,
    env: Arc<RwLock<BTreeMap<String, String>>>,
}

impl CredentialProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply ambient environment values without mutating the process env.
    /// Builder form; use [`Self::set_env`] to update after registration
    /// (swarm review R16-RA05).
    pub fn with_env(self, env: BTreeMap<String, String>) -> Self {
        self.set_env(env);
        self
    }

    /// Replace the ambient environment for every clone sharing this provider,
    /// including one already registered in a context.
    pub fn set_env(&self, env: BTreeMap<String, String>) {
        *self.env.write().expect("credentials env poisoned") = env;
    }

    /// Store a non-empty value in the writable source.
    pub fn set(&self, name: impl Into<String>, value: impl Into<String>) -> bool {
        let name = name.into();
        let value = value.into();
        // Names must be resolvable via `$NAME` references; reject lowercase
        // or otherwise invalid names so a config typo cannot look successful
        // (swarm review R2-IV01).
        if value.trim().is_empty() || !is_valid_env_name(&name) {
            return false;
        }
        self.state
            .write()
            .expect("credential state poisoned")
            .store
            .insert(name, value);
        true
    }

    pub fn unset(&self, name: &str) {
        self.state
            .write()
            .expect("credential state poisoned")
            .store
            .remove(name);
    }

    /// Resolve per call: stored value wins, then ambient environment.
    pub fn resolve(&self, reference: &CredentialRef) -> Option<ResolvedCredential> {
        if let Some(value) = self
            .state
            .read()
            .expect("credential state poisoned")
            .store
            .get(&reference.name)
            .cloned()
            .filter(|value| !value.trim().is_empty())
        {
            return Some(ResolvedCredential {
                value,
                source: "store".to_string(),
            });
        }
        self.env
            .read()
            .expect("credentials env poisoned")
            .get(&reference.name)
            .cloned()
            .filter(|v| !v.trim().is_empty())
            .map(|value| ResolvedCredential {
                value,
                source: "env".to_string(),
            })
    }

    /// Describe without exposing the value.
    pub fn describe(&self, reference: &CredentialRef) -> CredentialInfo {
        match self.resolve(reference) {
            Some(resolved) => CredentialInfo {
                configured: true,
                source: Some(resolved.source),
            },
            None => CredentialInfo {
                configured: false,
                source: None,
            },
        }
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<CredentialProvider> {
    let provider = CredentialProvider::new();
    ctx.provide("credentials", provider.clone()).await?;
    Ok(provider)
}
