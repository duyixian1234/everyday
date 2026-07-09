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
    // 统一安装 rustls ring crypto provider。
    // cargo feature unification 可能让 ring（email 的 tokio-rustls）与 aws-lc-rs（传递依赖）
    // 同时启用，rustls 0.23+ 拒绝自动选择 → panic。入口处显式安装 ring 即可。
    // 重复安装返回 Err，是 no-op，用 `let _` 吞掉。
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 拦截 module/action 之后的 --help/-h。
    // clap 的内置 --help 会在顶层拦截，导致 `everyday cal add --help` 显示顶层帮助
    // 而非 cal add 的 action 帮助。这里在 clap 解析前预扫描：若 --help/-h 出现在
    // module 之后，直接输出对应层级的帮助并退出；出现在 module 之前则交给 clap 处理。
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(target) = detect_subcommand_help(&raw_args) {
        let (code, text) = render_help_target(target);
        println!("{text}");
        std::process::exit(code);
    }

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
    // `--account` 是 clap 的 global flag，被 clap 消费到 `cli.account`，不放入 `cli.args`。
    // 注入到 args 前部，让模块的 `parse_simple_args` 能解析 `--account <name>`。
    let action = cli.action.as_deref().unwrap_or("");
    let mut full_args: Vec<String> = Vec::new();
    if let Some(acc) = &cli.account {
        full_args.push("--account".to_string());
        full_args.push(acc.clone());
    }
    full_args.extend(cli.args.iter().cloned());
    let result = module.execute(action, &full_args).await;
    finalize(result, mode)
}

/// 生成模块帮助文本（列出该模块支持的 actions）。
fn module_help(module_name: &str) -> Result<String> {
    // config 模块不在 ModuleRegistry 中，走专门帮助。
    if module_name == "config" {
        return Ok(config_help(None));
    }
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

/// 生成单个 action 的帮助文本（详细用法）。
fn action_help(module_name: &str, action_name: &str) -> Result<String> {
    // config 模块走专门帮助。
    if module_name == "config" {
        return Ok(config_help(Some(action_name)));
    }
    let cfg = Arc::new(Config::default());
    let registry = ModuleRegistry::build(cfg, None)?;
    let module = registry.get(module_name)?;
    let action = module
        .actions()
        .into_iter()
        .find(|a| a.name == action_name)
        .ok_or_else(|| AgentError::UnknownAction(format!("{} {}", module_name, action_name)))?;
    Ok(format!(
        "{} {} — {}\n\nUsage: {}\n\nGlobal flags: --json, --account <NAME>\n",
        module_name, action_name, action.description, action.usage
    ))
}

/// 预扫描识别的帮助目标。
#[derive(Debug)]
enum HelpTarget {
    Module(String),
    Action(String, String),
}

/// 预扫描 raw args（不含程序名），检测出现在 module 之后的 `--help`/`-h`。
///
/// 返回 `Some(target)` 表示需要拦截并输出子命令帮助；返回 `None` 表示不拦截
///（`--help` 出现在 module 之前或根本不存在），交给 clap 正常处理。
///
/// 解析规则：
/// - `--json`：布尔全局 flag，跳过
/// - `--account <value>`：占两个 token，跳过
/// - `--account=<value>`：单 token，跳过
/// - `--key=value`：单 token flag，跳过
/// - 其他非 `--` 开头的 token：依次填充 module → action
/// - `--help` / `-h`：按当前已识别的 module/action 层级返回帮助目标
fn detect_subcommand_help(args: &[String]) -> Option<HelpTarget> {
    let mut module: Option<String> = None;
    let mut action: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" || arg == "-h" {
            return match (&module, &action) {
                (None, _) => None, // --help 在 module 之前 → 交给 clap
                (Some(m), None) => Some(HelpTarget::Module(m.clone())),
                (Some(m), Some(a)) => Some(HelpTarget::Action(m.clone(), a.clone())),
            };
        }

        // 跳过全局 flag，不影响位置参数识别。
        if arg == "--json" {
            i += 1;
            continue;
        }
        if arg == "--account" {
            i += 2; // --account 和它的值
            continue;
        }
        if arg.starts_with("--account=") {
            i += 1;
            continue;
        }
        // --key=value 形式的 flag（单 token）。
        if arg.starts_with("--") && arg.contains('=') {
            i += 1;
            continue;
        }

        // 位置参数 → 依次填充 module、action。
        if !arg.starts_with("--") {
            if module.is_none() {
                module = Some(arg.clone());
            } else if action.is_none() {
                action = Some(arg.clone());
            }
        }

        i += 1;
    }

    None
}

