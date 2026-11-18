//! Pure Rust port of `@deepseek-ai/dsh-compaction/checkpoint`.
//!
//! Source: `packages/compaction/compaction/src/checkpoint.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Source marker for replacement user messages created by a compaction.

pub const COMPACT_CHECKPOINT_KIND: &str = "compact-checkpoint";

/// Message source carried by a concrete compaction checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionCheckpointSource {
    pub kind: &'static str,
    pub compaction_id: String,
    pub source_command_id: Option<String>,
}

/// Create a checkpoint source correlated with one compaction transaction.
pub fn compact_checkpoint_source(
    compaction_id: impl Into<String>,
    source_command_id: Option<String>,
) -> CompactionCheckpointSource {
    CompactionCheckpointSource {
        kind: COMPACT_CHECKPOINT_KIND,
        compaction_id: compaction_id.into(),
        source_command_id,
    }
}

/// Render the source as a JSON value (upstream frozen object equivalent).
pub fn checkpoint_source_json(source: &CompactionCheckpointSource) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("kind".to_string(), serde_json::json!(source.kind));
    map.insert(
        "compactionId".to_string(),
        serde_json::json!(source.compaction_id),
    );
    if let Some(command_id) = &source.source_command_id {
        map.insert("sourceCommandId".to_string(), serde_json::json!(command_id));
    }
    serde_json::Value::Object(map)
}

/// Whether a persisted message source identifies a compaction checkpoint.
pub fn is_compact_checkpoint_source(source: &serde_json::Value) -> bool {
    source
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(|value| value.as_str())
        == Some(COMPACT_CHECKPOINT_KIND)
}
