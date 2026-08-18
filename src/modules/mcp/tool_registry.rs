//! MCP protocol projection — the single testable seam of the `mcp` module.
//!
//! Every `(module, action)` pair of the CLI is projected into an MCP **tool**
//! named `<module>_<action>` with a JSON Schema generated from
//! `module_arg_spec()` — the same source of truth the clap CLI uses
//! ([F007](../../../docs/adr/F007-clap-subcommand-tree.md)). Because the schema
//! is derived, the MCP surface can never drift from the CLI. See
//! [F014](../../../docs/adr/F014-mcp-module.md) and `CONTEXT.md` §MCP.

use crate::error::{AgentError, Result};
use crate::modules::{ActionArgSpec, ArgKind, ModuleRegistry, Positional};
use crate::output::RenderMode;
use crate::shared::request_context::RequestContext;
use std::collections::HashMap;
use std::sync::Arc;

/// One projected MCP tool definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    /// Tool name: `<module>_<action>`.
    pub name: String,
    /// Human description for the MCP client / model.
    pub description: String,
    /// JSON Schema (draft 2020-12 style) for the tool's arguments.
    pub input_schema: serde_json::Value,
}

/// Build the JSON Schema for one action's arguments from its declared arg spec.
///
/// Mapping (single source of truth = `module_arg_spec()`):
/// - `ArgKind::Value` → `{"type": "string"}`
/// - `ArgKind::Bool`  → `{"type": "boolean"}`
/// - `ArgKind::Multi` → `{"type": "array", "items": {"type": "string"}}`
/// - positional slot  → `args` array property (`required` when `Exactly(n)`)
/// - global `--account` → optional `account` string property (CLI semantics)
fn action_schema(action: &ActionArgSpec) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "account".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "target account (CLI --account); defaults to the configured account",
        }),
    );
    for a in action.args {
        let prop = match a.kind {
            crate::modules::ArgKind::Value => serde_json::json!({
                "type": "string",
                "description": a.help,
            }),
            crate::modules::ArgKind::Bool => serde_json::json!({
                "type": "boolean",
                "description": a.help,
            }),
            crate::modules::ArgKind::Multi => serde_json::json!({
                "type": "array",
                "items": { "type": "string" },
                "description": a.help,
            }),
        };
        properties.insert(a.name.to_string(), prop);
    }

    let positional_required = matches!(
        action.positional,
        Positional::Exactly(_) | Positional::OneOrMore
    );
    if !matches!(action.positional, Positional::None) {
        properties.insert(
            "args".to_string(),
            serde_json::json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "positional arguments, in order",
            }),
        );
    }

    let mut schema = serde_json::json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false,
    });
    if positional_required {
        schema["required"] = serde_json::json!(["args"]);
    }
    schema
}

/// Whether a `(module, action)` mutates data — used to mark MCP tool
/// descriptions as writes.
///
/// A superset of [`crate::modules::sync::is_write_action`] (which gates file
/// auto-sync): here every state-changing action counts, including network
/// writes. Kept next to its sibling so the two sets drift only deliberately.
fn is_write(module: &str, action: &str) -> bool {
    crate::modules::sync::is_write_action(module, action)
        || matches!(
            (module, action),
            ("mail", "send")
                | ("rss", "follow" | "unfollow")
                | ("auth", "login" | "logout")
                | ("timeline", "sync")
                | ("task", "run")
        )
}

/// One projected `(module, action)`: its tool definition plus the metadata
/// needed to dispatch a call.
struct Projected {
    def: ToolDef,
    module: &'static str,
    action: &'static str,
    spec: &'static ActionArgSpec,
}

