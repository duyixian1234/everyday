//! `search` module: cross-module unified search aggregator.
//!
//! Phase 11 ([S001-S006](../../docs/adr/S001-search-architecture.md)).
//! CLI:
//! - `everyday search "<query>" [--module a,b,c] [--since 7d] [--limit N] [--json]`
//!
//! Implementation: build a [`SearchRegistry`] from the configured accounts,
//! call its `query`, and render the result. Module-level warnings (one
//! per failed provider) surface via stderr (text) or a structured
//! `_warning` line (`--json`) — consistent with the ops-log style
//! ([R001](../../docs/adr/R001-thread-local-json-mode.md)).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::bookmark::local as bookmark_local;
use crate::modules::memory;
use crate::modules::note::local as note_local;
use crate::modules::parse_simple_args;
use crate::modules::timeline::parse_source_list;
use crate::modules::todo::local as todo_local;
use crate::modules::{Executor, calendar, email, rss_items};
use crate::output::Output;
use crate::search::{SearchOutcome, SearchQuery, SearchRegistry};
use crate::util::datetime::parse_since;

/// Default per-module hard cap (matches what each provider uses
/// internally). The aggregator's hard cap, [DEFAULT_GLOBAL_LIMIT], is
/// applied to the merged result.
const DEFAULT_GLOBAL_LIMIT: usize = 20;

/// Modules searchable (mail joined in v1.1, see ADR S007 — it queries the
/// local envelope cache rather than IMAP `SEARCH`; memory joined in v1.2,
/// see ADR K003 — single global provider over the current-state view).
/// See [S005](../../docs/adr/S005-time-semantics-scope.md).
pub const SEARCHABLE_MODULES: &[&str] =
    &["note", "todo", "bookmark", "rss", "cal", "mail", "memory"];

pub struct SearchModule {
    config: Arc<Config>,
}

impl SearchModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

// ============ service layer (P1 wiring, [F012](../../docs/adr/F012-architecture-deepening-phase.md)) ============

/// Search service trait: domain method, no `Output` in sight (P1).
///
/// `ConfigSearchBackend` builds the [`SearchRegistry`] from config and runs the
/// query; `testkit::MockSearchBackend` (tests) returns fixed data. `dispatch`
/// is the only place that maps CLI args → service call → `Output`.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Run a unified search against the registered providers.
    async fn query(
        &self,
        q: &SearchQuery,
        module_filter: &[String],
        global_limit: usize,
    ) -> Result<SearchOutcome>;
}

/// Real backend: holds the full `Config` (search is a cross-module
/// orchestrator, [F012 P2b](../../docs/adr/F012-architecture-deepening-phase.md)).
pub struct ConfigSearchBackend {
    config: Arc<Config>,
}

impl ConfigSearchBackend {
    /// Build the [`SearchRegistry`] for the current config.
    ///
    /// One provider per local account of note/todo/bookmark (local-only since
    /// v0.13.0, [R019](../../docs/adr/R019-remove-notion-provider.md)).
    /// RSS has no account concept, so a single `RssSearchProvider` is added
    /// when at least one feed is configured.
    pub fn build_registry(&self) -> SearchRegistry {
        let mut reg = SearchRegistry::new();

        for acc in &self.config.note.accounts {
            reg.register(Arc::new(note_local::NoteSearchProvider::new(acc.clone())));
        }
        for acc in &self.config.todo.accounts {
            reg.register(Arc::new(todo_local::TodoSearchProvider::new(acc.clone())));
        }
        for acc in &self.config.bookmark.accounts {
            reg.register(Arc::new(bookmark_local::BookmarkSearchProvider::new(
                acc.clone(),
            )));
        }
        for acc in &self.config.calendar.accounts {
            reg.register(Arc::new(calendar::CalSearchProvider::new(
                acc.clone(),
                acc.ignore_calendars.clone(),
            )));
        }
        if !self.config.rss.feeds.is_empty() {
            reg.register(Arc::new(rss_items::RssSearchProvider::new()));
        }
        if !self.config.mail.accounts.is_empty() {
            // Single global provider: scans the whole envelope cache across
            // all accounts (local-first, see ADR S007).
            reg.register(Arc::new(email::MailSearchProvider::new()));
        }
        // Memory is single-instance (K004); register unconditionally.
        // Empty database just yields zero hits — no reason to gate on
        // config presence.
        reg.register(memory::search_provider());
        reg
    }
}

