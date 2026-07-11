//! CLI 参数定义与 clap 子命令树构建。
//!
//! 每个模块通过 `Executor::module_arg_spec()` 以数据形式声明自己的
//! 参数结构；本模块据此构建 `clap::Command` 树（module → action → flags），
//! 由 clap 负责校验与 `--help`，取代原先手写的 `detect_subcommand_help` /
//! `module_help` / `action_help`（那些函数需要重建整个 ModuleRegistry 只为拿帮助）。

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, ModuleRegistry, Positional};

/// 取值 flag：`--name VALUE`。
fn value_flag(spec: &ArgSpec) -> Arg {
    Arg::new(spec.name)
        .long(spec.name)
        .help(spec.help)
        .value_name(spec.name)
        .num_args(1)
}

/// 布尔开关：`--name`（无值）。
fn bool_flag(spec: &ArgSpec) -> Arg {
    Arg::new(spec.name)
        .long(spec.name)
        .help(spec.help)
        .action(ArgAction::SetTrue)
}

/// 可重复取值 flag：`--name V` 可多次，收集为列表。
fn multi_flag(spec: &ArgSpec) -> Arg {
    Arg::new(spec.name)
        .long(spec.name)
        .help(spec.help)
        .value_name(spec.name)
        .action(ArgAction::Append)
}

/// 把单个 action 的参数声明转成 clap 子命令。
fn build_action_command(spec: &ActionArgSpec) -> Command {
    let mut cmd = Command::new(spec.name)
        .about(spec.description)
        .after_help(format!("Usage: {}", spec.usage));
    for a in spec.args {
        cmd = cmd.arg(match a.kind {
            ArgKind::Value => value_flag(a),
            ArgKind::Bool => bool_flag(a),
            ArgKind::Multi => multi_flag(a),
        });
    }
    match spec.positional {
        Positional::None => {}
        Positional::OptionalSingle => {
            cmd = cmd.arg(
                Arg::new("args")
                    .help("positional arguments")
                    .num_args(0..=1),
            );
        }
        Positional::Exactly(n) => {
            cmd = cmd.arg(
                Arg::new("args")
                    .help("positional arguments")
                    .num_args(n as usize),
            );
        }
    }
    cmd
}

/// 由模块参数声明构建该模块的 clap 子命令（含各 action 子子命令）。
pub(crate) fn build_module_command(spec: &ModuleArgSpec) -> Command {
    let mut cmd = Command::new(spec.name)
        .about(spec.description)
        .subcommand_required(true)
        .arg_required_else_help(true);
    for a in spec.actions {
        cmd = cmd.subcommand(build_action_command(a));
    }
    cmd
}

/// 构建顶层命令：全局 `--json` / `--account` + 各模块子命令。
///
/// `--account` 是全局 flag，在任何层级出现都由 clap 在顶层消费，
/// 之后由 `main.rs` 注入到模块参数里；故各模块的参数声明中无需（也不应）重复它。
pub(crate) fn build_root_command(registry: &ModuleRegistry) -> Command {
    let mut cmd = Command::new("everyday")
        .version(env!("CARGO_PKG_VERSION"))
        .about("The Rust-powered hands for your AI Agent")
        .long_about(
            "Unified CLI: everyday <module> <action> [options].\n\
             Modules: mail, cal, rss, note, todo, bookmark, timeline, config.",
        )
        .arg(
            Arg::new("json")
                .long("json")
                .help("输出纯净 JSON（AI Agent 交互主模式）")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("account")
                .long("account")
                .help("覆盖模块的默认账户")
                .value_name("NAME")
                .num_args(1)
                .global(true),
        );
    for m in registry.modules.values() {
        cmd = cmd.subcommand(build_module_command(&m.module_arg_spec()));
    }
    cmd
}

/// 把某个 action 的 `ArgMatches` 还原成 `Vec<String>`，形态与旧 `parse_simple_args`
/// 的输入一致（`--key value` / `--key=value` / 布尔 `--key` / 位置参数原样），
/// 以便模块继续用 `parse_simple_args` 解析，最小化改动面。
///
/// 关键：每个 flag 严格按其在 `ActionArgSpec` 中声明的 `ArgKind` 读取类型，
/// 避免用错误类型 `get_one`/`get_many` 触发 clap 的 downcast panic
/// （如 bool flag 用 `String` 取值，或反之）。
///
/// - 取值 flag：`--name value`
/// - 可重复 flag：`--name v1 --name v2`
/// - 布尔开关：`--name`
/// - 位置参数（`args` id）：原样推送，不加 `--` 前缀
///
/// 全局 `--json` / `--account` 不在此还原（`json` 走线程局部，`account` 由 `main.rs` 注入）。
pub(crate) fn matches_to_args(m: &ArgMatches, spec: &ActionArgSpec) -> Vec<String> {
    let mut out = Vec::new();
    // 位置参数（仅当该 action 声明了位置参数时才存在 `args` id；否则读取会 panic）。
    if !matches!(spec.positional, Positional::None)
        && let Some(vals) = m.get_many::<String>("args")
    {
        for v in vals {
            out.push(v.clone());
        }
    }
    // 按声明逐个还原，类型与 clap 声明严格一致。
    for a in spec.args {
        match a.kind {
            ArgKind::Bool => {
                if m.get_flag(a.name) {
                    out.push(format!("--{}", a.name));
                }
            }
            ArgKind::Value => {
                if let Some(v) = m.get_one::<String>(a.name) {
                    out.push(format!("--{}", a.name));
                    out.push(v.clone());
                }
            }
            ArgKind::Multi => {
                if let Some(vals) = m.get_many::<String>(a.name) {
                    for v in vals {
                        out.push(format!("--{}", a.name));
                        out.push(v.clone());
                    }
                }
            }
        }
    }
    out
}
