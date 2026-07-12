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
    async fn execute(&self, action: &str, args: &[String]) -> Result<Output>;
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
        modules.insert(
            "config",
            Box::new(crate::modules::config::ConfigModule::new()),
        );
        modules.insert(
            "mail",
            Box::new(crate::modules::email::EmailModule::new(config.clone())),
        );
        modules.insert(
            "cal",
            Box::new(crate::modules::calendar::CalendarModule::new(
                config.clone(),
            )),
        );
        modules.insert(
            "rss",
            Box::new(crate::modules::rss::RssModule::new(config.clone())),
        );
        modules.insert(
            "note",
            Box::new(crate::modules::note::NoteModule::new(config.clone())),
        );
        modules.insert(
            "todo",
            Box::new(crate::modules::todo::TodoModule::new(config.clone())),
        );
        modules.insert(
            "bookmark",
            Box::new(crate::modules::bookmark::BookmarkModule::new(
                config.clone(),
            )),
        );
        modules.insert(
            "timeline",
            Box::new(crate::modules::timeline::TimelineModule::new(
                config.clone(),
            )),
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
}

// ---- module submodule declarations ----
pub mod bookmark;
pub mod bookmark_local;
pub mod calendar;
pub mod config;
pub mod email;
pub mod email_cache;
pub mod email_pool;
pub mod local;
pub mod note;
pub mod note_local;
pub mod rss;
pub mod timeline;
pub mod todo;
pub mod todo_local;

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
        async fn execute(&self, _a: &str, _args: &[String]) -> Result<Output> {
            Ok(Output::text("ok"))
        }
    }

    #[tokio::test]
    async fn trait_object_dispatch_works() {
        let m: Box<dyn Executor> = Box::new(DummyModule);
        let out = m.execute("anything", &[]).await.unwrap();
        assert_eq!(out.render(crate::output::RenderMode::Text), "ok");
    }
}
