//! Cross-module unified search: types, trait, and aggregator.
//!
//! Phase 11 wires every participating module to a single `Searchable` trait
//! and exposes the aggregator as the `search` first-class module
//! ([S001](../docs/adr/S001-search-architecture.md)).
//!
//! Two key contracts:
//! - `SearchQuery` / `Hit` ([S002](../docs/adr/S002-hit-normalization.md))
//!   are the **stable shape** consumed by AI agents; module-specific
//!   schemas are mapped into `Hit` by each provider.
//! - Execution is **concurrent, best-effort** ([S004](../docs/adr/S004-execution-model.md)):
//!   `join_all` fans out, per-module failures are surfaced as `warnings`,
//!   and only when **every** provider fails does the query error.
//!
//! Query semantics — tokenize `raw` by whitespace, OR over tokens, match
//! via case-insensitive GLOB substring ([S003](../docs/adr/S003-query-semantics.md)).
//! The GLOB builder lives here so every local provider can reuse it.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Config;
use crate::error::{AgentError, Result};

/// Per-call query passed to every [`Searchable::search`] implementation.
#[derive(Debug, Clone)]
#[allow(dead_code)] // public API: registered providers arrive in subsequent commits.
pub struct SearchQuery {
    /// Original query string (free text). Tokenized into `tokens`.
    pub raw: String,
    /// Optional lower-bound (UTC). Items with no `ts` are unaffected; items
    /// with `ts` outside the window are filtered out by the aggregator.
    pub since: Option<DateTime<Utc>>,
    /// Per-module cap override (default 50). Per [S004](../docs/adr/S004-execution-model.md)
    /// the aggregator bounds each provider's work; modules should not exceed it.
    pub limit: Option<usize>,
}

impl SearchQuery {
    /// Construct from a raw string.
    #[allow(dead_code)] // public API: consumed by SearchRegistry and provider impls later.
    pub fn new(raw: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            since: None,
            limit: None,
        }
    }

    /// Tokenize `raw` by whitespace, dropping empty tokens. Tokens are
    /// returned verbatim — matching is case-insensitive GLOB but the
    /// substring itself is the user's input.
    #[allow(dead_code)] // public API: consumed by SearchRegistry and provider impls later.
    pub fn tokens(&self) -> Vec<&str> {
        self.raw
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .collect()
    }
}

/// A single normalized hit returned by [`Searchable::search`].
///
/// One Hit = one entity in one module. The aggregator merges hits from
/// multiple modules; downstream agents see a single flat list.
///
/// Field meanings ([S002](../docs/adr/S002-hit-normalization.md)):
/// - `module` is the module id (`note` / `todo` / ...).
/// - `ts` is the module's **primary** time, UTC ([S005](../docs/adr/S005-time-semantics-scope.md)).
///   Local-timezone rendering is the consumer's concern.
/// - `url` is optional: empty for purely-local entities; agents use
///   `module` + `id` to act via the respective module's actions.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)] // public API: registered providers arrive in subsequent commits.
pub struct Hit {
    pub module: &'static str,
    pub account: Option<String>,
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub url: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub kind: &'static str,
}

/// Per-module warning surfaced when a provider's `search` returns `Err`
/// but at least one other provider succeeded. Surfaced via stderr (text)
/// or a structured `_warning` line (`--json`) by the search module —
/// consistent with the ops-log warning style ([R001](../docs/adr/R001-thread-local-json-mode.md)).
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)] // public API: surfaced by the search module in a later commit.
pub struct SearchWarning {
    pub module: String,
    pub message: String,
}

/// Aggregated result returned by [`SearchRegistry::query`].
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // public API: consumed by the search module in a later commit.
pub struct SearchOutcome {
    /// Merged + sorted + capped hits (see [S004](../docs/adr/S004-execution-model.md)).
    pub hits: Vec<Hit>,
    /// Per-module errors collected during fan-out. Empty when every
    /// provider succeeded; also empty when *no* provider was selected.
    pub warnings: Vec<SearchWarning>,
}

/// Searchable capability trait.
///
/// Modules that expose searchable items implement this trait. The aggregator
/// holds `Arc<dyn Searchable>` and fans out concurrently via `join_all`.
///
/// `#[async_trait]` is used because native `async fn in trait` (stable since
/// Rust 1.75) does not make a trait dyn-compatible; we need `Arc<dyn Searchable>`
/// for concurrent fan-out and module-conditional registration. `async_trait`
/// is already a project dep (used by `TimelineProvider`, see
/// [L004](../docs/adr/L004-timeline-provider-pull-only.md)) — no new dep is added.
/// See [S001](../docs/adr/S001-search-architecture.md).
#[async_trait]
pub trait Searchable: Send + Sync {
    /// Module id (`"note"`, `"todo"`, `"cal"`, ...). Stable; used by
    /// `--module` filtering and `Hit::module`.
    fn module_name(&self) -> &'static str;

