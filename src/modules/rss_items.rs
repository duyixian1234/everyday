//! RSS local item cache + cross-module search (Phase 11).
//!
//! Per ADR [S005](../../docs/adr/S005-time-semantics-scope.md), RSS gains a
//! local SQLite item cache so search is **offline-capable** (consistent with
//! the other local modules). Live-fetch-on-search was rejected (slow and
//! rate-limit prone).
//!
//! Data model:
//! - `rss_items(guid PRIMARY KEY, feed_name, feed_url, title, summary, link,
//!   author, published, fetched_at)`: one row per fetched entry, keyed by
//!   its natural id (`<entry id>` or fallback to the entry link).
//!
//! Lifecycle:
//! - `digest` / `fetch` insert rows after a successful fetch
//!   ([`upsert_items`]); writes are best-effort (cache failures do not
//!   surface to the user).
//! - [`RssSearchProvider`] queries this cache via GLOB on title / summary.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::{Executor, Row};

use crate::config::Config;
use crate::config::RssFeed;
use crate::error::{AgentError, Result};
use crate::modules::rss;
use crate::search::{Hit, SearchQuery, Searchable};

/// Per-module hard cap, enforced inside the provider
/// ([S004](../../docs/adr/S004-execution-model.md)).
const SEARCH_PER_MODULE_CAP: usize = 50;

const CREATE_RSS_ITEMS_SQL: &str = "CREATE TABLE IF NOT EXISTS rss_items (\
    guid TEXT PRIMARY KEY, \
    feed_name TEXT NOT NULL, \
    feed_url TEXT NOT NULL DEFAULT '', \
    title TEXT NOT NULL DEFAULT '', \
    summary TEXT NOT NULL DEFAULT '', \
    link TEXT NOT NULL DEFAULT '', \
    author TEXT NOT NULL DEFAULT '', \
    published TEXT, \
    fetched_at TEXT NOT NULL)";

const IX_RSS_ITEMS_PUB_SQL: &str = "CREATE INDEX IF NOT EXISTS ix_rss_items_pub \
    ON rss_items(published)";

/// Resolve the RSS items db path: `~/.config/everyday/rss-items.db`.
fn rss_items_db_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("rss-items.db"))
}

/// Open the RSS items db (creating if needed) and ensure tables exist.
pub async fn open() -> Result<SqlitePool> {
    let path = rss_items_db_path()?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    pool.execute(CREATE_RSS_ITEMS_SQL).await?;
    pool.execute(IX_RSS_ITEMS_PUB_SQL).await?;
    Ok(pool)
}

