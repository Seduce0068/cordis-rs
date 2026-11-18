//! Reusable Rust port of `@deepseek-ai/dsh-mcp-resources`.
//!
//! Source: `packages/mcp/mcp-resources/src/index.ts` (+ tools.ts, render.ts)
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers three read-only resource tools over a provider registry.

use std::collections::HashMap;
use std::sync::Arc;

use crate::core_services::{ToolDefinition, ToolError, ToolHandler, ToolOutput, ToolRegistry};
use crate::{Context, Result};

pub const MCP_RESOURCES_SERVICE: &str = "mcpResources";
pub const LIST_RESOURCES: &str = "list_mcp_resources";
pub const LIST_TEMPLATES: &str = "list_mcp_resource_templates";
pub const READ_RESOURCE: &str = "read_mcp_resource";

/// Simplified provider contract: a single request method + JSON params.
pub type ResourceProvider =
    Arc<dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value> + Send + Sync>;

/// Global provider registry. Upstream selects providers from the calling
/// agent's scoped registry; core services have no Scope type, so this port
/// intentionally uses one global map (scope isolation is not implemented).
#[derive(Clone, Default)]
pub struct McpResourceRegistry {
    providers: Arc<std::sync::RwLock<HashMap<String, ResourceProvider>>>,
}

impl McpResourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider for one server. Returns true when replaced.
    pub fn register(&self, server: impl Into<String>, provider: ResourceProvider) -> bool {
        self.providers
            .write()
            .expect("mcp resource registry poisoned")
            .insert(server.into(), provider)
            .is_some()
    }

    pub fn unregister(&self, server: &str) -> bool {
        self.providers
            .write()
            .expect("mcp resource registry poisoned")
            .remove(server)
            .is_some()
    }

    pub fn servers(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .providers
            .read()
            .expect("mcp resource registry poisoned")
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    fn call(
        &self,
        server: &str,
        method: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, ToolError> {
        let provider = self
            .providers
            .read()
            .expect("mcp resource registry poisoned")
            .get(server)
            .cloned()
            .ok_or_else(|| {
                // Do not enumerate registered servers to the model (info leak).
                ToolError::InvalidArgs(format!("No MCP resource provider for server '{server}'"))
            })?;
        provider(method, params).map_err(|error| ToolError::Execution(error.to_string()))
    }
}

fn server_arg(args: &serde_json::Value) -> std::result::Result<String, ToolError> {
    args.get("server")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("server must be a string".to_string()))
}

fn list_handler(registry: McpResourceRegistry, method: &'static str) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let registry = registry.clone();
        Box::pin(async move {
            let server = server_arg(&args)?;
            let mut params = serde_json::json!({});
            if let Some(cursor) = args.get("cursor").and_then(|value| value.as_str()) {
                params["cursor"] = serde_json::json!(cursor);
            }
            let result = registry.call(&server, method, params)?;
            Ok(ToolOutput {
                rendered: vec![format!(
                    "{} resource(s) from {server}",
                    result
                        .get("resources")
                        .and_then(|v| v.as_array())
                        .map_or(0, Vec::len)
                )],
                content: result,
            })
        })
    })
}

fn read_handler(registry: McpResourceRegistry) -> ToolHandler {
    Arc::new(move |args, _exec| {
        let registry = registry.clone();
        Box::pin(async move {
            let server = server_arg(&args)?;
            let uri = args
                .get("uri")
                .and_then(|value| value.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("uri must be a string".to_string()))?;
            let result =
                registry.call(&server, "resources/read", serde_json::json!({ "uri": uri }))?;
            Ok(ToolOutput {
                rendered: vec![format!("Read {uri} from {server}")],
                content: result,
            })
        })
    })
}

/// The `dsh-mcp-resources` plugin.
pub struct McpResourcesPlugin;

#[async_trait::async_trait]
impl crate::Plugin for McpResourcesPlugin {
    fn name(&self) -> &str {
        "mcp-resources"
    }

    fn inject(&self) -> &[&str] {
        &["tools"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        let _ = ctx.require::<ToolRegistry>("tools").await?;
        Ok(())
    }
}

/// Mount the registry service and the three tools. Returns the registry so
/// hosts can register providers.
pub async fn mount(ctx: &Context) -> Result<McpResourceRegistry> {
    let tools = ctx.require::<ToolRegistry>("tools").await?;
    let registry = McpResourceRegistry::new();
    let _ = ctx.provide(MCP_RESOURCES_SERVICE, registry.clone()).await?;

    let result_registry = registry.clone();
    let tools_for_effect = tools.clone();
    ctx.effect(move |scope| {
        let tools = tools_for_effect.clone();
        let registry = registry.clone();
        async move {
            let defs = [
                (
                    LIST_RESOURCES,
                    "List MCP resources",
                    list_handler(registry.clone(), "resources/list"),
                ),
                (
                    LIST_TEMPLATES,
                    "List MCP resource templates",
                    list_handler(registry.clone(), "resources/templates/list"),
                ),
                (
                    READ_RESOURCE,
                    "Read an MCP resource",
                    read_handler(registry),
                ),
            ];
            for (name, description, handler) in defs {
                let disposer = tools.register(ToolDefinition::new(name, description, handler));
                scope.on_dispose(move || async move {
                    let _ = disposer.dispose().await;
                    Ok(())
                });
            }
            Ok(())
        }
    })
    .await?;
    Ok(result_registry)
}