    /// Run the search and return hits within the module's internal cap.
    ///
    /// Returning `Err` does NOT abort the aggregator — the error is
    /// captured into a `SearchWarning` and the other providers still run
    /// ([S004](../docs/adr/S004-execution-model.md)).
    async fn search(&self, q: &SearchQuery, cfg: &Config) -> Result<Vec<Hit>>;
}

/// Aggregator: holds the registered `Searchable` providers and runs the
/// concurrent fan-out.
///
/// `register` and `query` are the only public methods; providers can be
/// added in any order.
#[allow(dead_code)] // public API: registered providers arrive in subsequent commits.
pub struct SearchRegistry {
    providers: Vec<Arc<dyn Searchable>>,
}

impl SearchRegistry {
    /// New empty registry.
    #[allow(dead_code)] // public API: consumed by the search module later.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register one provider. Order is preserved for stable fan-out
    /// and stable hit merging (see `query`).
    #[allow(dead_code)] // public API: registered providers arrive in subsequent commits.
    pub fn register(&mut self, p: Arc<dyn Searchable>) {
        self.providers.push(p);
    }

    /// Registered module names, in registration order.
    #[allow(dead_code)] // public API: surfaced by the search module.
    pub fn modules(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.module_name()).collect()
    }

    /// Run a unified search.
    ///
    /// 1. Filter by `module_filter` (empty = all registered).
    /// 2. Concurrent fan-out via [`tokio::join_all` equivalent for Searchable`].
    /// 3. Best-effort: per-provider `Err` -> `SearchWarning`; only when
    ///    **every** provider fails does `query` return `AgentError`
    ///    ([S004](../docs/adr/S004-execution-model.md)).
    /// 4. Cap & limit:
    ///    - Per-module internal cap is enforced by each provider (50, see ADR).
    ///    - Global `limit` defaults to 20; applied after merge + sort.
    /// 5. Empty result with no warnings is **not** an error (exit 0).
    #[allow(dead_code)] // public API: consumed by the search module later.
    pub async fn query(
        &self,
        q: &SearchQuery,
        cfg: &Config,
        module_filter: &[String],
        global_limit: usize,
    ) -> Result<SearchOutcome> {
        if q.raw.trim().is_empty() {
            return Err(AgentError::InvalidArgument(
                "search query must not be empty".into(),
            ));
        }

        // 1. Filter target providers.
        let targets: Vec<&Arc<dyn Searchable>> = if module_filter.is_empty() {
            self.providers.iter().collect()
        } else {
            self.providers
                .iter()
                .filter(|p| module_filter.iter().any(|m| m == p.module_name()))
                .collect()
        };

        // An empty-after-filter state is not an error: surface zero hits, exit 0.
        // (Previously: the only failure path was "every module failed". An empty
        // selection has nobody to fail, so this is the "no matches" path.)
        if targets.is_empty() {
            return Ok(SearchOutcome::default());
        }

        // 2. Concurrent fan-out. Each future returns Result<Vec<Hit>>; we
        // unwrap one layer (the future) and then per-provider error vs hit.
        let futures = targets
            .iter()
            .map(|p| async move { (p.module_name(), p.search(q, cfg).await) });
        let results: Vec<(&'static str, Result<Vec<Hit>>)> =
            futures::future::join_all(futures).await;

        // 3. Best-effort: aggregate hits + warnings.
        let mut hits = Vec::new();
        let mut warnings = Vec::new();
        for (module, res) in results {
            match res {
                Ok(items) => hits.extend(items),
                Err(e) => warnings.push(SearchWarning {
                    module: module.to_string(),
                    message: e.message(),
                }),
            }
        }

        // If every provider failed -> error. We pick the last warning's message
        // to surface root cause, but include the count for context.
        if hits.is_empty() && !warnings.is_empty() {
            let summary = warnings
                .iter()
                .map(|w| format!("{}: {}", w.module, w.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AgentError::Other(format!(
                "all {} search providers failed: {}",
                warnings.len(),
                summary
            )));
        }

        // 4. Global `since` filter (per-module ts in window or None).
        if let Some(since) = q.since {
            hits.retain(|h| h.ts.is_none_or(|ts| ts >= since));
        }

        // 5. Sort by ts desc (None sorts last). Stable order preserves registration
        // order among ties, so identical timestamps don't shuffle.
        hits.sort_by(|a, b| match (a.ts, b.ts) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        // 6. Global limit.
        if hits.len() > global_limit {
            hits.truncate(global_limit);
        }

        Ok(SearchOutcome { hits, warnings })
    }
}

impl Default for SearchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a case-insensitive GLOB substring pattern from a token.
///
/// Returns `None` for tokens containing GLOB metacharacters (`*`, `?`, `[`).
/// Callers should treat `None` as "token can't be safely turned into GLOB;
/// skip this token to avoid injection". This is the conservative path:
/// user-supplied tokens are NEVER wrapped into a GLOB that would match
/// arbitrary text.
#[allow(dead_code)] // helper for upcoming local SQLite GLOB providers.
pub fn glob_substring(token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    if token.contains(['*', '?', '[', ']']) {
        return None;
    }
    Some(format!("*{}*", token.to_ascii_lowercase()))
}

/// Build an OR-of-tokens SQL filter expression for one column, plus the
/// bound parameter list. Returns the SQL fragment (e.g.
/// `lower(col) GLOB ?1 OR lower(col) GLOB ?2`) and a Vec<String> of the
/// patterns to bind (already lowered).
///
/// Example:
/// ```
/// use everyday::search::build_glob_or;
/// let (sql, params) = build_glob_or("title", &["rust", "cli"]);
/// assert_eq!(sql, "(lower(title) GLOB ?1 OR lower(title) GLOB ?2)");
/// assert_eq!(params, vec!["*rust*".to_string(), "*cli*".to_string()]);
/// ```
#[allow(dead_code)] // helper for upcoming local SQLite GLOB providers.
pub fn build_glob_or(column: &str, tokens: &[&str]) -> (String, Vec<String>) {
    let mut patterns = Vec::with_capacity(tokens.len());
    for t in tokens {
        // Lowercase the token at bind time, so `?N` matches `lower(col)`.
        let lower = t.to_ascii_lowercase();
        patterns.push(format!("*{lower}*"));
    }
    let placeholders: Vec<String> = (1..=patterns.len()).map(|i| format!("?{i}")).collect();
    let expr = placeholders
        .iter()
        .map(|p| format!("lower({column}) GLOB {p}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = if placeholders.is_empty() {
        // No usable tokens -> short-circuit false so callers can
        // skip the OR clause entirely.
        "0=1".to_string()
    } else {
        format!("({expr})")
    };
    (sql, patterns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_tokens_splits_whitespace() {
        let q = SearchQuery::new("rust  cli   timeline");
        assert_eq!(q.tokens(), vec!["rust", "cli", "timeline"]);
    }

    #[test]
    fn search_query_tokens_skips_empty() {
        let q = SearchQuery::new("  rust  ");
        assert_eq!(q.tokens(), vec!["rust"]);
    }

    #[test]
    fn glob_substring_lowercases_and_wraps() {
        assert_eq!(glob_substring("Rust").as_deref(), Some("*rust*"));
        assert_eq!(glob_substring("cli").as_deref(), Some("*cli*"));
    }

    #[test]
    fn glob_substring_rejects_glob_metachars() {
        // Conservative: user tokens must never be wrapped into a wildcard
        // GLOB. Reject *, ?, [, ] to avoid injection.
        assert!(glob_substring("*").is_none());
        assert!(glob_substring("a?b").is_none());
        assert!(glob_substring("[abc]").is_none());
        assert!(glob_substring("").is_none());
    }

    #[test]
    fn build_glob_or_emits_placeholders() {
        let (sql, params) = build_glob_or("title", &["Rust", "CLI"]);
        assert_eq!(sql, "(lower(title) GLOB ?1 OR lower(title) GLOB ?2)");
        assert_eq!(params, vec!["*rust*", "*cli*"]);
    }

    #[test]
    fn build_glob_or_short_circuits_when_no_tokens() {
        let (sql, params) = build_glob_or("title", &[]);
        // No usable tokens -> false predicate (no rows match).
        assert_eq!(sql, "0=1");
        assert!(params.is_empty());
    }

    /// Provider that returns Ok with a fixed list of hits, used to verify
    /// the aggregator's fan-out + cap path.
    struct FixedProvider {
        module: &'static str,
        hits: Vec<Hit>,
    }
    #[async_trait]
    impl Searchable for FixedProvider {
        fn module_name(&self) -> &'static str {
            self.module
        }
        async fn search(&self, _q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>> {
            Ok(self.hits.clone())
        }
    }

    /// Provider that always errors.
    struct ErrorProvider {
        module: &'static str,
    }
    #[async_trait]
    impl Searchable for ErrorProvider {
        fn module_name(&self) -> &'static str {
            self.module
        }
        async fn search(&self, _q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>> {
            Err(AgentError::Network("boom".into()))
        }
    }

    fn hit(module: &'static str, id: &str, ts_secs: i64) -> Hit {
        Hit {
            module,
            account: None,
            id: id.to_string(),
            title: format!("title-{id}"),
            snippet: String::new(),
            url: None,
            ts: DateTime::<Utc>::from_timestamp(ts_secs, 0),
            kind: "item",
        }
    }

    #[tokio::test]
    async fn registry_empty_query_is_invalid() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits: vec![],
        }));
        let cfg = Config::default();
        let err = reg
            .query(&SearchQuery::new("   "), &cfg, &[], 20)
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn registry_empty_when_filter_excludes_everything() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits: vec![hit("note", "n1", 100)],
        }));
        let cfg = Config::default();
        let out = reg
            .query(&SearchQuery::new("rust"), &cfg, &["todo".to_string()], 20)
            .await
            .unwrap();
        assert!(out.hits.is_empty());
        assert!(out.warnings.is_empty());
    }

    #[tokio::test]
    async fn registry_fans_out_and_merges() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits: vec![hit("note", "n1", 200), hit("note", "n2", 100)],
        }));
        reg.register(Arc::new(FixedProvider {
            module: "todo",
            hits: vec![hit("todo", "t1", 300)],
        }));
        let cfg = Config::default();
        let out = reg
            .query(&SearchQuery::new("rust"), &cfg, &[], 20)
            .await
            .unwrap();
        assert_eq!(out.hits.len(), 3);
        // Sorted ts desc: 300, 200, 100.
        assert_eq!(out.hits[0].id, "t1");
        assert_eq!(out.hits[1].id, "n1");
        assert_eq!(out.hits[2].id, "n2");
        assert!(out.warnings.is_empty());
    }

    #[tokio::test]
    async fn registry_caps_global_limit_after_merge() {
        let mut reg = SearchRegistry::new();
        let hits: Vec<Hit> = (0..10)
            .map(|i| hit("note", &format!("n{i}"), 100 + i as i64))
            .collect();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits,
        }));
        let cfg = Config::default();
        let out = reg
            .query(&SearchQuery::new("x"), &cfg, &[], 3)
            .await
            .unwrap();
        assert_eq!(out.hits.len(), 3);
        // 109 (newest) comes first.
        assert_eq!(out.hits[0].id, "n9");
    }

    #[tokio::test]
    async fn registry_best_effort_partial_failure() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits: vec![hit("note", "n1", 100)],
        }));
        reg.register(Arc::new(ErrorProvider { module: "todo" }));
        let cfg = Config::default();
        let out = reg
            .query(&SearchQuery::new("x"), &cfg, &[], 20)
            .await
            .unwrap();
        // The successful module's hit is preserved; the failing module's
        // error is captured as a warning (not bubbled up).
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(out.warnings[0].module, "todo");
        assert!(out.warnings[0].message.contains("boom"));
    }

    #[tokio::test]
    async fn registry_total_failure_returns_error() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(ErrorProvider { module: "note" }));
        reg.register(Arc::new(ErrorProvider { module: "todo" }));
        let cfg = Config::default();
        let err = reg
            .query(&SearchQuery::new("x"), &cfg, &[], 20)
            .await
            .unwrap_err();
        // When every module fails, surface an `Other` with module summary.
        assert_eq!(err.type_name(), "Other");
        assert!(err.message().contains("note"));
        assert!(err.message().contains("todo"));
    }

    #[tokio::test]
    async fn registry_since_filter_drops_out_of_window() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits: vec![hit("note", "old", 100), hit("note", "new", 500)],
        }));
        let cfg = Config::default();
        let mut q = SearchQuery::new("x");
        q.since = DateTime::<Utc>::from_timestamp(200, 0);
        let out = reg.query(&q, &cfg, &[], 20).await.unwrap();
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].id, "new");
    }

    #[tokio::test]
    async fn registry_filter_by_module_subset() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits: vec![hit("note", "n1", 100)],
        }));
        reg.register(Arc::new(FixedProvider {
            module: "todo",
            hits: vec![hit("todo", "t1", 200)],
        }));
        let cfg = Config::default();
        let out = reg
            .query(&SearchQuery::new("x"), &cfg, &["note".to_string()], 20)
            .await
            .unwrap();
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].module, "note");
    }

    #[test]
    fn registry_modules_lists_in_order() {
        let mut reg = SearchRegistry::new();
        reg.register(Arc::new(FixedProvider {
            module: "note",
            hits: vec![],
        }));
        reg.register(Arc::new(FixedProvider {
            module: "todo",
            hits: vec![],
        }));
        assert_eq!(reg.modules(), vec!["note", "todo"]);
    }
}
