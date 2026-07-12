//! Local SQLite provider for the note module. See [N001](../../../docs/adr/N001-notion-note-module.md) / [F005](../../../docs/adr/F005-default-provider-local.md).
//!
//! Mirrors the Notion provider's `search` / `list` / `create` / `read` / `append`
//! / `update` semantics; data lives in the account's configured local SQLite file.
//! The local provider needs no credentials (credentials are owned by the `auth` module).
//!
//! This file implements [`LocalNoteBackend`], one of the two `NoteBackend` backends
//! ([R016](../../../docs/adr/R016-action-backend-di.md)); it returns the same domain
//! structs as the Notion backend so the module's render layer is provider-agnostic
//! ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).

use async_trait::async_trait;
use serde_json::{Map, Value};
use sqlx::{Row, SqlitePool};

use crate::config::{Config, NoteAccount};
use crate::error::{AgentError, Result};
use crate::modules::local::{connect, resolve_db_path};
use crate::modules::note::backend::{
    NoteAppended, NoteBackend, NoteCreated, NoteListEntry, NoteRead, NoteSummary, NoteUpdated,
};
use crate::search::{Hit, SearchQuery, Searchable};

const CREATE_NOTES_SQL: &str = "CREATE TABLE IF NOT EXISTS notes (\
    id TEXT PRIMARY KEY, \
    title TEXT NOT NULL, \
    content TEXT NOT NULL DEFAULT '', \
    created_at TEXT NOT NULL, \
    updated_at TEXT NOT NULL)";

const CREATE_PROPS_SQL: &str = "CREATE TABLE IF NOT EXISTS note_props (\
    note_id TEXT NOT NULL, \
    key TEXT NOT NULL, \
    value TEXT NOT NULL, \
    PRIMARY KEY (note_id, key))";

/// Open the connection and ensure tables exist.
async fn open(account: &NoteAccount) -> Result<SqlitePool> {
    let path = resolve_db_path("note", &account.name, account.db_path.as_deref())?;
    let pool = connect(&path).await?;
    sqlx::query(CREATE_NOTES_SQL).execute(&pool).await?;
    sqlx::query(CREATE_PROPS_SQL).execute(&pool).await?;
    Ok(pool)
}

/// Generate a short unique ID (note prefix `n`; impl at [`crate::util::id::gen_id`]).
fn gen_id() -> String {
    crate::util::id::gen_id("n")
}

/// Load a note's properties into a `key -> value` map.
async fn load_props(pool: &SqlitePool, note_id: &str) -> Result<Map<String, Value>> {
    let rows = sqlx::query("SELECT key, value FROM note_props WHERE note_id = ?1 ORDER BY key")
        .bind(note_id)
        .fetch_all(pool)
        .await?;
    let mut m = Map::new();
    for r in &rows {
        m.insert(
            r.get::<String, _>("key"),
            Value::String(r.get::<String, _>("value")),
        );
    }
    Ok(m)
}

// ============ actions ============
//
// Each returns a typed domain struct (never `Output`); the module owns rendering.

/// `note search --query Q [--limit N]` (local): fuzzy search by title.
pub async fn search(account: &NoteAccount, query: &str, limit: usize) -> Result<Vec<NoteSummary>> {
    let limit = (limit as i64).min(100);
    let pool = open(account).await?;

    let pattern = format!("%{query}%");
    let rows = sqlx::query(
        "SELECT id, title, updated_at FROM notes WHERE title LIKE ?1 \
         ORDER BY updated_at DESC LIMIT ?2",
    )
    .bind(&pattern)
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    let mut out = Vec::new();
    for r in &rows {
        out.push(NoteSummary {
            id: r.get::<String, _>("id"),
            kind: "page".to_string(),
            title: r.get::<String, _>("title"),
            url: None,
            updated: r.get::<String, _>("updated_at"),
        });
    }
    Ok(out)
}

/// `note list [--limit N]` (local): list all notes.
pub async fn list(account: &NoteAccount, limit: usize) -> Result<Vec<NoteListEntry>> {
    let limit = (limit as i64).min(100);
    let pool = open(account).await?;

    let rows =
        sqlx::query("SELECT id, title, updated_at FROM notes ORDER BY updated_at DESC LIMIT ?1")
            .bind(limit)
            .fetch_all(&pool)
            .await?;

    let mut out = Vec::new();
    for r in &rows {
        let id: String = r.get("id");
        let props = load_props(&pool, &id).await?;
        out.push(NoteListEntry {
            id,
            title: r.get::<String, _>("title"),
            url: None,
            updated: r.get::<String, _>("updated_at"),
            properties: props,
        });
    }
    Ok(out)
}

