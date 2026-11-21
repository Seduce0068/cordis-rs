//! Reusable Rust port of `@deepseek-ai/dsh-deepseek-llm-api-extensions`.
//!
//! Source: `packages/llm/deepseek-llm-api-extensions/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! A registry where plugins own independent top-level DeepSeek request fields,
//! with one preparation and joint-acceptance transaction per request.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::{Context, Result};

pub const DEEPSEEK_EXTENSIONS_SERVICE: &str = "deepseekLlmApiExtensions";

pub type ExtensionProvider = Arc<
    dyn Fn(&serde_json::Value) -> std::result::Result<Option<PreparedField>, String> + Send + Sync,
>;

pub type AcceptCallback = Arc<dyn Fn() -> std::result::Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct PreparedField {
    pub value: serde_json::Value,
    pub accept: Option<AcceptCallback>,
}

#[derive(Clone)]
pub struct DeepSeekLlmApiExtensionRegistry {
    providers: Arc<Mutex<HashMap<String, ExtensionProvider>>>,
}

impl Default for DeepSeekLlmApiExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DeepSeekLlmApiExtensionRegistry {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register the sole provider of one field. Returns false when the field
    /// is blank/untrimmed or already registered (no mutation in either case).
    pub fn register(&self, field: &str, provider: ExtensionProvider) -> bool {
        if field.is_empty() || field.trim() != field {
            return false;
        }
        let mut providers = self.providers.lock().expect("extension registry poisoned");
        if providers.contains_key(field) {
            return false;
        }
        providers.insert(field.to_string(), provider);
        true
    }

    pub fn unregister(&self, field: &str) -> bool {
        self.providers
            .lock()
            .expect("extension registry poisoned")
            .remove(field)
            .is_some()
    }

    /// Prepare all fields. A provider whose preparation fails is omitted and
    /// reported once per field *per call*: a persistent failure is visible in
    /// every `PreparedExtensions.failures` (swarm review R2-OB03). Values are
    /// deep-cloned (serde_json ownership) so providers retain no alias.
    pub fn prepare(&self, request: &serde_json::Value, cancelled: bool) -> PreparedExtensions {
        if cancelled {
            return PreparedExtensions::new();
        }
        let entries: Vec<(String, ExtensionProvider)> = {
            let providers = self.providers.lock().expect("extension registry poisoned");
            providers
                .iter()
                .map(|(field, provider)| (field.clone(), provider.clone()))
                .collect()
        };
        let mut fields = serde_json::Map::new();
        let mut callbacks: Vec<AcceptCallback> = Vec::new();
        let mut failures: Vec<(String, String)> = Vec::new();
        let mut warned: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (field, provider) in entries {
            match provider(request) {
                Ok(Some(prepared)) => {
                    fields.insert(field, prepared.value);
                    if let Some(accept) = prepared.accept {
                        callbacks.push(accept);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    if warned.insert(field.clone()) {
                        failures.push((field, error));
                    }
                }
            }
        }
        PreparedExtensions {
            fields: serde_json::Value::Object(fields),
            failures,
            callbacks: Arc::new(Mutex::new(callbacks)),
            accepted: Arc::new((Mutex::new(AcceptState::Pending), std::sync::Condvar::new())),
        }
    }
}

/// Settlement state for acceptance callbacks, so concurrent callers join the
/// same result instead of re-running callbacks (swarm review R2-ER).
enum AcceptState {
    Pending,
    Running,
    Done(std::result::Result<(), String>),
}

#[derive(Clone)]
pub struct PreparedExtensions {
    pub fields: serde_json::Value,
    /// Fields omitted from this request because preparation failed, with the
    /// first error per field (upstream's warn-once behaviour, made observable).
    pub failures: Vec<(String, String)>,
    callbacks: Arc<Mutex<Vec<AcceptCallback>>>,
    accepted: Arc<(Mutex<AcceptState>, std::sync::Condvar)>,
}

impl PreparedExtensions {
    fn new() -> Self {
        Self {
            fields: serde_json::Value::Object(serde_json::Map::new()),
            failures: Vec::new(),
            callbacks: Arc::new(Mutex::new(Vec::new())),
            accepted: Arc::new((Mutex::new(AcceptState::Pending), std::sync::Condvar::new())),
        }
    }

    /// Run every acceptance callback once. Concurrent callers join the same
    /// settlement (idempotent). Rust has no `AggregateError`: multiple
    /// failures are joined into one message.
    pub fn accept(&self) -> std::result::Result<(), String> {
        let (lock, cvar) = self.accepted.as_ref();
        let mut state = lock.lock().expect("extension acceptance poisoned");
        loop {
            match &*state {
                AcceptState::Done(result) => return result.clone(),
                AcceptState::Running => {
                    state = cvar.wait(state).expect("extension acceptance poisoned");
                }
                AcceptState::Pending => {
                    *state = AcceptState::Running;
                    break;
                }
            }
        }
        // Release the guard before invoking user code: callbacks may re-enter
        // `accept` or register more callbacks (swarm review R2-ER).
        drop(state);

        // Clone the callback list so its guard is not held during callbacks.
        let callbacks = self
            .callbacks
            .lock()
            .expect("extension acceptance poisoned")
            .clone();
        let mut failures = Vec::new();
        for callback in &callbacks {
            if let Err(error) = callback() {
                failures.push(error);
            }
        }
        let result = match failures.len() {
            0 => Ok(()),
            1 => Err(failures.remove(0)),
            _ => Err(format!(
                "DeepSeek LLM API extension acceptance failed: {}",
                failures.join("; ")
            )),
        };
        *lock.lock().expect("extension acceptance poisoned") = AcceptState::Done(result.clone());
        cvar.notify_all();
        result
    }
}

/// The `dsh-deepseek-llm-api-extensions` plugin (pure service, no inject).
pub struct DeepSeekLlmApiExtensions;

#[async_trait::async_trait]
impl crate::Plugin for DeepSeekLlmApiExtensions {
    fn name(&self) -> &str {
        "deepseek-llm-api-extensions"
    }

    fn inject(&self) -> &[&str] {
        &[]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let registry = DeepSeekLlmApiExtensionRegistry::new();
        ctx.provide(DEEPSEEK_EXTENSIONS_SERVICE, registry).await?;
        Ok(())
    }
}

/// Mount the registry service on a context.
pub async fn mount(ctx: &Context) -> Result<DeepSeekLlmApiExtensionRegistry> {
    let registry = DeepSeekLlmApiExtensionRegistry::new();
    ctx.provide(DEEPSEEK_EXTENSIONS_SERVICE, registry.clone())
        .await?;
    Ok(registry)
}
