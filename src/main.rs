//! Everyday 入口点。
//!
//! 流程：解析 CLI → 加载配置 → 查找模块 → 执行 → 渲染 → 退出码。
//! `config` 模块特殊处理（需要写配置）。

mod cli;
mod modules;
mod ops_log;
mod shared;
mod util;

// 让共享设施保持稳定的 crate::X 路径（物理位置在 shared/ 下，对上层透明）。
pub(crate) use shared::{config, error, keyring_user, notion_client, output};

use std::sync::Arc;

use clap::Parser;

use crate::cli::Cli;
use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::ModuleRegistry;
use crate::output::{RenderMode, finalize, mode_from_json_flag, render_error};

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
    // `--json` 若出现在模块动作之后的 trailing args 中（如 `rss digest --limit 5 --json`），
    // clap 的 `trailing_var_arg` 会把它吞进模块 args 而非识别为全局 flag。
    // 这里在 clap 解析前已拿到 raw_args，额外扫描一次，确保 `--json` 任何位置都生效
    // （这是 AI Agent 交互的主模式，丢失会静默退回文本）。
    let json_flag = cli.json || raw_args.iter().any(|a| a == "--json");
    let mode = mode_from_json_flag(json_flag);

    // 把 JSON 模式同步到线程局部变量，供模块深层辅助函数（如 note_local 的
    // 渲染分支）查询。避免它们再次扫描 std::env::args()（会被宿主进程污染）。
    crate::util::json_mode::set_json_mode(json_flag);

    let (code, output) = run(cli, mode).await;
    println!("{output}");
    std::process::exit(code);
}

async fn run(cli: Cli, mode: RenderMode) -> (i32, String) {
    // config 模块现走 Executor trait + ModuleRegistry，与其它模块统一分发。
    // 见 src/modules/config.rs。

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
    let registry = match ModuleRegistry::build(config.clone(), cli.account.as_deref()) {
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

    // Ops-log AOP hook：成功执行后，若是 notion 账户的写操作，记录到 ops-log。
    // 失败不阻断用户命令，但 ops-log 写失败不应静默 —— 之前用 `let _ =`
    // 完全吞掉 DB 错误，导致 timeline 永远静默缺失 notion 写记录，调试黑洞。
    // 现在按模式分流：
    // - --json：返回结构化错误字段，Agent 可观测。
    // - 文本模式：eprintln! 到 stderr，避免污染主输出（stdout）。
    if let Ok(ref output) = result
        && let Err(e) =
            ops_log::maybe_log_op(&cli.module, action, cli.account.as_deref(), &config, output)
                .await
    {
        match mode {
            RenderMode::Json => {
                eprintln!(
                    "{{\"_warning\":\"ops_log_failed\",\"module\":\"{}\",\"message\":\"{}\"}}",
                    cli.module,
                    e.message().replace('"', "'")
                );
            }
            RenderMode::Text => {
                eprintln!("warning: ops-log write failed: {}", e.message());
            }
        }
    }

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
        out.push_str(&format!(
            "  {:<8} {}\n          {}\n",
            a.name, a.description, a.usage
        ));
    }
    out.push_str("\nGlobal flags: --json, --account <NAME>\n");
    Ok(out)
}

/// 生成单个 action 的帮助文本（详细用法）。
fn action_help(module_name: &str, action_name: &str) -> Result<String> {
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



#[cfg(test)]
mod tests {
    use super::*;

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
