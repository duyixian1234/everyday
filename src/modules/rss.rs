//! RSS/Atom subscription module.
//!
//! Full implementation: feed management (follow/list/unfollow, written to `[[rss.feeds]]`)
//! plus fetch aggregation (digest/fetch, reqwest fetch + feed-rs parse).
//!
//! Design notes [F008](../../docs/adr/F008-rss-module.md):
//! - Feeds are public URLs, so **no keyring is needed** (unlike mail/cal).
//! - Config writes use a localized toml::Value edit (touching only the `rss.feeds` array),
//!   preserving mail/calendar and other sections and their field order to avoid clobbering
//!   other accounts.
//! - digest/fetch fetch concurrently with reqwest (with timeout + UA) and parse with feed-rs;
//!   a single feed failure is non-fatal (best-effort), consistent with the calendar module's
//!   per-calendar failure degradation.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::join_all;

use crate::config::{Config, RssFeed, RssModuleConfig};
use crate::error::{AgentError, Result};
use crate::modules::rss_items;
use crate::modules::{Executor, parse_simple_args};
use crate::output::Output;

/// Display row for a single aggregated entry, plus its sort key.
#[derive(Debug, Clone)]
pub struct RssEntryRow {
    pub feed: String,
    pub title: String,
    /// Feed summary直出, truncated to ~200 chars (the digest JSON contract;
    /// text mode truncates further at render time). `fetch` carries it but
    /// its render layer does not output it — the info-level difference
    /// between digest and fetch (F008 amendment).
    pub summary: String,
    pub published: String,
    pub author: String,
    pub link: String,
    /// Publish time used for sorting (entries without a time sort last).
    pub sort_key: Option<DateTime<Utc>>,
}

/// Options for `digest`, parsed from CLI flags by `dispatch`.
#[derive(Debug, Clone, Default)]
pub struct RssDigestOptions {
    pub limit: usize,
    pub name: Option<String>,
    pub category: Option<String>,
    /// Absolute time-window lower bound (`--since`). Entries without a
    /// published time are dropped when set (timeline rss provider semantics).
    pub since: Option<DateTime<Utc>>,
    /// Force a live fetch (and cache refresh) instead of reading the cache.
    pub fresh: bool,
}

/// Receipt for a followed feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssFollowReceipt {
    pub name: String,
    pub url: String,
}

/// Receipt for an unfollowed feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssUnfollowReceipt {
    pub name: String,
}

/// Result of one fetch: `feed` on success, `error` on failure.
struct FetchedFeed {
    name: String,
    feed: Option<feed_rs::model::Feed>,
    error: Option<String>,
}

/// Lightweight projection of a feed entry suitable for the local item cache
/// (Phase 11). Avoids leaking `feed_rs::model::Entry` outside this module.
#[derive(Debug, Clone)]
pub struct EntryForCache {
    pub guid: String,
    pub title: String,
    pub summary: String,
    pub link: String,
    pub author: String,
    pub published: Option<chrono::DateTime<Utc>>,
}

impl EntryForCache {
    /// Project a `feed_rs::model::Entry` into the cache-friendly shape.
    pub fn from_entry(feed_name: &str, e: &feed_rs::model::Entry) -> Self {
        let title = e
            .title
            .as_ref()
            .map(|t| t.content.clone())
            .unwrap_or_default();
        let summary = e
            .summary
            .as_ref()
            .map(|s| crate::util::strings::truncate_chars(s.content.as_str(), 500).to_string())
            .unwrap_or_default();
        let link = pick_link(&e.links);
        let author = e
            .authors
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let guid = if !e.id.is_empty() {
            e.id.clone()
        } else {
            // Fall back to a feed_name-scoped hash of the link so the
            // natural key remains stable across re-fetches.
            format!("{feed_name}::{link}")
        };
        Self {
            guid,
            title,
            summary,
            link,
            author,
            published: e.published,
        }
    }
}

pub struct RssModule {
    config: RssModuleConfig,
}

