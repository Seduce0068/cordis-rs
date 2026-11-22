//! Reusable Rust port of `@deepseek-ai/dsh-compaction-image-offload`.
//!
//! Source: `packages/compaction/compaction-image-offload/src/index.ts`
//! (+ image-offload.ts, projection.ts) in deepseek-ai/deepseek-harness (MIT),
//! read as untrusted reference data.
//! Pure selection / validation logic for recovering from context-overflow
//! errors caused by large images, without spending retry budget.

use crate::{Context, Result};

pub const IMAGE_OFFLOAD_REQUIRED_CODE: &str = "IMAGE_OFFLOAD_REQUIRED";
pub const IMAGE_OFFLOAD_PROJECTION_KEY: &str = "image/offload";
pub const EVENT_TYPE: &str = "image/offload";

#[derive(Debug, Clone, PartialEq)]
pub struct OffloadTarget {
    pub seq: u64,
    pub image_indexes: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Retry,
    SummaryHandled,
    Noop,
}

/// Pick the oldest eligible images from user/tool messages. Upstream walks
/// most-recent-first over source event seqs and selects depth-first image
/// indexes; here callers pass events oldest-first with their image indexes.
pub fn select_oldest(
    candidates: &[(u64, Vec<usize>)],
    already_offloaded: &[(u64, Vec<usize>)],
    count: usize,
) -> Vec<OffloadTarget> {
    select_oldest_after(candidates, already_offloaded, count, 0)
}

/// Like [`select_oldest`] but skips events at or before `after_seq`. Callers
/// pass the last `compaction/checkpoint` seq so already-summarized selections
/// are not applied twice (reviewer finding: compaction only shadows, never
/// deletes).
pub fn select_oldest_after(
    candidates: &[(u64, Vec<usize>)],
    already_offloaded: &[(u64, Vec<usize>)],
    count: usize,
    after_seq: u64,
) -> Vec<OffloadTarget> {
    let mut picked = Vec::new();
    for (seq, indexes) in candidates {
        if *seq <= after_seq {
            continue;
        }
        let done = already_offloaded
            .iter()
            .find(|(s, _)| s == seq)
            .map(|(_, indexes)| indexes.as_slice())
            .unwrap_or(&[]);
        let fresh: Vec<usize> = indexes
            .iter()
            .copied()
            .filter(|index| !done.contains(index))
            .collect();
        if !fresh.is_empty() {
            picked.push(OffloadTarget {
                seq: *seq,
                image_indexes: fresh,
            });
            if picked.len() >= count {
                break;
            }
        }
    }
    picked
}

/// Decide how to react to an error carrying `IMAGE_OFFLOAD_REQUIRED`.
pub fn decide(error_code: Option<&str>, offload_succeeded: bool, is_summary: bool) -> Decision {
    if error_code != Some(IMAGE_OFFLOAD_REQUIRED_CODE) {
        return Decision::Noop;
    }
    if is_summary {
        if offload_succeeded {
            Decision::SummaryHandled
        } else {
            Decision::Noop
        }
    } else if offload_succeeded {
        Decision::Retry
    } else {
        Decision::Noop
    }
}

/// Validate a projection payload. Rejects unknown keys, empty targets,
/// negative/non-integer seqs and non-monotonic image indexes.
pub fn validate_payload(
    data: &serde_json::Value,
) -> std::result::Result<Vec<OffloadTarget>, String> {
    let object = data.as_object().ok_or("data must be an object")?;
    if object.len() != 1 || !object.contains_key("targets") {
        return Err("data must have exactly one key: targets".to_string());
    }
    let targets = data
        .get("targets")
        .and_then(|value| value.as_array())
        .ok_or("targets must be an array")?;
    if targets.is_empty() {
        return Err("targets must not be empty".to_string());
    }
    let mut out = Vec::new();
    for target in targets {
        let map = target.as_object().ok_or("target must be an object")?;
        if map.len() != 2 || !map.contains_key("seq") || !map.contains_key("imageIndexes") {
            return Err("target must have exactly seq and imageIndexes".to_string());
        }
        let seq = map
            .get("seq")
            .and_then(|value| value.as_u64())
            .ok_or("seq must be a non-negative integer")?;
        let indexes = map
            .get("imageIndexes")
            .and_then(|value| value.as_array())
            .ok_or("imageIndexes must be an array")?;
        let mut previous: Option<usize> = None;
        let mut parsed = Vec::new();
        for index in indexes {
            let value = usize::try_from(
                index
                    .as_u64()
                    .ok_or("image index must be a non-negative integer")?,
            )
            .map_err(|_| "image index out of range for this platform".to_string())?;
            if previous.is_some_and(|prev| value <= prev) {
                return Err("imageIndexes must be strictly increasing".to_string());
            }
            previous = Some(value);
            parsed.push(value);
        }
        if parsed.is_empty() {
            return Err("imageIndexes must not be empty".to_string());
        }
        out.push(OffloadTarget {
            seq,
            image_indexes: parsed,
        });
    }
    Ok(out)
}

/// Build the portable `image/offload` payload for a set of targets.
pub fn offload_payload(targets: &[OffloadTarget]) -> serde_json::Value {
    serde_json::json!({
        "targets": targets.iter().map(|target| serde_json::json!({
            "seq": target.seq,
            "imageIndexes": target.image_indexes,
        })).collect::<Vec<_>>(),
    })
}

/// The `dsh-compaction-image-offload` plugin. Selection/validation live in
/// pure helpers; the host wires them to `agent/request-error` and
/// `compaction/summary-error` using its own session surface.
pub struct CompactionImageOffload;

#[async_trait::async_trait]
impl crate::Plugin for CompactionImageOffload {
    fn name(&self) -> &str {
        "compaction-image-offload"
    }

    fn inject(&self) -> &[&str] {
        &["agents", "sessions"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx
            .require::<crate::services::agents::AgentRegistry>("agents")
            .await?;
        let _ = ctx
            .require::<crate::services::sessions::SessionStore>("sessions")
            .await?;
        Ok(())
    }
}

pub async fn mount(ctx: &Context) -> Result<()> {
    let _ = crate::Fiber::mount_retained(ctx, Box::new(CompactionImageOffload)).await?;
    Ok(())
}
