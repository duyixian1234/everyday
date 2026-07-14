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
use crate::modules::local::is_local_provider;
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

    /// Build the [`SearchRegistry`] for the current config.
    ///
    /// Local provider accounts are registered one provider per account;
    /// Notion-backed accounts are skipped in v1 (live-fetch-on-search was
    /// rejected by [S005](../../docs/adr/S005-time-semantics-scope.md)).
    /// RSS has no account concept, so a single `RssSearchProvider` is added
    /// when at least one feed is configured.
    pub fn build_registry(&self) -> SearchRegistry {
        let mut reg = SearchRegistry::new();

        for acc in &self.config.note.accounts {
            if is_local_provider(&acc.provider) {
                reg.register(Arc::new(note_local::NoteSearchProvider::new(acc.clone())));
            }
        }
        for acc in &self.config.todo.accounts {
            if is_local_provider(&acc.provider) {
                reg.register(Arc::new(todo_local::TodoSearchProvider::new(acc.clone())));
            }
        }
        for acc in &self.config.bookmark.accounts {
            if is_local_provider(&acc.provider) {
                reg.register(Arc::new(bookmark_local::BookmarkSearchProvider::new(
                    acc.clone(),
                )));
            }
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
impl Executor for SearchModule {
    fn description(&self) -> &'static str {
        "Cross-module unified search: query note / todo / bookmark / rss / cal / mail / memory in one shot."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static QUERY_ARGS: &[ArgSpec] = &[
            ArgSpec {
                name: "module",
                help: "模块过滤：note,todo,bookmark,rss,cal,mail,memory（逗号分隔）",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "since",
                help: "相对起点：YYYY-MM-DD 或 30m/2h/1d/7d",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "limit",
                help: "全局条数上限（默认 20）",
                kind: ArgKind::Value,
            },
        ];
        static ACTIONS: &[ActionArgSpec] = &[ActionArgSpec {
            name: "query",
            description: "跨模块统一搜索",
            usage: "everyday search \"<query>\" [--module a,b,c] [--since 7d] [--limit N]",
            args: QUERY_ARGS,
            positional: Positional::OptionalSingle,
        }];
        ModuleArgSpec {
            name: "search",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        // search has only one action; the default positional arg carries
        // the query string.
        let (flags, positional) = parse_simple_args(args);
        let json_mode = crate::util::json_mode::is_json();

        match action {
            "" | "query" => self.do_query(&positional, &flags, json_mode).await,
            other => Err(AgentError::UnknownAction(format!("search {other}"))),
        }
    }
}

impl SearchModule {
    /// Run a unified search.
    async fn do_query(
        &self,
        positional: &[String],
        flags: &std::collections::HashMap<String, String>,
        json_mode: bool,
    ) -> Result<Output> {
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

        let registry = self.build_registry();
        let outcome = registry
            .query(&sq, &self.config, &module_filter, global_limit)
            .await?;

        render_search(&outcome, &sq, json_mode)
    }
}

/// Render a `SearchOutcome` to the appropriate `Output` variant.
///
/// Text mode: a header line + a table (one row per hit).
/// `--json` mode: a flat JSON array of hit objects, identical in shape to
/// `Hit`'s `Serialize` impl. Warnings, if any, are emitted via `eprintln!`
/// (per [R001](../../docs/adr/R001-thread-local-json-mode.md) — `--json`
/// keeps stderr structured).
fn render_search(outcome: &SearchOutcome, q: &SearchQuery, json_mode: bool) -> Result<Output> {
    // Surface warnings to stderr (both modes); in --json mode the
    // structured shape is preserved by keeping them off stdout.
    for w in &outcome.warnings {
        if json_mode {
            eprintln!(
                "{{\"_warning\":\"search_provider_failed\",\"module\":\"{}\",\"message\":\"{}\"}}",
                w.module,
                w.message.replace('"', "'")
            );
        } else {
            eprintln!(
                "warning: search provider '{}' failed: {}",
                w.module, w.message
            );
        }
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

    /// Build a SearchModule with an empty config; memory is registered
    /// unconditionally (single-instance, see K004) so the registry is
    /// not empty — only the account-bound providers are absent.
    #[test]
    fn build_registry_empty_config() {
        let cfg = Arc::new(Config::default());
        let m = SearchModule::new(cfg);
        let reg = m.build_registry();
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
            default_database_id: None,
            default_page_id: None,
            db_path: None,
        });
        cfg.todo.accounts.push(crate::config::TodoAccount {
            name: "work".into(),
            provider: "local".into(),
            parent_page_id: None,
            default_database_id: None,
            db_path: None,
        });
        let m = SearchModule::new(Arc::new(cfg));
        let reg = m.build_registry();
        let mods = reg.modules();
        assert!(mods.contains(&"note"));
        assert!(mods.contains(&"todo"));
    }

    /// Notion-backed accounts are intentionally skipped in v1.
    #[test]
    fn build_registry_skips_notion_accounts() {
        let mut cfg = Config::default();
        cfg.note.accounts.push(crate::config::NoteAccount {
            name: "personal".into(),
            provider: "notion".into(),
            default_database_id: None,
            default_page_id: None,
            db_path: None,
        });
        let m = SearchModule::new(Arc::new(cfg));
        let reg = m.build_registry();
        // No note provider for notion accounts in v1.
        assert!(!reg.modules().contains(&"note"));
    }

    /// RSS provider is registered when at least one feed is configured.
    #[test]
    fn build_registry_registers_rss_only_when_feeds_exist() {
        let mut cfg = Config::default();
        // Empty feeds list: no rss provider.
        let m = SearchModule::new(Arc::new(cfg.clone()));
        let reg = m.build_registry();
        assert!(!reg.modules().contains(&"rss"));

        // With one feed: rss provider appears.
        cfg.rss.feeds.push(crate::config::RssFeed {
            name: "hn".into(),
            url: "https://hnrss.org/frontpage".into(),
            category: None,
        });
        let m = SearchModule::new(Arc::new(cfg));
        let reg = m.build_registry();
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
        let m = SearchModule::new(Arc::new(cfg));
        let reg = m.build_registry();
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
}