impl RssModule {
    pub fn new(config: RssModuleConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for RssModule {
    fn description(&self) -> &'static str {
        "RSS/Atom feed reader: follow, list, unfollow, aggregate (digest), fetch."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!(
                "follow",
                "关注一个 RSS/Atom feed",
                "everyday rss follow --name N --url URL [--category C]",
                &[
                    flag!("name", "feed 名称"),
                    flag!("url", "feed URL（http/https）"),
                    flag!("category", "分类"),
                ]
            ),
            cli_action!("list", "列出已关注的 feed", "everyday rss list", &[]),
            cli_action!(
                "unfollow",
                "取消关注",
                "everyday rss unfollow --name N",
                &[flag!("name", "feed 名称")]
            ),
            cli_action!(
                "digest",
                "聚合阅读视图：跨源条目摘要（本地缓存优先，--fresh 强刷）",
                "everyday rss digest [--limit N] [--name FEED] [--category C] [--since 30m|7d|YYYY-MM-DD] [--fresh]",
                &[
                    flag!("limit", "条数上限"),
                    flag!("name", "按 feed 名过滤"),
                    flag!("category", "按分类过滤"),
                    flag!("since", "时间窗（30m/7d/日期），按 published 过滤"),
                    flag!("fresh", "强制实时抓取并刷新缓存", Bool),
                ]
            ),
            cli_action!(
                "fetch",
                "抓取并展示 feed 文章：--name N 订阅源（写缓存）或 <url> 直接调试（不写缓存）",
                "everyday rss fetch (--name N | <url>) [--limit N]",
                &[flag!("name", "feed 名称"), flag!("limit", "条数上限"),],
                Positional::OptionalSingle
            ),
        ];
        ModuleArgSpec {
            name: "rss",
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
        let (flags, positional) = parse_simple_args(args);
        // DI seam (P1, [F012](../../docs/adr/F012-architecture-deepening-phase.md)):
        // the backend owns config-file reads/writes + network; `dispatch` only maps
        // CLI args → service calls → Output. Tests inject a `MockRssBackend`.
        let backend = RealRssBackend::new(self.config.clone());
        dispatch(&backend, action, &flags, &positional).await
    }

    /// P3 health: the feeds config is loadable; no network probe.
    async fn health_check(&self) -> Result<crate::modules::HealthStatus> {
        use crate::modules::HealthStatus;
        match crate::modules::rss::load_config_value() {
            Ok(_) => Ok(HealthStatus::healthy()),
            Err(e) => Ok(HealthStatus::degraded(format!("config: {}", e.message()))),
        }
    }
}

/// RSS service trait: domain methods, no `Output` in sight (P1, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
#[async_trait]
pub trait RssBackend: Send + Sync {
    /// Follow a feed (writes to the config file).
    async fn follow(
        &self,
        name: &str,
        url: &str,
        category: Option<&str>,
    ) -> Result<RssFollowReceipt>;
    /// List all subscribed feeds.
    fn list(&self) -> Result<Vec<RssFeed>>;
    /// Unfollow a feed (removes from the config file).
    async fn unfollow(&self, name: &str) -> Result<RssUnfollowReceipt>;
    /// Fetch all matching feeds concurrently and aggregate entries (best-effort).
    /// Cache-first: reads the local item cache unless `--fresh`; falls back to
    /// a live fetch when the cache is unavailable, empty, or filters match none.
    async fn digest(&self, opts: &RssDigestOptions) -> Result<Vec<RssEntryRow>>;
    /// Fetch a single subscribed feed's entries (writes the item cache).
    async fn fetch(&self, name: &str, limit: usize) -> Result<Vec<RssEntryRow>>;
    /// Stateless debug fetch: grab any URL directly — no subscription-list
    /// lookup, no cache write (does not pollute rss-items.db or search).
    async fn fetch_url(&self, url: &str, limit: usize) -> Result<Vec<RssEntryRow>>;
}

/// Real backend: config-file reads/writes for follow/list/unfollow; reqwest + feed-rs for digest/fetch.
pub struct RealRssBackend {
    config: RssModuleConfig,
    /// Item-cache db path override (tests only); `None` = default `~/.config/everyday/rss-items.db`.
    cache_db_path: Option<std::path::PathBuf>,
}

impl RealRssBackend {
    pub fn new(config: RssModuleConfig) -> Self {
        Self {
            config,
            cache_db_path: None,
        }
    }

    /// Build a backend whose item cache points at an explicit db file
    /// (tests only — keeps unit tests off the real user config dir).
    #[cfg(test)]
    fn with_cache_db(config: RssModuleConfig, path: std::path::PathBuf) -> Self {
        Self {
            config,
            cache_db_path: Some(path),
        }
    }

    /// Open the item cache, honoring the test override when set.
    async fn open_cache(&self) -> Result<sqlx::SqlitePool> {
        match &self.cache_db_path {
            Some(p) => rss_items::open_at(p).await,
            None => rss_items::open().await,
        }
    }

    /// Read the digest rows from the local item cache (`None` when the cache
    /// is unavailable — missing/unreadable db, or the query fails).
    async fn read_cached_digest(
        &self,
        feed_names: &[&str],
        opts: &RssDigestOptions,
    ) -> Option<Vec<RssEntryRow>> {
        let pool = self.open_cache().await.ok()?;
        rss_items::query_items(&pool, feed_names, opts.since, opts.limit)
            .await
            .ok()
    }

