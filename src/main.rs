//! Entry point for the `everyday` binary.
//!
//! Pipeline: build the clap subcommand tree → parse → resolve module →
//! execute → render → exit code.
//!
//! clap handles argument validation and `--help` natively (no need to
//! rebuild a registry for help; see [F007](../docs/adr/F007-clap-subcommand-tree.md)),
//! while modules are still dispatched dynamically through the `Executor` trait
//! + `ModuleRegistry` (see [F001](../docs/adr/F001-cli-shape.md)).

mod cli;
mod modules;
mod ops_log;
mod search;
mod shared;
mod util;

// Keep a stable `crate::X` path for shared facilities even though they live
// physically under `shared/` — transparent to upper layers.
pub(crate) use shared::{config, error, notion_client, output};

use std::sync::Arc;

use clap::ArgMatches;

use crate::cli::{build_root_command, matches_to_args};
use crate::config::Config;
use crate::modules::ModuleRegistry;
use crate::output::{RenderMode, finalize, mode_from_json_flag, render_error};

#[tokio::main]
async fn main() {
    // Install the rustls ring crypto provider once. Re-installing returns Err,
    // which is a harmless no-op here.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Build a registry from the *default* config (no disk read) purely to
    // generate the clap subcommand tree. This keeps `--help` working even
    // when the on-disk config is corrupted (clap handles --help at parse time).
    let tree_registry = match ModuleRegistry::build(Arc::new(Config::default())) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", render_error(&e, RenderMode::Text));
            std::process::exit(1);
        }
    };
    let cmd = build_root_command(&tree_registry);
    let matches = cmd.get_matches();

    let json_flag = matches.get_one::<bool>("json").copied().unwrap_or(false);
    let mode = mode_from_json_flag(json_flag);
    // Mirror the JSON mode into a thread-local flag so deep helper functions
    // can query it without re-scanning `env::args`.
    crate::util::json_mode::set_json_mode(json_flag);

    let (code, output) = run(matches, mode).await;
    println!("{output}");
    std::process::exit(code);
}

async fn run(matches: ArgMatches, mode: RenderMode) -> (i32, String) {
    // Resolve module / action. clap guarantees the module subcommand exists
    // (subcommand_required), so the empty case is only a defensive fallback.
    let Some((module_name, module_matches)) = matches.subcommand() else {
        return (
            2,
            "error: missing module; run `everyday --help`".to_string(),
        );
    };

    // Load the real config: missing file → empty default (no error);
    // corrupted file → error surfaced to the user.
    let config = match Config::load_or_default() {
        Ok(c) => Arc::new(c),
        Err(e) => return (1, render_error(&e, mode)),
    };

    // Build the real registry (inject the real config).
    let registry = match ModuleRegistry::build(config.clone()) {
        Ok(r) => r,
        Err(e) => return (1, render_error(&e, mode)),
    };

    let module = match registry.get(module_name) {
        Ok(m) => m,
        Err(e) => return (1, render_error(&e, mode)),
    };

    // Resolve the action. The module-level subcommand_required guarantees it
    // exists; when absent, clap has already shown help and exited.
    let (action_name, action_matches) = module_matches
        .subcommand()
        .unwrap_or((module_name, module_matches));

    // Reconstruct the action's `ArgMatches` into the `Vec<String>` the module
    // expects (type-safe, no panic), then inject the global `--account`
    // (handled here to avoid `matches_to_args` regenerating it).
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

    // Ops-log AOP hook: after a successful execution that is a Notion-account
    // write, record it to the ops-log. Failure does not block the user command,
    // but an ops-log write failure must not be silent — route it by mode to
    // stderr / JSON (see [L007](../docs/adr/L007-notion-ops-log.md),
    // [R001](../docs/adr/R001-thread-local-json-mode.md)).
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
