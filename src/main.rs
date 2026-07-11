//! Everyday 入口点。
//!
//! 流程：构建 clap 子命令树 → 解析 → 查找模块 → 执行 → 渲染 → 退出码。
//! clap 负责参数校验与 `--help`（原生子命令帮助，无需重建 registry）；
//! 模块仍通过 `Executor` trait + `ModuleRegistry` 动态分发。

mod cli;
mod modules;
mod ops_log;
mod shared;
mod util;

// 让共享设施保持稳定的 crate::X 路径（物理位置在 shared/ 下，对上层透明）。
pub(crate) use shared::{config, error, keyring_user, notion_client, output};

use std::sync::Arc;

use clap::ArgMatches;

use crate::cli::{build_root_command, matches_to_args};
use crate::config::Config;
use crate::modules::ModuleRegistry;
use crate::output::{RenderMode, finalize, mode_from_json_flag, render_error};

#[tokio::main]
async fn main() {
    // 统一安装 rustls ring crypto provider（见原 cli.rs 注释）。重复安装返回 Err 是 no-op。
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 用默认配置（不读磁盘）构建注册表，仅为生成 clap 子命令树。
    // 这样即使磁盘 config 损坏，`--help` 仍可用（clap 在解析阶段就处理 --help 并退出）。
    let tree_registry = match ModuleRegistry::build(Arc::new(Config::default())) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", render_error(&e, RenderMode::Text));
            std::process::exit(1);
        }
    };
    let cmd = build_root_command(&tree_registry);
    let matches = cmd.get_matches();

    // 全局 flag。
    let json_flag = matches.get_one::<bool>("json").copied().unwrap_or(false);
    let mode = mode_from_json_flag(json_flag);
    // 把 JSON 模式同步到线程局部变量，供模块深层辅助函数查询（避免再次扫描 env::args）。
    crate::util::json_mode::set_json_mode(json_flag);

    let (code, output) = run(matches, mode).await;
    println!("{output}");
    std::process::exit(code);
}

async fn run(matches: ArgMatches, mode: RenderMode) -> (i32, String) {
    // 解析出 module / action。clap 已确保 module 子命令存在（subcommand_required）。
    let Some((module_name, module_matches)) = matches.subcommand() else {
        return (
            2,
            "error: missing module; run `everyday --help`".to_string(),
        );
    };

    // 加载真实配置（文件不存在则默认空配置，不报错；损坏则报错）。
    let config = match Config::load_or_default() {
        Ok(c) => Arc::new(c),
        Err(e) => return (1, render_error(&e, mode)),
    };

    // 构建真实注册表（注入真实配置）。
    let registry = match ModuleRegistry::build(config.clone()) {
        Ok(r) => r,
        Err(e) => return (1, render_error(&e, mode)),
    };

    // 查找模块。
    let module = match registry.get(module_name) {
        Ok(m) => m,
        Err(e) => return (1, render_error(&e, mode)),
    };

    // 解析 action（module 级 subcommand_required 保证存在；无 action 时 clap 已显示帮助并退出）。
    let (action_name, action_matches) = module_matches
        .subcommand()
        .unwrap_or((module_name, module_matches));

    // 按声明把 action 的 ArgMatches 还原成模块预期的 `Vec<String>`（类型安全，不 panic），
    // 并注入全局 `--account`（main.rs 单独处理，避免 matches_to_args 重复生成）。
    let spec = module.module_arg_spec();
    let action_spec = spec.actions.iter().find(|a| a.name == action_name);
    let mut args: Vec<String> = match action_spec {
        Some(a) => matches_to_args(action_matches, a),
        None => Vec::new(),
    };
    if let Some(acc) = matches.get_one::<String>("account") {
        args.push("--account".to_string());
        args.push(acc.clone());
    }

    let result = module.execute(action_name, &args).await;

    // Ops-log AOP hook：成功执行后，若是 notion 账户的写操作，记录到 ops-log。
    // 失败不阻断用户命令，但 ops-log 写失败不应静默 —— 按模式分流到 stderr / JSON。
    if let Ok(ref output) = result
        && let Err(e) = ops_log::maybe_log_op(
            module_name,
            action_name,
            matches.get_one::<String>("account").map(|s| s.as_str()),
            &config,
            output,
        )
        .await
    {
        match mode {
            RenderMode::Json => {
                eprintln!(
                    "{{\"_warning\":\"ops_log_failed\",\"module\":\"{}\",\"message\":\"{}\"}}",
                    module_name,
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
