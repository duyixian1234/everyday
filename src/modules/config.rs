//! config 模块：读写 `~/.config/everyday/config.toml`。
//!
//! 实现 `Executor` trait，与其它模块统一通过 `ModuleRegistry` 分发，
//! 消除 main.rs 中的特殊分支（`if cli.module == "config"`）。

use async_trait::async_trait;
use std::path::Path;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::{Executor, parse_simple_args};
use crate::output::{Output, RenderMode};

/// config 模块：无配置依赖（直接读 / 写文件），构造时不需要 Arc<Config>。
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
        // config 模块需要 RenderMode 来决定 list 的输出格式（TOML 文本 / JSON）。
        // 与其它模块一样，通过 thread-local 读取（main.rs 在启动时设置）。
        let mode = if crate::util::json_mode::is_json() {
            RenderMode::Json
        } else {
            RenderMode::Text
        };
        let (_flags, positional) = parse_simple_args(args);
        run_config(action, &positional, mode).await
    }
}

/// 与原 main.rs::run_config 同语义；Executor::execute 调用此处。
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

/// 沿点分路径读取 toml::Value，支持 table 字段与 array 索引（如 `mail.accounts.0.name`）。
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

/// 设置点分路径的值并保存。自动推断值类型（bool / int / float / string）。
/// 支持 table 字段与 array 索引（自动扩展数组）。
fn set_config_path(path: &str, raw_value: &str) -> Result<()> {
    let cfg_path = Config::config_path()?;
    // 读现有文件为 toml::Value（不存在则空表）。
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

/// 在 toml::Value 上按路径段插入值；自动创建中间 table 与 array。
fn upsert_dotted(root: &mut toml::Value, segs: &[&str], val: toml::Value) -> Result<()> {
    if segs.is_empty() {
        return Err(AgentError::InvalidArgument("empty path".into()));
    }
    let (last, rest) = segs.split_last().unwrap();
    let mut cur = root;
    for seg in rest {
        // 数组索引（纯数字）。
        if let Ok(idx) = seg.parse::<usize>() {
            cur = ensure_array_index(cur, idx)?;
        } else {
            let table = cur
                .as_table_mut()
                .ok_or_else(|| AgentError::InvalidArgument(format!("'{seg}' not a table")))?;
            // key 不存在则创建空 table（最后一段会覆盖）。
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

/// 把字符串 raw_value 解析成最合适的 toml 值类型。
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

/// toml::Value 转终端友好字符串。
fn value_to_display_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        // 其它类型走 toml Display（与 list 模式风格一致）。
        other => other.to_string(),
    }
}

/// `everyday config init` 写入的示例 config。
/// 与 config.example.toml 保持同步（手写避免 include_str 依赖）。
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
