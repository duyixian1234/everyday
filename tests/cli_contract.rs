//! CLI contract tests — the automated line of defense against breaking CLI changes.
//!
//! Rationale (see docs/adr/G001-quality-tools-suite.md):
//! v0.8 / v0.12 / v0.13 shipped breaking changes (removed subcommands, changed
//! signatures, removed providers) that were only caught by manual ADR review.
//! `cargo-semver-checks` cannot help here — this crate is a pure `[[bin]]` with
//! no public Rust API. These tests lock the *CLI contract* instead:
//! top-level command set, per-module action sets, and the config file shape.
//!
//! Deliberately NOT golden-snapshotting full `--help` text — help copy changes
//! (wording, punctuation) must not fail CI. Only the contract (names) is locked.

use assert_cmd::Command;

/// Top-level subcommand set. Adding a module? Add it here too.
/// Removing one? That is a BREAKING change — bump major, add an ADR, update this list.
const TOP_LEVEL_COMMANDS: &[&str] = &[
    "health",
    "config",
    "bookmark",
    "search",
    "auth",
    "mail",
    "memory",
    "sync",
    "timeline",
    "rss",
    "todo",
    "note",
    "cal",
];

/// Per-module action set: (module, actions).
const MODULE_ACTIONS: &[(&str, &[&str])] = &[
    ("auth", &["login", "logout", "verify", "list"]),
    ("mail", &["folders", "list", "read", "search", "send"]),
    ("cal", &["calendars", "list", "add", "delete"]),
    ("rss", &["follow", "list", "unfollow", "digest", "fetch"]),
    ("note", &["search", "create", "read", "append", "update", "list"]),
    ("todo", &["list", "add", "start", "complete", "delete"]),
    ("bookmark", &["add", "list"]),
    ("timeline", &["today", "yesterday", "week", "month", "sync"]),
    ("memory", &["add", "get", "relation", "list", "delete", "graph", "history"]),
    ("config", &["path", "list", "get", "set", "init"]),
    ("search", &["query"]),
    ("sync", &["sync"]),
];

/// clap renders one subcommand per line as `  <name> <description>`.
fn has_command(help: &str, name: &str) -> bool {
    help.lines().any(|l| l.starts_with(&format!("  {name} ")) || l == format!("  {name}"))
}

fn run_help(args: &[&str]) -> String {
    let out = Command::cargo_bin("everyday").unwrap().args(args).output().unwrap();
    assert!(out.status.success(), "`everyday {} --help` failed", args.join(" "));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn top_level_command_set_is_contract() {
    let help = run_help(&["--help"]);
    for cmd in TOP_LEVEL_COMMANDS {
        assert!(
            has_command(&help, cmd),
            "top-level command `{cmd}` is missing from `everyday --help` — \
             removing a module is a BREAKING change (see G001)"
        );
    }
}

#[test]
fn module_action_sets_are_contract() {
    for (module, actions) in MODULE_ACTIONS {
        let help = run_help(&[module, "--help"]);
        for action in *actions {
            assert!(
                has_command(&help, action),
                "action `{module} {action}` is missing — removing an action is a \
                 BREAKING change (see G001)"
            );
        }
    }
}

#[test]
fn config_example_shape_is_contract() {
    let content = std::fs::read_to_string("config.example.toml")
        .expect("config.example.toml must exist at crate root");
    let doc: toml::Value = content.parse().expect("config.example.toml must parse as TOML");

    // Sections whose removal would silently break user configs.
    assert!(doc.get("default_account").is_some(), "`[default_account]` must exist");
    for section in ["mail", "calendar", "rss", "note", "todo", "bookmark", "webdav"] {
        let v = doc.get(section).unwrap_or_else(|| panic!("`[{section}]` section must exist"));
        assert!(
            v.get("accounts").is_some() || v.get("feeds").is_some(),
            "`[{section}]` must contain an array (`accounts` or `feeds`)"
        );
    }
}
