//! Everyday 入口点。
//!
//! 流程：解析 CLI → 加载配置 → 查找模块 → 执行 → 渲染 → 退出码。
//! `config` 模块特殊处理（需要写配置）。

// 基础架构阶段：部分公共 API（AgentError::Auth/Network、Config::save/keyring_service、
// ModuleRegistry::module_names、Output::json）为 Phase 6 模块预留，暂未调用。
// 待邮件/日历/网络模块实现后移除此 allow。
#![allow(dead_code)]

mod cli;
mod config;
mod error;
mod modules;
mod output;

use std::sync::Arc;

use clap::Parser;

use crate::cli::Cli;
use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::ModuleRegistry;
use crate::output::{finalize, mode_from_json_flag, render_error, Output, RenderMode};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let mode = mode_from_json_flag(cli.json);

    let (code, output) = run(cli, mode).await;
    println!("{output}");
    std::process::exit(code);
}

async fn run(cli: Cli, mode: RenderMode) -> (i32, String) {
    // config 模块走专门处理（需要读写配置文件）。
    if cli.module == "config" {
        let result = run_config(cli.action.as_deref(), &cli.args, mode).await;
        return finalize(result, mode);
    }

    // 无 action：显示模块帮助（需要先有 registry）。
    if cli.action.is_none() {
        match module_help(&cli.module) {
            Ok(text) => return (0, text),
            Err(e) => return (1, render_error(&e, mode)),
        }
    }

    // 加载配置（文件不存在则默认空配置，不报错）。
    let config = match Config::load_or_default() {
        Ok(c) => Arc::new(c),
        Err(e) => return (1, render_error(&e, mode)),
    };

    // 构建注册表。
    let registry = match ModuleRegistry::build(config, cli.account.as_deref()) {
        Ok(r) => r,
        Err(e) => return (1, render_error(&e, mode)),
    };

    // 查找模块。
    let module = match registry.get(&cli.module) {
        Ok(m) => m,
        Err(e) => return (1, render_error(&e, mode)),
    };

    // 执行。
    let action = cli.action.as_deref().unwrap_or("");
    let result = module.execute(action, &cli.args).await;
    finalize(result, mode)
}

/// 生成模块帮助文本（列出该模块支持的 actions）。
fn module_help(module_name: &str) -> Result<String> {
    // 用一个空配置构建 registry 只为拿 actions 文档。
    let cfg = Arc::new(Config::default());
    let registry = ModuleRegistry::build(cfg, None)?;
    let module = registry.get(module_name)?;
    let mut out = format!(
        "{} — {}\n\nUsage: everyday {} <action> [options]\n\nActions:\n",
        module.name(),
        module.description(),
        module.name()
    );
    for a in module.actions() {
        out.push_str(&format!("  {:<8} {}\n          {}\n", a.name, a.description, a.usage));
    }
    out.push_str("\nGlobal flags: --json, --account <NAME>\n");
    Ok(out)
}

// ---- config 子命令 ----

async fn run_config(action: Option<&str>, args: &[String], mode: RenderMode) -> Result<Output> {
    let action = action.unwrap_or("list");
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
            let path = args
                .first()
                .ok_or_else(|| AgentError::InvalidArgument("usage: everyday config get <dotted.path>".into()))?;
            let cfg = Config::load_or_default()?;
            let toml_val: toml::Value = toml::Value::try_from(&cfg)
                .map_err(|e| AgentError::Config(format!("serialize: {e}")))?;
            let v = get_dotted(&toml_val, path)?;
            Ok(Output::text(value_to_display_string(&v)))
        }
        "set" => {
            let (path, value) = (
                args.first()
                    .ok_or_else(|| AgentError::InvalidArgument("usage: everyday config set <dotted.path> <value>".into()))?,
                args.get(1)
                    .ok_or_else(|| AgentError::InvalidArgument("usage: everyday config set <dotted.path> <value>".into()))?,
            );
            set_config_path(path, value)?;
            Ok(Output::text(format!("set {path} = {value}")))
        }
        "init" => {
            let path = Config::config_path()?;
            if path.exists() {
                return Ok(Output::text(format!("config already exists: {}", path.display())));
            }
            let example = example_config();
            std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))?;
            std::fs::write(&path, example)?;
            Ok(Output::text(format!("created config at: {}", path.display())))
        }
        other => Err(AgentError::UnknownAction(format!("config {other}"))),
    }
}

/// 沿点分路径读取 toml::Value。
fn get_dotted(root: &toml::Value, path: &str) -> Result<toml::Value> {
    let mut cur = root.clone();
    for seg in path.split('.') {
        cur = cur
            .as_table()
            .and_then(|t| t.get(seg))
            .cloned()
            .ok_or_else(|| AgentError::InvalidArgument(format!("path segment '{seg}' not found")))?;
    }
    Ok(cur)
}

/// 设置点分路径的值并保存。自动推断值类型（bool / int / float / string）。
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
    set_dotted(&mut root, &segs, new_val)?;

    let text = toml::to_string_pretty(&root)
        .map_err(|e| AgentError::Config(format!("serialize: {e}")))?;
    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cfg_path, text)?;
    Ok(())
}

fn set_dotted(root: &mut toml::Value, segs: &[&str], value: toml::Value) -> Result<()> {
    if segs.is_empty() {
        return Err(AgentError::InvalidArgument("empty path".into()));
    }
    let table = root
        .as_table_mut()
        .ok_or_else(|| AgentError::InvalidArgument("config root must be a table".into()))?;
    if segs.len() == 1 {
        table.insert(segs[0].to_string(), value);
        return Ok(());
    }
    let entry = table
        .entry(segs[0].to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    set_dotted(entry, &segs[1..], value)
}

fn parse_value(s: &str) -> toml::Value {
    if s == "true" {
        return toml::Value::Boolean(true);
    }
    if s == "false" {
        return toml::Value::Boolean(false);
    }
    if let Ok(i) = s.parse::<i64>() {
        return toml::Value::Integer(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        return toml::Value::Float(f);
    }
    toml::Value::String(s.to_string())
}

fn value_to_display_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn example_config() -> String {
    r#"# Everyday configuration
# Passwords are NEVER stored here — use the system keyring instead.
# keyring service convention: everyday/<module>/<account>

[default_account]
mail = "work"
calendar = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 465
username = "me@example.com"
tls = true

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/me"
username = "me"

[[rss.feeds]]
name = "hackernews"
url = "https://hnrss.org/frontpage"
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_value_infers_types() {
        assert!(parse_value("true").is_bool());
        assert!(parse_value("42").is_integer());
        assert!(parse_value("3.14").is_float());
        assert!(parse_value("hello").is_str());
    }

    #[test]
    fn dotted_get_traverses() {
        let t: toml::Value = toml::from_str("a = { b = { c = 1 } }").unwrap();
        let v = get_dotted(&t, "a.b.c").unwrap();
        assert_eq!(v.as_integer(), Some(1));
    }
}
