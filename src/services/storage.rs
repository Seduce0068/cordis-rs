//! `ctx.storage`: named backend registry plus schema-validated domains.
//!
//! Upstream: `packages/storage/storage/src/index.ts` and
//! `storage-domain/src/index.ts` in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data. This port combines the hub and the
//! domain form with an in-memory KV backend and JSON-value validation.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct DomainSpec {
    pub name: String,
    pub required_fields: Vec<String>,
}

impl DomainSpec {
    pub fn new(name: impl Into<String>, required_fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            required_fields,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StorageError {
    UnknownBackend(String),
    UnknownDomain(String),
    InvalidRecord { domain: String, missing: String },
    NotFound(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownBackend(name) => write!(f, "unknown storage backend: {name}"),
            Self::UnknownDomain(name) => write!(f, "unknown storage domain: {name}"),
            Self::InvalidRecord { domain, missing } => {
                write!(f, "record for {domain} is missing field {missing}")
            }
            Self::NotFound(key) => write!(f, "storage key not found: {key}"),
        }
    }
}

#[derive(Default)]
struct BackendState {
    kv: BTreeMap<String, serde_json::Value>,
}

#[derive(Default, Clone)]
pub struct MemoryBackend {
    state: Arc<RwLock<BackendState>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, key: impl Into<String>, value: serde_json::Value) {
        self.state
            .write()
            .expect("storage backend poisoned")
            .kv
            .insert(key.into(), value);
    }

    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.state
            .read()
            .expect("storage backend poisoned")
            .kv
            .get(key)
            .cloned()
    }

    pub fn keys(&self) -> Vec<String> {
        self.state
            .read()
            .expect("storage backend poisoned")
            .kv
            .keys()
            .cloned()
            .collect()
    }
}

#[derive(Default)]
struct HubState {
    backends: BTreeMap<String, MemoryBackend>,
    domains: BTreeMap<String, DomainSpec>,
}

/// The storage hub with its domain form.
#[derive(Default, Clone)]
pub struct StorageHub {
    state: Arc<RwLock<HubState>>,
}

impl StorageHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_backend(&self, name: impl Into<String>, backend: MemoryBackend) {
        self.state
            .write()
            .expect("storage hub poisoned")
            .backends
            .insert(name.into(), backend);
    }

    pub fn backend(&self, name: &str) -> Option<MemoryBackend> {
        self.state
            .read()
            .expect("storage hub poisoned")
            .backends
            .get(name)
            .cloned()
    }

    pub fn define_domain(&self, spec: DomainSpec) {
        self.state
            .write()
            .expect("storage hub poisoned")
            .domains
            .insert(spec.name.clone(), spec);
    }

    pub fn write(
        &self,
        domain: &str,
        backend: &str,
        key: impl Into<String>,
        record: serde_json::Value,
    ) -> std::result::Result<(), StorageError> {
        let spec = self
            .state
            .read()
            .expect("storage hub poisoned")
            .domains
            .get(domain)
            .cloned()
            .ok_or_else(|| StorageError::UnknownDomain(domain.to_string()))?;
        if let Some(map) = record.as_object() {
            for field in &spec.required_fields {
                if !map.contains_key(field) {
                    return Err(StorageError::InvalidRecord {
                        domain: domain.to_string(),
                        missing: field.clone(),
                    });
                }
            }
        }
        let backend = self
            .backend(backend)
            .ok_or_else(|| StorageError::UnknownBackend(backend.to_string()))?;
        backend.put(key, record);
        Ok(())
    }

    pub fn read(
        &self,
        domain: &str,
        backend: &str,
        key: &str,
    ) -> std::result::Result<serde_json::Value, StorageError> {
        // Validate the domain exists even though reads are opaque KV.
        let _ = self
            .state
            .read()
            .expect("storage hub poisoned")
            .domains
            .get(domain)
            .cloned()
            .ok_or_else(|| StorageError::UnknownDomain(domain.to_string()))?;
        let backend = self
            .backend(backend)
            .ok_or_else(|| StorageError::UnknownBackend(backend.to_string()))?;
        backend
            .get(key)
            .ok_or_else(|| StorageError::NotFound(key.to_string()))
    }
}

pub async fn provide(ctx: &crate::Context) -> Result<StorageHub> {
    let hub = StorageHub::new();
    ctx.provide("storage", hub.clone()).await?;
    Ok(hub)
}

/// Helper: convert a storage error to a harness error.
pub fn storage_error(error: StorageError) -> Error {
    Error::Other(error.to_string())
}