/// The single projection source shared by `mcp tools` and `mcp serve`.
///
/// Every `(module, action)` of the registry becomes a tool; the `mcp` module
/// itself is excluded — projecting `mcp_serve`/`mcp_tools` would expose
/// recursive server control with no consumer.
fn project(registry: &ModuleRegistry) -> Vec<Projected> {
    let mut entries: Vec<(&str, &dyn crate::modules::Executor)> = registry
        .modules
        .iter()
        .map(|(k, v)| (*k, v.as_ref()))
        .collect();
    // Deterministic order (alphabetical by module name) for stable output.
    entries.sort_by_key(|(k, _)| *k);

    let mut out = Vec::new();
    for (module_name, module) in entries {
        if module_name == "mcp" {
            continue;
        }
        let spec = module.module_arg_spec();
        for action in spec.actions {
            let mut description = action.description.to_string();
            if is_write(module_name, action.name) {
                description.push_str(" [WRITE]");
            }
            out.push(Projected {
                def: ToolDef {
                    name: format!("{module_name}_{}", action.name),
                    description,
                    input_schema: action_schema(action),
                },
                module: module_name,
                action: action.name,
                spec: action,
            });
        }
    }
    out
}

/// Project every `(module, action)` of a registry into MCP tool definitions.
pub fn project_tools(registry: &ModuleRegistry) -> Result<Vec<ToolDef>> {
    Ok(project(registry).into_iter().map(|p| p.def).collect())
}

/// Outcome of one tool call, mapped to MCP semantics: the JSON-rendered text
/// and whether the call failed (`isError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallOutcome {
    /// Rendered `--json` output (or the error message on failure).
    pub text: String,
    /// True when the tool ran but failed (`isError`); false on success.
    pub is_error: bool,
}