#[async_trait]
impl SearchBackend for ConfigSearchBackend {
    async fn query(
        &self,
        q: &SearchQuery,
        module_filter: &[String],
        global_limit: usize,
    ) -> Result<SearchOutcome> {
        let registry = self.build_registry();
        registry
            .query(q, &self.config, module_filter, global_limit)
            .await
    }
}

/// Build the search backend for the current config.
pub fn for_config(config: &Arc<Config>) -> Box<dyn SearchBackend> {
    Box::new(ConfigSearchBackend {
        config: config.clone(),
    })
}

#[async_trait]
impl Executor for SearchModule {
    fn description(&self) -> &'static str {
        "Cross-module unified search: query note / todo / bookmark / rss / cal / mail / memory in one shot."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgSpec, ModuleArgSpec, Positional};
        static QUERY_ARGS: &[ArgSpec] = &[
            flag!(
                "module",
                "模块过滤：note,todo,bookmark,rss,cal,mail,memory（逗号分隔）"
            ),
            flag!("since", "相对起点：YYYY-MM-DD 或 30m/2h/1d/7d"),
            flag!("limit", "全局条数上限（默认 20）"),
        ];
        static ACTIONS: &[ActionArgSpec] = &[cli_action!(
            "query",
            "跨模块统一搜索",
            "everyday search \"<query>\" [--module a,b,c] [--since 7d] [--limit N]",
            QUERY_ARGS,
            Positional::OptionalSingle
        )];
        ModuleArgSpec {
            name: "search",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(
        &self,
        action: &str,
        args: &[String],
        _ctx: &crate::shared::request_context::RequestContext,
    ) -> Result<Output> {
        // search has only one action; the default positional arg carries
        // the query string.
        let backend = for_config(&self.config);
        dispatch(&*backend, action, args).await
    }
}

/// CLI dispatch: parse args → call the [`SearchBackend`] service method →
/// render to `Output` (P1 wiring, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
///
/// This is the only function in the search module that touches `Output`;
/// service methods are output-free and directly testable via
/// [`testkit::MockSearchBackend`].
async fn dispatch(backend: &dyn SearchBackend, action: &str, args: &[String]) -> Result<Output> {
    let (flags, positional) = parse_simple_args(args);
    let json_mode = crate::util::json_mode::is_json();

    match action {
        "" | "query" => {
            // The query string: positional[0] or an explicit --query flag.
            let query = flags
                .get("query")
                .cloned()
                .or_else(|| positional.first().cloned())
                .ok_or_else(|| {
                    AgentError::InvalidArgument(
                        "search requires a query string (positional arg or --query Q)".into(),
                    )
                })?;

            let mut sq = SearchQuery::new(query);
            if let Some(s) = flags.get("since") {
                sq.since = Some(parse_since(s)?);
            }
            if let Some(limit_str) = flags.get("limit") {
                let parsed: usize = limit_str.parse().map_err(|_| {
                    AgentError::InvalidArgument(format!(
                        "invalid --limit '{limit_str}', expected non-negative integer"
                    ))
                })?;
                sq.limit = Some(parsed);
            }

            // --module allow-list (validated against v1 search scope).
            let module_filter = parse_source_list(flags.get("module"), SEARCHABLE_MODULES)?;

            // Global limit: --limit overrides default; but --limit also
            // applies to per-module in sq.limit. The aggregator expects the
            // global limit as a separate argument (see SearchRegistry::query).
            let global_limit = sq.limit.unwrap_or(DEFAULT_GLOBAL_LIMIT);

            let outcome = backend.query(&sq, &module_filter, global_limit).await?;
            render_search(&outcome, &sq, json_mode)
        }
        other => Err(AgentError::UnknownAction(format!("search {other}"))),
    }
}