    /// Live digest: concurrent fetch of all matching feeds + best-effort cache
    /// upsert. Filtered by `--since` (undated entries dropped under a window).
    async fn fetch_live_digest(
        &self,
        feeds: &[RssFeed],
        opts: &RssDigestOptions,
    ) -> Result<Vec<RssEntryRow>> {
        let client = build_client()?;
        let tasks: Vec<_> = feeds.iter().map(|f| fetch_one(&client, f)).collect();
        let results = join_all(tasks).await;

        let mut rows: Vec<RssEntryRow> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        // Phase 11: cache writes piggy-back on the same fetch. Best-effort: a
        // cache failure must not block the user's digest output [S005].
        let cache_pool = self.open_cache().await.ok();
        for r in results {
            match r.feed {
                Some(f) => {
                    // Project entries into the cache-friendly shape and write.
                    if let Some(pool) = &cache_pool {
                        let entries: Vec<EntryForCache> = f
                            .entries
                            .iter()
                            .map(|e| EntryForCache::from_entry(&r.name, e))
                            .collect();
                        // Find the matching RssFeed for url metadata.
                        if let Some(feed) = self.config.feeds.iter().find(|x| x.name == r.name) {
                            let _ = rss_items::upsert_items(pool, feed, &entries).await;
                        }
                    }
                    for e in &f.entries {
                        rows.push(build_entry_row(&r.name, e));
                    }
                }
                None => errors.push(format!("{}: {}", r.name, r.error.unwrap_or_default())),
            }
        }

        // All failed -> error; partially failed -> still output the successful part (best-effort).
        if rows.is_empty() && !errors.is_empty() {
            return Err(AgentError::Network(errors.join("; ")));
        }

        rows.sort_by(|a, b| cmp_opt_dt_desc(&a.sort_key, &b.sort_key));
        // `--since`: keep only entries published at/after the window; undated
        // entries are dropped (timeline rss provider window semantics).
        if let Some(since) = opts.since {
            rows.retain(|r| r.sort_key.is_some_and(|dt| dt >= since));
        }
        rows.truncate(opts.limit);
        Ok(rows)
    }
}

/// CLI dispatch: parse flags → call the backend service method → render to `Output`.
/// The only place in the rss module that touches `Output` for actions.
async fn dispatch(
    backend: &dyn RssBackend,
    action: &str,
    flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    match action {
        "follow" => {
            let name = flags.get("name").ok_or_else(|| {
                AgentError::InvalidArgument("follow requires --name <name>".into())
            })?;
            let url = flags
                .get("url")
                .ok_or_else(|| AgentError::InvalidArgument("follow requires --url <url>".into()))?;
            let receipt = backend
                .follow(name, url, flags.get("category").map(|s| s.as_str()))
                .await?;
            Ok(Output::text(format!(
                "followed feed '{}' ({})",
                receipt.name, receipt.url
            )))
        }
        "list" => {
            let feeds = backend.list()?;
            Ok(render_list(feeds))
        }
        "unfollow" => {
            let name = flags.get("name").ok_or_else(|| {
                AgentError::InvalidArgument("unfollow requires --name <name>".into())
            })?;
            let receipt = backend.unfollow(name).await?;
            Ok(Output::text(format!("unfollowed feed '{}'", receipt.name)))
        }
        "digest" => {
            let opts = RssDigestOptions {
                limit: flags
                    .get("limit")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(30),
                name: flags.get("name").cloned(),
                category: flags.get("category").cloned(),
                since: match flags.get("since") {
                    Some(s) => Some(crate::util::datetime::parse_since(s)?),
                    None => None,
                },
                fresh: flags.get("fresh").map(|v| v == "true").unwrap_or(false),
            };
            let rows = backend.digest(&opts).await?;
            Ok(render_digest(rows))
        }
        "fetch" => {
            let limit = flags
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(20);
            let name = flags.get("name");
            let url = positional.first();
            let rows = match (name, url) {
                (Some(n), None) => backend.fetch(n, limit).await?,
                (None, Some(u)) => backend.fetch_url(u, limit).await?,
                (Some(_), Some(_)) => {
                    return Err(AgentError::InvalidArgument(
                        "fetch accepts exactly one of --name <name> or a feed <url>".into(),
                    ));
                }
                (None, None) => {
                    return Err(AgentError::InvalidArgument(
                        "fetch requires --name <name> or a feed <url> (stateless debug)".into(),
                    ));
                }
            };
            Ok(render_fetch(rows))
        }
        other => Err(AgentError::UnknownAction(format!("rss {other}"))),
    }
}

/// `rss list` rows: name / URL / category.
fn render_list(feeds: Vec<RssFeed>) -> Output {
    let rows = feeds
        .into_iter()
        .map(|f| {
            vec![
                f.name.clone(),
                f.url.clone(),
                f.category.clone().unwrap_or_default(),
            ]
        })
        .collect();
    Output::records(vec!["name".into(), "url".into(), "category".into()], rows)
}

/// `rss digest` rows: feed / title / summary / published / author / link.
/// The summary cell is truncated to ~80 chars in text mode (table readability)
/// but carries the full ~200 chars in JSON (downstream agent input).
fn render_digest(rows: Vec<RssEntryRow>) -> Output {
    let out_rows = rows
        .into_iter()
        .map(|r| {
            vec![
                crate::output::TypedValue::text(r.feed),
                crate::output::TypedValue::text(r.title),
                crate::output::TypedValue::truncated_text(r.summary, 80),
                crate::output::TypedValue::text(r.published),
                crate::output::TypedValue::text(r.author),
                crate::output::TypedValue::text(r.link),
            ]
        })
        .collect();
    Output::typed_records(
        vec![
            "feed".into(),
            "title".into(),
            "summary".into(),
            "published".into(),
            "author".into(),
            "link".into(),
        ],
        out_rows,
    )
}

/// `rss fetch` rows: title / published / author / link.
fn render_fetch(rows: Vec<RssEntryRow>) -> Output {
    let out_rows = rows
        .into_iter()
        .map(|r| vec![r.title, r.published, r.author, r.link])
        .collect();
    Output::records(
        vec![
            "title".into(),
            "published".into(),
            "author".into(),
            "link".into(),
        ],
        out_rows,
    )
}

#[async_trait]
impl RssBackend for RealRssBackend {
    async fn follow(
        &self,
        name: &str,
        url: &str,
        category: Option<&str>,
    ) -> Result<RssFollowReceipt> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(AgentError::InvalidArgument(format!(
                "invalid feed url (must start with http:// or https://): {url}"
            )));
        }
        let feed = RssFeed {
            name: name.to_string(),
            url: url.to_string(),
            category: category.map(|s| s.to_string()),
        };