/// 渲染帮助目标为 (exit_code, text)。
fn render_help_target(target: HelpTarget) -> (i32, String) {
    match target {
        HelpTarget::Module(m) => match module_help(&m) {
            Ok(text) => (0, text),
            Err(e) => (1, e.to_string()),
        },
        HelpTarget::Action(m, a) => match action_help(&m, &a) {
            Ok(text) => (0, text),
            Err(e) => (1, e.to_string()),
        },
    }
}

/// config 模块的帮助文本（config 不在 ModuleRegistry 中，单独处理）。
fn config_help(action: Option<&str>) -> String {
    /// config 模块支持的 actions：(name, description, usage)
    const CONFIG_ACTIONS: &[(&str, &str, &str)] = &[
        ("path", "Show config file path", "everyday config path"),
        ("list", "List all config (TOML or JSON)", "everyday config list"),
        ("get", "Get a config value by dotted path", "everyday config get <dotted.path>"),
        ("set", "Set a config value by dotted path", "everyday config set <dotted.path> <value>"),
        ("init", "Create config from example", "everyday config init"),
    ];

    match action {
        None => {
            let mut out = "config — Configuration management\n\nUsage: everyday config <action> [options]\n\nActions:\n".to_string();
            for (name, desc, usage) in CONFIG_ACTIONS {
                out.push_str(&format!("  {name:<8} {desc}\n          {usage}\n"));
            }
            out.push_str("\nGlobal flags: --json, --account <NAME>\n");
            out
        }
        Some(a) => {
            match CONFIG_ACTIONS.iter().find(|(name, _, _)| *name == a) {
                Some((name, desc, usage)) => format!(
                    "config {name} — {desc}\n\nUsage: {usage}\n\nGlobal flags: --json, --account <NAME>\n"
                ),
                None => format!(
                    "unknown config action: {a}\n\nUse `everyday config --help` to list actions.\n"
                ),
            }
        }
    }
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

/// 沿点分路径读取 toml::Value，支持 table 字段与 array 索引（如 `mail.accounts.0.name`）。
fn get_dotted(root: &toml::Value, path: &str) -> Result<toml::Value> {
    let mut cur = root.clone();
    for seg in path.split('.') {
        cur = if let Some(table) = cur.as_table() {
            table
                .get(seg)
                .cloned()
                .ok_or_else(|| AgentError::InvalidArgument(format!("path segment '{seg}' not found")))?
        } else if let Some(arr) = cur.as_array() {
            let idx: usize = seg
                .parse()
                .map_err(|_| AgentError::InvalidArgument(format!("array index '{seg}' not a number")))?;
            arr.get(idx)
                .cloned()
                .ok_or_else(|| AgentError::InvalidArgument(format!("array index {idx} out of bounds")))?
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
    match root {
        toml::Value::Table(table) => {
            if segs.len() == 1 {
                table.insert(segs[0].to_string(), value);
                return Ok(());
            }
            let entry = table
                .entry(segs[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            set_dotted(entry, &segs[1..], value)
        }
        toml::Value::Array(arr) => {
            let idx: usize = segs[0]
                .parse()
                .map_err(|_| AgentError::InvalidArgument(format!("array index '{}' not a number", segs[0])))?;
            if arr.len() <= idx {
                arr.resize(idx + 1, toml::Value::Table(toml::value::Table::new()));
            }
            if segs.len() == 1 {
                arr[idx] = value;
                return Ok(());
            }
            set_dotted(&mut arr[idx], &segs[1..], value)
        }
        _ => Err(AgentError::InvalidArgument(
            "cannot index into non-table/non-array value".into(),
        )),
    }
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
# 可选：忽略指定日历（按 displayname 匹配，不区分大小写）
ignore_calendars = ["好友生日"]

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

    #[test]
    fn dotted_get_array_index() {
        let t: toml::Value =
            toml::from_str("accounts = [{ name = 'work' }, { name = 'home' }]").unwrap();
        assert_eq!(
            get_dotted(&t, "accounts.0.name").unwrap().as_str(),
            Some("work")
        );
        assert_eq!(
            get_dotted(&t, "accounts.1.name").unwrap().as_str(),
            Some("home")
        );
        assert!(get_dotted(&t, "accounts.5.name").is_err());
    }

    #[test]
    fn set_dotted_into_array_index() {
        let mut t: toml::Value = toml::from_str("accounts = [{ name = 'work' }]").unwrap();
        set_dotted(&mut t, &["accounts", "0", "imap_host"], toml::Value::String("imap.x.com".into())).unwrap();
        assert_eq!(
            get_dotted(&t, "accounts.0.imap_host").unwrap().as_str(),
            Some("imap.x.com")
        );
    }

    // ---- detect_subcommand_help 测试 ----

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_before_module_returns_none() {
        // --help 在 module 之前 → 交给 clap
        assert!(detect_subcommand_help(&args(&["--help"])).is_none());
        assert!(detect_subcommand_help(&args(&["--help", "cal"])).is_none());
        assert!(detect_subcommand_help(&args(&["-h", "cal", "add"])).is_none());
    }

    #[test]
    fn help_without_module_returns_none() {
        assert!(detect_subcommand_help(&args(&[])).is_none());
        assert!(detect_subcommand_help(&args(&["--json"])).is_none());
    }

    #[test]
    fn help_after_module_targets_module() {
        match detect_subcommand_help(&args(&["cal", "--help"])) {
            Some(HelpTarget::Module(m)) => assert_eq!(m, "cal"),
            other => panic!("expected Module, got {other:?}"),
        }
        match detect_subcommand_help(&args(&["cal", "-h"])) {
            Some(HelpTarget::Module(m)) => assert_eq!(m, "cal"),
            other => panic!("expected Module, got {other:?}"),
        }
    }

    #[test]
    fn help_after_action_targets_action() {
        match detect_subcommand_help(&args(&["cal", "add", "--help"])) {
            Some(HelpTarget::Action(m, a)) => {
                assert_eq!(m, "cal");
                assert_eq!(a, "add");
            }
            other => panic!("expected Action, got {other:?}"),
        }
        match detect_subcommand_help(&args(&["cal", "add", "-h"])) {
            Some(HelpTarget::Action(m, a)) => {
                assert_eq!(m, "cal");
                assert_eq!(a, "add");
            }
            other => panic!("expected Action, got {other:?}"),
        }
    }

    #[test]
    fn help_with_global_flags_works() {
        // --json before module, --help after module
        match detect_subcommand_help(&args(&["--json", "cal", "--help"])) {
            Some(HelpTarget::Module(m)) => assert_eq!(m, "cal"),
            other => panic!("expected Module, got {other:?}"),
        }
        // --account with separate value
        match detect_subcommand_help(&args(&["cal", "list", "--account", "work", "--help"])) {
            Some(HelpTarget::Action(m, a)) => {
                assert_eq!(m, "cal");
                assert_eq!(a, "list");
            }
            other => panic!("expected Action, got {other:?}"),
        }
        // --account=value form
        match detect_subcommand_help(&args(&["--account=work", "cal", "add", "--help"])) {
            Some(HelpTarget::Action(m, a)) => {
                assert_eq!(m, "cal");
                assert_eq!(a, "add");
            }
            other => panic!("expected Action, got {other:?}"),
        }
        // --key=value flags in trailing args are skipped
        match detect_subcommand_help(&args(&["cal", "add", "--title=foo", "--help"])) {
            Some(HelpTarget::Action(m, a)) => {
                assert_eq!(m, "cal");
                assert_eq!(a, "add");
            }
            other => panic!("expected Action, got {other:?}"),
        }
    }

    #[test]
    fn help_embedded_in_value_not_detected() {
        // --title=--help → --help is a value, not a flag → should NOT be detected
        assert!(detect_subcommand_help(&args(&["cal", "add", "--title=--help"])).is_none());
    }

    // ---- config_help 测试 ----

    #[test]
    fn config_help_lists_all_actions() {
        let help = config_help(None);
        assert!(help.contains("config — Configuration management"));
        assert!(help.contains("path"));
        assert!(help.contains("list"));
        assert!(help.contains("get"));
        assert!(help.contains("set"));
        assert!(help.contains("init"));
    }

    #[test]
    fn config_help_for_known_action() {
        let help = config_help(Some("get"));
        assert!(help.contains("config get —"));
        assert!(help.contains("everyday config get <dotted.path>"));
    }

    #[test]
    fn config_help_for_unknown_action() {
        let help = config_help(Some("bogus"));
        assert!(help.contains("unknown config action: bogus"));
    }

    // ---- action_help 集成测试 ----

    #[test]
    fn action_help_for_known_action() {
        let help = action_help("cal", "add").unwrap();
        assert!(help.contains("cal add —"));
        assert!(help.contains("--title"));
        assert!(help.contains("--start"));
        assert!(help.contains("--end"));
    }

    #[test]
    fn action_help_for_unknown_action_errors() {
        assert!(action_help("cal", "bogus").is_err());
    }

    #[test]
    fn action_help_for_config_module() {
        let help = action_help("config", "set").unwrap();
        assert!(help.contains("config set —"));
        assert!(help.contains("everyday config set <dotted.path> <value>"));
    }

    #[test]
    fn module_help_for_config_module() {
        let help = module_help("config").unwrap();
        assert!(help.contains("config — Configuration management"));
        assert!(help.contains("Actions:"));
    }
}