/// `note create --title T [--prop K:V ...]` (local): create a note.
pub async fn create(
    account: &NoteAccount,
    title: &str,
    props: &[(String, String)],
) -> Result<NoteCreated> {
    let pool = open(account).await?;
    let id = gen_id();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO notes (id, title, content, created_at, updated_at) VALUES (?1, ?2, '', ?3, ?3)",
    )
    .bind(&id)
    .bind(title)
    .bind(&now)
    .execute(&pool)
    .await?;

    let mut count = 0usize;
    for (k, v) in props {
        upsert_prop(&pool, &id, k, v).await?;
        count += 1;
    }

    Ok(NoteCreated {
        id,
        title: title.to_string(),
        url: None,
        database_id: None,
        prop_count: count,
        resource: "note",
    })
}

/// `note read <page_id>` (local): read title + properties + body.
pub async fn read(account: &NoteAccount, page_id: &str) -> Result<NoteRead> {
    let pool = open(account).await?;

    let row = sqlx::query("SELECT title, content FROM notes WHERE id = ?1")
        .bind(page_id)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| {
            AgentError::InvalidArgument(format!("no note with id '{page_id}' in local database"))
        })?;
    let title: String = row.get("title");
    let content: String = row.get("content");
    let props = load_props(&pool, page_id).await?;

    Ok(NoteRead {
        id: page_id.to_string(),
        title,
        url: None,
        properties: props,
        content,
    })
}

/// `note append <page_id> --text TEXT` (local): append text to the end of the body.
pub async fn append(account: &NoteAccount, page_id: &str, text: &str) -> Result<NoteAppended> {
    if text.trim().is_empty() {
        return Err(AgentError::InvalidArgument(
            "nothing to append (empty text)".into(),
        ));
    }
    let pool = open(account).await?;
    let row = sqlx::query("SELECT content FROM notes WHERE id = ?1")
        .bind(page_id)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| {
            AgentError::InvalidArgument(format!("no note with id '{page_id}' in local database"))
        })?;
    let existing: String = row.get("content");
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let new_content = format!("{existing}{separator}{text}");
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE notes SET content = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(&new_content)
        .bind(&now)
        .bind(page_id)
        .execute(&pool)
        .await?;

    let appended = text.lines().filter(|l| !l.trim().is_empty()).count().max(1);
    Ok(NoteAppended {
        id: page_id.to_string(),
        url: None,
        appended,
        resource: "note",
        unit: "line",
    })
}

/// `note update <page_id> --prop K:V ...` (local): update (upsert) properties.
pub async fn update(
    account: &NoteAccount,
    page_id: &str,
    props: &[(String, String)],
) -> Result<NoteUpdated> {
    if props.is_empty() {
        return Err(AgentError::InvalidArgument(
            "update requires at least one --prop K:V".into(),
        ));
    }
    let pool = open(account).await?;
    // Ensure the note exists.
    let exists = sqlx::query("SELECT 1 FROM notes WHERE id = ?1")
        .bind(page_id)
        .fetch_optional(&pool)
        .await?
        .is_some();
    if !exists {
        return Err(AgentError::InvalidArgument(format!(
            "no note with id '{page_id}' in local database"
        )));
    }

    let mut count = 0usize;
    for (k, v) in props {
        upsert_prop(&pool, page_id, k, v).await?;
        count += 1;
    }
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE notes SET updated_at = ?1 WHERE id = ?2")
        .bind(&now)
        .bind(page_id)
        .execute(&pool)
        .await?;

    Ok(NoteUpdated {
        id: page_id.to_string(),
        url: None,
        updated_count: count,
        resource: "note",
    })
}

// ============ LocalNoteBackend (R016) ============

/// Local SQLite implementation of [`NoteBackend`].
pub struct LocalNoteBackend {
    account: NoteAccount,
}

