//! `ctx.llm`: provider-neutral LLM seam with a stream call.
//!
//! Upstream: `packages/llm/llm/src/index.ts` in deepseek-ai/deepseek-harness
//! (MIT), read as untrusted reference data. The seam registers providers by
//! name and offers a request/stream vocabulary; retry and account providers
//! are separate plugins.

use std::pin::Pin;
use std::sync::{Arc, RwLock};

use crate::Result;
use crate::primitives::Disposer;

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    Text(String),
    /// Provider-side failure (e.g. `HTTP 429 rate limited`). `LlmService::stream`
    /// turns this into an `Error::Other` so retry policies can inspect it.
    Error(String),
    Done,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmResponse {
    pub text: String,
    pub model: String,
}

pub type StreamHandler = Arc<
    dyn Fn(LlmRequest) -> Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send + 'static>>
        + Send
        + Sync,
>;

/// A registered provider: a named model + stream handler.
#[derive(Clone)]
pub struct LlmProvider {
    name: String,
    model: String,
    handler: StreamHandler,
}

impl LlmProvider {
    pub fn new(name: impl Into<String>, model: impl Into<String>, handler: StreamHandler) -> Self {
        Self {
            name: name.into(),
            model: model.into(),
            handler,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

#[derive(Default)]
struct RegistryState {
    providers: Vec<(u64, LlmProvider)>,
    next_id: u64,
    default: Option<String>,
}

/// The shared LLM registry.
#[derive(Default, Clone)]
pub struct LlmService {
    state: Arc<RwLock<RegistryState>>,
}

/// Read the current default provider name (used by the disposer to detect
/// removal of the default).
fn guard_default_name(state: &std::sync::Arc<std::sync::RwLock<RegistryState>>) -> Option<String> {
    state.read().expect("llm registry poisoned").default.clone()
}

impl LlmService {
    pub fn register(&self, provider: LlmProvider) -> Disposer {
        let state = self.state.clone();
        let mut guard = state.write().expect("llm registry poisoned");
        let id = guard.next_id;
        guard.next_id += 1;
        if guard.default.is_none() {
            guard.default = Some(provider.name().to_string());
        }
        guard.providers.push((id, provider));
        drop(guard);

        let default_name = guard_default_name(&state);
        Disposer::new(move || {
            let state = state.clone();
            let default_name = default_name.clone();
            async move {
                let mut guard = state.write().expect("llm registry poisoned");
                guard.providers.retain(|(current, _)| *current != id);
                // If the removed provider was the default, clear the stale
                // name so resolve(None) falls back to no provider rather than
                // always returning None from a dangling lookup (swarm review
                // R5-LL01). Hosts can set_default() an alternative.
                if guard.default.as_deref() == default_name.as_deref()
                    && !guard
                        .providers
                        .iter()
                        .any(|(_, p)| Some(p.name()) == default_name.as_deref())
                {
                    guard.default = None;
                }
                Ok(())
            }
        })
    }

    pub fn set_default(&self, name: &str) -> bool {
        let mut guard = self.state.write().expect("llm registry poisoned");
        if guard.providers.iter().any(|(_, p)| p.name() == name) {
            guard.default = Some(name.to_string());
            true
        } else {
            false
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.state
            .read()
            .expect("llm registry poisoned")
            .providers
            .iter()
            .map(|(_, p)| p.name().to_string())
            .collect()
    }

    /// Resolve a provider by name, or the default one when `None`.
    pub fn resolve(&self, name: Option<&str>) -> Option<LlmProvider> {
        let guard = self.state.read().expect("llm registry poisoned");
        let selected = name.or(guard.default.as_deref())?;
        guard
            .providers
            .iter()
            .find(|(_, p)| p.name() == selected)
            .map(|(_, p)| p.clone())
    }

    /// Stream a complete response from a resolved provider.
    pub async fn stream(&self, name: Option<&str>, request: LlmRequest) -> Result<LlmResponse> {
        use futures::StreamExt;

        let provider = self
            .resolve(name)
            .ok_or_else(|| crate::Error::ServiceNotFound("llm provider".to_string()))?;
        let handler = provider.handler.clone();
        let model = provider.model().to_string();
        let mut stream = handler(request);
        let mut text = String::new();
        let mut completed = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                StreamChunk::Text(part) => text.push_str(&part),
                StreamChunk::Error(message) => {
                    return Err(crate::Error::Other(message));
                }
                StreamChunk::Done => {
                    completed = true;
                    break;
                }
            }
        }
        // A stream that ends without Done was cut short (transport close,
        // timeout, decorator abort). Returning the partial text as a success
        // would hide the failure (swarm review R2-ER, high).
        if !completed {
            // No 3-digit code: a protocol violation is not retryable via
            // RetryPolicy::error_code, and partial text is discarded (fail-closed).
            return Err(crate::Error::Other(
                "LLM stream ended without Done".to_string(),
            ));
        }
        Ok(LlmResponse { text, model })
    }
}

/// Build a provider from a host-supplied async function that returns one text
/// response. Useful for external backends (ACP, HTTP) that produce a whole
/// answer rather than a token stream.
pub fn from_async_text<F, Fut>(
    name: impl Into<String>,
    model: impl Into<String>,
    f: F,
) -> LlmProvider
where
    F: Fn(LlmRequest) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::result::Result<String, String>> + Send + 'static,
{
    LlmProvider::new(
        name,
        model,
        Arc::new(move |request| {
            let future = f(request);
            use futures::StreamExt;
            futures::stream::once(async move {
                match future.await {
                    Ok(text) => StreamChunk::Text(text),
                    Err(error) => StreamChunk::Error(error),
                }
            })
            .chain(futures::stream::once(async { StreamChunk::Done }))
            .boxed()
        }),
    )
}

/// A simple provider that echoes the last user message (keyless tests/demos).
pub fn echo_provider(name: impl Into<String>) -> LlmProvider {
    let name = name.into();
    LlmProvider::new(
        name.clone(),
        format!("echo-{name}"),
        Arc::new(move |request| {
            let last = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == "user")
                .map(|message| message.content.clone())
                .unwrap_or_default();
            Box::pin(futures::stream::iter(vec![
                StreamChunk::Text(last),
                StreamChunk::Done,
            ]))
        }),
    )
}

pub async fn provide(ctx: &crate::Context) -> Result<LlmService> {
    let service = LlmService::default();
    ctx.provide("llm", service.clone()).await?;
    Ok(service)
}
