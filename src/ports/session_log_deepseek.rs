//! Reusable Rust port of `@deepseek-ai/dsh-session-log-deepseek`.
//!
//! Source: `packages/session/session-log-deepseek/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Contributes a `dsh_session_log` request field carrying an incremental
//! prefix of the session log; acceptance watermarks live in the log itself.

use std::sync::Arc;

use crate::core_services::SessionStore;
use crate::ports::deepseek_llm_api_extensions::{DeepSeekLlmApiExtensionRegistry, PreparedField};
use crate::{Context, Error, Result};

pub const FIELD_NAME: &str = "dsh_session_log";
pub const WATERMARK_EVENT: &str = "sessionLog/watermark";
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionLogConfig {
    pub enabled: bool,
    pub max_bytes: usize,
}

impl Default for SessionLogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

impl SessionLogConfig {
    pub fn validate(&self) -> std::result::Result<(), Error> {
        if self.max_bytes == 0 {
            return Err(Error::Other("maxBytes must be positive".to_string()));
        }
        Ok(())
    }
}

/// Restore the last fully accepted event seq from watermark events. Only the
/// current format generation (`version: 1`) is honoured.
pub fn accepted_seq(events: &[serde_json::Value]) -> u64 {
    let mut last = 0;
    for event in events {
        if event.get("type").and_then(|v| v.as_str()) != Some(WATERMARK_EVENT) {
            continue;
        }
        if event.get("version").and_then(|v| v.as_u64()) != Some(1) {
            continue; // foreign format generation ignored
        }
        if let Some(seq) = event.get("last_seq").and_then(|v| v.as_u64()) {
            last = last.max(seq);
        }
    }
    last
}

/// Build the longest event prefix whose serialization fits `max_bytes`.
/// Events are kept whole (never byte-sliced, which could panic on UTF-8
/// boundaries or produce invalid JSON). Returns the selected events and the
/// seq of the last fully included one.
pub fn fitting_prefix(
    sessions: &SessionStore,
    session_id: &str,
    after_seq: u64,
    max_bytes: usize,
) -> std::result::Result<(Vec<serde_json::Value>, u64), Error> {
    let session = sessions
        .get(session_id)
        .ok_or_else(|| Error::Other(format!("session {session_id} not found")))?;
    let header = serde_json::json!({ "version": 1, "id": session.id() });
    let mut selected = Vec::new();
    let mut last_seq = after_seq;
    for event in session.events() {
        if event.seq <= after_seq || event.event_type == WATERMARK_EVENT {
            continue; // control events are not part of the uploaded log
        }
        let candidate = serde_json::json!({
            "seq": event.seq,
            "type": event.event_type,
            "data": event.data,
        });
        let mut included = selected.clone();
        included.push(candidate.clone());
        let envelope = serde_json::json!({
            "header": header,
            "events": { "included": included },
        });
        let total = serde_json::to_vec(&envelope)
            .map_err(|error| Error::Other(error.to_string()))?
            .len();
        // `envelope` already includes the header; comparing `total` alone
        // avoids double-counting it against the byte budget.
        if total > max_bytes {
            break; // indivisible event deferred to the next request
        }
        selected.push(candidate);
        last_seq = event.seq;
    }
    Ok((selected, last_seq))
}

/// Build the `dsh_session_log` prepared field for one request.
pub fn prepare_field(
    sessions: &SessionStore,
    request: &serde_json::Value,
    config: &SessionLogConfig,
) -> std::result::Result<Option<PreparedField>, Error> {
    config.validate()?;
    if !config.enabled {
        return Ok(None);
    }
    let Some(session_id) = request
        .get("sessionId")
        .and_then(|value| value.as_str())
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let session = match sessions.get(&session_id) {
        Some(session) => session,
        None => return Ok(None),
    };
    let watermark_events: Vec<serde_json::Value> = session
        .events()
        .into_iter()
        .map(|event| {
            serde_json::json!({
                "type": event.event_type,
                "version": event.data.get("version").cloned().unwrap_or(serde_json::Value::Null),
                "last_seq": event.data.get("last_seq").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    let after_seq = accepted_seq(&watermark_events);
    let (events, through_seq) = fitting_prefix(sessions, &session_id, after_seq, config.max_bytes)?;
    if events.is_empty() {
        return Ok(None);
    }

    let session_for_accept = session.clone();
    let accept = Arc::new(move || {
        session_for_accept.append(
            WATERMARK_EVENT,
            serde_json::json!({ "version": 1, "last_seq": through_seq }),
        );
        Ok(())
    }) as crate::ports::deepseek_llm_api_extensions::AcceptCallback;

    Ok(Some(PreparedField {
        value: serde_json::json!({
            "version": 1,
            "sessionId": session_id,
            "afterSeq": after_seq,
            "throughSeq": through_seq,
            "events": events,
        }),
        accept: Some(accept),
    }))
}

/// The `dsh-session-log-deepseek` plugin.
pub struct SessionLogDeepSeek {
    pub config: SessionLogConfig,
}

#[async_trait::async_trait]
impl crate::Plugin for SessionLogDeepSeek {
    fn name(&self) -> &str {
        "session-log-deepseek"
    }

    fn inject(&self) -> &[&str] {
        &["deepseekLlmApiExtensions", "sessions"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        self.config.validate()?;
        let extensions = ctx
            .require::<DeepSeekLlmApiExtensionRegistry>(
                crate::ports::deepseek_llm_api_extensions::DEEPSEEK_EXTENSIONS_SERVICE,
            )
            .await?;
        let sessions = ctx.require::<SessionStore>("sessions").await?;
        let config = self.config.clone();

        let provider = Arc::new(move |request: &serde_json::Value| {
            prepare_field(&sessions, request, &config).map_err(|error| error.to_string())
        }) as crate::ports::deepseek_llm_api_extensions::ExtensionProvider;

        ctx.effect(move |scope| {
            let extensions = extensions.clone();
            let provider = provider.clone();
            async move {
                if !extensions.register(FIELD_NAME, provider) {
                    return Err(crate::Error::Other(format!(
                        "field {FIELD_NAME} already registered"
                    )));
                }
                scope.on_dispose(move || async move {
                    extensions.unregister(FIELD_NAME);
                    Ok(())
                });
                Ok(())
            }
        })
        .await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context, config: SessionLogConfig) -> Result<()> {
    config.validate()?;
    if ctx
        .get::<DeepSeekLlmApiExtensionRegistry>(
            crate::ports::deepseek_llm_api_extensions::DEEPSEEK_EXTENSIONS_SERVICE,
        )
        .is_none()
    {
        let registry = DeepSeekLlmApiExtensionRegistry::new();
        let _ = ctx
            .provide(
                crate::ports::deepseek_llm_api_extensions::DEEPSEEK_EXTENSIONS_SERVICE,
                registry,
            )
            .await?;
    }
    let fiber = crate::Fiber::mount(ctx, Box::new(SessionLogDeepSeek { config })).await?;
    ctx.effect(move |scope| {
        let fiber = fiber.clone();
        async move {
            scope.on_dispose(move || async move {
                fiber.unload().await?;
                Ok(())
            });
            Ok(())
        }
    })
    .await?;
    Ok(())
}
