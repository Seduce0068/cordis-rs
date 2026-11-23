//! Reusable Rust port of `@deepseek-ai/dsh-mcp-resources`.
//!
//! Source: `packages/mcp/mcp-resources/src/index.ts` (+ tools.ts, render.ts)
//! in deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Registers three read-only resource tools over a provider registry.

use std::collections::HashMap;
use std::sync::Arc;

use crate::core_services::{
    ToolDefinition, ToolError, ToolExecContext, ToolHandler, ToolOutput, ToolRegistry,
};
use crate::{Context, Result};

/// Caller scope used to isolate providers. More specific scopes win; a call
/// falls back to less specific ones so a global provider remains visible.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScopeKey {
    Global,
    Agent(String),
    Session(String),
    Both { agent: String, session: String },
}

impl ScopeKey {
    fn from_exec(exec: &ToolExecContext) -> Self {
        match (&exec.agent, &exec.session) {
            (Some(agent), Some(session)) => Self::Both {
                agent: agent.clone(),
                session: session.clone(),
            },
            (Some(agent), None) => Self::Agent(agent.clone()),
            (None, Some(session)) => Self::Session(session.clone()),
            (None, None) => Self::Global,
        }
    }

    fn fallbacks(&self) -> Vec<&Self> {
        let mut out = vec![self];
        match self {
            Self::Both { .. } => {
                out.push(&Self::Global);
            }
            Self::Agent(_) | Self::Session(_) => {
                out.push(&Self::Global);
            }
            Self::Global => {}
        }
        out
    }
}

#[derive(Clone)]
struct ProviderEntry {
    provider: ResourceProvider,
    allowed_schemes: Vec<String>,
}

pub const MCP_RESOURCES_SERVICE: &str = "mcpResources";
pub const LIST_RESOURCES: &str = "list_mcp_resources";
pub const LIST_TEMPLATES: &str = "list_mcp_resource_templates";
pub const READ_RESOURCE: &str = "read_mcp_resource";

/// Simplified provider contract: a single request method + JSON params.
pub type ResourceProvider =
    Arc<dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value> + Send + Sync>;

/// Provider registry with caller-scope isolation and URI scheme allowlists.
#[derive(Clone, Default)]
pub struct McpResourceRegistry {
    providers: Arc<std::sync::RwLock<HashMap<ScopeKey, HashMap<String, ProviderEntry>>>>,
}

impl McpResourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a globally visible provider (backwards compatible).
    pub fn register(&self, server: impl Into<String>, provider: ResourceProvider) -> bool {
        self.register_scoped(ScopeKey::Global, server, provider, Vec::new())
    }

    /// Register a provider restricted to a scope with a URI scheme allowlist.
    /// An empty allowlist permits any scheme (legacy behaviour).
    pub fn register_scoped(
        &self,
        scope: ScopeKey,
        server: impl Into<String>,
        provider: ResourceProvider,
        allowed_schemes: Vec<String>,
    ) -> bool {
        self.providers
            .write()
            .expect("mcp resource registry poisoned")
            .entry(scope)
            .or_default()
            .insert(
                server.into(),
                ProviderEntry {
                    provider,
                    allowed_schemes,
                },
            )
            .is_none() // true on first registration, like other registries (R2-AP05)
    }

    /// Remove a globally registered server.
    pub fn unregister(&self, server: &str) -> bool {
        self.unregister_scoped(&ScopeKey::Global, server)
    }

    pub fn unregister_scoped(&self, scope: &ScopeKey, server: &str) -> bool {
        let mut guard = self
            .providers
            .write()
            .expect("mcp resource registry poisoned");
        let removed = guard
            .get_mut(scope)
            .and_then(|map| map.remove(server))
            .is_some();
        // Drop the scope key entirely once it has no providers left, so an
        // unregistered scope does not linger (swarm review R2-AP05).
        let empty = guard.get(scope).is_some_and(|map| map.is_empty());
        if removed && empty {
            guard.remove(scope);
        }
        removed
    }

    pub fn servers(&self) -> Vec<String> {
        let mut names = std::collections::HashSet::new();
        for map in self
            .providers
            .read()
            .expect("mcp resource registry poisoned")
            .values()
        {
            names.extend(map.keys().cloned());
        }
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort();
        names
    }

    fn call(
        &self,
        exec: &ToolExecContext,
        server: &str,
        method: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, ToolError> {
        let scope = ScopeKey::from_exec(exec);
        let guard = self
            .providers
            .read()
            .expect("mcp resource registry poisoned");
        let mut entry: Option<ProviderEntry> = None;
        for candidate in scope.fallbacks() {
            if let Some(found) = guard.get(candidate).and_then(|map| map.get(server)) {
                entry = Some(found.clone());
                break;
            }
        }
        let entry = entry.ok_or_else(|| {
            // Do not enumerate registered servers to the model (info leak).
            ToolError::InvalidArgs(format!("No MCP resource provider for server '{server}'"))
        })?;
        drop(guard);
        // Scheme allowlist is a local hardening extension (upstream has none).
        if let Some(uri) = params.get("uri").and_then(|value| value.as_str())
            && !entry.allowed_schemes.is_empty()
        {
            let allowed = entry.allowed_schemes.iter().any(|scheme| {
                uri.starts_with(&format!("{scheme}://")) || uri.starts_with(&format!("{scheme}:"))
            });
            if !allowed {
                return Err(ToolError::InvalidArgs(format!(
                    "URI scheme not allowed for server '{server}'"
                )));
            }
        }
        (entry.provider)(method, params).map_err(|error| ToolError::Execution(error.to_string()))
    }
}

fn server_arg(args: &serde_json::Value) -> std::result::Result<String, ToolError> {
    args.get("server")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("server must be a string".to_string()))
}

fn list_handler(registry: McpResourceRegistry, method: &'static str) -> ToolHandler {
    Arc::new(move |args, exec| {
        let registry = registry.clone();
        let exec = exec.clone();
        Box::pin(async move {
            let server = server_arg(&args)?;
            let mut params = serde_json::json!({});
            if let Some(cursor) = args.get("cursor").and_then(|value| value.as_str()) {
                params["cursor"] = serde_json::json!(cursor);
            }
            let result = registry.call(&exec, &server, method, params)?;
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
    Arc::new(move |args, exec| {
        let registry = registry.clone();
        let exec = exec.clone();
        Box::pin(async move {
            let server = server_arg(&args)?;
            let uri = args
                .get("uri")
                .and_then(|value| value.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("uri must be a string".to_string()))?;
            let result = registry.call(
                &exec,
                &server,
                "resources/read",
                serde_json::json!({ "uri": uri }),
            )?;
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
    let service_disposer = ctx.provide(MCP_RESOURCES_SERVICE, registry.clone()).await?;

    let result_registry = registry.clone();
    let tools_for_effect = tools.clone();
    let effect_disposer = service_disposer.clone();
    let effect = ctx
        .effect(move |scope| {
            let tools = tools_for_effect.clone();
            let registry = registry.clone();
            let service_disposer = effect_disposer.clone();
            async move {
                // Roll the service back together with the tool registrations when
                // setup fails (swarm review R14-RB06).
                scope.on_dispose(move || async move { service_disposer.dispose().await });
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
                    scope.on_dispose(move || async move { disposer.dispose().await });
                }
                Ok(())
            }
        })
        .await;
    if let Err(error) = effect {
        // The disposer had not reached the effect scope yet (swarm review
        // R15-AS06), so roll the service back explicitly.
        service_disposer.dispose().await?;
        return Err(error);
    }
    Ok(result_registry)
}
