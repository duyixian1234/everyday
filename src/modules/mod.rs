//! Module layer: defines the [`Executor`] trait and [`ModuleRegistry`].
//!
//! Each feature module (mail, calendar, RSS) implements `Executor`; the
//! main program dispatches only through `Box<dyn Executor>`, keeping
//! `main.rs` minimal.
//!
//! Positioning: `everyday` is the unified interface through which an AI
//! Agent reaches the outside world (mail / calendar / news). It does not
//! embed generic capabilities an agent can do directly via the shell —
//! file search, HTTP, system monitoring, etc. See
//! [F003](../../docs/adr/F003-module-scope-external-integration.md).

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::output::Output;
use crate::shared::request_context::RequestContext;

/// Module executor trait.
///
/// Each module holds its own config (injected at construction with the
/// relevant account config). The main program looks up the trait object by
/// name via [`ModuleRegistry`] and calls [`Executor::execute`].
#[async_trait]
pub trait Executor: Send + Sync {
    /// One-line description.
    fn description(&self) -> &'static str;

    /// Returns the module's argument-structure declaration (the single
    /// source of truth for clap subcommanding).
    ///
    /// `cli.rs` builds the `clap::Command` tree from this (module → action →
    /// flags); the module itself need not know about clap. `--account` is a
    /// global flag and is not declared here. See
    /// [F007](../../docs/adr/F007-clap-subcommand-tree.md).
    fn module_arg_spec(&self) -> ModuleArgSpec;

    /// Execute the given action.
    ///
    /// - `action`: the action name (e.g. `list`, `send`, `status`)
    /// - `args`: the remaining command-line arguments (parsed by the module)
    /// - `ctx`: the per-request context (request id / deadline / caller), passed
    ///   explicitly since v0.12 — see
    ///   [F013](../../docs/adr/F013-request-context-explicit-parameter.md)
    async fn execute(&self, action: &str, args: &[String], ctx: &RequestContext) -> Result<Output>;

    // ---- Lifecycle hooks (P3, [F012](../../docs/adr/F012-architecture-deepening-phase.md)) ----
    //
    // Default implementations are no-ops / healthy; modules opt in by
    // overriding. These enable resource management, health monitoring and
    // graceful shutdown without touching `execute` (additive, non-breaking).

    /// Initialize resources. Called once before the first `execute` when the
    /// dispatcher is lifecycle-aware. Default: no-op.
    fn initialize(&self) -> Result<()> {
        Ok(())
    }

    /// Report module health (used by `everyday health`). Default: healthy.
    ///
    /// Modules override to probe cheap local resources (cache DB openable,
    /// keyring credential present) — never network calls, which would make
    /// `everyday health` slow and dependent on external services.
    async fn health_check(&self) -> Result<HealthStatus> {
        Ok(HealthStatus::healthy())
    }

    /// Graceful shutdown. Called once before process exit when the dispatcher
    /// is lifecycle-aware. Default: no-op.
    fn shutdown(&self) {}
}

/// Module health status (P3).
///
/// `healthy` + a one-line detail; modules may report degraded (e.g. cache DB
/// unreadable) without failing the whole command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStatus {
    /// Whether the module considers itself operational.
    pub healthy: bool,
    /// One-line human-readable detail (e.g. "cache db: open").
    pub detail: String,
}

impl HealthStatus {
    /// A healthy module with a default "ok" detail.
    pub fn healthy() -> Self {
        Self {
            healthy: true,
            detail: "ok".to_string(),
        }
    }

    /// A degraded module (still runnable, but something is off).
    pub fn degraded(detail: impl Into<String>) -> Self {
        Self {
            healthy: false,
            detail: detail.into(),
        }
    }
}

/// Shared helper for DB-backed modules' `health_check`: open the pool, then
/// report `healthy` (or `degraded("<label> db: <error>")` on failure).
///
/// Used by memory / timeline (and any future DB-backed module); the probe is
/// local-only by design ([F012 P3](../../docs/adr/F012-architecture-deepening-phase.md)).
pub(crate) async fn db_health<F, Fut>(label: &str, open: F) -> Result<HealthStatus>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<sqlx::SqlitePool>>,
{
    match open().await {
        Ok(pool) => {
            pool.close().await;
            Ok(HealthStatus::healthy())
        }
        Err(e) => Ok(HealthStatus::degraded(format!(
            "{label} db: {}",
            e.message()
        ))),
    }
}

