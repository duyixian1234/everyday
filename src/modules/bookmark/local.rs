//! Local SQLite provider for the `bookmark` module [B001](../../../docs/adr/B001-bookmark-dual-provider.md).
//!
//! `LocalBookmarkBackend` implements [`BookmarkBackend`] ([R016](../../../docs/adr/R016-action-backend-di.md)),
//! a parity implementation of `init-db` / `add` / `list` alongside the Notion provider
//! [B001](../../../docs/adr/B001-bookmark-dual-provider.md), with data persisted in the
//! account's local SQLite file. The local provider needs no credentials (credentials are
//! owned by the `auth` module), and `init-db` only creates the table and reports the path.
//!
//! Output shape (column names / JSON keys) is deliberately kept in sync with the Notion
//! version in `notion.rs`: `id` / `title` / `url` / `tags`.

use async_trait::async_trait;
use sqlx::Row;

use crate::config::{BookmarkAccount, Config};
use crate::error::Result;
use crate::modules::bookmark::backend::{
    BookmarkAdded, BookmarkBackend, BookmarkInitDb, BookmarkItem,
};
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
    /// `init-db` (local): create the table and report the database path.
    async fn init_db(&self, _parent: Option<&str>) -> Result<BookmarkInitDb> {
        let path = resolve_db_path(
            "bookmark",
            &self.account.name,
            self.account.db_path.as_deref(),
        )?;
        let _ = open(&self.account).await?;
        let path_str = path.to_string_lossy().to_string();
        Ok(BookmarkInitDb {
            account: self.account.name.clone(),
            provider: "local",
            db_path: Some(path_str),
            database_id: None,
            url: None,
        })
    }

    /// `add --url U --title T [--tags a,b]` (local): collect a bookmark.
    async fn add(
        &self,
        url: &str,
        title: &str,
        tags: &[String],
        _db_id: Option<&str>,
    ) -> Result<BookmarkAdded> {
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
            database_id: None,
        })
    }

    /// `list [--tag TAG]` (local): list bookmarks, optionally filtered by tag.
    async fn list(&self, tag: Option<&str>, _db_id: Option<&str>) -> Result<Vec<BookmarkItem>> {
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

        // Load tags per row and assemble BookmarkItem.
        let mut items: Vec<BookmarkItem> = Vec::with_capacity(rows.len());
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
            items.push(BookmarkItem {
                id,
                url: r.get("url"),
                title: r.get("title"),
                tags,
            });
        }
        Ok(items)
    }
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
/// Notion accounts are skipped in v1 (live-fetch-on-search rejected by
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
    let mut query = sqlx::query(&sql);
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
            parent_page_id: None,
            default_database_id: None,
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
                None,
            )
            .await
            .unwrap();
        backend
            .add(
                "https://doc.rust-lang.org",
                "Rust 文档",
                &["rust".into(), "doc".into()],
                None,
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
        let items = backend.list(Some("doc"), None).await.unwrap();
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
                None,
            )
            .await
            .unwrap();
        backend
            .add(
                "https://example.org",
                "Some page",
                &["python".into(), "docs".into()],
                None,
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
