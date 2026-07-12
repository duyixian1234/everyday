//! Local SQLite provider for the bookmark module [B001](../../docs/adr/B001-bookmark-dual-provider.md).
//!
//! Parity implementation of `add` / `list` semantics with the Notion provider; data lands in the
//! account's configured local SQLite file. `login` is meaningless for the local provider (no
//! credentials needed), and `init-db` only creates the table and reports the path.
//!
//! Data model:
//! - `bookmarks(id, url, title, created_at)`: one bookmark = URL + title.
//! - `bookmark_tags(bookmark_id, tag)`: a bookmark's tags (many-to-many), used for exact tag filtering.
//!
//! The output shape (column names / JSON keys) is deliberately kept in sync with the Notion version
//! in `bookmark.rs`: `id` / `title` / `url` / `tags`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};

use crate::config::{BookmarkAccount, Config};
use crate::error::{AgentError, Result};
use crate::modules::bookmark::BookmarkItem;
use crate::modules::local::{connect, mode_json, resolve_db_path};
use crate::output::Output;
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
async fn open(account: &BookmarkAccount) -> Result<SqlitePool> {
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

// ============ actions ============

/// `bookmark login` (local): the local provider needs no credentials.
pub fn login(account: &BookmarkAccount) -> Result<Output> {
    Ok(Output::text(format!(
        "bookmark account '{}' uses the local sqlite provider; no login required",
        account.name
    )))
}

/// `bookmark init-db` (local): create the table and report the database path.
pub async fn init_db(account: &BookmarkAccount) -> Result<Output> {
    let path = resolve_db_path("bookmark", &account.name, account.db_path.as_deref())?;
    let _ = open(account).await?;
    let path_str = path.to_string_lossy().to_string();
    if mode_json() {
        Ok(Output::Json(
            json!({ "account": account.name, "db_path": path_str, "provider": "local" }),
        ))
    } else {
        Ok(Output::text(format!(
            "initialized local bookmark database for account '{}'\n{}",
            account.name, path_str
        )))
    }
}

/// `bookmark add --url U --title T [--tags a,b]` (local): collect a bookmark.
pub async fn add(account: &BookmarkAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let url = flags
        .get("url")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --url <url>".into()))?;
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --title <title>".into()))?;
    let tags = crate::modules::local::parse_tags(flags.get("tags"));
    let pool = open(account).await?;
    let id = gen_id();
    let created_at = chrono::Utc::now().to_rfc3339();

    sqlx::query("INSERT INTO bookmarks (id, url, title, created_at) VALUES (?1, ?2, ?3, ?4)")
        .bind(&id)
        .bind(url)
        .bind(title)
        .bind(&created_at)
        .execute(&pool)
        .await?;

    for tag in &tags {
        sqlx::query("INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag) VALUES (?1, ?2)")
            .bind(&id)
            .bind(tag)
            .execute(&pool)
            .await?;
    }

    if mode_json() {
        Ok(Output::Json(
            json!({ "id": id, "url": url, "title": title, "tags": tags }),
        ))
    } else {
        Ok(Output::text(format!(
            "added bookmark '{}' (id={}, tags={})",
            title,
            id,
            tags.join(", ")
        )))
    }
}

/// `bookmark list [--tag TAG]` (local): list bookmarks, optionally filtered by tag.
pub async fn list(account: &BookmarkAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let pool = open(account).await?;

    // Base query: JOIN bookmark_tags when filtering by tag, otherwise take all.
    let rows = if let Some(tag) = flags.get("tag") {
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
            url: r.get::<String, _>("url"),
            title: r.get::<String, _>("title"),
            tags,
        });
    }

    if mode_json() {
        let arr: Vec<Value> = items
            .iter()
            .map(|it| serde_json::to_value(it).unwrap_or(Value::Null))
            .collect();
        Ok(Output::Json(Value::Array(arr)))
    } else {
        let table_rows = items
            .iter()
            .map(|it| {
                vec![
                    it.id.clone(),
                    it.title.clone(),
                    it.url.clone(),
                    it.tags.join(", "),
                ]
            })
            .collect();
        Ok(Output::records(
            vec!["id".into(), "title".into(), "url".into(), "tags".into()],
            table_rows,
        ))
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

// ============ Helpers ============

// parse_tags: see `crate::modules::local::parse_tags` — shared by both bookmark providers [R009](../../docs/adr/R009-notion-common-local-module.md).

// ============ Cross-module search (Phase 11) ============

/// Per-module hard cap, enforced inside the provider
/// ([S004](../../docs/adr/S004-execution-model.md)).
const SEARCH_PER_MODULE_CAP: usize = 50;

/// Cross-module search (Phase 11): return bookmark hits whose `title`,
/// `url`, or any tag matches the query (OR over tokens, case-insensitive
/// GLOB).
///
/// `ts` is `created_at` (UTC, RFC3339) — the module's primary time
/// ([S005](../../docs/adr/S005-time-semantics-scope.md)).
///
/// `Hit.url` carries the bookmark URL when present (so search consumers
/// can open the link without re-running `bookmark list`).
///
/// Notion accounts are skipped in v1 (live-fetch-on-search rejected by
/// [S005](../../docs/adr/S005-time-semantics-scope.md)).
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
    async fn count_tag(pool: &SqlitePool, tag: &str) -> i64 {
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

        let mut f1 = HashMap::new();
        f1.insert("url".into(), "https://www.rust-lang.org".into());
        f1.insert("title".into(), "Rust 官网".into());
        f1.insert("tags".into(), "rust,lang".into());
        add(&acc, &f1).await.unwrap();

        let mut f2 = HashMap::new();
        f2.insert("url".into(), "https://doc.rust-lang.org".into());
        f2.insert("title".into(), "Rust 文档".into());
        f2.insert("tags".into(), "rust,doc".into());
        add(&acc, &f2).await.unwrap();

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
        let mut fr = HashMap::new();
        fr.insert("tag".into(), "doc".into());
        let out = list(&acc, &fr).await.unwrap();
        let rows = match out {
            Output::Records { rows, .. } => rows,
            Output::Json(v) => v
                .as_array()
                .unwrap()
                .iter()
                .map(|it| {
                    vec![
                        it["id"].as_str().unwrap_or("").to_string(),
                        it["title"].as_str().unwrap_or("").to_string(),
                        it["url"].as_str().unwrap_or("").to_string(),
                        it["tags"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default(),
                    ]
                })
                .collect(),
            other => panic!("unexpected output: {other:?}"),
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][1], "Rust 文档");

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn add_missing_url_errors() {
        let acc = tmp_account();
        let mut f = HashMap::new();
        f.insert("title".into(), "no url".into());
        let err = add(&acc, &f).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
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
        // Bookmark 1: title contains "rust", URL distinct.
        let mut f1 = HashMap::new();
        f1.insert("url".into(), "https://www.rust-lang.org".into());
        f1.insert("title".into(), "Rust 官网".into());
        f1.insert("tags".into(), "rust,lang".into());
        add(&acc, &f1).await.unwrap();

        // Bookmark 2: only a tag matches ("python").
        let mut f2 = HashMap::new();
        f2.insert("url".into(), "https://example.org".into());
        f2.insert("title".into(), "Some page".into());
        f2.insert("tags".into(), "python,docs".into());
        add(&acc, &f2).await.unwrap();

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