/// clap subcommanding: each module declares its argument structure as data,
/// and `cli.rs` converts it into a `clap::Command` tree. Single source of
/// truth, avoiding duplicated parsing scattered inside `execute`. See
/// [F007](../../docs/adr/F007-clap-subcommand-tree.md).
#[derive(Debug, Clone, Copy)]
pub enum ArgKind {
    /// Value flag: `--name VALUE`
    Value,
    /// Boolean switch: `--name` (no value)
    Bool,
    /// Repeatable value flag: `--name V` may appear multiple times, collected
    /// into a list (e.g. note's `--prop`)
    Multi,
}

/// A single argument declaration.
pub struct ArgSpec {
    pub name: &'static str,
    pub help: &'static str,
    pub kind: ArgKind,
}

/// Positional-argument shape.
#[derive(Debug, Clone, Copy)]
pub enum Positional {
    /// No positional arguments (pure flag command).
    None,
    /// Exactly N positional arguments (e.g. `config set <path> <value>` is `Exactly(2)`).
    Exactly(u8),
    /// Optional single positional argument (0 or 1, e.g. `note read [<page_id>]`).
    OptionalSingle,
}

/// Argument declaration for a single action (subcommand).
pub struct ActionArgSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub usage: &'static str,
    pub args: &'static [ArgSpec],
    /// Positional-argument declaration (e.g. `config set <path> <value>`,
    /// `note read <page_id>`). Positionals are captured under the single
    /// clap id `args` and reconstructed verbatim by `matches_to_args`.
    pub positional: Positional,
}

/// Module-level argument declaration (single source of truth for clap subcommanding).
pub struct ModuleArgSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub actions: &'static [ActionArgSpec],
}

/// Declare one CLI action with compact syntax (P1, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
///
/// Replaces the hand-written `ActionArgSpec { name, description, usage, args,
/// positional }` literal (~15 lines per action) with a single call; combined
/// with [`flag!`] this cuts ArgSpec boilerplate by roughly 70%. Positional is
/// optional and defaults to [`Positional::None`].
///
/// ```
/// cli_action!("list", "列出邮件", "everyday mail list [--unread]", &[flag!("unread", "仅未读", Bool)])
/// ```
#[macro_export]
macro_rules! cli_action {
    ($name:literal, $description:literal, $usage:literal, $args:expr) => {
        $crate::modules::ActionArgSpec {
            name: $name,
            description: $description,
            usage: $usage,
            args: $args,
            positional: $crate::modules::Positional::None,
        }
    };
    ($name:literal, $description:literal, $usage:literal, $args:expr, $positional:expr) => {
        $crate::modules::ActionArgSpec {
            name: $name,
            description: $description,
            usage: $usage,
            args: $args,
            positional: $positional,
        }
    };
}

/// Declare one flag with compact syntax (P1, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
///
/// Default kind is [`ArgKind::Value`] (`--name VALUE`); pass `Bool` for a
/// switch (`--name`) or `Multi` for a repeatable value flag.
#[macro_export]
macro_rules! flag {
    ($name:literal, $help:literal) => {
        $crate::modules::ArgSpec {
            name: $name,
            help: $help,
            kind: $crate::modules::ArgKind::Value,
        }
    };
    ($name:literal, $help:literal, Bool) => {
        $crate::modules::ArgSpec {
            name: $name,
            help: $help,
            kind: $crate::modules::ArgKind::Bool,
        }
    };
    ($name:literal, $help:literal, Multi) => {
        $crate::modules::ArgSpec {
            name: $name,
            help: $help,
            kind: $crate::modules::ArgKind::Multi,
        }
    };
}

