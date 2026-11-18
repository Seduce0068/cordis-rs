//! Resilience decorators for the LLM stream seam.
//!
//! These wrap an existing `StreamHandler` with timeout and retry guarantees
//! without changing any service type. A timeout yields a `StreamChunk::Error`
//! carrying an HTTP-style `504` code so the existing `RetryPolicy` (which
//! extracts three-digit codes from error strings) can decide to retry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;

use crate::services::llm::{LlmRequest, StreamChunk, StreamHandler};
use crate::services::retry::RetryPolicy;

/// `TIMEOUT ` prefix used for timeout error chunks (contract with retry).
pub const TIMEOUT_PREFIX: &str = "TIMEOUT 504: ";

/// Wrap a handler so each chunk wait is bounded by `budget`. On timeout the
/// stream yields one `StreamChunk::Error("TIMEOUT 504: ...")` and terminates;
/// `LlmService::stream` converts that into `Error::Other` naturally.
pub fn with_llm_timeout(handler: StreamHandler, budget: Duration) -> StreamHandler {
    Arc::new(move |request: LlmRequest| {
        let stream = handler(request);
        let budget = budget;
        Box::pin(futures::stream::unfold(
            (stream, budget),
            |(mut stream, budget)| async move {
                match tokio::time::timeout(budget, stream.next()).await {
                    Ok(Some(chunk)) => Some((chunk, (stream, budget))),
                    Ok(None) => None,
                    Err(_) => Some((
                        StreamChunk::Error(format!(
                            "{TIMEOUT_PREFIX}llm request timed out after {}ms",
                            budget.as_millis()
                        )),
                        (stream, Duration::ZERO),
                    )),
                }
            },
        ))
    })
}

/// Wrap a handler so a failed stream is retried per `policy`. The whole call
/// (all chunks until `Done`) is drained and re-dispatched on a retryable error.
/// Do not combine this with AgentLoop's own retry (double backoff).
pub fn with_llm_retry(handler: StreamHandler, policy: RetryPolicy) -> StreamHandler {
    Arc::new(move |request: LlmRequest| {
        let handler = handler.clone();
        let policy = policy.clone();
        Box::pin(
            futures::stream::once(async move {
                let mut attempt = 0u32;
                let mut text = String::new();
                loop {
                    let mut stream = handler(request.clone());
                    let mut failed = None;
                    let mut done = false;
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            StreamChunk::Text(part) => text.push_str(&part),
                            StreamChunk::Error(reason) => {
                                failed = Some(reason);
                                break;
                            }
                            StreamChunk::Done => {
                                done = true;
                                break;
                            }
                        }
                    }
                    if done || failed.is_none() {
                        break;
                    }
                    let reason = failed.unwrap_or_default();
                    let start = Instant::now();
                    if policy.should_retry_message(&reason, attempt)
                        == crate::services::retry::RetryDecision::Stop
                    {
                        return StreamChunk::Error(reason);
                    }
                    attempt += 1;
                    let delay = policy.backoff_ms(attempt, 0.5);
                    if delay > 0 {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                    let _ = start;
                    text.clear(); // discard partial response on retry
                }
                StreamChunk::Text(text)
            })
            .chain(futures::stream::once(async { StreamChunk::Done })),
        )
    })
}
