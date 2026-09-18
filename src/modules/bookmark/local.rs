//! Local SQLite provider for the `bookmark` module.
//!
//! `LocalBookmarkBackend` implements [`BookmarkBackend`] ([R016](../../../docs/adr/R016-action-backend-di.md)),
//! with data persisted in the account's local SQLite file. The local provider needs no
//! credentials; the tables are created lazily on first use. The Notion provider was
//! removed in v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)).

use async_trait::async_trait;
use sqlx::Row;

use crate::config::{BookmarkAccount, Config};
use crate::error::Result;
use crate::modules::bookmark::backend::{BookmarkAdded, BookmarkBackend, BookmarkItem};
use crate::modules::local::{connect, resolve_db_path};
use crate::search::{Hit, SearchQuery, Searchable};

/// Table creation statements: the bookmark master table + the tag association table.
const CREATE_BOOKMARKS_SQL: &str = "CREATE TABLE IF NOT EXISTS bookmarks (\
    id TEXT PRIMARY KEY, \
    url TEXT NOT NULL, \
    title TEXT NOT NULL, \
    created_at TEXT NOT NULL)";

const CREATE_TAGS_SQL: &str = "CREATE TABLE IF NOT EXISTS bookmark_tags (\
    bookmark_id TEXT NOT NULL, \
    tag TEXT NOT NULL, \
    PRIMARY KEY (bookmark_id, tag))";

/// Open the connection and ensure the tables exist.
async fn open(account: &BookmarkAccount) -> Result<sqlx::SqlitePool> {
    let path = resolve_db_path("bookmark", &account.name, account.db_path.as_deref())?;
    let pool = connect(&path).await?;
    sqlx::query(CREATE_BOOKMARKS_SQL).execute(&pool).await?;
    sqlx::query(CREATE_TAGS_SQL).execute(&pool).await?;
    Ok(pool)
}

/// Generate a short unique ID (bookmark prefix `b`; see [`crate::util::id::gen_id`]).
fn gen_id() -> String {
    crate::util::id::gen_id("b")
}

// ============ Backend ============

/// Local SQLite implementation of [`BookmarkBackend`].
pub struct LocalBookmarkBackend {
    account: BookmarkAccount,
}

impl LocalBookmarkBackend {
    pub fn new(account: BookmarkAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl BookmarkBackend for LocalBookmarkBackend {
    /// `add --url U --title T [--tags a,b]` (local): collect a bookmark.
    async fn add(&self, url: &str, title: &str, tags: &[String]) -> Result<BookmarkAdded> {
        let pool = open(&self.account).await?;
        let id = gen_id();
        let created_at = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO bookmarks (id, url, title, created_at) VALUES (?1, ?2, ?3, ?4)")
            .bind(&id)
            .bind(url)
            .bind(title)
            .bind(&created_at)
            .execute(&pool)
            .await?;

        for tag in tags {
            sqlx::query("INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag) VALUES (?1, ?2)")
                .bind(&id)
                .bind(tag)
                .execute(&pool)
                .await?;
        }

        Ok(BookmarkAdded {
            id,
            url: url.to_string(),
            title: title.to_string(),
            tags: tags.to_vec(),
        })
    }

    /// `list [--tag TAG]` (local): list bookmarks, optionally filtered by tag.
    async fn list(&self, tag: Option<&str>) -> Result<Vec<BookmarkItem>> {
        let pool = open(&self.account).await?;

        // Base query: JOIN bookmark_tags when filtering by tag, otherwise take all.
        let rows = if let Some(tag) = tag {
            let sql = "SELECT b.id, b.url, b.title, b.created_at FROM bookmarks b \
                JOIN bookmark_tags t ON t.bookmark_id = b.id \
                WHERE t.tag = ?1 ORDER BY b.created_at DESC, b.id DESC";
            sqlx::query(sql).bind(tag).fetch_all(&pool).await?
        } else {
            let sql = "SELECT id, url, title, created_at FROM bookmarks \
                ORDER BY created_at DESC, id DESC";
            sqlx::query(sql).fetch_all(&pool).await?
        };

        let ids: Vec<String> = rows.iter().map(|r| r.get("id")).collect();
        let tags_by_id = load_tags(&pool, &ids).await?;
        Ok(rows
            .iter()
            .map(|r| {
                let id: String = r.get("id");
                BookmarkItem {
                    tags: tags_by_id.get(&id).cloned().unwrap_or_default(),
                    id,
                    url: r.get("url"),
                    title: r.get("title"),
                }
            })
            .collect())
    }

    /// `search <query> [--limit N]` (local): GLOB-match title / url / tags.
    ///
    /// Shares [`search_for_search`]'s predicate so the module action and the
    /// cross-module `everyday search` can never disagree.
    async fn search(&self, query: &str, limit: Option<usize>) -> Result<Vec<BookmarkItem>> {
        let mut q = SearchQuery::new(query);
        q.limit = limit;
        let hits = search_for_search(&self.account, &q).await?;
        // Hydrate tags for the module-facing view; `Hit` carries none.
        let pool = open(&self.account).await?;
        let ids: Vec<String> = hits.iter().map(|h| h.id.clone()).collect();
        let tags_by_id = load_tags(&pool, &ids).await?;
        Ok(hits
            .into_iter()
            .map(|h| BookmarkItem {
                tags: tags_by_id.get(&h.id).cloned().unwrap_or_default(),
                id: h.id,
                url: h.url.unwrap_or_default(),
                title: h.title,
            })
            .collect())
    }
}

/// Load the tags of several bookmarks in **one** query, keyed by bookmark id.
///
/// One `IN (...)` query rather than one per row: `list` and `search` both
/// hydrate tags, and the per-row form made a 50-hit search issue 51 queries.
/// A bookmark with no tags simply has no entry, so callers default to empty.
async fn load_tags(
    pool: &sqlx::SqlitePool,
    ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let mut by_id: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(by_id);
    }
    let placeholders = (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT bookmark_id, tag FROM bookmark_tags \
         WHERE bookmark_id IN ({placeholders}) ORDER BY bookmark_id, tag"
    );
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for id in ids {
        query = query.bind(id);
    }
    for row in query.fetch_all(pool).await? {
        by_id
            .entry(row.get::<String, _>("bookmark_id"))
            .or_default()
            .push(row.get::<String, _>("tag"));
    }
    Ok(by_id)
}

// ============ Timeline data ingestion ============

/// Timeline ingestion: raw bookmark entry data.
pub struct BookmarkTimelineEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub tags: Vec<String>,
    pub created_at: String,
}

