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

/// Attach the positional-argument slot an action declared.
fn add_positional(mut cmd: Command, positional: Positional) -> Command {
    match positional {
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
    add_positional(cmd, spec.positional)
}

/// Build a module's clap subcommand (including each action sub-subcommand).
pub(crate) fn build_module_command(spec: &ModuleArgSpec) -> Command {
    // A single-action module may be invoked without naming the action:
    // `everyday sync` ≡ `everyday sync sync`, `everyday search "x"` ≡
    // `everyday search query "x"` (main.rs falls back to the module's only
    // action). The action's flags and positional slot are mirrored at module
    // level so `everyday sync --push-only` / `everyday search --module note`
    // parse; the explicit form still works too.
    let omitable = spec.actions.len() == 1;
    let mut cmd = Command::new(spec.name)
        .about(spec.description)
        .subcommand_required(!omitable)
        .arg_required_else_help(!omitable);
    if omitable {
        let action = &spec.actions[0];
        for a in action.args {
            cmd = cmd.arg(match a.kind {
                ArgKind::Value => value_flag(a),
                ArgKind::Bool => bool_flag(a),
                ArgKind::Multi => multi_flag(a),
            });
        }
        cmd = add_positional(cmd, action.positional);
    }
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
                 Modules: mail, cal, rss, note, todo, bookmark, timeline, memory, config, auth.",
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
        )
        // Root-level ops command (P3, [F012](../docs/adr/F012-architecture-deepening-phase.md)):
        // `everyday health` runs every module's health_check. It is not a module
        // (no business actions), so it is added statically here and dispatched
        // specially in main.rs.
        .subcommand(Command::new("health").about("运行所有模块健康检查（仅本地探测，无网络调用）"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::{ArgKind, Positional};

    fn single_action_spec() -> ModuleArgSpec {
        static A: &[ArgSpec] = &[ArgSpec {
            name: "flag-a",
            help: "h",
            kind: ArgKind::Bool,
        }];
        static ACTS: &[ActionArgSpec] = &[ActionArgSpec {
            name: "query",
            description: "d",
            usage: "u",
            args: A,
            positional: Positional::OptionalSingle,
        }];
        ModuleArgSpec {
            name: "search",
            description: "d",
            actions: ACTS,
        }
    }

    fn multi_action_spec() -> ModuleArgSpec {
        static ACTS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "list",
                description: "d",
                usage: "u",
                args: &[],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "add",
                description: "d",
                usage: "u",
                args: &[],
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "bookmark",
            description: "d",
            actions: ACTS,
        }
    }

    #[test]
    fn single_action_module_accepts_omitted_action() {
        // The command under test is the module subcommand itself (name
        // "search"), so argv[0] is the module name.
        let cmd = build_module_command(&single_action_spec());
        // `search` (no action) parses; no action subcommand was invoked.
        let m = cmd.clone().try_get_matches_from(["search"]).unwrap();
        assert!(m.subcommand().is_none());
        assert!(!m.get_flag("flag-a"));
        // `search --flag-a` — mirrored flag at module level.
        let m2 = cmd
            .clone()
            .try_get_matches_from(["search", "--flag-a"])
            .unwrap();
        assert!(m2.get_flag("flag-a"));
        // Positional mirror: `search "hello"`.
        let m3 = cmd
            .clone()
            .try_get_matches_from(["search", "hello"])
            .unwrap();
        assert_eq!(
            m3.get_one::<String>("args").map(String::as_str),
            Some("hello")
        );
        // Explicit action form still works: `search query "hello"`.
        let m4 = cmd
            .clone()
            .try_get_matches_from(["search", "query", "hello"])
            .unwrap();
        assert_eq!(m4.subcommand().unwrap().0, "query");
    }

    #[test]
    fn multi_action_module_still_requires_action() {
        let cmd = build_module_command(&multi_action_spec());
        // `bookmark` without an action must fail (clap help path).
        assert!(cmd.clone().try_get_matches_from(["bookmark"]).is_err());
        // With an action it parses.
        assert!(
            cmd.clone()
                .try_get_matches_from(["bookmark", "list"])
                .is_ok()
        );
    }
}