/// Module registry.
///
/// Built by injecting config and an optional `--account` override; each
/// module reads only the account config it needs.
pub struct ModuleRegistry {
    pub(crate) modules: HashMap<&'static str, Box<dyn Executor>>,
}

impl ModuleRegistry {
    /// Build all modules from config.
    pub fn build(config: Arc<Config>) -> Result<Self> {
        let mut modules: HashMap<&'static str, Box<dyn Executor>> = HashMap::new();

        // Register each module. The module itself decides whether it needs
        // account config and whether missing config is tolerated.
        //
        // Business modules receive only their config **subset** (P2b,
        // [F012](../../docs/adr/F012-architecture-deepening-phase.md)): mail/cal/rss/
        // note/todo/bookmark depend on just their own section, removing hidden
        // dependencies on the full `Config`. Cross-module orchestrators
        // (timeline/search/auth) keep `Arc<Config>` — they genuinely need every
        // section.
        modules.insert(
            "config",
            Box::new(crate::modules::config::ConfigModule::new()),
        );
        modules.insert(
            "mail",
            Box::new(crate::modules::email::EmailModule::new(
                config.mail_module_config(),
            )),
        );
        modules.insert(
            "cal",
            Box::new(crate::modules::calendar::CalendarModule::new(
                config.calendar_module_config(),
            )),
        );
        modules.insert(
            "rss",
            Box::new(crate::modules::rss::RssModule::new(
                config.rss_module_config(),
            )),
        );
        modules.insert(
            "note",
            Box::new(crate::modules::note::NoteModule::new(
                config.note_module_config(),
            )),
        );
        modules.insert(
            "todo",
            Box::new(crate::modules::todo::TodoModule::new(
                config.todo_module_config(),
            )),
        );
        modules.insert(
            "bookmark",
            Box::new(crate::modules::bookmark::BookmarkModule::new(
                config.bookmark_module_config(),
            )),
        );
        modules.insert(
            "timeline",
            Box::new(crate::modules::timeline::TimelineModule::new(
                config.clone(),
            )),
        );

        // Memory module (Phase 15, ADR K001–K004): single global instance,
        // no account, no `auth` module touch.
        modules.insert(
            "memory",
            Box::new(crate::modules::memory::MemoryModule::new()),
        );

        // Cross-module unified search (Phase 11, ADR S001–S006).
        modules.insert(
            "search",
            Box::new(crate::modules::search::SearchModule::new(config.clone())),
        );

        // Top-level credential lifecycle (Phase 12, ADR R013–R015).
        modules.insert(
            "auth",
            Box::new(crate::modules::auth::AuthModule::new(config.clone())),
        );

        Ok(Self { modules })
    }

    /// Look up a module by name.
    pub fn get(&self, name: &str) -> Result<&dyn Executor> {
        self.modules
            .get(name)
            .map(|b| b.as_ref())
            .ok_or_else(|| AgentError::ModuleNotFound(name.to_string()))
    }

    /// Run every module's `health_check` (P3). Best-effort: a module reporting
    /// degraded is not an error here — the caller renders all rows.
    pub async fn health_check_all(&self) -> Vec<(&'static str, HealthStatus)> {
        let mut out = Vec::with_capacity(self.modules.len());
        for (name, module) in &self.modules {
            let status = match module.health_check().await {
                Ok(s) => s,
                Err(e) => HealthStatus::degraded(format!("error: {}", e.message())),
            };
            out.push((*name, status));
        }
        // Deterministic order: config first, then alphabetical, then memory.
        out.sort_by_key(|(name, _)| match *name {
            "config" => (0, *name),
            "memory" => (2, *name),
            _ => (1, *name),
        });
        out
    }

    /// Run every module's `initialize` (P3). Best-effort; failures are returned
    /// as `(module, error)` pairs so the caller can decide how strict to be.
    pub fn initialize_all(&self) -> Vec<(&'static str, AgentError)> {
        let mut errors = Vec::new();
        for (name, module) in &self.modules {
            if let Err(e) = module.initialize() {
                errors.push((*name, e));
            }
        }
        errors
    }

    /// Run every module's `shutdown` (P3). Best-effort, never fails.
    pub fn shutdown_all(&self) {
        for module in self.modules.values() {
            module.shutdown();
        }
    }
}