impl LocalNoteBackend {
    /// Construct from a configured account.
    pub fn new(account: NoteAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl NoteBackend for LocalNoteBackend {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<NoteSummary>> {
        search(&self.account, query, limit).await
    }

    async fn list(&self, _db_id: Option<&str>, limit: usize) -> Result<Vec<NoteListEntry>> {
        list(&self.account, limit).await
    }

    async fn create(
        &self,
        title: &str,
        _db_id: Option<&str>,
        props: &[(String, String)],
    ) -> Result<NoteCreated> {
        create(&self.account, title, props).await
    }

    async fn read(&self, page_id: &str) -> Result<NoteRead> {
        read(&self.account, page_id).await
    }

    async fn append(&self, page_id: &str, text: &str) -> Result<NoteAppended> {
        append(&self.account, page_id, text).await
    }

    async fn update(&self, page_id: &str, props: &[(String, String)]) -> Result<NoteUpdated> {
        update(&self.account, page_id, props).await
    }
}

// ============ Timeline data fetch ============

/// Used for Timeline fetch: raw note entry data.
pub struct NoteTimelineEntry {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Incremental Timeline fetch: return notes whose `created_at` or `updated_at` falls in the window.
///
/// Local provider degradation semantics: multiple updates collapse into a single `updated` event (latest updated_at). See [L001](../../../docs/adr/L001-append-only-event-log.md).
pub async fn fetch_for_timeline(
    account: &NoteAccount,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<NoteTimelineEntry>> {
    let pool = open(account).await?;
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    let rows = sqlx::query(
        "SELECT id, title, created_at, updated_at FROM notes \
         WHERE (created_at >= ?1 AND created_at <= ?2) \
            OR (updated_at >= ?1 AND updated_at <= ?2) \
         ORDER BY created_at ASC",
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_all(&pool)
    .await?;

    let entries: Vec<NoteTimelineEntry> = rows
        .iter()
        .map(|r| NoteTimelineEntry {
            id: r.get("id"),
            title: r.get("title"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect();
    Ok(entries)
}

// ============ helpers ============

/// Insert or update a single property.
async fn upsert_prop(pool: &SqlitePool, note_id: &str, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO note_props (note_id, key, value) VALUES (?1, ?2, ?3) \
         ON CONFLICT(note_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind(note_id)
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

// ============ Cross-module search (Phase 11) ============

/// Per-module hard cap, enforced inside the provider
/// ([S004](../../../docs/adr/S004-execution-model.md)).
const SEARCH_PER_MODULE_CAP: usize = 50;

/// Maximum snippet length returned to the aggregator. Long bodies are
/// truncated at this many characters; the aggregator caps further by
/// `global_limit`, so the upstream consumer never sees arbitrarily large
/// snippets ([S002](../../../docs/adr/S002-hit-normalization.md)).
const SNIPPET_MAX_CHARS: usize = 200;

/// Cross-module search (Phase 11): return note hits whose `title` or
/// `content` matches the query.
///
/// - Tokenize `q.raw` by whitespace, OR over tokens, case-insensitive
///   GLOB substring over `title` OR `content`
///   ([S003](../../../docs/adr/S003-query-semantics.md)).
/// - Per-module hard cap = [`SEARCH_PER_MODULE_CAP`] (50); the
///   aggregator applies its own global cap on top.
/// - `ts` is `updated_at` (UTC, RFC3339) — the module's primary edit
///   time ([S005](../../../docs/adr/S005-time-semantics-scope.md)).
/// - `snippet` is the first [`SNIPPET_MAX_CHARS`] chars of `content`.
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub async fn search_for_search(account: &NoteAccount, q: &SearchQuery) -> Result<Vec<Hit>> {
    let tokens: Vec<&str> = q.tokens();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // Build the WHERE clause manually so we can mix title + content in
    // one statement with continuous placeholder numbering.
    let mut params: Vec<String> = Vec::new();
    let mut conds: Vec<String> = Vec::new();
    for t in &tokens {
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        params.push(format!("*{lower}*"));
        let idx = params.len();
        conds.push(format!("lower(title) GLOB ?{idx}"));
        params.push(format!("*{lower}*"));
        let idx2 = params.len();
        conds.push(format!("lower(content) GLOB ?{idx2}"));
    }
    if conds.is_empty() {
        return Ok(Vec::new());
    }
    let where_clause = conds.join(" OR ");

    let cap = q.limit.unwrap_or(SEARCH_PER_MODULE_CAP);
    params.push(cap.to_string());
    let cap_idx = params.len();

    let sql = format!(
        "SELECT id, title, content, updated_at FROM notes \
         WHERE {where_clause} \
         ORDER BY updated_at DESC, id ASC LIMIT ?{cap_idx}"
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
            let title: String = r.get("title");
            let content: String = r.get("content");
            let updated_at: String = r.get("updated_at");
            let ts = crate::util::datetime::parse_rfc3339(&updated_at);
            let snippet = snippet_from_content(&content, SNIPPET_MAX_CHARS);
            Hit {
                module: "note",
                account: Some(account.name.clone()),
                id,
                title,
                snippet,
                // local note has no URL; agents use module+id.
                url: None,
                ts,
                kind: "page",
            }
        })
        .collect();
    Ok(hits)
}

/// Build a short snippet from the note body: first non-empty line,
/// truncated at `max_chars`. Whitespace is normalized; trailing `…`
/// marks truncation.
fn snippet_from_content(content: &str, max_chars: usize) -> String {
    let first = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.chars().count() <= max_chars {
        first.to_string()
    } else {
        let truncated: String = first.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

/// Provider adapter: implements [`Searchable`] for one local note account.
///
/// One provider per local account. Notion accounts are not searchable in
/// v1 (consistent with [S005](../../../docs/adr/S005-time-semantics-scope.md):
/// live-fetch-on-search rejected; local cache only).
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub struct NoteSearchProvider {
    account: NoteAccount,
}

impl NoteSearchProvider {
    /// Construct from a configured local account.
    #[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
    pub fn new(account: NoteAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl Searchable for NoteSearchProvider {
    fn module_name(&self) -> &'static str {
        "note"
    }

    async fn search(&self, q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>> {
        // Local provider: skip silently on empty query (the aggregator
        // already enforces non-empty raw, but defensive).
        if q.raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        search_for_search(&self.account, q).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_account() -> NoteAccount {
        let file = std::env::temp_dir().join(format!("everyday-note-test-{}.db", gen_id()));
        NoteAccount {
            name: "test".into(),
            provider: "local".into(),
            default_database_id: None,
            default_page_id: None,
            db_path: Some(file.to_string_lossy().to_string()),
        }
    }

    #[test]
    fn gen_id_has_prefix() {
        assert!(gen_id().starts_with('n'));
    }

    #[tokio::test]
    async fn create_append_update_read_roundtrip() {
        let acc = tmp_account();
        let props = vec![("类型".to_string(), "文章".to_string())];
        create(&acc, "Rust 笔记", &props).await.unwrap();

        // Fetch the id.
        let pool = open(&acc).await.unwrap();
        let id: String = sqlx::query("SELECT id FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("id");

        // Append body.
        append(&acc, &id, "第一行\n第二行").await.unwrap();

        // Update properties.
        let umulti = vec![("状态".to_string(), "已读".to_string())];
        update(&acc, &id, &umulti).await.unwrap();

        // Verify content and properties.
        let content: String = sqlx::query("SELECT content FROM notes WHERE id = ?1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("content");
        assert!(content.contains("第一行"));
        let props = load_props(&pool, &id).await.unwrap();
        assert_eq!(props.get("类型").unwrap(), "文章");
        assert_eq!(props.get("状态").unwrap(), "已读");

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn search_matches_title() {
        let acc = tmp_account();
        create(&acc, "SQLite 存储", &[]).await.unwrap();

        let pool = open(&acc).await.unwrap();
        let rows = sqlx::query("SELECT id FROM notes WHERE title LIKE '%SQLite%'")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn read_missing_note_errors() {
        let acc = tmp_account();
        let err = read(&acc, "ghost").await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    /// Cross-module search: matches both title and content; OR semantics
    /// over tokens (the second token matches a different row).
    #[tokio::test]
    async fn search_for_search_matches_title_and_content() {
        let acc = tmp_account();
        // Note A: title contains "rust", body contains "sqlite".
        create(&acc, "Rust 笔记", &[]).await.unwrap();
        let pool = open(&acc).await.unwrap();
        let id_a: String = sqlx::query("SELECT id FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("id");
        append(&acc, &id_a, "stored in sqlite").await.unwrap();

        // Note B: title "rust cli 工具" (matches rust), body "时间线".
        create(&acc, "rust cli 工具", &[]).await.unwrap();

        // Single-token query "sql" — only Note A (body match).
        let q = SearchQuery::new("sql");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id_a);
        assert_eq!(hits[0].module, "note");
        assert!(hits[0].snippet.contains("sqlite"));

        // Single-token query "rust" — both notes match via title.
        let q = SearchQuery::new("rust");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 2);

        // OR-of-tokens query "sql cli" — both notes match: A via body
        // ("sqlite"), B via title ("cli"). This proves OR semantics: a
        // row qualifies if any token matches any column.
        let q = SearchQuery::new("sql cli");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 2);
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&id_a.as_str()));

        // --limit override caps results.
        let mut q = SearchQuery::new("rust");
        q.limit = Some(1);
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 1);

        // Empty query produces no hits (defensive guard).
        let q = SearchQuery::new("   ");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert!(hits.is_empty());

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }
}
