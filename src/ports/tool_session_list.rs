//! Reusable Rust port of `@deepseek-ai/dsh-tool-session-list`.
//!
//! Source: `packages/session-query/tool-session-list/src/index.ts` (+ list.ts)
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers `list_workspace` and `list_session` read-only tools over the
//! shared workspace / session / sessionQuery seams.

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::services::session_query::SessionQueryEngine;
use crate::services::sessions::SessionStore;
use crate::services::workspace::WorkspaceRegistry;
use crate::{Context, Result};

pub const DEFAULT_PAGE_LIMIT: usize = 20;
pub const MAX_PAGE_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub limit: usize,
    pub offset: usize,
}

/// Validate page bounds (limit 1..100, offset representable as usize).
pub fn resolve_page(
    limit: Option<u64>,
    offset: Option<u64>,
) -> std::result::Result<Page, ToolError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT as u64);
    let offset = offset.unwrap_or(0);
    if limit == 0 || limit > MAX_PAGE_LIMIT as u64 {
        return Err(ToolError::InvalidArgs(format!(
            "limit must be between 1 and {MAX_PAGE_LIMIT}"
        )));
    }
    // usize::try_from prevents silent truncation on 32-bit hosts (swarm
    // review R12-IV07).
    let limit = usize::try_from(limit).map_err(|_| {
        ToolError::InvalidArgs("limit is out of range for this platform".to_string())
    })?;
    let offset = usize::try_from(offset).map_err(|_| {
        ToolError::InvalidArgs("offset is out of range for this platform".to_string())
    })?;
    Ok(Page { limit, offset })
}

fn paginate<T>(items: Vec<T>, page: &Page) -> Vec<T> {
    items
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect()
}

fn list_workspace_handler(workspaces: WorkspaceRegistry) -> ToolHandler {
    use std::sync::Arc;
    Arc::new(move |args, _exec| {
        let workspaces = workspaces.clone();
        Box::pin(async move {
            let page = resolve_page(
                args.get("limit").and_then(|value| value.as_u64()),
                args.get("offset").and_then(|value| value.as_u64()),
            )?;
            let rows: Vec<_> = workspaces
                .list()
                .into_iter()
                .map(|record| {
                    serde_json::json!({
                        "id": record.id,
                        "title": record.title,
                        "path": record.path,
                        "archived": record.archived,
                    })
                })
                .collect();
            let total = rows.len();
            let rows = paginate(rows, &page);
            Ok(ToolOutput {
                content: serde_json::json!({
                    "workspaces": rows,
                    "total": total,
                    "limit": page.limit,
                    "offset": page.offset,
                }),
                rendered: vec![format!("{total} workspace(s)")],
            })
        })
    })
}

fn list_session_handler(
    sessions: SessionStore,
    query: SessionQueryEngine,
    workspaces: WorkspaceRegistry,
) -> ToolHandler {
    use std::sync::Arc;
    Arc::new(move |args, _exec| {
        let sessions = sessions.clone();
        let query = query.clone();
        let workspaces = workspaces.clone();
        Box::pin(async move {
            let page = resolve_page(
                args.get("limit").and_then(|value| value.as_u64()),
                args.get("offset").and_then(|value| value.as_u64()),
            )?;
            let workspace_id = args
                .get("workspace_id")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let archived = args
                .get("archived")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);

            // Reject an unknown workspace rather than silently returning an
            // empty session list (swarm review R3-AP02).
            let allowed: Option<Vec<String>> = match workspace_id.as_deref() {
                Some(id) => match workspaces.get(id) {
                    Some(_) => Some(workspaces.sessions_in(id)),
                    None => {
                        return Err(ToolError::InvalidArgs(format!("workspace not found: {id}")));
                    }
                },
                None => None,
            };
            // Archived sessions are those not present in the live store.
            let live: std::collections::HashSet<String> = sessions
                .list()
                .into_iter()
                .map(|session| session.id().to_string())
                .collect();
            let mut ids = query.session_ids();
            ids.retain(|id| {
                let matches_workspace = allowed
                    .as_ref()
                    .map(|list| list.contains(id))
                    .unwrap_or(true);
                let matches_archived = !live.contains(id) == archived;
                matches_workspace && matches_archived
            });
            ids.sort();
            let total = ids.len();
            let ids = paginate(ids, &page);
            let rows: Vec<_> = ids
                .into_iter()
                .map(|id| {
                    let last = query
                        .get(&id)
                        .and_then(|session| session.events().last().cloned());
                    let updated_at = last.map(|event| event.seq).unwrap_or(0);
                    serde_json::json!({
                        "id": id,
                        "activity_seq": updated_at,
                    })
                })
                .collect();
            Ok(ToolOutput {
                content: serde_json::json!({
                    "sessions": rows,
                    "total": total,
                    "limit": page.limit,
                    "offset": page.offset,
                }),
                rendered: vec![format!("{total} session(s)")],
            })
        })
    })
}

/// The `dsh-tool-session-list` plugin.
pub struct ToolSessionList;

#[async_trait::async_trait]
impl crate::Plugin for ToolSessionList {
    fn name(&self) -> &str {
        "tool-session-list"
    }

    fn inject(&self) -> &[&str] {
        &["tools", "sessions", "sessionQuery", "workspace"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx.require::<ToolRegistry>("tools").await?;
        Ok(())
    }
}

/// Mount with the shared seams.
pub async fn mount(ctx: &Context) -> Result<ToolRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let sessions = ctx.require::<SessionStore>("sessions").await?;
    let query = ctx.require::<SessionQueryEngine>("sessionQuery").await?;
    let workspaces = ctx.require::<WorkspaceRegistry>("workspace").await?;
    let tools_for_effect = tools.clone();

    let _ = ctx
        .effect(move |scope| {
            let tools = tools_for_effect.clone();
            let sessions = sessions.clone();
            let query = query.clone();
            let workspaces = workspaces.clone();
            async move {
                let defs = [
                    (
                        "list_workspace",
                        "List registered workspaces",
                        list_workspace_handler((*workspaces).clone()),
                    ),
                    (
                        "list_session",
                        "List sessions with filters",
                        list_session_handler(
                            (*sessions).clone(),
                            (*query).clone(),
                            (*workspaces).clone(),
                        ),
                    ),
                ];
                for (name, description, handler) in defs {
                    let disposer = tools.register(ToolDefinition::new(name, description, handler));
                    scope.on_dispose(move || async move { disposer.dispose().await });
                }
                Ok(())
            }
        })
        .await;
    Ok((*tools).clone())
}