        let mut root = load_config_value()?;
        append_feed(&mut root, &feed)?;
        save_config_value(&root)?;
        Ok(RssFollowReceipt {
            name: feed.name,
            url: feed.url,
        })
    }

    fn list(&self) -> Result<Vec<RssFeed>> {
        Ok(self.config.feeds.clone())
    }

    async fn unfollow(&self, name: &str) -> Result<RssUnfollowReceipt> {
        let mut root = load_config_value()?;
        let removed = remove_feed(&mut root, name)?;
        if !removed {
            return Err(AgentError::InvalidArgument(format!(
                "feed '{name}' not found"
            )));
        }
        save_config_value(&root)?;
        Ok(RssUnfollowReceipt {
            name: name.to_string(),
        })
    }

    async fn digest(&self, opts: &RssDigestOptions) -> Result<Vec<RssEntryRow>> {
        let feeds = filter_feeds(&self.config.feeds, opts)?;
        if feeds.is_empty() {
            return Err(AgentError::InvalidArgument(
                "no feeds to fetch (add one with `everyday rss follow --name N --url URL`)".into(),
            ));
        }

        // Cache-first (spec): default reads the local item cache — zero network
        // (L005 spirit); `--fresh` forces a live fetch. Falls back to live when
        // the cache is unavailable, empty, or the filters match nothing, so any
        // situation still yields output.
        let feed_names: Vec<&str> = feeds.iter().map(|f| f.name.as_str()).collect();
        let cached = if opts.fresh {
            None
        } else {
            self.read_cached_digest(&feed_names, opts).await
        };
        if let Some(rows) = digest_cache_source(cached, opts.fresh) {
            return Ok(rows);
        }
        self.fetch_live_digest(&feeds, opts).await
    }

    async fn fetch(&self, name: &str, limit: usize) -> Result<Vec<RssEntryRow>> {
        let feed = self
            .config
            .feeds
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| AgentError::InvalidArgument(format!("feed '{name}' not found")))?;

        let client = build_client()?;
        let res = fetch_one(&client, feed).await;
        let f = res.feed.ok_or_else(|| {
            AgentError::Network(res.error.unwrap_or_else(|| "fetch failed".into()))
        })?;

        // Phase 11: also write to the local cache (best-effort).
        if let Ok(pool) = self.open_cache().await {
            let entries: Vec<EntryForCache> = f
                .entries
                .iter()
                .map(|e| EntryForCache::from_entry(&feed.name, e))
                .collect();
            let _ = rss_items::upsert_items(&pool, feed, &entries).await;
        }

        let mut rows: Vec<RssEntryRow> = f
            .entries
            .iter()
            .map(|e| build_entry_row(&feed.name, e))
            .collect();
        rows.sort_by(|a, b| cmp_opt_dt_desc(&a.sort_key, &b.sort_key));
        rows.truncate(limit);
        Ok(rows)
    }

    async fn fetch_url(&self, url: &str, limit: usize) -> Result<Vec<RssEntryRow>> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(AgentError::InvalidArgument(format!(
                "invalid feed url (must start with http:// or https://): {url}"
            )));
        }
        let probe = RssFeed {
            name: url.to_string(),
            url: url.to_string(),
            category: None,
        };
        let client = build_client()?;
        let res = fetch_one(&client, &probe).await;
        let f = res.feed.ok_or_else(|| {
            AgentError::Network(res.error.unwrap_or_else(|| "fetch failed".into()))
        })?;

        // Stateless: no subscription-list lookup, no cache write — debugging an
        // arbitrary source must not pollute rss-items.db or cross-module search.
        let mut rows: Vec<RssEntryRow> =
            f.entries.iter().map(|e| build_entry_row(url, e)).collect();
        rows.sort_by(|a, b| cmp_opt_dt_desc(&a.sort_key, &b.sort_key));
        rows.truncate(limit);
        Ok(rows)
    }
}

/// Cache-first source decision for `digest` (the fallback rule):
/// - `--fresh` → always live;
/// - cache unavailable (`None`) or empty → live (表空/过滤无结果 → 回退实时);
/// - cache has rows → read the cache directly.
fn digest_cache_source(cached: Option<Vec<RssEntryRow>>, fresh: bool) -> Option<Vec<RssEntryRow>> {
    if fresh {
        return None;
    }
    cached.filter(|rows| !rows.is_empty())
}

// ============ Config read/write (localized edit of rss.feeds) ============

/// Read the config file into a toml::Value (empty table if absent/empty).
fn load_config_value() -> Result<toml::Value> {
    let path = Config::config_path()?;
    if !path.exists() {
        return Ok(toml::Value::Table(toml::value::Table::new()));
    }
    let text = std::fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        return Ok(toml::Value::Table(toml::value::Table::new()));
    }
    Ok(toml::from_str(&text)?)
}

/// Write a toml::Value back to the config file (creating the parent dir).
fn save_config_value(root: &toml::Value) -> Result<()> {
    let path = Config::config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(root)
        .map_err(|e| AgentError::Config(format!("serialize config: {e}")))?;
    std::fs::write(&path, text)?;
    Ok(())
}

/// Append a feed to `rss.feeds` (creating the table/array if absent).
/// Error if a feed with the same name already exists, to avoid duplicates.
fn append_feed(root: &mut toml::Value, feed: &RssFeed) -> Result<()> {
    let table = root
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("config root is not a table".into()))?;
    let rss = table
        .entry("rss")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    let rss_table = rss
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("rss is not a table".into()))?;
    let feeds = rss_table
        .entry("feeds")
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    let arr = feeds
        .as_array_mut()
        .ok_or_else(|| AgentError::Config("rss.feeds is not an array".into()))?;

    if arr
        .iter()
        .any(|f| f.get("name").and_then(|n| n.as_str()) == Some(&feed.name))
    {
        return Err(AgentError::InvalidArgument(format!(
            "feed '{}' already exists",
            feed.name
        )));
    }

    let mut entry = toml::value::Table::new();
    entry.insert("name".into(), toml::Value::String(feed.name.clone()));
    entry.insert("url".into(), toml::Value::String(feed.url.clone()));
    if let Some(cat) = &feed.category {
        entry.insert("category".into(), toml::Value::String(cat.clone()));
    }
    arr.push(toml::Value::Table(entry));
    Ok(())
}

/// Remove the feed with the given name from `rss.feeds`; returns whether anything was removed.
fn remove_feed(root: &mut toml::Value, name: &str) -> Result<bool> {
    let Some(rss) = root.as_table_mut().and_then(|t| t.get_mut("rss")) else {
        return Ok(false);
    };
    let Some(feeds) = rss.as_table_mut().and_then(|t| t.get_mut("feeds")) else {
        return Ok(false);
    };
    let Some(arr) = feeds.as_array_mut() else {
        return Ok(false);
    };
    let before = arr.len();
    arr.retain(|f| f.get("name").and_then(|n| n.as_str()) != Some(name));
    Ok(arr.len() < before)
}

