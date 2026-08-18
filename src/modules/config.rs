//! config module: reads and writes `~/.config/everyday/config.toml`.
//!
//! Implements the `Executor` trait so config is dispatched uniformly through
//! `ModuleRegistry` like every other module, removing the special branch in
//! main.rs (`if cli.module == "config"`) [R012](../../docs/adr/R012-config-executor-trait.md).

use async_trait::async_trait;
use std::path::Path;
use std::str::FromStr;

use crate::config::{Config, ConfigEditor};
use crate::error::{AgentError, Result};
use crate::modules::{Executor, parse_simple_args};
use crate::output::{Output, RenderMode};

/// config module: has no config dependency (reads/writes the file directly), so construction needs no Arc<Config>.
pub struct ConfigModule;

impl ConfigModule {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ConfigModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Executor for ConfigModule {
    fn description(&self) -> &'static str {
        "Configuration management: view / edit / create config.toml."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!("path", "显示配置文件路径", "everyday config path", &[]),
            cli_action!("list", "列出当前配置（脱敏）", "everyday config list", &[]),
            cli_action!(
                "get",
                "读取某个配置项",
                "everyday config get <dotted.path>",
                &[],
                Positional::Exactly(1)
            ),
            cli_action!(
                "set",
                "设置某个配置项",
                "everyday config set <dotted.path> <value>",
                &[],
                Positional::Exactly(2)
            ),
            cli_action!("init", "生成默认配置文件", "everyday config init", &[]),
        ];
        ModuleArgSpec {
            name: "config",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(
        &self,
        action: &str,
        args: &[String],
        _ctx: &crate::shared::request_context::RequestContext,
    ) -> Result<Output> {
        // config needs RenderMode to choose the `list` output format (TOML text / JSON).
        // Like other modules, it reads the mode via the thread-local set by main.rs at startup [R001](../../docs/adr/R001-thread-local-json-mode.md).
        let mode = if crate::util::json_mode::is_json() {
            RenderMode::Json
        } else {
            RenderMode::Text
        };
        let (_flags, positional) = parse_simple_args(args);
        run_config(action, &positional, mode).await
    }
}

/// Same semantics as the original main.rs::run_config; called by Executor::execute.
pub(crate) async fn run_config(action: &str, args: &[String], mode: RenderMode) -> Result<Output> {
    let action = if action.is_empty() { "list" } else { action };
    match action {
        "path" => {
            let p = Config::config_path()?;
            Ok(Output::text(p.display().to_string()))
        }
        "list" => {
            // Text mode renders the raw comment-preserving document (ADR R022);
            // JSON mode keeps the parsed `Config` struct (the agent contract).
            match mode {
                RenderMode::Json => {
                    let cfg = Config::load_or_default()?;
                    let v = serde_json::to_value(&cfg)?;
                    Ok(Output::Json(v))
                }
                RenderMode::Text => {
                    let path = Config::config_path()?;
                    let text = if path.exists() {
                        std::fs::read_to_string(&path)?
                    } else {
                        String::new()
                    };
                    Ok(Output::text(text))
                }
            }
        }
        "get" => {
            let path = args.first().ok_or_else(|| {
                AgentError::InvalidArgument("usage: everyday config get <dotted.path>".into())
            })?;
            let cfg_path = Config::config_path()?;
            let text = if cfg_path.exists() {
                std::fs::read_to_string(&cfg_path)?
            } else {
                String::new()
            };
            let doc: toml_edit::DocumentMut = toml_edit::DocumentMut::from_str(&text)
                .map_err(|e| AgentError::Config(format!("parse: {e}")))?;
            let v = get_dotted_doc(&doc, path)?;
            Ok(Output::text(value_to_display_string(&v)))
        }
        "set" => {
            let (path, value) = (
                args.first().ok_or_else(|| {
                    AgentError::InvalidArgument(
                        "usage: everyday config set <dotted.path> <value>".into(),
                    )
                })?,
                args.get(1).ok_or_else(|| {
                    AgentError::InvalidArgument(
                        "usage: everyday config set <dotted.path> <value>".into(),
                    )
                })?,
            );
            ConfigEditor::open()?.set_dotted(path, value)?;
            Ok(Output::text(format!("set {path} = {value}")))
        }
        "init" => {
            let path = Config::config_path()?;
            if path.exists() {
                return Ok(Output::text(format!(
                    "config already exists: {}",
                    path.display()
                )));
            }
            let example = example_config();
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
            std::fs::write(&path, example)?;
            Ok(Output::text(format!(
                "created config at: {}",
                path.display()
            )))
        }
        other => Err(AgentError::UnknownAction(format!("config {other}"))),
    }
}

/// Read a toml_edit::Value by walking a dotted path against a DocumentMut;
/// supports table fields and array indices (e.g. `mail.accounts.0.name`).
fn get_dotted_doc(doc: &toml_edit::DocumentMut, path: &str) -> Result<toml_edit::Value> {
    let segs: Vec<&str> = path.split('.').collect();
    let first = segs
        .first()
        .ok_or_else(|| AgentError::InvalidArgument(format!("empty path `{path}`")))?;
    let mut cur: toml_edit::Item =
        doc.as_table().get(first).cloned().ok_or_else(|| {
            AgentError::InvalidArgument(format!("path segment '{first}' not found"))
        })?;
    for seg in &segs[1..] {
        cur = if let Ok(idx) = seg.parse::<usize>() {
            if let Some(arr) = cur.as_array() {
                let v = arr.get(idx).cloned().ok_or_else(|| {
                    AgentError::InvalidArgument(format!("array index {idx} out of bounds"))
                })?;
                toml_edit::Item::Value(v)
            } else if let Some(aot) = cur.as_array_of_tables() {
                let t = aot.get(idx).cloned().ok_or_else(|| {
                    AgentError::InvalidArgument(format!("array index {idx} out of bounds"))
                })?;
                toml_edit::Item::Table(t)
            } else {
                return Err(AgentError::InvalidArgument(format!("'{seg}' not an array")));
            }
        } else {
            let table = cur
                .as_table()
                .ok_or_else(|| AgentError::InvalidArgument(format!("'{seg}' not a table")))?;
            table.get(seg).cloned().ok_or_else(|| {
                AgentError::InvalidArgument(format!("path segment '{seg}' not found"))
            })?
        };
    }
    cur.as_value()
        .cloned()
        .ok_or_else(|| AgentError::InvalidArgument(format!("path `{path}` is not a scalar value")))
}

/// Convert a toml_edit::Value into a terminal-friendly string.
fn value_to_display_string(v: &toml_edit::Value) -> String {
    match v {
        toml_edit::Value::String(s) => s.to_string(),
        // Other types use toml_edit's Display (consistent with `config list`).
        other => other.to_string(),
    }
}

/// Sample config written by `everyday config init`.
/// Kept in sync with config.example.toml (hand-written to avoid an include_str dependency).
fn example_config() -> String {
    include_str!("../../config.example.toml").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> toml_edit::DocumentMut {
        toml_edit::DocumentMut::from_str(text).unwrap()
    }

    #[test]
    fn get_dotted_simple_path() {
        let d = doc(r#"
[default_account]
mail = "work"
"#);
        let v = get_dotted_doc(&d, "default_account.mail").unwrap();
        assert_eq!(v.as_str(), Some("work"));
    }

    #[test]
    fn get_dotted_array_index() {
        let d = doc(r#"
[[mail.accounts]]
name = "personal"
[[mail.accounts]]
name = "work"
"#);
        let v = get_dotted_doc(&d, "mail.accounts.1.name").unwrap();
        assert_eq!(v.as_str(), Some("work"));
    }

    #[test]
    fn get_dotted_missing_segment_errors() {
        let d = doc("");
        assert!(get_dotted_doc(&d, "missing.key").is_err());
    }

    #[test]
    fn get_dotted_non_scalar_errors() {
        let d = doc("[a]\nb = 1\n");
        assert!(get_dotted_doc(&d, "a").is_err());
    }
}