/// Timeline incremental fetch: return bookmarks whose `created_at` falls within the window.
pub async fn fetch_for_timeline(
    account: &BookmarkAccount,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<BookmarkTimelineEntry>> {
    let pool = open(account).await?;
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    let rows = sqlx::query(
        "SELECT id, url, title, created_at FROM bookmarks \
         WHERE created_at >= ?1 AND created_at <= ?2 \
         ORDER BY created_at ASC",
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_all(&pool)
    .await?;

    let mut entries = Vec::with_capacity(rows.len());
    for r in &rows {
        let id: String = r.get("id");
        let tag_rows =
            sqlx::query("SELECT tag FROM bookmark_tags WHERE bookmark_id = ?1 ORDER BY tag")
                .bind(&id)
                .fetch_all(&pool)
                .await?;
        let tags: Vec<String> = tag_rows
            .iter()
            .map(|tr| tr.get::<String, _>("tag"))
            .collect();
        entries.push(BookmarkTimelineEntry {
            id,
            url: r.get("url"),
            title: r.get("title"),
            tags,
            created_at: r.get("created_at"),
        });
    }
    Ok(entries)
}

// ============ Cross-module search (Phase 11) ============

/// Per-module hard cap, enforced inside the provider
/// ([S004](../../../docs/adr/S004-execution-model.md)).
const SEARCH_PER_MODULE_CAP: usize = 50;

/// Cross-module search (Phase 11): return bookmark hits whose `title`,
/// `url`, or any tag matches the query (OR over tokens, case-insensitive
/// GLOB).
///
/// `ts` is `created_at` (UTC, RFC3339) — the module's primary time
/// ([S005](../../../docs/adr/S005-time-semantics-scope.md)).
///
/// Search covers the account's local SQLite store (live-fetch-on-search rejected by
/// [S005](../../../docs/adr/S005-time-semantics-scope.md)).
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub async fn search_for_search(account: &BookmarkAccount, q: &SearchQuery) -> Result<Vec<Hit>> {
    let tokens: Vec<&str> = q.tokens();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // Build a WHERE clause that ORs GLOB matches across title, url, and
    // (via JOIN bookmark_tags) tag columns.
    let mut params: Vec<String> = Vec::new();
    let mut conds: Vec<String> = Vec::new();
    for t in &tokens {
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        for col in ["b.title", "b.url", "t.tag"] {
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

    // LEFT JOIN so a bookmark with no tags still surfaces when its
    // title/url matches.
    let sql = format!(
        "SELECT b.id, b.url, b.title, b.created_at FROM bookmarks b \
         LEFT JOIN bookmark_tags t ON t.bookmark_id = b.id \
         WHERE {where_clause} \
         GROUP BY b.id \
         ORDER BY b.created_at DESC, b.id ASC \
         LIMIT ?{cap_idx}"
    );

    let pool = open(account).await?;
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for p in &params {
        query = query.bind(p);
    }

    let rows = query.fetch_all(&pool).await?;
    let hits = rows
        .iter()
        .map(|r| {
            let id: String = r.get("id");
            let url: String = r.get("url");
            let title: String = r.get("title");
            let created_at: String = r.get("created_at");
            let ts = crate::util::datetime::parse_rfc3339(&created_at);
            let snippet = if url.is_empty() {
                String::new()
            } else {
                url.clone()
            };
            Hit {
                module: "bookmark",
                account: Some(account.name.clone()),
                id,
                title,
                snippet,
                url: if url.is_empty() { None } else { Some(url) },
                ts,
                kind: "bookmark",
                // This module's own SQLite DB is the source of truth (F020).
                stale: false,
            }
        })
        .collect();
    Ok(hits)
}

/// Provider adapter: implements [`Searchable`] for one local bookmark account.
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub struct BookmarkSearchProvider {
    account: BookmarkAccount,
}

impl BookmarkSearchProvider {
    /// Construct from a configured local account.
    #[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
    pub fn new(account: BookmarkAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl Searchable for BookmarkSearchProvider {
    fn module_name(&self) -> &'static str {
        "bookmark"
    }

    async fn search(&self, q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>> {
        if q.raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        search_for_search(&self.account, q).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_account() -> BookmarkAccount {
        let file = std::env::temp_dir().join(format!("everyday-bookmark-test-{}.db", gen_id()));
        BookmarkAccount {
            name: "test".into(),
            provider: "local".into(),
            db_path: Some(file.to_string_lossy().to_string()),
        }
    }

    /// Count bookmarks under a given tag (exact match via JOIN bookmark_tags).
    async fn count_tag(pool: &sqlx::SqlitePool, tag: &str) -> i64 {
        sqlx::query(
            "SELECT COUNT(*) as c FROM bookmarks b \
             JOIN bookmark_tags t ON t.bookmark_id = b.id WHERE t.tag = ?1",
        )
        .bind(tag)
        .fetch_one(pool)
        .await
        .unwrap()
        .get::<i64, _>("c")
    }

    #[tokio::test]
    async fn add_and_list_roundtrip() {
        let acc = tmp_account();
        let backend = LocalBookmarkBackend::new(acc.clone());

        backend
            .add(
                "https://www.rust-lang.org",
                "Rust 官网",
                &["rust".into(), "lang".into()],
            )
            .await
            .unwrap();
        backend
            .add(
                "https://doc.rust-lang.org",
                "Rust 文档",
                &["rust".into(), "doc".into()],
            )
            .await
            .unwrap();

        let pool = open(&acc).await.unwrap();

        // All 2 rows.
        let all: i64 = sqlx::query("SELECT COUNT(*) as c FROM bookmarks")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("c");
        assert_eq!(all, 2);

        // Filter by tag (exact match via JOIN bookmark_tags): rust -> 2, doc -> 1, lang -> 1.
        assert_eq!(count_tag(&pool, "rust").await, 2);
        assert_eq!(count_tag(&pool, "doc").await, 1);
        assert_eq!(count_tag(&pool, "lang").await, 1);

        // list output shape is correct (Records in text mode, array in JSON mode).
        let items = backend.list(Some("doc")).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Rust 文档");

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[test]
    fn parse_tags_local_splits() {
        // The full test for the shared helper lives in local.rs; here we only verify the alias call.
        assert_eq!(
            crate::modules::local::parse_tags(Some(&"a, b ,c".to_string())),
            vec!["a", "b", "c"]
        );
    }

    /// Cross-module search (Phase 11): GLOB over title / url / tag, OR
    /// semantics over tokens, LEFT JOIN keeps tag-less bookmarks visible.
    #[tokio::test]
    async fn search_for_search_matches_title_url_and_tag() {
        let acc = tmp_account();
        let backend = LocalBookmarkBackend::new(acc.clone());
        backend
            .add(
                "https://www.rust-lang.org",
                "Rust 官网",
                &["rust".into(), "lang".into()],
            )
            .await
            .unwrap();
        backend
            .add(
                "https://example.org",
                "Some page",
                &["python".into(), "docs".into()],
            )
            .await
            .unwrap();

        // Query "rust" -> only bookmark 1 (title GLOB).
        let q = SearchQuery::new("rust");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].title.contains("Rust"));
        assert_eq!(hits[0].url.as_deref(), Some("https://www.rust-lang.org"));

        // Query "python" -> only bookmark 2 (tag GLOB).
        let q = SearchQuery::new("python");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].title.contains("Some page"));

        // OR-of-tokens "rust python" -> both bookmarks.
        let q = SearchQuery::new("rust python");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 2);

        // URL match: "example.org" matches only bookmark 2.
        let q = SearchQuery::new("example.org");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].url.as_deref().unwrap().contains("example.org"));

        // Empty query -> no hits.
        let q = SearchQuery::new("  ");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert!(hits.is_empty());

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }
}