/// Filter feeds: `--name` and `--category` match exactly (case-sensitive).
///
/// If `--name` is given but no feed matches, return `InvalidArgument` (not found).
fn filter_feeds(feeds: &[RssFeed], opts: &RssDigestOptions) -> Result<Vec<RssFeed>> {
    let name = opts.name.as_deref();
    let category = opts.category.as_deref();
    let mut out = Vec::new();
    for f in feeds {
        if let Some(n) = name
            && f.name != n
        {
            continue;
        }
        if let Some(c) = category {
            match &f.category {
                Some(fc) if fc == c => {}
                _ => continue,
            }
        }
        out.push(f.clone());
    }
    if let Some(n) = name
        && out.is_empty()
    {
        return Err(AgentError::InvalidArgument(format!("feed '{n}' not found")));
    }
    Ok(out)
}

// ============ Network fetch ============

/// Build a reqwest client with timeout and UA (rustls-tls, reusing the ring provider installed by main.rs).
fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("everyday/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AgentError::Network(format!("build http client: {e}")))
}

/// Fetch a single feed and parse it into a Feed (returns an error on failure, never panics).
async fn fetch_one(client: &reqwest::Client, feed: &RssFeed) -> FetchedFeed {
    match client.get(&feed.url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return FetchedFeed {
                    name: feed.name.clone(),
                    feed: None,
                    error: Some(format!("HTTP {}", resp.status())),
                };
            }
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    return FetchedFeed {
                        name: feed.name.clone(),
                        feed: None,
                        error: Some(format!("read body: {e}")),
                    };
                }
            };
            match feed_rs::parser::parse(bytes.as_ref()) {
                Ok(f) => FetchedFeed {
                    name: feed.name.clone(),
                    feed: Some(f),
                    error: None,
                },
                Err(e) => FetchedFeed {
                    name: feed.name.clone(),
                    feed: None,
                    error: Some(format!("parse: {e}")),
                },
            }
        }
        Err(e) => FetchedFeed {
            name: feed.name.clone(),
            feed: None,
            error: Some(e.to_string()),
        },
    }
}

// ============ Entry row construction ============

/// Feed summary直出, truncated to ~200 chars — the digest row / JSON contract
/// (the item cache stores up to 500; timeline uses 200 too, so the data口径
/// matches across digest / timeline / search). No ellipsis: JSON is consumed
/// by downstream agents, and text mode appends one at render time.
pub fn summary_for_row(content: &str) -> String {
    crate::util::strings::truncate_chars(content, 200).to_string()
}

/// Build a display row from a feed-rs Entry.
fn build_entry_row(feed_name: &str, entry: &feed_rs::model::Entry) -> RssEntryRow {
    let title = entry
        .title
        .as_ref()
        .map(|t| t.content.clone())
        .unwrap_or_default();
    let summary = entry
        .summary
        .as_ref()
        .map(|s| summary_for_row(s.content.as_str()))
        .unwrap_or_default();
    let published = entry.published;
    let author = entry
        .authors
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let link = pick_link(&entry.links);
    let published_str = published
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "—".into());
    RssEntryRow {
        feed: feed_name.to_string(),
        title,
        summary,
        published: published_str,
        author,
        link,
        sort_key: published,
    }
}

/// Pick the display link: prefer `rel="alternate"`, else the first one; empty string if none.
fn pick_link(links: &[feed_rs::model::Link]) -> String {
    links
        .iter()
        .find(|l| l.rel.as_deref() == Some("alternate"))
        .or_else(|| links.first())
        .map(|l| l.href.clone())
        .unwrap_or_default()
}

/// Compare by publish time, descending (entries without a time sort last).
fn cmp_opt_dt_desc(a: &Option<DateTime<Utc>>, b: &Option<DateTime<Utc>>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(x), Some(y)) => y.cmp(x), // descending: newest first
        (Some(_), None) => std::cmp::Ordering::Less, // dated entries sort before undated
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

// ============ Timeline data ingestion ============

/// Timeline ingestion: raw RSS entry data.
pub struct RssTimelineEntry {
    pub feed_name: String,
    pub feed_url: String,
    pub title: String,
    pub summary: String,
    pub link: String,
    pub author: String,
    pub published: Option<DateTime<Utc>>,
    pub guid: String,
}