/// Insert or replace a single fetched entry. `INSERT OR REPLACE` is used
/// so an updated entry (same `guid`, newer `summary`/`title`) supersedes
/// the cached row.
pub async fn upsert_one(
    pool: &SqlitePool,
    feed_name: &str,
    feed_url: &str,
    entry: &rss::EntryForCache,
    fetched_at: DateTime<Utc>,
) -> Result<()> {
    let guid = if !entry.guid.is_empty() {
        entry.guid.clone()
    } else if !entry.link.is_empty() {
        entry.link.clone()
    } else {
        return Ok(()); // nothing addressable
    };
    let published = entry.published.map(|p| p.to_rfc3339());
    sqlx::query(
        "INSERT OR REPLACE INTO rss_items \
         (guid, feed_name, feed_url, title, summary, link, author, published, fetched_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&guid)
    .bind(feed_name)
    .bind(feed_url)
    .bind(&entry.title)
    .bind(&entry.summary)
    .bind(&entry.link)
    .bind(&entry.author)
    .bind(published.as_deref().unwrap_or(""))
    .bind(fetched_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Batch upsert: insert each entry from `entries` keyed by the feed name.
/// Failures are swallowed — cache writes are best-effort and must not
/// surface to the user.
pub async fn upsert_items(
    pool: &SqlitePool,
    feed: &RssFeed,
    entries: &[rss::EntryForCache],
) -> Result<usize> {
    let now = Utc::now();
    let mut count = 0usize;
    for e in entries {
        if upsert_one(pool, &feed.name, &feed.url, e, now)
            .await
            .is_ok()
        {
            count += 1;
        }
    }
    Ok(count)
}

/// Cross-module search (Phase 11): return RSS hits whose `title` or
/// `summary` matches the query (OR over tokens, case-insensitive GLOB).
///
/// `ts` is the entry's `published` time when available — UTC, RFC3339
/// ([S005](../../docs/adr/S005-time-semantics-scope.md)). `Hit.url` carries
/// the entry link for direct navigation.
///
/// Note: RSS has no account concept, so `Hit.account` is `None`.
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub async fn search_for_search(q: &SearchQuery) -> Result<Vec<Hit>> {
    let tokens: Vec<&str> = q.tokens();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut params: Vec<String> = Vec::new();
    let mut conds: Vec<String> = Vec::new();
    for t in &tokens {
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        for col in ["title", "summary"] {
            params.push(format!("*{lower}*"));
            let idx = params.len();
            conds.push(format!("lower({col}) GLOB ?{idx}"));
        }
    }
    if conds.is_empty() {
        return Ok(Vec::new());
    }
    let where_clause = conds.join(" OR ");

    let cap = q.limit.unwrap_or(SEARCH_PER_MODULE_CAP);
    params.push(cap.to_string());
    let cap_idx = params.len();

    let sql = format!(
        "SELECT guid, feed_name, feed_url, title, summary, link, author, published \
         FROM rss_items \
         WHERE {where_clause} \
         ORDER BY (published IS NULL), published DESC, fetched_at DESC \
         LIMIT ?{cap_idx}"
    );

    let pool = open().await?;
    let mut query = sqlx::query(&sql);
    for p in &params {
        query = query.bind(p);
    }

    let rows = query.fetch_all(&pool).await?;
    let hits = rows
        .iter()
        .map(|r| {
            let guid: String = r.get("guid");
            let title: String = r.get("title");
            let summary: String = r.get("summary");
            let link: String = r.get("link");
            let _author: String = r.get("author");
            let published: Option<String> = r.get("published");
            let ts = published
                .as_deref()
                .and_then(crate::util::datetime::parse_rfc3339);
            let snippet = if summary.len() > 200 {
                format!("{}…", &summary[..200])
            } else {
                summary
            };
            Hit {
                module: "rss",
                account: None,
                id: guid,
                title,
                snippet,
                url: if link.is_empty() { None } else { Some(link) },
                ts,
                kind: "item",
            }
        })
        .collect();
    Ok(hits)
}

/// Provider adapter: implements [`Searchable`] for the single RSS cache.
///
/// One provider for the whole RSS module — RSS has no account concept,
/// so a single search covers all feeds. The cache is opened lazily by
/// [`search_for_search`] and auto-created on first use.
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub struct RssSearchProvider;

impl RssSearchProvider {
    /// Construct the singleton provider.
    #[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
    pub fn new() -> Self {
        Self
    }
}

impl Default for RssSearchProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Searchable for RssSearchProvider {
    fn module_name(&self) -> &'static str {
        "rss"
    }

    async fn search(&self, q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>> {
        if q.raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        // Cache may not exist yet (fresh user); missing db is not an error —
        // it just means no items to search.
        let path = rss_items_db_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        search_for_search(q).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build a fresh test pool pointed at a temp db. Returns
    /// (pool, db_path) so callers can clean up.
    async fn tmp_pool() -> (SqlitePool, std::path::PathBuf) {
        let file = std::env::temp_dir().join(format!(
            "everyday-rss-items-test-{}.db",
            crate::util::id::gen_id("ri")
        ));
        let opts = SqliteConnectOptions::new()
            .filename(&file)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        pool.execute(CREATE_RSS_ITEMS_SQL).await.unwrap();
        pool.execute(IX_RSS_ITEMS_PUB_SQL).await.unwrap();
        (pool, file)
    }

    #[tokio::test]
    async fn upsert_then_count_returns_match() {
        let (pool, path) = tmp_pool().await;
        let feed = RssFeed {
            name: "hn".into(),
            url: "https://example.com/feed".into(),
            category: Some("tech".into()),
        };
        let entries = vec![
            rss::EntryForCache {
                guid: "guid-1".into(),
                title: "Rust 1.95 released".into(),
                summary: "performance improvements".into(),
                link: "https://example.com/1".into(),
                author: "bob".into(),
                published: Some(Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap()),
            },
            rss::EntryForCache {
                guid: "guid-2".into(),
                title: "python type hints".into(),
                summary: "static typing".into(),
                link: "https://example.com/2".into(),
                author: "alice".into(),
                published: Some(Utc.with_ymd_and_hms(2026, 7, 8, 14, 0, 0).unwrap()),
            },
        ];
        let n = upsert_items(&pool, &feed, &entries).await.unwrap();
        assert_eq!(n, 2);

        let row = sqlx::query("SELECT COUNT(*) as c FROM rss_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = row.get("c");
        assert_eq!(count, 2);

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_guid() {
        let (pool, path) = tmp_pool().await;
        let feed = RssFeed {
            name: "hn".into(),
            url: "https://example.com/feed".into(),
            category: None,
        };
        let e1 = rss::EntryForCache {
            guid: "guid-x".into(),
            title: "old title".into(),
            summary: "old summary".into(),
            link: "https://example.com/x".into(),
            author: "old".into(),
            published: None,
        };
        upsert_items(&pool, &feed, std::slice::from_ref(&e1))
            .await
            .unwrap();

        let e2 = rss::EntryForCache {
            guid: "guid-x".into(),
            title: "new title".into(),
            summary: "new summary".into(),
            link: "https://example.com/x".into(),
            author: "new".into(),
            published: Some(Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap()),
        };
        upsert_items(&pool, &feed, std::slice::from_ref(&e2))
            .await
            .unwrap();

        let row = sqlx::query("SELECT title, summary, author FROM rss_items WHERE guid = ?1")
            .bind("guid-x")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("title"), "new title");
        assert_eq!(row.get::<String, _>("summary"), "new summary");
        assert_eq!(row.get::<String, _>("author"), "new");

        let _ = std::fs::remove_file(path);
    }

    /// Verify the search SQL template (the one used by `search_for_search`)
    /// against a populated cache: title / summary GLOB + OR-of-tokens +
    /// case-insensitive via lower().
    #[tokio::test]
    async fn search_sql_template_matches_title_or_summary() {
        let (pool, path) = tmp_pool().await;
        let feed = RssFeed {
            name: "hn".into(),
            url: "https://example.com/feed".into(),
            category: None,
        };
        upsert_items(
            &pool,
            &feed,
            &[
                rss::EntryForCache {
                    guid: "g1".into(),
                    title: "Rust 1.95 released".into(),
                    summary: "performance improvements".into(),
                    link: "https://example.com/1".into(),
                    author: "bob".into(),
                    published: Some(Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap()),
                },
                rss::EntryForCache {
                    guid: "g2".into(),
                    title: "type hints in python".into(),
                    summary: "static typing benefits".into(),
                    link: "https://example.com/2".into(),
                    author: "alice".into(),
                    published: Some(Utc.with_ymd_and_hms(2026, 7, 8, 14, 0, 0).unwrap()),
                },
            ],
        )
        .await
        .unwrap();

        // Run the same SQL shape that `search_for_search` issues.
        async fn run_query(pool: &SqlitePool, tokens: &[&str]) -> Vec<String> {
            let mut params: Vec<String> = Vec::new();
            let mut conds: Vec<String> = Vec::new();
            for t in tokens {
                let lower = t.to_ascii_lowercase();
                for col in ["title", "summary"] {
                    params.push(format!("*{lower}*"));
                    let idx = params.len();
                    conds.push(format!("lower({col}) GLOB ?{idx}"));
                }
            }
            let where_clause = conds.join(" OR ");
            let sql =
                format!("SELECT guid FROM rss_items WHERE {where_clause} ORDER BY published DESC");
            let mut query = sqlx::query(&sql);
            for p in &params {
                query = query.bind(p);
            }
            let rows = query.fetch_all(pool).await.unwrap();
            rows.iter().map(|r| r.get::<String, _>("guid")).collect()
        }

        // Single token "rust" -> only g1 (title).
        let ids = run_query(&pool, &["rust"]).await;
        assert_eq!(ids, vec!["g1".to_string()]);

        // OR-of-tokens "rust typing" -> both: g1 via title, g2 via summary.
        let ids = run_query(&pool, &["rust", "typing"]).await;
        assert_eq!(ids.len(), 2);

        // Case-insensitive: "RUST" matches g1.
        let ids = run_query(&pool, &["RUST"]).await;
        assert_eq!(ids, vec!["g1".to_string()]);

        let _ = std::fs::remove_file(path);
    }
}