/// Render a `SearchOutcome` to the appropriate `Output` variant.
///
/// Text mode: a header line + a table (one row per hit).
/// `--json` mode: a flat JSON array of hit objects, identical in shape to
/// `Hit`'s `Serialize` impl. Warnings, if any, are emitted as `warn!`
/// tracing events (per [R001](../../docs/adr/R001-thread-local-json-mode.md)
/// — `--json` keeps stderr structured; the layer renders `_warning` shapes).
fn render_search(outcome: &SearchOutcome, q: &SearchQuery, json_mode: bool) -> Result<Output> {
    // Surface warnings to stderr (both modes); in --json mode the
    // structured shape is preserved by keeping them off stdout. The layer
    // renders `{"_warning": "search_provider_failed", ...}` in JSON mode and
    // the `warning: ...` line in text mode, both from this single event
    // (serde_json escapes `message` correctly — no quote-munging needed).
    for w in &outcome.warnings {
        tracing::warn!(
            target: "everyday",
            _warning = "search_provider_failed",
            module = %w.module,
            message = %w.message,
            warning_text = %format!("warning: search provider '{}' failed: {}", w.module, w.message),
        );
    }

    if json_mode {
        let arr: Vec<Value> = outcome
            .hits
            .iter()
            .map(|h| {
                json!({
                    "module": h.module,
                    "account": h.account,
                    "id": h.id,
                    "title": h.title,
                    "snippet": h.snippet,
                    "url": h.url,
                    "ts": h.ts.map(|t| t.to_rfc3339()),
                    "kind": h.kind,
                })
            })
            .collect();
        Ok(Output::Json(Value::Array(arr)))
    } else {
        if outcome.hits.is_empty() {
            // Empty result, exit 0 (consistent with timeline's empty
            // result behavior; see [S004](../../docs/adr/S004-execution-model.md)).
            return Ok(Output::text(format!("no hits for \"{}\"", q.raw)));
        }
        let headers: Vec<String> = vec![
            "module".into(),
            "account".into(),
            "id".into(),
            "title".into(),
            "snippet".into(),
            "ts".into(),
        ];
        let rows: Vec<Vec<String>> = outcome
            .hits
            .iter()
            .map(|h| {
                vec![
                    h.module.to_string(),
                    h.account.clone().unwrap_or_default(),
                    h.id.clone(),
                    h.title.clone(),
                    h.snippet.clone(),
                    h.ts.map(|t| t.to_rfc3339()).unwrap_or_default(),
                ]
            })
            .collect();
        Ok(Output::records(headers, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a ConfigSearchBackend with an empty config; memory is registered
    /// unconditionally (single-instance, see K004) so the registry is
    /// not empty — only the account-bound providers are absent.
    #[test]
    fn build_registry_empty_config() {
        let cfg = Arc::new(Config::default());
        let backend = ConfigSearchBackend { config: cfg };
        let reg = backend.build_registry();
        let mods = reg.modules();
        // memory is single-instance; it should always be present.
        assert!(mods.contains(&"memory"));
        // account-bound providers should NOT be present (no accounts).
        assert!(!mods.contains(&"note"));
        assert!(!mods.contains(&"todo"));
        assert!(!mods.contains(&"bookmark"));
        assert!(!mods.contains(&"cal"));
        assert!(!mods.contains(&"mail"));
        assert!(!mods.contains(&"rss"));
    }

    /// A configured local note account yields one note provider.
    #[test]
    fn build_registry_registers_local_providers() {
        let mut cfg = Config::default();
        cfg.note.accounts.push(crate::config::NoteAccount {
            name: "personal".into(),
            provider: "local".into(),
            default_page_id: None,
            db_path: None,
        });
        cfg.todo.accounts.push(crate::config::TodoAccount {
            name: "work".into(),
            provider: "local".into(),
            db_path: None,
        });
        let backend = ConfigSearchBackend {
            config: Arc::new(cfg),
        };
        let reg = backend.build_registry();
        let mods = reg.modules();
        assert!(mods.contains(&"note"));
        assert!(mods.contains(&"todo"));
    }

    /// RSS provider is registered when at least one feed is configured.
    #[test]
    fn build_registry_registers_rss_only_when_feeds_exist() {
        let mut cfg = Config::default();
        // Empty feeds list: no rss provider.
        let backend = ConfigSearchBackend {
            config: Arc::new(cfg.clone()),
        };
        let reg = backend.build_registry();
        assert!(!reg.modules().contains(&"rss"));

        // With one feed: rss provider appears.
        cfg.rss.feeds.push(crate::config::RssFeed {
            name: "hn".into(),
            url: "https://hnrss.org/frontpage".into(),
            category: None,
        });
        let backend = ConfigSearchBackend {
            config: Arc::new(cfg),
        };
        let reg = backend.build_registry();
        assert!(reg.modules().contains(&"rss"));
    }

    /// Mail provider is registered (single global instance) when at least one
    /// mail account is configured.
    #[test]
    fn build_registry_registers_mail_when_accounts_exist() {
        let mut cfg = Config::default();
        cfg.mail.accounts.push(crate::config::MailAccount {
            name: "work".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            username: "me@example.com".into(),
            tls: true,
        });
        let backend = ConfigSearchBackend {
            config: Arc::new(cfg),
        };
        let reg = backend.build_registry();
        assert!(reg.modules().contains(&"mail"));
    }

    /// Module spec exposes a single `query` action.
    #[test]
    fn module_arg_spec_has_query_action() {
        let m = SearchModule::new(Arc::new(Config::default()));
        let spec = m.module_arg_spec();
        assert_eq!(spec.name, "search");
        assert_eq!(spec.actions.len(), 1);
        assert_eq!(spec.actions[0].name, "query");
    }

    // ============ P1 dispatch tests (Mock backend) ============

    /// Test-only in-memory backend for dispatch tests.
    pub(crate) mod testkit {
        use super::super::*;
        use std::sync::Mutex;

        #[derive(Default)]
        pub struct MockSearchBackend {
            pub outcome: SearchOutcome,
            /// Query string of the last `query` call.
            pub last_raw: Mutex<Option<String>>,
            /// Module filter of the last call.
            pub last_filter: Mutex<Option<Vec<String>>>,
        }

        #[async_trait]
        impl SearchBackend for MockSearchBackend {
            async fn query(
                &self,
                q: &SearchQuery,
                module_filter: &[String],
                _global_limit: usize,
            ) -> Result<SearchOutcome> {
                *self.last_raw.lock().unwrap() = Some(q.raw.clone());
                *self.last_filter.lock().unwrap() = Some(module_filter.to_vec());
                Ok(self.outcome.clone())
            }
        }
    }

    use testkit::MockSearchBackend;

    #[tokio::test]
    async fn dispatch_query_forwards_query_and_filter() {
        let mock = MockSearchBackend::default();
        let out = dispatch(
            &mock,
            "query",
            &[
                "rust cli".to_string(),
                "--module".to_string(),
                "note,todo".to_string(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(mock.last_raw.lock().unwrap().as_deref(), Some("rust cli"));
        assert_eq!(
            mock.last_filter.lock().unwrap().as_deref(),
            Some(["note".to_string(), "todo".to_string()].as_slice())
        );
        // Empty outcome → text "no hits" (exit 0).
        if let Output::Text(s) = out {
            assert!(s.contains("no hits"));
        } else {
            panic!("expected Text output");
        }
    }

    #[tokio::test]
    async fn dispatch_query_requires_query_string() {
        let mock = MockSearchBackend::default();
        let err = dispatch(&mock, "query", &[]).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn dispatch_query_invalid_limit_errors() {
        let mock = MockSearchBackend::default();
        let err = dispatch(
            &mock,
            "query",
            &["x".to_string(), "--limit".to_string(), "-1".to_string()],
        )
        .await
        .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn dispatch_query_invalid_module_errors() {
        let mock = MockSearchBackend::default();
        let err = dispatch(
            &mock,
            "query",
            &["x".to_string(), "--module".to_string(), "bogus".to_string()],
        )
        .await
        .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn dispatch_unknown_action_errors() {
        let mock = MockSearchBackend::default();
        let err = dispatch(&mock, "bogus", &["x".to_string()])
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "UnknownAction");
    }
}
