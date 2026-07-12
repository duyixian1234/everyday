//! config module: reads and writes `~/.config/everyday/config.toml`.
//!
//! Implements the `Executor` trait so config is dispatched uniformly through
//! `ModuleRegistry` like every other module, removing the special branch in
//! main.rs (`if cli.module == "config"`) [R012](../../docs/adr/R012-config-executor-trait.md).

use async_trait::async_trait;
use std::path::Path;

use crate::config::Config;
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
            ActionArgSpec {
                name: "path",
                description: "显示配置文件路径",
                usage: "everyday config path",
                args: &[],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "list",
                description: "列出当前配置（脱敏）",
                usage: "everyday config list",
                args: &[],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "get",
                description: "读取某个配置项",
                usage: "everyday config get <dotted.path>",
                args: &[],
                positional: Positional::Exactly(1),
            },
            ActionArgSpec {
                name: "set",
                description: "设置某个配置项",
                usage: "everyday config set <dotted.path> <value>",
                args: &[],
                positional: Positional::Exactly(2),
            },
            ActionArgSpec {
                name: "init",
                description: "生成默认配置文件",
                usage: "everyday config init",
                args: &[],
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "config",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
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
            let cfg = Config::load_or_default()?;
            match mode {
                RenderMode::Json => {
                    let v = serde_json::to_value(&cfg)?;
                    Ok(Output::Json(v))
                }
                RenderMode::Text => {
                    let toml_str = toml::to_string_pretty(&cfg)
                        .map_err(|e| AgentError::Config(format!("serialize: {e}")))?;
                    Ok(Output::text(toml_str))
                }
            }
        }
        "get" => {
            let path = args.first().ok_or_else(|| {
                AgentError::InvalidArgument("usage: everyday config get <dotted.path>".into())
            })?;
            let cfg = Config::load_or_default()?;
            let toml_val: toml::Value = toml::Value::try_from(&cfg)
                .map_err(|e| AgentError::Config(format!("serialize: {e}")))?;
            let v = get_dotted(&toml_val, path)?;
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
            set_config_path(path, value)?;
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

/// Read a toml::Value by walking a dotted path; supports table fields and array indices (e.g. `mail.accounts.0.name`).
fn get_dotted(root: &toml::Value, path: &str) -> Result<toml::Value> {
    let mut cur = root.clone();
    for seg in path.split('.') {
        cur = if let Some(table) = cur.as_table() {
            table.get(seg).cloned().ok_or_else(|| {
                AgentError::InvalidArgument(format!("path segment '{seg}' not found"))
            })?
        } else if let Some(arr) = cur.as_array() {
            let idx: usize = seg.parse().map_err(|_| {
                AgentError::InvalidArgument(format!("array index '{seg}' not a number"))
            })?;
            arr.get(idx).cloned().ok_or_else(|| {
                AgentError::InvalidArgument(format!("array index {idx} out of bounds"))
            })?
        } else {
            return Err(AgentError::InvalidArgument(format!(
                "path segment '{seg}' not found"
            )));
        };
    }
    Ok(cur)
}

/// Set the value at a dotted path and persist it. The value type is inferred automatically (bool / int / float / string).
/// Supports table fields and array indices (arrays are extended automatically).
fn set_config_path(path: &str, raw_value: &str) -> Result<()> {
    let cfg_path = Config::config_path()?;
    // Read the existing file into a toml::Value (empty table if absent).
    let mut root: toml::Value = if cfg_path.exists() {
        let text = std::fs::read_to_string(&cfg_path)?;
        if text.trim().is_empty() {
            toml::Value::Table(toml::value::Table::new())
        } else {
            toml::from_str(&text)?
        }
    } else {
        toml::Value::Table(toml::value::Table::new())
    };

    let new_val = parse_value(raw_value);
    let segs: Vec<&str> = path.split('.').collect();
    upsert_dotted(&mut root, &segs, new_val)?;

    let text =
        toml::to_string_pretty(&root).map_err(|e| AgentError::Config(format!("serialize: {e}")))?;
    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, text)?;
    Ok(())
}