/// A callable tool registry bound to a `ModuleRegistry`.
///
/// Reuses the module's `Executor::execute` unchanged: the same dispatch path
/// the CLI uses, with a fresh `RequestContext` per call. Tool calls are
/// serialized behind a mutex because everyday modules assume a
/// single-invocation lifecycle (a single MCP client sends serial calls, so
/// this costs nothing).
pub struct ToolRegistry {
    registry: Arc<ModuleRegistry>,
    tools: Vec<ToolDef>,
    /// tool name → (module name, action name, action arg spec)
    dispatch: HashMap<String, (&'static str, &'static str, &'static ActionArgSpec)>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl ToolRegistry {
    /// Build the registry (project all tools) from a fully-assembled registry.
    pub fn new(registry: Arc<ModuleRegistry>) -> Result<Self> {
        let projected = project(&registry);
        let mut tools = Vec::with_capacity(projected.len());
        let mut dispatch = HashMap::with_capacity(projected.len());
        for p in projected {
            dispatch.insert(p.def.name.clone(), (p.module, p.action, p.spec));
            tools.push(p.def);
        }
        Ok(Self {
            registry,
            tools,
            dispatch,
            lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// The projected tool definitions (stable order, sorted by tool name
    /// within module order).
    pub fn tools(&self) -> &[ToolDef] {
        &self.tools
    }

    /// Execute one tool call against the underlying registry.
    ///
    /// Returns `Err` only for an unknown tool name (the server maps this to a
    /// protocol-level `METHOD_NOT_FOUND`). A tool that ran but failed comes
    /// back as [`CallOutcome::is_error`] = true.
    pub async fn call(
        &self,
        name: &str,
        arguments: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallOutcome> {
        let (module, action, spec) = self
            .dispatch
            .get(name)
            .copied()
            .ok_or_else(|| AgentError::InvalidArgument(format!("unknown tool: {name}")))?;
        let args = args_from_json(spec, arguments)?;
        let ctx = RequestContext::mcp(crate::shared::request_context::generate_request_id());

        let module_obj = self.registry.get(module)?;
        let _guard = self.lock.lock().await;
        match module_obj.execute(action, &args, &ctx).await {
            Ok(out) => Ok(CallOutcome {
                text: out.render(RenderMode::Json),
                is_error: false,
            }),
            Err(e) => Ok(CallOutcome {
                text: e.message(),
                is_error: true,
            }),
        }
    }
}

/// Convert MCP JSON arguments into the CLI's `Vec<String>` form, driven by the
/// action's arg spec (so the shape matches `module_arg_spec()` exactly).
///
/// - `account` → `--account <v>` (global flag semantics)
/// - `ArgKind::Value` → `--name <v>`
/// - `ArgKind::Bool`  → `--name` when true
/// - `ArgKind::Multi` → `--name <v>` per array element
/// - positional slot → appended verbatim
fn args_from_json(
    spec: &ActionArgSpec,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if let Some(acc) = args.get("account").and_then(|v| v.as_str()) {
        out.push("--account".to_string());
        out.push(acc.to_string());
    }
    for a in spec.args {
        let Some(val) = args.get(a.name) else {
            continue;
        };
        match a.kind {
            ArgKind::Value => {
                if let Some(s) = val.as_str() {
                    out.push(format!("--{}", a.name));
                    out.push(s.to_string());
                }
            }
            ArgKind::Bool => {
                if val.as_bool() == Some(true) {
                    out.push(format!("--{}", a.name));
                }
            }
            ArgKind::Multi => {
                if let Some(arr) = val.as_array() {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            out.push(format!("--{}", a.name));
                            out.push(s.to_string());
                        }
                    }
                }
            }
        }
    }
    match spec.positional {
        Positional::None => {}
        _ => {
            if let Some(arr) = args.get("args").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::modules::ModuleRegistry;
    use serde_json::json;
    use std::sync::Arc;

    fn default_registry() -> Arc<ModuleRegistry> {
        ModuleRegistry::build(Arc::new(Config::default())).unwrap()
    }

    fn tool_named<'a>(tools: &'a [ToolDef], name: &str) -> &'a ToolDef {
        tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("tool {name} not found"))
    }

    #[test]
    fn projects_every_module() {
        let registry = default_registry();
        let tools = project_tools(&registry).unwrap();
        let mut modules: Vec<&str> = registry
            .modules
            .keys()
            .filter(|k| **k != "mcp")
            .copied()
            .collect();
        modules.sort_unstable();
        for m in modules {
            assert!(
                tools.iter().any(|t| t.name.starts_with(&format!("{m}_"))),
                "module {m} has no projected tool"
            );
        }
    }

    #[test]
    fn tool_names_are_unique_and_slugged() {
        let tools = project_tools(&default_registry()).unwrap();
        let mut seen = std::collections::HashSet::new();
        for t in &tools {
            assert!(
                seen.insert(t.name.clone()),
                "duplicate tool name {}",
                t.name
            );
            // `<module>_<action>`; neither segment contains an underscore.
            let (module, action) = t.name.split_once('_').unwrap();
            assert!(!module.is_empty() && !action.is_empty());
        }
        assert!(!tools.is_empty());
    }

    #[test]
    fn excludes_mcp_itself() {
        let tools = project_tools(&default_registry()).unwrap();
        assert!(!tools.iter().any(|t| t.name.starts_with("mcp_")));
    }

    #[test]
    fn schema_has_account_and_typed_flags() {
        let tools = project_tools(&default_registry()).unwrap();
        // `mail list` (mail module) — known flags exist; account always present.
        let t = tool_named(&tools, "mail_list");
        let props = t.input_schema["properties"].as_object().unwrap();
        assert_eq!(props["account"]["type"], "string");
        assert!(props.contains_key("folder") || props.contains_key("limit"));
    }

    #[test]
    fn schema_models_boolean_flag() {
        let tools = project_tools(&default_registry()).unwrap();
        // Find a Bool flag on some tool and assert it renders as boolean.
        let mut found = false;
        for t in &tools {
            let props = t.input_schema["properties"].as_object().unwrap();
            for (k, v) in props {
                if v["type"] == "boolean" {
                    found = true;
                    assert_ne!(k, "account");
                }
            }
        }
        assert!(found, "no boolean flag projected anywhere");
    }

    #[test]
    fn write_tools_are_marked() {
        let tools = project_tools(&default_registry()).unwrap();
        let t = tool_named(&tools, "mail_send");
        assert!(
            t.description.contains("[WRITE]"),
            "mail_send must be marked write: {}",
            t.description
        );
        let t = tool_named(&tools, "memory_add");
        assert!(t.description.contains("[WRITE]"));
        let t = tool_named(&tools, "task_run");
        assert!(t.description.contains("[WRITE]"));
        let t = tool_named(&tools, "config_path");
        assert!(!t.description.contains("[WRITE]"));
        let t = tool_named(&tools, "mail_list");
        assert!(!t.description.contains("[WRITE]"));
    }

    #[test]
    fn schema_models_positional_args() {
        let tools = project_tools(&default_registry()).unwrap();
        // `config get` takes exactly one positional (`config get <dotted.path>`).
        let t = tool_named(&tools, "config_get");
        assert!(t.input_schema["properties"]["args"].is_object());
        assert_eq!(
            t.input_schema["required"],
            serde_json::json!(["args"]),
            "Exactly(1) positional must be required"
        );
        // `config path` has no positional → no args property, no required.
        let t2 = tool_named(&tools, "config_path");
        assert!(t2.input_schema["properties"].get("args").is_none());
        assert!(t2.input_schema.get("required").is_none());
    }

    // ---- args_from_json ----

    fn config_get_spec() -> &'static ActionArgSpec {
        let registry = default_registry();
        let m = registry.get("config").unwrap();
        let spec = m.module_arg_spec();
        spec.actions.iter().find(|a| a.name == "get").unwrap()
    }

    fn mail_list_spec() -> &'static ActionArgSpec {
        let registry = default_registry();
        let m = registry.get("mail").unwrap();
        let spec = m.module_arg_spec();
        spec.actions.iter().find(|a| a.name == "list").unwrap()
    }

