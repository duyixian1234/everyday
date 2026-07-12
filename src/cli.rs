//! CLI argument definitions and clap subcommand-tree construction.
//!
//! Each module declares its argument structure as data via
//! `Executor::module_arg_spec()`; this module turns that into a `clap::Command`
//! tree (module → action → flags). clap owns validation and `--help`,
//! replacing the old hand-rolled `detect_subcommand_help` / `module_help` /
//! `action_help` helpers (which rebuilt the whole `ModuleRegistry` just for help).
//! See [F007](../docs/adr/F007-clap-subcommand-tree.md).

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, ModuleRegistry, Positional};

/// Builds a value-taking flag: `--name VALUE`.
fn value_flag(spec: &ArgSpec) -> Arg {
    Arg::new(spec.name)
        .long(spec.name)
        .help(spec.help)
        .value_name(spec.name)
        .num_args(1)
}

/// Boolean switch: `--name` (no value).
fn bool_flag(spec: &ArgSpec) -> Arg {
    Arg::new(spec.name)
        .long(spec.name)
        .help(spec.help)
        .action(ArgAction::SetTrue)
}

/// Repeatable value flag: `--name V` may appear multiple times, collected into a list.
fn multi_flag(spec: &ArgSpec) -> Arg {
    Arg::new(spec.name)
        .long(spec.name)
        .help(spec.help)
        .value_name(spec.name)
        .action(ArgAction::Append)
}

/// Turn a single action's argument spec into a clap subcommand.
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

/// Build a module's clap subcommand (including each action sub-subcommand).
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

/// Build the top-level command: global `--json` / `--account` + one subcommand per module.
///
/// `--account` is a global flag consumed by clap at the top level wherever it
/// appears, then injected into module args by `main.rs`; module arg specs must
/// NOT redeclare it.
/// See [F007](../docs/adr/F007-clap-subcommand-tree.md).
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

/// Reconstruct an action's `ArgMatches` into the `Vec<String>` shape that the
/// old `parse_simple_args` consumed (`--key value` / `--key=value` / boolean
/// `--key` / positional args verbatim), so modules keep using `parse_simple_args`
/// with minimal change surface.
/// See [R005](../docs/adr/R005-parse-simple-args.md).
///
/// Key: each flag is read with exactly the `ArgKind` declared in `ActionArgSpec`,
/// avoiding a clap downcast panic from a mismatched `get_one`/`get_many` type
/// (e.g. reading a bool flag as `String`, or vice versa).
///
/// - value flag:    `--name value`
/// - repeatable flag: `--name v1 --name v2`
/// - boolean switch:  `--name`
/// - positional (`args` id): pushed verbatim, no `--` prefix
///
/// Global `--json` / `--account` are NOT reconstructed here (`json` goes through
/// the thread-local flag; `account` is injected by `main.rs`).
pub(crate) fn matches_to_args(m: &ArgMatches, spec: &ActionArgSpec) -> Vec<String> {
    let mut out = Vec::new();
    // Positionals exist only when the action declared them; reading "args"
    // otherwise would panic.
    if !matches!(spec.positional, Positional::None)
        && let Some(vals) = m.get_many::<String>("args")
    {
        for v in vals {
            out.push(v.clone());
        }
    }
    // Reconstruct each flag by its declared kind, matching clap's type exactly.
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