/// Timeline incremental fetch: fetch all feeds, return entries whose publish time falls in the window.
pub async fn fetch_for_timeline(
    config: &Config,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<RssTimelineEntry>> {
    if config.rss.feeds.is_empty() {
        return Ok(Vec::new());
    }
    let client = build_client()?;
    let tasks: Vec<_> = config
        .rss
        .feeds
        .iter()
        .map(|f| fetch_one(&client, f))
        .collect();
    let results = join_all(tasks).await;

    let mut entries = Vec::new();
    for r in &results {
        if let Some(f) = &r.feed {
            for e in &f.entries {
                let published = e.published;
                // Filter: keep entries whose publish time is within the window (skip those without one).
                if let Some(pub_dt) = published
                    && (pub_dt < from || pub_dt > to)
                {
                    continue;
                }
                let title = e
                    .title
                    .as_ref()
                    .map(|t| t.content.clone())
                    .unwrap_or_default();
                let summary = e
                    .summary
                    .as_ref()
                    .map(|s| {
                        let content = s.content.as_str();
                        if content.chars().count() > 200 {
                            format!("{}...", crate::util::strings::truncate_chars(content, 200))
                        } else {
                            content.to_string()
                        }
                    })
                    .unwrap_or_default();
                let author = e
                    .authors
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let link = pick_link(&e.links);
                let guid = if !e.id.is_empty() {
                    e.id.clone()
                } else if !link.is_empty() {
                    link.clone()
                } else {
                    String::new()
                };
                let feed_url = config
                    .rss
                    .feeds
                    .iter()
                    .find(|f| f.name == r.name)
                    .map(|f| f.url.clone())
                    .unwrap_or_default();
                entries.push(RssTimelineEntry {
                    feed_name: r.name.clone(),
                    feed_url,
                    title,
                    summary,
                    link,
                    author,
                    published,
                    guid,
                });
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // ---- P1 service layer (F012) ----

    /// In-memory backend driving `dispatch` without config-file writes or network.
    #[derive(Default)]
    struct MockRssBackend {
        feeds: Vec<RssFeed>,
    }

    #[async_trait]
    impl RssBackend for MockRssBackend {
        async fn follow(
            &self,
            name: &str,
            url: &str,
            _cat: Option<&str>,
        ) -> Result<RssFollowReceipt> {
            Ok(RssFollowReceipt {
                name: name.into(),
                url: url.into(),
            })
        }
        fn list(&self) -> Result<Vec<RssFeed>> {
            Ok(self.feeds.clone())
        }
        async fn unfollow(&self, name: &str) -> Result<RssUnfollowReceipt> {
            Ok(RssUnfollowReceipt { name: name.into() })
        }
        async fn digest(&self, _opts: &RssDigestOptions) -> Result<Vec<RssEntryRow>> {
            Ok(vec![RssEntryRow {
                feed: "a".into(),
                title: "t".into(),
                summary: "s".into(),
                published: "2026-08-05".into(),
                author: "author".into(),
                link: "https://x".into(),
                sort_key: None,
            }])
        }
        async fn fetch(&self, name: &str, _limit: usize) -> Result<Vec<RssEntryRow>> {
            Err(AgentError::InvalidArgument(format!(
                "feed '{name}' not found"
            )))
        }
        async fn fetch_url(&self, url: &str, _limit: usize) -> Result<Vec<RssEntryRow>> {
            Ok(vec![RssEntryRow {
                feed: url.into(),
                title: "t".into(),
                summary: "s".into(),
                published: "2026-08-05".into(),
                author: "author".into(),
                link: "https://x".into(),
                sort_key: None,
            }])
        }
    }

    #[tokio::test]
    async fn dispatch_list_uses_mock_backend_without_config_or_network() {
        let mock = MockRssBackend {
            feeds: vec![RssFeed {
                name: "a".into(),
                url: "u1".into(),
                category: Some("tech".into()),
            }],
        };
        let out = dispatch(&mock, "list", &HashMap::new(), &[]).await.unwrap();
        match out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["name", "url", "category"]);
                assert_eq!(rows, vec![vec!["a", "u1", "tech"]]);
            }
            other => panic!("expected Records, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_follow_validates_and_renders() {
        let mock = MockRssBackend::default();
        // Missing --name -> InvalidArgument before the backend is called.
        let err = dispatch(&mock, "follow", &HashMap::new(), &[])
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        // Invalid url scheme is validated by RealRssBackend, not dispatch; the mock
        // returns a receipt directly, so dispatch renders the confirmation text.
        let mut flags = HashMap::new();
        flags.insert("name".into(), "hn".into());
        flags.insert("url".into(), "https://hnrss.org".into());
        let out = dispatch(&mock, "follow", &flags, &[]).await.unwrap();
        assert_eq!(
            out.render(crate::output::RenderMode::Text),
            "followed feed 'hn' (https://hnrss.org)"
        );
    }

    #[tokio::test]
    async fn dispatch_digest_renders_rows() {
        let mock = MockRssBackend::default();
        let out = dispatch(&mock, "digest", &HashMap::new(), &[])
            .await
            .unwrap();
        match out {
            Output::TypedRecords { headers, rows } => {
                assert_eq!(
                    headers,
                    vec!["feed", "title", "summary", "published", "author", "link"]
                );
                let str_rows: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| r.iter().map(|c| c.as_display()).collect())
                    .collect();
                assert_eq!(
                    str_rows,
                    vec![vec!["a", "t", "s", "2026-08-05", "author", "https://x"]]
                );
            }
            other => panic!("expected TypedRecords, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_digest_since_parses_and_renders() {
        let mock = MockRssBackend::default();
        // Relative duration + date forms both parse without error (the mock
        // ignores opts; dispatch just validates and renders).
        for since in ["24h", "7d", "2026-08-01"] {
            let mut flags = HashMap::new();
            flags.insert("since".into(), since.into());
            let out = dispatch(&mock, "digest", &flags, &[]).await.unwrap();
            assert!(matches!(out, Output::TypedRecords { .. }));
        }
        // Invalid duration -> InvalidArgument, surfaced before the backend.
        let mut flags = HashMap::new();
        flags.insert("since".into(), "bogus".into());
        let err = dispatch(&mock, "digest", &flags, &[]).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn dispatch_digest_fresh_flag_parses() {
        let mock = MockRssBackend::default();
        let mut flags = HashMap::new();
        flags.insert("fresh".into(), "true".into());
        let out = dispatch(&mock, "digest", &flags, &[]).await.unwrap();
        assert!(matches!(out, Output::TypedRecords { .. }));
    }

    #[tokio::test]
    async fn dispatch_fetch_requires_name_or_url() {
        let mock = MockRssBackend::default();
        // Neither --name nor <url> -> InvalidArgument.
        let err = dispatch(&mock, "fetch", &HashMap::new(), &[])
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn dispatch_fetch_url_positional_renders_rows() {
        let mock = MockRssBackend::default();
        let out = dispatch(
            &mock,
            "fetch",
            &HashMap::new(),
            &["https://example.com/feed.xml".into()],
        )
        .await
        .unwrap();
        match out {
            Output::Records { headers, rows } => {
                // fetch columns unchanged: no feed, no summary.
                assert_eq!(headers, vec!["title", "published", "author", "link"]);
                assert_eq!(rows, vec![vec!["t", "2026-08-05", "author", "https://x"]]);
            }
            other => panic!("expected Records, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_fetch_rejects_both_name_and_url() {
        let mock = MockRssBackend::default();
        let mut flags = HashMap::new();
        flags.insert("name".into(), "hn".into());
        let err = dispatch(
            &mock,
            "fetch",
            &flags,
            &["https://example.com/feed.xml".into()],
        )
        .await
        .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[test]
    fn digest_cache_source_fallback_rules() {
        let row = RssEntryRow {
            feed: "a".into(),
            title: "t".into(),
            summary: "s".into(),
            published: "2026-08-05".into(),
            author: "".into(),
            link: "https://x".into(),
            sort_key: None,
        };
        // --fresh forces live even when the cache has rows.
        assert!(digest_cache_source(Some(vec![row.clone()]), true).is_none());
        // Cache unavailable (None) -> live.
        assert!(digest_cache_source(None, false).is_none());
        // Cache empty (表空) -> live.
        assert!(digest_cache_source(Some(Vec::new()), false).is_none());
        // Non-empty cache hit -> read the cache directly.
        assert_eq!(
            digest_cache_source(Some(vec![row.clone()]), false).map(|r| r.len()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn real_backend_digest_serves_cache_without_network() {
        // Seed a temp item cache, then run digest against it: the cache-first
        // path must return rows with zero network (feed url never contacted).
        let file = std::env::temp_dir().join(format!(
            "everyday-rss-digest-test-{}.db",
            crate::util::id::gen_id("rd")
        ));
        let pool = rss_items::open_at(&file).await.unwrap();
        let feed = RssFeed {
            name: "hn".into(),
            url: "https://example.invalid/feed".into(),
            category: Some("tech".into()),
        };
        let t = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        rss_items::upsert_items(
            &pool,
            &feed,
            &[EntryForCache {
                guid: "g1".into(),
                title: "cached title".into(),
                summary: "cached summary".into(),
                link: "https://x/1".into(),
                author: "bob".into(),
                published: Some(t),
            }],
        )
        .await
        .unwrap();
        pool.close().await;

        let config = RssModuleConfig { feeds: vec![feed] };
        let backend = RealRssBackend::with_cache_db(config, file.clone());
        let rows = backend
            .digest(&RssDigestOptions {
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].feed, "hn");
        assert_eq!(rows[0].title, "cached title");
        assert_eq!(rows[0].summary, "cached summary");
        assert!(rows[0].published.contains("2026-07-09"));

        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn real_backend_digest_category_resolves_to_feed_names_in_cache() {
        // `--category` under the cache path resolves to a feed-name set from
        // config, then filters the cache by those names.
        let file = std::env::temp_dir().join(format!(
            "everyday-rss-digest-test-{}.db",
            crate::util::id::gen_id("rc")
        ));
        let pool = rss_items::open_at(&file).await.unwrap();
        let tech = RssFeed {
            name: "tech-feed".into(),
            url: "https://example.invalid/tech".into(),
            category: Some("tech".into()),
        };
        let other = RssFeed {
            name: "other-feed".into(),
            url: "https://example.invalid/other".into(),
            category: Some("news".into()),
        };
        let t = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        rss_items::upsert_items(
            &pool,
            &tech,
            &[EntryForCache {
                guid: "t1".into(),
                title: "tech item".into(),
                summary: "s".into(),
                link: "https://x/1".into(),
                author: "".into(),
                published: Some(t),
            }],
        )
        .await
        .unwrap();
        rss_items::upsert_items(
            &pool,
            &other,
            &[EntryForCache {
                guid: "o1".into(),
                title: "other item".into(),
                summary: "s".into(),
                link: "https://x/2".into(),
                author: "".into(),
                published: Some(t),
            }],
        )
        .await
        .unwrap();
        pool.close().await;

        let config = RssModuleConfig {
            feeds: vec![tech, other],
        };
        let backend = RealRssBackend::with_cache_db(config, file.clone());
        let rows = backend
            .digest(&RssDigestOptions {
                category: Some("tech".into()),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "tech item");

        let _ = std::fs::remove_file(file);
    }

    /// Minimal Atom sample used to parse into an Entry (author/link semantics are clearer than RSS2).
    const SAMPLE_RSS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Test Feed</title>
  <entry>
    <title>Hello World</title>
    <summary>a short summary</summary>
    <link href="http://example.com/1"/>
    <published>2026-07-09T14:00:00Z</published>
    <updated>2026-07-09T14:00:00Z</updated>
    <author><name>Bob</name></author>
  </entry>
  <entry>
    <title>Second</title>
    <link rel="self" href="http://example.com/2"/>
  </entry>
</feed>"#;

    #[test]
    fn append_and_remove_feed_on_value() {
        let mut root = toml::Value::Table(toml::value::Table::new());
        // Seed a mail account to verify the localized edit does not corrupt it.
        // Note: toml::Value indexing does not auto-insert; insert must be used explicitly.
        root.as_table_mut().unwrap().insert(
            "mail".into(),
            toml::from_str("accounts = [{ name = 'work' }]").unwrap(),
        );

        let feed = RssFeed {
            name: "hn".into(),
            url: "https://hnrss.org/frontpage".into(),
            category: Some("tech".into()),
        };
        append_feed(&mut root, &feed).unwrap();
        // Adding a duplicate should error.
        assert!(append_feed(&mut root, &feed).is_err());

        // rss.feeds now contains one feed.
        let arr = root
            .get("rss")
            .and_then(|r| r.get("feeds"))
            .and_then(|f| f.as_array())
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").unwrap().as_str(), Some("hn"));
        assert_eq!(arr[0].get("category").unwrap().as_str(), Some("tech"));

        // The mail account was not corrupted.
        assert_eq!(
            root.get("mail")
                .and_then(|m| m.get("accounts"))
                .and_then(|a| a.as_array())
                .unwrap()
                .len(),
            1
        );

        // Remove.
        assert!(remove_feed(&mut root, "hn").unwrap());
        assert!(!remove_feed(&mut root, "hn").unwrap()); // a second remove returns false
    }

    #[test]
    fn filter_feeds_by_name_and_category() {
        let feeds = vec![
            RssFeed {
                name: "a".into(),
                url: "u1".into(),
                category: Some("tech".into()),
            },
            RssFeed {
                name: "b".into(),
                url: "u2".into(),
                category: None,
            },
            RssFeed {
                name: "c".into(),
                url: "u3".into(),
                category: Some("tech".into()),
            },
        ];
        // By category tech.
        let opts = RssDigestOptions {
            category: Some("tech".into()),
            ..Default::default()
        };
        let f = filter_feeds(&feeds, &opts).unwrap();
        assert_eq!(f.len(), 2);
        // Exact match by name.
        let opts = RssDigestOptions {
            name: Some("b".into()),
            ..Default::default()
        };
        let f = filter_feeds(&feeds, &opts).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "b");
        // Name not found -> error.
        let opts = RssDigestOptions {
            name: Some("z".into()),
            ..Default::default()
        };
        assert!(filter_feeds(&feeds, &opts).is_err());
    }

    #[test]
    fn cmp_opt_dt_desc_sorts_correctly() {
        let t1 = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 8, 14, 0, 0).unwrap();
        // The newer (t1) should sort before the undated.
        assert_eq!(cmp_opt_dt_desc(&Some(t1), &None), std::cmp::Ordering::Less);
        // Descending: t1 (new) should precede t2 (old) -> t1.cmp(t2) yields Less.
        assert_eq!(
            cmp_opt_dt_desc(&Some(t1), &Some(t2)),
            std::cmp::Ordering::Less
        );
        // Two undated entries are equal.
        assert_eq!(cmp_opt_dt_desc(&None, &None), std::cmp::Ordering::Equal);
    }

    #[test]
    fn pick_link_prefers_alternate() {
        use feed_rs::model::Link;
        let links = vec![
            Link {
                href: "http://x/self".into(),
                rel: Some("self".into()),
                media_type: None,
                href_lang: None,
                title: None,
                length: None,
            },
            Link {
                href: "http://x/alt".into(),
                rel: Some("alternate".into()),
                media_type: None,
                href_lang: None,
                title: None,
                length: None,
            },
        ];
        assert_eq!(pick_link(&links), "http://x/alt");
        // When no alternate, take the first.
        let links = vec![Link {
            href: "http://x/only".into(),
            rel: Some("self".into()),
            media_type: None,
            href_lang: None,
            title: None,
            length: None,
        }];
        assert_eq!(pick_link(&links), "http://x/only");
        assert_eq!(pick_link(&[]), "");
    }

    #[test]
    fn rss_list_renders_rows() {
        let feeds = vec![
            RssFeed {
                name: "a".into(),
                url: "u1".into(),
                category: None,
            },
            RssFeed {
                name: "b".into(),
                url: "u2".into(),
                category: Some("cat".into()),
            },
        ];
        let out = render_list(feeds);
        if let Output::Records { headers, rows } = out {
            assert_eq!(headers, vec!["name", "url", "category"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0][0], "a");
            assert_eq!(rows[1][2], "cat");
        } else {
            panic!("expected Records output");
        }
    }

    #[test]
    fn build_entry_row_from_parsed_rss() {
        let feed = feed_rs::parser::parse(SAMPLE_RSS.as_bytes()).expect("parse sample rss");
        assert_eq!(feed.entries.len(), 2);

        // First entry: has title/summary/link/author/time.
        let row = build_entry_row("test", &feed.entries[0]);
        assert_eq!(row.title, "Hello World");
        assert_eq!(row.summary, "a short summary");
        assert_eq!(row.link, "http://example.com/1"); // Atom link defaults to rel=alternate
        assert_eq!(row.author, "Bob");
        assert!(row.published.contains("2026"));
        assert!(row.sort_key.is_some());

        // Second entry: no summary/author/no time -> empty summary, fallbacks, placeholder.
        let row = build_entry_row("test", &feed.entries[1]);
        assert_eq!(row.title, "Second");
        assert_eq!(row.summary, "");
        assert_eq!(row.link, "http://example.com/2"); // when no alternate, take the first link
        assert_eq!(row.author, "");
        assert!(row.sort_key.is_none());
        assert_eq!(row.published, "—");
    }

    #[test]
    fn summary_for_row_truncates_to_200_chars() {
        // Short summaries pass through verbatim (no ellipsis — JSON contract).
        assert_eq!(summary_for_row("hello"), "hello");
        assert_eq!(summary_for_row(""), "");
        // Long summaries are cut at 200 chars, keeping whole chars (CJK-safe).
        let long = "测".repeat(300);
        let t = summary_for_row(&long);
        assert_eq!(t.chars().count(), 200);
        assert!(t.chars().all(|c| c == '测'));
        // Exactly 200 chars is unchanged.
        let exact = "x".repeat(200);
        assert_eq!(summary_for_row(&exact).chars().count(), 200);
    }

    #[test]
    fn render_digest_truncates_summary_text_mode_only() {
        let row = RssEntryRow {
            feed: "a".into(),
            title: "t".into(),
            summary: "x".repeat(85),
            published: "2026-08-05".into(),
            author: "author".into(),
            link: "https://x".into(),
            sort_key: None,
        };
        // Text mode: the summary cell is truncated to 80 chars + ellipsis.
        let out = render_digest(vec![row.clone()]);
        let text = out.clone().render(crate::output::RenderMode::Text);
        assert!(text.contains(&format!("{}…", "x".repeat(80))));
        // JSON mode: the full ~200-char summary is carried verbatim.
        let json = out.render(crate::output::RenderMode::Json);
        assert!(json.contains(&"x".repeat(85)));
    }
}