    fn args(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn args_positional_and_account() {
        let spec = config_get_spec();
        let out = args_from_json(spec, &args(json!({"args": ["default_account.mail"]}))).unwrap();
        assert_eq!(out, vec!["default_account.mail"]);
        let out = args_from_json(spec, &args(json!({"account": "work", "args": ["a.b"]}))).unwrap();
        assert_eq!(out, vec!["--account", "work", "a.b"]);
    }

    #[test]
    fn args_boolean_flag_only_when_true() {
        let spec = mail_list_spec();
        let out = args_from_json(spec, &args(json!({"unread": true}))).unwrap();
        assert!(out.iter().any(|a| a == "--unread"));
        assert!(!out.iter().any(|a| a == "--unread=false"));
        let out = args_from_json(spec, &args(json!({"unread": false}))).unwrap();
        assert!(!out.iter().any(|a| a == "--unread"));
    }

    #[test]
    fn args_multi_flag_repeats() {
        static ARGS: &[crate::modules::ArgSpec] = &[crate::modules::ArgSpec {
            name: "tag",
            help: "repeatable tag",
            kind: ArgKind::Multi,
            allow_hyphen_values: false,
        }];
        let synthetic = ActionArgSpec {
            name: "x",
            description: "x",
            usage: "x",
            args: ARGS,
            positional: Positional::None,
        };
        let out = args_from_json(&synthetic, &args(json!({"tag": ["a", "b"]}))).unwrap();
        assert_eq!(
            out,
            vec![
                "--tag".to_string(),
                "a".into(),
                "--tag".to_string(),
                "b".into()
            ]
        );
    }

    // ---- ToolRegistry::call ----

    #[tokio::test]
    async fn call_returns_json_rendered_output() {
        let registry = default_registry();
        let tr = ToolRegistry::new(registry).unwrap();
        let out = tr.call("config_path", &args(json!({}))).await.unwrap();
        assert!(!out.is_error);
        assert!(!out.text.is_empty());
    }

    #[tokio::test]
    async fn call_marks_failure_as_is_error() {
        let registry = default_registry();
        let tr = ToolRegistry::new(registry).unwrap();
        // config get with no positional args → module-level invalid argument.
        let out = tr.call("config_get", &args(json!({}))).await.unwrap();
        assert!(out.is_error);
        assert!(!out.text.is_empty());
    }

    #[tokio::test]
    async fn call_unknown_tool_errors() {
        let registry = default_registry();
        let tr = ToolRegistry::new(registry).unwrap();
        let err = tr.call("no_such_tool", &args(json!({}))).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn registry_tools_exclude_mcp() {
        let registry = default_registry();
        let tr = ToolRegistry::new(registry).unwrap();
        assert!(!tr.tools().iter().any(|t| t.name.starts_with("mcp_")));
    }
}
