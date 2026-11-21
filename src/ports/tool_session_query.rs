//! Reusable Rust port of `@deepseek-ai/dsh-tool-session-query` (core tools).
//!
//! Registers `session_search` and `session_trace` on `ctx.tools`, backed by
//! the shared `ctx.sessionQuery` engine. Source:
//! `packages/experimental/tool-session-query/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::session_query::SessionQueryEngine;
use crate::{Context, Result};

#[derive(Debug, Clone)]
pub struct ToolSessionQueryConfig {
    pub max_search_results: usize,
}

impl Default for ToolSessionQueryConfig {
    fn default() -> Self {
        Self {
            max_search_results: 100,
        }
    }
}

fn search_handler(engine: SessionQueryEngine, config: ToolSessionQueryConfig) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let engine = engine.clone();
        let config = config.clone();
        Box::pin(async move {
            let query = args
                .get("query")
                .and_then(|value| value.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("query must be a string".to_string()))?;
            if query.trim().is_empty() {
                return Err(ToolError::InvalidArgs(
                    "query must not be empty".to_string(),
                ));
            }
            let mut hits = engine.search(query);
            hits.truncate(config.max_search_results);
            let rendered = hits
                .iter()
                .map(|hit| {
                    format!(
                        "{}#{} [{}]: {}",
                        hit.session_id, hit.event_seq, hit.event_type, hit.text
                    )
                })
                .collect::<Vec<_>>();
            let hit_values: Vec<serde_json::Value> = hits
                .iter()
                .map(|hit| {
                    serde_json::json!({
                        "session_id": hit.session_id,
                        "event_seq": hit.event_seq,
                        "event_type": hit.event_type,
                        "text": hit.text,
                    })
                })
                .collect();
            Ok(ToolOutput {
                content: serde_json::json!({ "hits": hit_values }),
                rendered: if rendered.is_empty() {
                    vec!["No matching sessions.".to_string()]
                } else {
                    rendered
                },
            })
        })
    })
}

fn trace_handler(engine: SessionQueryEngine) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let engine = engine.clone();
        Box::pin(async move {
            let session_id = args
                .get("session_id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("session_id must be a string".to_string()))?;
            let trace = engine.trace(session_id);
            let rendered = trace
                .iter()
                .map(|event| format!("{} [{}]", event.seq, event.event_type))
                .collect::<Vec<_>>();
            let event_values: Vec<serde_json::Value> = trace
                .iter()
                .map(|event| {
                    serde_json::json!({
                        "seq": event.seq,
                        "type": event.event_type,
                        "data": event.data,
                    })
                })
                .collect();
            Ok(ToolOutput {
                content: serde_json::json!({ "events": event_values }),
                rendered: if rendered.is_empty() {
                    vec!["Session not found or empty.".to_string()]
                } else {
                    rendered
                },
            })
        })
    })
}

/// The `dsh-tool-session-query` plugin.
pub struct ToolSessionQuery {
    pub config: ToolSessionQueryConfig,
}

#[async_trait]
impl crate::Plugin for ToolSessionQuery {
    fn name(&self) -> &str {
        "tool-session-query"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "sessionQuery"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let tools = ctx.require::<ToolRegistry>("tools").await?;
        let engine = ctx.require::<SessionQueryEngine>("sessionQuery").await?;
        let config = self.config.clone();
        ctx.effect(move |scope| {
            let tools = tools.clone();
            let engine = engine.clone();
            let config = config.clone();
            async move {
                let d1 = tools.register(ToolDefinition::new(
                    "session_search",
                    "Search prior sessions for matching history.",
                    search_handler((*engine).clone(), config),
                ));
                let d2 = tools.register(ToolDefinition::new(
                    "session_trace",
                    "Read the event trace of one session.",
                    trace_handler((*engine).clone()),
                ));
                scope.on_dispose(move || async move { d1.dispose().await });
                scope.on_dispose(move || async move { d2.dispose().await });
                Ok(())
            }
        })
        .await
    }
}

/// Mount with the shared tools and sessionQuery seams.
pub async fn mount(ctx: &Context, config: ToolSessionQueryConfig) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    if ctx.get_untyped("sessionQuery").is_none() {
        crate::core_services::provide_session_query(ctx).await?;
    }
    let _ = crate::Fiber::mount_retained(ctx, Box::new(ToolSessionQuery { config })).await?;
    Ok((*tools).clone())
}
