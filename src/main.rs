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
mod search;
mod shared;
mod util;

// Keep a stable `crate::X` path for shared facilities even though they live
// physically under `shared/` — transparent to upper layers.
pub(crate) use shared::{config, error, output};

use std::sync::Arc;

use clap::ArgMatches;

use crate::cli::{build_root_command, matches_to_args};
use crate::config::Config;
use crate::modules::ModuleRegistry;
use crate::output::{Output, RenderMode, finalize, mode_from_json_flag, render_error};

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

    // Leveled logging (default quiet): `-v` = INFO, `-vv` = DEBUG; the
    // subscriber writes to stderr in the project's text/JSON shapes. Must be
    // installed before any dispatch so middleware events route through it.
    let verbose = matches.get_count("verbose");
    crate::util::logging::init(verbose, json_flag);

    let (code, output) = run(matches, mode).await;
    println!("{output}");
    std::process::exit(code);
}

async fn run(matches: ArgMatches, mode: RenderMode) -> (i32, String) {
    // Request context (P4, [F012](../docs/adr/F012-architecture-deepening-phase.md)):
    // one context per invocation, passed explicitly through the middleware
    // stack into each module's `execute` (explicit-parameter form, v0.12 —
    // see [F013](../docs/adr/F013-request-context-explicit-parameter.md)).
    let ctx = crate::shared::request_context::RequestContext::cli(
        crate::shared::request_context::generate_request_id(),
    );

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

    // Lifecycle (P3): initialize every module once, before any dispatch. An
    // initialize failure is surfaced as a warning, not a hard error — the
    // module may still function for its core actions, and `everyday health`
    // reports the underlying state.
    for (name, e) in registry.initialize_all() {
        tracing::warn!(
            target: "everyday",
            _warning = "initialize_failed",
            module = %name,
            message = %e,
            warning_text = %format!("warning: {name} initialize failed: {e}"),
        );
    }

    // `everyday health` (P3): root-level ops command, not a module. Runs every
    // module's health_check and renders one row per module.
    if module_name == "health" {
        let out = run_health(&registry, mode).await;
        registry.shutdown_all();
        return out;
    }

    let module = match registry.get(module_name) {
        Ok(m) => m,
        Err(e) => {
            registry.shutdown_all();
            return (1, render_error(&e, mode));
        }
    };

    // Reconstruct the action's `ArgMatches` into the `Vec<String>` the module
    // expects (type-safe, no panic), then inject the global `--account`
    // (handled here to avoid `matches_to_args` regenerating it).
    let spec = module.module_arg_spec();

    // Resolve the action. For single-action modules the action may be
    // omitted: `everyday sync` ≡ `everyday sync sync`, `everyday search "x"`
    // ≡ `everyday search query "x"` (cli.rs mirrors the action's flags at
    // module level; here we fall back to the module's only action name).
    let (action_name, action_matches) = match module_matches.subcommand() {
        Some((name, m)) => (name, m),
        None => (
            spec.actions.first().map(|a| a.name).unwrap_or(module_name),
            module_matches,
        ),
    };
    let action_spec = spec.actions.iter().find(|a| a.name == action_name);
    let mut args: Vec<String> = match action_spec {
        Some(a) => matches_to_args(action_matches, a),
        None => Vec::new(),
    };
    if let Some(acc) = matches.get_one::<String>("account") {
        args.push("--account".to_string());
        args.push(acc.clone());
    }

    // Middleware stack (P5): default = LoggingMiddleware. Dispatch goes through
    // the chain so cross-cutting concerns (logging, timing) live here, not in
    // modules. See [F012](../docs/adr/F012-architecture-deepening-phase.md).
    let middleware: Vec<Box<dyn crate::shared::middleware::Middleware>> =
        vec![Box::new(crate::shared::middleware::LoggingMiddleware)];
    let result = crate::shared::middleware::run_with_middleware(
        &middleware,
        &ctx,
        module_name,
        module,
        action_name,
        &args,
    )
    .await;

    // Auto-sync (opt-in, D003): after a successful write command, best-effort
    // push changed files to WebDAV. Never blocks the exit code — a push
    // failure only prints a warning. Query paths never sync (L005).
    if result.is_ok() && crate::modules::sync::is_write_action(module_name, action_name) {
        crate::modules::sync::auto_sync_after_write(config.clone()).await;
    }

    // Lifecycle (P3): graceful shutdown after the action completes.
    registry.shutdown_all();

    finalize(result, mode)
}

/// `everyday health` (P3, [F012](../docs/adr/F012-architecture-deepening-phase.md)):
/// run every module's health_check and render one row per module. All rows
/// render regardless of state; the exit code is 0 when every module is healthy
/// and 1 when any module is degraded (so scripts can gate on it).
async fn run_health(registry: &ModuleRegistry, mode: RenderMode) -> (i32, String) {
    let rows = registry.health_check_all().await;
    let any_unhealthy = rows.iter().any(|(_, s)| !s.healthy);
    let output = match mode {
        RenderMode::Json => {
            let arr: Vec<serde_json::Value> = rows
                .iter()
                .map(|(name, s)| {
                    serde_json::json!({
                        "module": name,
                        "healthy": s.healthy,
                        "detail": s.detail,
                    })
                })
                .collect();
            Output::Json(serde_json::Value::Array(arr))
        }
        RenderMode::Text => {
            let table_rows: Vec<Vec<String>> = rows
                .iter()
                .map(|(name, s)| {
                    vec![
                        name.to_string(),
                        if s.healthy {
                            "ok".to_string()
                        } else {
                            "degraded".to_string()
                        },
                        s.detail.clone(),
                    ]
                })
                .collect();
            Output::records(
                vec!["module".into(), "status".into(), "detail".into()],
                table_rows,
            )
        }
    };
    let text = output.render(mode);
    // Exit code: 0 when all healthy, 1 when any module is degraded.
    (if any_unhealthy { 1 } else { 0 }, text)
}
