//! `mcp` module — everyday as a Model Context Protocol (MCP) server over stdio.
//!
//! Every other module's `(module, action)` is projected into an MCP tool
//! (see [`tool_registry`]); `everyday mcp serve` runs the server, `everyday
//! mcp tools` prints the projected tool list for debugging. Design decisions:
//! [F014](../../../docs/adr/F014-mcp-module.md) and `CONTEXT.md` §MCP.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, Tool,
};
use rmcp::service::RequestContext as McpRequestContext;
use rmcp::transport::stdio;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};

use crate::error::{AgentError, Result};
use crate::modules::{Executor, ModuleRegistry};
use crate::output::Output;
use crate::shared::request_context::RequestContext;

use self::tool_registry::ToolRegistry;

/// Protocol projection — the pure, transport-free seam
/// ([F014](../../../docs/adr/F014-mcp-module.md)).
pub mod tool_registry;

/// The `mcp` module.
///
/// Cross-module orchestrator: constructed with a cell that
/// [`ModuleRegistry::build`] fills with the fully-assembled registry after
/// construction (build-time self-reference — the module must project and
/// dispatch every module, so it needs the registry it lives in).
pub struct McpModule {
    registry: Arc<OnceLock<Arc<ModuleRegistry>>>,
}

impl McpModule {
    /// Create the module; the cell is populated by `ModuleRegistry::build`.
    pub fn new(registry: Arc<OnceLock<Arc<ModuleRegistry>>>) -> Self {
        Self { registry }
    }

    /// Resolve the populated registry (guaranteed set after `build`).
    fn registry(&self) -> Result<Arc<ModuleRegistry>> {
        self.registry
            .get()
            .cloned()
            .ok_or_else(|| AgentError::Other("mcp registry not initialized".into()))
    }

    /// Run the MCP stdio server until stdin EOF, then exit directly.
    async fn serve(&self) -> Result<Output> {
        let registry = match self.registry() {
            Ok(r) => r,
            Err(e) => return err_and_exit(&format!("mcp serve: {}", e.message())),
        };
        let tool_registry = match ToolRegistry::new(registry.clone()) {
            Ok(t) => Arc::new(t),
            Err(e) => return err_and_exit(&format!("mcp serve: {}", e.message())),
        };

        // Session lifecycle: initialize every module once (best-effort —
        // warnings go to stderr, never stdout).
        for (name, e) in registry.initialize_all() {
            eprintln!("warning: {name} initialize failed: {}", e.message());
        }

        let server = match McpServer::new(tool_registry) {
            Ok(s) => s,
            Err(e) => return err_and_exit(&format!("mcp serve: {}", e.message())),
        };
        let running = match server.serve(stdio()).await {
            Ok(r) => r,
            Err(e) => return err_and_exit(&format!("mcp serve: {e}")),
        };
        let _ = running.waiting().await;

        registry.shutdown_all();

        // Long-lived process: stdout is reserved for JSON-RPC. Exit directly,
        // bypassing main's final `println!`, so the protocol stays clean.
        std::process::exit(0);
    }
}

/// Startup-failure exit for `serve`: report on stderr and exit 1, never
/// letting an `Err(Output)` reach main's stdout render path (which would
/// corrupt the JSON-RPC channel).
fn err_and_exit(msg: &str) -> Result<Output> {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

#[async_trait]
impl Executor for McpModule {
    fn description(&self) -> &'static str {
        "MCP server: expose everyday capabilities as MCP tools over stdio."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ModuleArgSpec};
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!(
                "serve",
                "运行 MCP stdio server，将各模块能力暴露为 MCP tools",
                "everyday mcp serve",
                &[]
            ),
            cli_action!(
                "tools",
                "列出将被投影的 MCP tools（名称 / 描述 / JSON Schema）",
                "everyday mcp tools",
                &[]
            ),
        ];
        ModuleArgSpec {
            name: "mcp",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(
        &self,
        action: &str,
        _args: &[String],
        _ctx: &RequestContext,
    ) -> Result<Output> {
        match action {
            "tools" => {
                let registry = self.registry()?;
                let tools = crate::modules::mcp::tool_registry::project_tools(&registry)?;
                let list: Vec<serde_json::Value> = tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "input_schema": t.input_schema,
                        })
                    })
                    .collect();
                Ok(Output::Json(serde_json::Value::Array(list)))
            }
            // Long-lived process; blocks until stdin EOF then exits (never
            // returns an Output to main's render path).
            "serve" => self.serve().await,
            other => Err(AgentError::UnknownAction(format!("mcp {other}"))),
        }
    }
}

/// `rmcp` server adapter: wraps a [`ToolRegistry`] in the MCP `ServerHandler`
/// contract. Thin on purpose — all projection/dispatch logic lives in the
/// registry (the testable seam), this type only maps MCP requests to it.
#[derive(Clone)]
struct McpServer {
    /// Materialized `rmcp::Tool` list (built once at serve start).
    tools: Arc<Vec<Tool>>,
    tool_registry: Arc<ToolRegistry>,
}

impl McpServer {
    fn new(tool_registry: Arc<ToolRegistry>) -> Result<Self> {
        let tools = tool_registry
            .tools()
            .iter()
            .map(|t| {
                let input_schema: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_value(t.input_schema.clone())
                        .map_err(|e| AgentError::Other(format!("mcp tool schema: {e}")))?;
                Ok(Tool::new(
                    t.name.clone(),
                    t.description.clone(),
                    Arc::new(input_schema),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            tools: Arc::new(tools),
            tool_registry,
        })
    }
}

impl ServerHandler for McpServer {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: McpRequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        // Unpaginated: return the full tool list in one response.
        Ok(ListToolsResult {
            result_type: None,
            meta: None,
            next_cursor: None,
            ttl_ms: None,
            cache_scope: None,
            tools: (*self.tools).clone(),
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: McpRequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();
        match self.tool_registry.call(&name, &arguments).await {
            Ok(outcome) => {
                let content = vec![ContentBlock::text(outcome.text)];
                let result = if outcome.is_error {
                    CallToolResult::error(content)
                } else {
                    CallToolResult::success(content)
                };
                Ok(result.into())
            }
            // Unknown tool → protocol-level error, per rmcp's documented
            // choice for unroutable `tools/call` requests.
            Err(_) => Err(McpError::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn spec_declares_serve_and_tools() {
        let cell: Arc<OnceLock<Arc<ModuleRegistry>>> = Arc::new(OnceLock::new());
        let m = McpModule::new(cell);
        let spec = m.module_arg_spec();
        assert_eq!(spec.name, "mcp");
        let actions: Vec<&str> = spec.actions.iter().map(|a| a.name).collect();
        assert_eq!(actions, vec!["serve", "tools"]);
    }

    #[test]
    fn registry_uninitialized_errors() {
        let cell: Arc<OnceLock<Arc<ModuleRegistry>>> = Arc::new(OnceLock::new());
        let m = McpModule::new(cell);
        assert!(m.registry().is_err());
    }
}
