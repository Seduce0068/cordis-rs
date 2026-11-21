//! Reusable Rust port of the credential-gated part of
//! `@deepseek-ai/dsh-llm-deepseek-api-key`.
//!
//! Upstream resolves `apiKeyEnv` (default `DEEPSEEK_API_KEY`), follows the
//! official `/models` listing and registers the official provider. This port
//! keeps the config resolution and fail-closed credential gate; the actual
//! HTTP request is a host concern supplied through `from_async_text`.
//! Source: `packages/llm/llm-deepseek-api-key/src/{index,config}.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::sync::Arc;

use async_trait::async_trait;

use crate::Result;
use crate::core_services::{LlmRequest, LlmService};

pub const DEFAULT_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";
pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
pub const PROVIDER: &str = "deepseek-official";

#[derive(Debug, Clone)]
pub struct DeepSeekConfig {
    pub api_key_env: String,
    pub base_url: String,
    pub model: String,
    pub thinking: bool,
}

impl Default for DeepSeekConfig {
    fn default() -> Self {
        Self {
            api_key_env: DEFAULT_API_KEY_ENV.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: "deepseek-chat".to_string(),
            thinking: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    Missing(String),
}

impl DeepSeekConfig {
    /// Validate `base_url` before attaching an API key. Only HTTPS to the
    /// official DeepSeek host (or its subdomains) is allowed; userinfo and
    /// path tricks are rejected by parsing rather than string matching
    /// (R3-DE04). The default keeps existing hosts working.
    pub fn validated_endpoint(&self) -> std::result::Result<String, String> {
        let parsed =
            url::Url::parse(&self.base_url).map_err(|_| "invalid DeepSeek base_url".to_string())?;
        if parsed.scheme() != "https" {
            return Err("DeepSeek base_url must use https".to_string());
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err("DeepSeek base_url must not contain userinfo".to_string());
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| "DeepSeek base_url must contain a host".to_string())?;
        if host != "api.deepseek.com" && !host.ends_with(".api.deepseek.com") {
            return Err("DeepSeek base_url host is not allowed".to_string());
        }
        let mut endpoint = parsed;
        endpoint.set_path("/chat/completions");
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        Ok(endpoint.to_string())
    }
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(name) => write!(
                f,
                "no API key for provider {PROVIDER}; export {name} or store it through credentials"
            ),
        }
    }
}

/// Resolve the credential from the supplied environment map. Missing keys are
/// an error (upstream `MISSING_CREDENTIAL`), never a silent empty string.
pub fn resolve_api_key(
    config: &DeepSeekConfig,
    env: &std::collections::BTreeMap<String, String>,
) -> std::result::Result<String, CredentialError> {
    env.get(&config.api_key_env)
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CredentialError::Missing(config.api_key_env.clone()))
}

/// Build a transport-backed `StreamHandler` for the DeepSeek chat endpoint.
/// The API key is assembled into `Authorization` here (host side); the
/// transport never stores the secret, and attribution headers are always
/// merged so provider requests cannot suppress attribution.
pub fn handler_with_transport(
    config: &DeepSeekConfig,
    api_key: &str,
    transport: std::sync::Arc<dyn crate::plugin_ports::LlmTransport>,
) -> crate::services::llm::StreamHandler {
    let identity = crate::plugin_ports::AppIdentity::default();
    let mut headers = crate::plugin_ports::attribution_headers(&identity);
    // Validate before attaching the key so a hostile base_url cannot receive
    // it (R3-DE04). On failure, emit an error chunk without touching transport.
    let endpoint = match config.validated_endpoint() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let error = Arc::new(error);
            return Arc::new(move |_request| {
                let error = error.clone();
                Box::pin(futures::stream::once(async move {
                    crate::services::llm::StreamChunk::Error((*error).clone())
                }))
            });
        }
    };
    headers.push(("authorization".to_string(), format!("Bearer {api_key}")));
    crate::plugin_ports::handler_from_transport(transport, endpoint, headers)
}

/// The `llm-deepseek-api-key` plugin. `inject = ['llm']`.
pub struct LlmDeepSeek {
    pub config: DeepSeekConfig,
    /// Environment snapshot used for credential resolution.
    pub env: std::collections::BTreeMap<String, String>,
}

#[async_trait]
impl crate::Plugin for LlmDeepSeek {
    fn name(&self) -> &str {
        "llm-deepseek-api-key"
    }

    fn inject(&self) -> &[&str] {
        &["llm"]
    }

    async fn apply(&self, ctx: &crate::Context) -> Result<()> {
        let llm = ctx.require::<LlmService>("llm").await?;
        let config = self.config.clone();
        let env = self.env.clone();
        ctx.effect(move |scope| {
            let llm = llm.clone();
            let config = config.clone();
            let env = env.clone();
            async move {
                // Resolve once at mount to fail loudly on a missing key,
                // matching upstream's initial `options()` validation.
                let key = resolve_api_key(&config, &env)
                    .map_err(|error| crate::Error::Other(error.to_string()))?;
                let provider = crate::core_services::from_async_text(
                    PROVIDER,
                    config.model.clone(),
                    move |_request: LlmRequest| {
                        let key = key.clone();
                        let base = config.base_url.clone();
                        async move {
                            // A real host would POST to `{base}/chat/completions`;
                            // this port verifies the credential gate only.
                            Ok::<_, String>(format!(
                                "authenticated via {base} (key {} chars)",
                                key.len()
                            ))
                        }
                    },
                );
                let disposer = llm.register(provider);
                scope.on_dispose(move || async move { disposer.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with the shared LLM seam and a supplied environment snapshot.
pub async fn mount(
    ctx: &crate::Context,
    config: DeepSeekConfig,
    env: std::collections::BTreeMap<String, String>,
) -> Result<LlmService> {
    let llm = if let Some(existing) = ctx.get::<LlmService>("llm") {
        (*existing).clone()
    } else {
        crate::core_services::provide_llm(ctx).await?
    };
    let _ = crate::Fiber::mount_retained(ctx, Box::new(LlmDeepSeek { config, env })).await?;
    Ok(llm)
}