/// Insert a value into a toml::Value following the path segments; intermediate tables and arrays are created automatically.
fn upsert_dotted(root: &mut toml::Value, segs: &[&str], val: toml::Value) -> Result<()> {
    if segs.is_empty() {
        return Err(AgentError::InvalidArgument("empty path".into()));
    }
    let (last, rest) = segs.split_last().unwrap();
    let mut cur = root;
    for seg in rest {
        // Array index (pure number).
        if let Ok(idx) = seg.parse::<usize>() {
            cur = ensure_array_index(cur, idx)?;
        } else {
            let table = cur
                .as_table_mut()
                .ok_or_else(|| AgentError::InvalidArgument(format!("'{seg}' not a table")))?;
            // If the key is absent, create an empty table (the final segment overwrites).
            if !table.contains_key(*seg) {
                table.insert(
                    (*seg).to_string(),
                    toml::Value::Table(toml::value::Table::new()),
                );
            }
            cur = table.get_mut(*seg).unwrap();
        }
    }
    if let Ok(idx) = last.parse::<usize>() {
        let arr = cur
            .as_array_mut()
            .ok_or_else(|| AgentError::InvalidArgument(format!("'{last}' not an array")))?;
        ensure_array_len(arr, idx + 1);
        arr[idx] = val;
    } else {
        let table = cur
            .as_table_mut()
            .ok_or_else(|| AgentError::InvalidArgument(format!("'{last}' not a table")))?;
        table.insert((*last).to_string(), val);
    }
    Ok(())
}

fn ensure_array_index(v: &mut toml::Value, idx: usize) -> Result<&mut toml::Value> {
    let arr = v
        .as_array_mut()
        .ok_or_else(|| AgentError::InvalidArgument("not an array".into()))?;
    ensure_array_len(arr, idx + 1);
    Ok(&mut arr[idx])
}

fn ensure_array_len(arr: &mut Vec<toml::Value>, len: usize) {
    while arr.len() < len {
        arr.push(toml::Value::Table(toml::value::Table::new()));
    }
}

/// Parse a raw string into the most appropriate toml value type.
fn parse_value(raw: &str) -> toml::Value {
    if raw == "true" {
        return toml::Value::Boolean(true);
    }
    if raw == "false" {
        return toml::Value::Boolean(false);
    }
    if let Ok(n) = raw.parse::<i64>() {
        return toml::Value::Integer(n);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return toml::Value::Float(f);
    }
    toml::Value::String(raw.to_string())
}

/// Convert a toml::Value into a terminal-friendly string.
fn value_to_display_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        // Other types use toml's Display (consistent with the `list` mode style).
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

    #[test]
    fn parse_value_bool() {
        assert!(matches!(parse_value("true"), toml::Value::Boolean(true)));
        assert!(matches!(parse_value("false"), toml::Value::Boolean(false)));
    }

    #[test]
    fn parse_value_int() {
        assert!(matches!(parse_value("42"), toml::Value::Integer(42)));
        assert!(matches!(parse_value("-1"), toml::Value::Integer(-1)));
    }

    #[test]
    fn parse_value_float() {
        assert!(matches!(parse_value("3.14"), toml::Value::Float(_)));
    }

    #[test]
    fn parse_value_string() {
        assert!(matches!(parse_value("hello"), toml::Value::String(s) if s == "hello"));
    }

    #[test]
    fn get_dotted_simple_path() {
        let v: toml::Value = toml::from_str(
            r#"
[default_account]
mail = "work"
"#,
        )
        .unwrap();
        assert_eq!(
            get_dotted(&v, "default_account.mail").unwrap().as_str(),
            Some("work")
        );
    }

    #[test]
    fn get_dotted_array_index() {
        let v: toml::Value = toml::from_str(
            r#"
[[mail.accounts]]
name = "personal"
[[mail.accounts]]
name = "work"
"#,
        )
        .unwrap();
        assert_eq!(
            get_dotted(&v, "mail.accounts.1.name").unwrap().as_str(),
            Some("work")
        );
    }

    #[test]
    fn get_dotted_missing_segment_errors() {
        let v: toml::Value = toml::from_str("").unwrap();
        assert!(get_dotted(&v, "missing.key").is_err());
    }

    #[test]
    fn upsert_dotted_creates_intermediate_table() {
        let mut v = toml::Value::Table(toml::value::Table::new());
        upsert_dotted(
            &mut v,
            &["a", "b", "c"],
            toml::Value::String("x".to_string()),
        )
        .unwrap();
        assert_eq!(get_dotted(&v, "a.b.c").unwrap().as_str(), Some("x"));
    }
}