// ---- module submodule declarations ----
//
// note / todo / bookmark are directory modules (Phase 13, ADR R016/R017):
// each exposes `mod.rs` (Executor) + `backend.rs` (trait + factory) +
// `local.rs` (Local*Backend, was `*_local.rs`).
pub mod auth;
pub mod bookmark;
pub mod calendar;
pub mod config;
pub mod email;
pub mod email_cache;
pub mod email_pool;
pub mod local;
pub mod memory;
pub mod note;
pub mod rss;
pub mod rss_items;
pub mod search;
pub mod timeline;
pub mod todo;

/// Generic simple-argument parser, re-exported from [`crate::util::args`]
/// for backward compatibility with existing callers
/// (`crate::modules::parse_simple_args`).
pub use crate::util::args::parse_simple_args;

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyModule;
    #[async_trait]
    impl Executor for DummyModule {
        fn description(&self) -> &'static str {
            "test"
        }
        fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
            crate::modules::ModuleArgSpec {
                name: "dummy",
                description: "test",
                actions: &[],
            }
        }
        async fn execute(
            &self,
            _a: &str,
            _args: &[String],
            _ctx: &RequestContext,
        ) -> Result<Output> {
            Ok(Output::text("ok"))
        }
    }

    #[tokio::test]
    async fn trait_object_dispatch_works() {
        let m: Box<dyn Executor> = Box::new(DummyModule);
        let ctx = RequestContext::cli("test-req".into());
        let out = m.execute("anything", &[], &ctx).await.unwrap();
        assert_eq!(out.render(crate::output::RenderMode::Text), "ok");
    }

    // ---- P3: health_check_all ordering + degraded propagation ----

    struct HealthModule {
        status: HealthStatus,
    }
    #[async_trait]
    impl Executor for HealthModule {
        fn description(&self) -> &'static str {
            "health-test"
        }
        fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
            crate::modules::ModuleArgSpec {
                name: "health-test",
                description: "health-test",
                actions: &[],
            }
        }
        async fn execute(
            &self,
            _a: &str,
            _args: &[String],
            _ctx: &RequestContext,
        ) -> Result<Output> {
            Ok(Output::text("ok"))
        }
        async fn health_check(&self) -> Result<HealthStatus> {
            Ok(self.status.clone())
        }
    }

    #[tokio::test]
    async fn health_check_all_orders_config_first_memory_last() {
        let mut modules: HashMap<&'static str, Box<dyn Executor>> = HashMap::new();
        modules.insert(
            "zebra",
            Box::new(HealthModule {
                status: HealthStatus::healthy(),
            }),
        );
        modules.insert(
            "config",
            Box::new(HealthModule {
                status: HealthStatus::healthy(),
            }),
        );
        modules.insert(
            "memory",
            Box::new(HealthModule {
                status: HealthStatus::healthy(),
            }),
        );
        modules.insert(
            "alpha",
            Box::new(HealthModule {
                status: HealthStatus::healthy(),
            }),
        );
        let registry = ModuleRegistry { modules };
        let rows = registry.health_check_all().await;
        assert_eq!(rows[0].0, "config"); // config first
        assert_eq!(rows.last().unwrap().0, "memory"); // memory last
        assert!(rows.iter().all(|(_, s)| s.healthy));
    }

    #[tokio::test]
    async fn health_check_all_propagates_degraded() {
        let mut modules: HashMap<&'static str, Box<dyn Executor>> = HashMap::new();
        modules.insert(
            "broken",
            Box::new(HealthModule {
                status: HealthStatus::degraded("db: closed"),
            }),
        );
        modules.insert(
            "fine",
            Box::new(HealthModule {
                status: HealthStatus::healthy(),
            }),
        );
        let registry = ModuleRegistry { modules };
        let rows = registry.health_check_all().await;
        let broken = rows.iter().find(|(n, _)| *n == "broken").unwrap();
        assert!(!broken.1.healthy);
        assert_eq!(broken.1.detail, "db: closed");
    }
}
