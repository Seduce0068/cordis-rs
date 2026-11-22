//! HTTP transport seam for LLM adapters.
//!
//! A transport is injected into an adapter so the request/response path can be
//! verified with a local fake (no API key, no network). This is a pure library
//! port: no service registration, no new dependencies.

use std::sync::{Arc, RwLock};

/// An outbound LLM HTTP request (serialized provider payload).
#[derive(Clone, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact header values (Authorization carries the API key) so request
        // logs and test output can never leak credentials.
        let redacted: Vec<(&String, &str)> = self
            .headers
            .iter()
            .map(|(name, _)| (name, "<redacted>"))
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &redacted)
            .field("body", &self.body)
            .finish()
    }
}

impl HttpRequest {
    pub fn post(url: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            method: "POST".to_string(),
            url: url.into(),
            headers: Vec::new(),
            body: body.into(),
        }
    }
}

/// An HTTP response returned by a transport.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            body: body.into(),
        }
    }
}

/// Transport seam. Implemented by a real HTTP client in the host and by
/// `FakeTransport` in tests.
pub trait LlmTransport: Send + Sync {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String>;
}

/// Build a `StreamHandler` from a transport seam. The transport decides how to
/// reach the endpoint; this helper only converts failures to `StreamChunk::Error`
/// so existing `RetryPolicy`-based decorators can inspect HTTP codes.
pub fn handler_from_transport(
    transport: Arc<dyn LlmTransport>,
    endpoint: impl Into<String>,
    extra_headers: Vec<(String, String)>,
) -> crate::services::llm::StreamHandler {
    let endpoint = endpoint.into();
    Arc::new(move |request| {
        let transport = transport.clone();
        let endpoint = endpoint.clone();
        let extra_headers = extra_headers.clone();
        let model = request.model.clone();
        let messages = serde_json::json!(
            request
                .messages
                .iter()
                .map(|message| serde_json::json!({
                    "role": message.role,
                    "content": message.content,
                }))
                .collect::<Vec<_>>()
        );
        Box::pin(futures::stream::once(async move {
            // Merge caller headers (attribution + Authorization assembled by
            // the host) with the endpoint path. The transport never sees a
            // secret it did not receive via headers.
            let mut req = HttpRequest::post(
                endpoint,
                serde_json::json!({
                    "model": model,
                    "messages": messages,
                })
                .to_string(),
            );
            req.headers.extend(extra_headers);
            match transport.send(&req) {
                Ok(response) if (200..300).contains(&response.status) => {
                    crate::services::llm::StreamChunk::Text(response.body)
                }
                Ok(response) => crate::services::llm::StreamChunk::Error(format!(
                    "HTTP {} {}",
                    response.status, response.body
                )),
                Err(error) => crate::services::llm::StreamChunk::Error(error),
            }
        }))
    })
}

/// In-memory transport: records every request and replays queued responses.
#[derive(Clone, Default)]
pub struct FakeTransport {
    requests: Arc<RwLock<Vec<HttpRequest>>>,
    responses: Arc<RwLock<Vec<HttpResponse>>>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a response for the next `send` call.
    pub fn push_response(&self, response: HttpResponse) {
        self.responses
            .write()
            .expect("fake transport poisoned")
            .push(response);
    }

    /// All requests seen so far, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .read()
            .expect("fake transport poisoned")
            .clone()
    }
}

impl LlmTransport for FakeTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        {
            self.requests
                .write()
                .expect("fake transport poisoned")
                .push(request.clone());
        }
        // Drop the requests lock before taking responses (swarm review
        // R11-RS02): holding both invites lock-order contention.
        let mut responses = self.responses.write().expect("fake transport poisoned");
        if responses.is_empty() {
            Err("fake transport: no queued response".to_string())
        } else {
            Ok(responses.remove(0))
        }
    }
}
