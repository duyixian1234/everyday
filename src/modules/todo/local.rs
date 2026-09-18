//! Local SQLite provider for the `todo` module.
//!
//! `LocalTodoBackend` implements [`TodoBackend`] ([R016](../../../docs/adr/R016-action-backend-di.md)),
//! with data persisted in the account's local SQLite file. The local provider needs no
//! credentials; the table is created lazily on first use. The Notion provider was
//! removed in v0.13.0 ([R019](../../../docs/adr/R019-remove-notion-provider.md)).

use async_trait::async_trait;
use sqlx::Row;

use crate::config::{Config, TodoAccount};
use crate::error::{AgentError, Result};
use crate::modules::local::{connect, resolve_db_path};
use crate::modules::todo::backend::{
    STATUS_DONE, STATUS_TODO, TodoAdded, TodoBackend, TodoDeleted, TodoItem, TodoStatusSet,
};
use crate::search::{Hit, SearchQuery, Searchable};

/// Table creation SQL: the todos table (includes the `updated_at` column used for timeline incremental pulls).
const CREATE_SQL: &str = "CREATE TABLE IF NOT EXISTS todos (\
    id TEXT PRIMARY KEY, \
    title TEXT NOT NULL, \
    status TEXT NOT NULL, \
    due TEXT, \
    priority TEXT, \
    created_at TEXT NOT NULL, \
    updated_at TEXT NOT NULL DEFAULT '', \
    deleted_at TEXT)";

/// The current-state view: every live row, `deleted_at IS NULL`.
///
/// Read paths query this instead of the base table, so a forgotten guard cannot
/// leak a deleted row. Writes still need their own `deleted_at IS NULL`
/// condition (a view cannot protect an UPDATE), but reads are the leak-prone
/// majority and are now structurally safe. Same shape as `memory`'s
/// `current_state_view` ([R024](../../../docs/adr/R024-local-provider-soft-delete.md)).
const CREATE_VIEW_SQL: &str = "CREATE VIEW IF NOT EXISTS active_todos AS \
    SELECT id, title, status, due, priority, created_at, updated_at \
    FROM todos WHERE deleted_at IS NULL";

/// Open a connection and ensure the table, the `deleted_at` column, and the
/// current-state view exist.
async fn open(account: &TodoAccount) -> Result<sqlx::SqlitePool> {
    let path = resolve_db_path("todo", &account.name, account.db_path.as_deref())?;
    let pool = connect(&path).await?;
    sqlx::query(CREATE_SQL).execute(&pool).await?;
    // Migrations: older tables lack these columns; add them idempotently.
    // `deleted_at` is added here rather than only in CREATE TABLE so existing
    // databases gain it — `memory/store.rs` skips this and its old DBs
    // therefore never got the column.
    add_column_if_missing(&pool, "updated_at", "TEXT NOT NULL DEFAULT ''").await?;
    add_column_if_missing(&pool, "deleted_at", "TEXT").await?;
    sqlx::query(CREATE_VIEW_SQL).execute(&pool).await?;
    Ok(pool)
}

/// Add `column` to `todos` when absent. SQLite has no
/// `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, so this checks `pragma_table_info`
/// first.
async fn add_column_if_missing(pool: &sqlx::SqlitePool, column: &str, decl: &str) -> Result<()> {
    // `column`/`decl` are compile-time literals at both call sites; audited.
    let probe = sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM pragma_table_info('todos') WHERE name='{column}'"
    ));
    let row: Option<(i64,)> = sqlx::query_as(probe).fetch_optional(pool).await?;
    if let Some((count,)) = row
        && count == 0
    {
        let alter = sqlx::AssertSqlSafe(format!("ALTER TABLE todos ADD COLUMN {column} {decl}"));
        sqlx::query(alter).execute(pool).await?;
    }
    Ok(())
}

/// Generate a short unique ID (todo prefix `t`; see [`crate::util::id::gen_id`]).
fn gen_id() -> String {
    crate::util::id::gen_id("t")
}

// ============ Backend ============

/// Local SQLite implementation of [`TodoBackend`].
pub struct LocalTodoBackend {
    account: TodoAccount,
}

impl LocalTodoBackend {
    pub fn new(account: TodoAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl TodoBackend for LocalTodoBackend {
    /// `list [--all]` (local): list tasks, filtering done by default, ordered by due ascending (nulls last).
    async fn list(&self, all: bool) -> Result<Vec<TodoItem>> {
        let pool = open(&self.account).await?;

        let sql = if all {
            "SELECT id, title, status, due, priority FROM active_todos \
             ORDER BY (due IS NULL), due ASC, created_at ASC"
        } else {
            "SELECT id, title, status, due, priority FROM active_todos \
             WHERE status <> ?1 ORDER BY (due IS NULL), due ASC, created_at ASC"
        };

        let rows = if all {
            sqlx::query(sql).fetch_all(&pool).await?
        } else {
            sqlx::query(sql).bind(STATUS_DONE).fetch_all(&pool).await?
        };

        let items: Vec<TodoItem> = rows
            .iter()
            .map(|r| TodoItem {
                id: r.get("id"),
                title: r.get("title"),
                status: r.get("status"),
                due: r.get("due"),
                priority: r.get("priority"),
            })
            .collect();
        Ok(items)
    }

    /// `add --title T [--due DATE] [--priority P]` (local): create a new task.
    async fn add(
        &self,
        title: &str,
        due: Option<&str>,
        priority: Option<&str>,
    ) -> Result<TodoAdded> {
        let pool = open(&self.account).await?;
        let id = gen_id();
        let due = due.map(|s| s.to_string());
        let priority = priority.map(|s| s.to_string());
        let created_at = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO todos (id, title, status, due, priority, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        )
        .bind(&id)
        .bind(title)
        .bind(STATUS_TODO)
        .bind(&due)
        .bind(&priority)
        .bind(&created_at)
        .execute(&pool)
        .await?;

        Ok(TodoAdded {
            id,
            title: title.to_string(),
        })
    }

    /// `start` / `complete <id>` (local): update the task status.
    ///
    /// Guarded on `deleted_at IS NULL`: a soft-deleted todo is not addressable,
    /// and a view cannot protect an UPDATE.
    async fn set_status(&self, id: &str, status: &str) -> Result<TodoStatusSet> {
        let pool = open(&self.account).await?;
        let now = chrono::Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE todos SET status = ?1, updated_at = ?2 \
             WHERE id = ?3 AND deleted_at IS NULL",
        )
        .bind(status)
        .bind(&now)
        .bind(id)
        .execute(&pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(AgentError::InvalidArgument(format!(
                "no todo with id '{id}' in local database"
            )));
        }
        Ok(TodoStatusSet {
            id: id.to_string(),
            status: status.to_string(),
        })
    }

    /// `delete <id>` (local): **soft**-delete a task by stamping `deleted_at`.
    ///
    /// The row is retained so the deletion is recoverable in principle and so
    /// the timeline can report it ([R024](../../../docs/adr/R024-local-provider-soft-delete.md)).
    /// Visible through no read path: they all query `active_todos`, which
    /// filters `deleted_at IS NULL`.
    ///
    /// SELECT the title first, then UPDATE; the read both lets the delete
    /// receipt carry the title and makes "already deleted" indistinguishable
    /// from "never existed" — both are `InvalidArgument`, because a
    /// soft-deleted row is not addressable.
    async fn delete(&self, id: &str) -> Result<TodoDeleted> {
        let pool = open(&self.account).await?;
        let row = sqlx::query("SELECT title FROM todos WHERE id = ?1 AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&pool)
            .await?;
        let row = row.ok_or_else(|| {
            AgentError::InvalidArgument(format!("no todo with id '{id}' in local database"))
        })?;
        let title: String = row.try_get("title").unwrap_or_default();
        let title = if title.is_empty() {
            format!("(untitled) {id}")
        } else {
            title
        };
        let now = chrono::Utc::now().to_rfc3339();
        let res =
            sqlx::query("UPDATE todos SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL")
                .bind(&now)
                .bind(id)
                .execute(&pool)
                .await?;
        if res.rows_affected() == 0 {
            return Err(AgentError::InvalidArgument(format!(
                "no todo with id '{id}' in local database"
            )));
        }
        Ok(TodoDeleted {
            id: id.to_string(),
            title,
            status: "deleted".to_string(),
        })
    }
}

// ============ Timeline data fetch ============

/// Raw todo entry data for timeline pulls.
pub struct TodoTimelineEntry {
    pub id: String,
    pub title: String,
    pub status: String,
    pub due: Option<String>,
    pub priority: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Timeline incremental pull: return todos whose `created_at` or `updated_at`
/// falls within the window.
///
/// Local provider degraded semantics: pulled from the current-state snapshot,
/// not the full transfer history [L001](../../../docs/adr/L001-append-only-event-log.md).
/// - newly added todo -> `created` event
/// - status-changed todo -> event mapped from current status (e.g. `completed`)
pub async fn fetch_for_timeline(
    account: &TodoAccount,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<TodoTimelineEntry>> {
    let pool = open(account).await?;
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    // created_at in window (newly created) or updated_at in window (status changed).
    let rows = sqlx::query(
        "SELECT id, title, status, due, priority, created_at, updated_at FROM active_todos \
         WHERE (created_at >= ?1 AND created_at <= ?2) \
            OR (updated_at >= ?1 AND updated_at <= ?2 AND updated_at != '') \
         ORDER BY created_at ASC",
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_all(&pool)
    .await?;

    let entries: Vec<TodoTimelineEntry> = rows
        .iter()
        .map(|r| TodoTimelineEntry {
            id: r.get("id"),
            title: r.get("title"),
            status: r.get("status"),
            due: r.get("due"),
            priority: r.get("priority"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect();
    Ok(entries)
}

// ============ Cross-module search (Phase 11) ============

/// Per-module hard cap, enforced inside the provider
/// ([S004](../../../docs/adr/S004-execution-model.md)).
const SEARCH_PER_MODULE_CAP: usize = 50;

/// Cross-module search (Phase 11): return todo hits whose `title` matches
/// the query (OR over tokens, case-insensitive GLOB).
///
/// `ts` is `updated_at` (UTC, RFC3339) — the module's primary edit time
/// ([S005](../../../docs/adr/S005-time-semantics-scope.md)); falls back to
/// `created_at` when `updated_at` is the default empty string (untouched
/// after add).
///
/// Search covers the account's local SQLite store (live-fetch-on-search rejected by
/// [S005](../../../docs/adr/S005-time-semantics-scope.md)).
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub async fn search_for_search(account: &TodoAccount, q: &SearchQuery) -> Result<Vec<Hit>> {
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
        params.push(format!("*{lower}*"));
        let idx = params.len();
        conds.push(format!("lower(title) GLOB ?{idx}"));
    }
    if conds.is_empty() {
        return Ok(Vec::new());
    }
    let where_clause = conds.join(" OR ");

    let cap = q.limit.unwrap_or(SEARCH_PER_MODULE_CAP);
    params.push(cap.to_string());
    let cap_idx = params.len();

    // ORDER BY ts DESC: prefer updated_at when set, otherwise created_at.
    // SQLite's CASE expression compares two columns and picks the larger
    // (lexicographic RFC3339 = chronological).
    let sql = format!(
        "SELECT id, title, status, created_at, updated_at FROM active_todos \
         WHERE {where_clause} \
         ORDER BY (CASE WHEN updated_at = '' THEN created_at ELSE updated_at END) DESC, id ASC \
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
            let title: String = r.get("title");
            let status: String = r.get("status");
            let created_at: String = r.get("created_at");
            let updated_at: String = r.get("updated_at");
            // ts prefers updated_at, falls back to created_at.
            let ts_str = if updated_at.is_empty() {
                created_at
            } else {
                updated_at
            };
            let ts = crate::util::datetime::parse_rfc3339(&ts_str);
            let snippet = status; // single-token snippet for todos
            Hit {
                module: "todo",
                account: Some(account.name.clone()),
                id,
                title,
                snippet,
                url: None,
                ts,
                kind: "task",
                // This module's own SQLite DB is the source of truth (F020).
                stale: false,
            }
        })
        .collect();
    Ok(hits)
}

/// Provider adapter: implements [`Searchable`] for one local todo account.
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub struct TodoSearchProvider {
    account: TodoAccount,
}

impl TodoSearchProvider {
    /// Construct from a configured local account.
    #[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
    pub fn new(account: TodoAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl Searchable for TodoSearchProvider {
    fn module_name(&self) -> &'static str {
        "todo"
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

    fn tmp_account() -> TodoAccount {
        let file = std::env::temp_dir().join(format!("everyday-todo-test-{}.db", gen_id()));
        TodoAccount {
            name: "test".into(),
            provider: "local".into(),
            db_path: Some(file.to_string_lossy().to_string()),
        }
    }

    #[tokio::test]
    async fn add_list_and_status_roundtrip() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        backend
            .add("写代码", Some("2026-07-15"), None)
            .await
            .unwrap();

        let pool = open(&acc).await.unwrap();
        let rows = sqlx::query("SELECT id, status FROM todos")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let id: String = rows[0].get("id");
        assert_eq!(rows[0].get::<String, _>("status"), STATUS_TODO);

        backend.set_status(&id, STATUS_DONE).await.unwrap();
        let status: String = sqlx::query("SELECT status FROM todos WHERE id = ?1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("status");
        assert_eq!(status, STATUS_DONE);

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn set_status_missing_id_errors() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        let err = backend.set_status("ghost", STATUS_DONE).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    /// Cross-module search (Phase 11): OR-of-tokens, GLOB on title, snippet
    /// carries the status, ts prefers updated_at over created_at.
    #[tokio::test]
    async fn search_for_search_matches_title_with_or() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        backend.add("Rust 重构", None, None).await.unwrap();
        backend.add("修复 cli 启动 bug", None, None).await.unwrap();
        backend.add("时间线文档", None, None).await.unwrap();

        // Single token "rust" — only the first todo.
        let q = SearchQuery::new("rust");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].module, "todo");
        assert!(hits[0].title.contains("Rust"));
        assert_eq!(hits[0].snippet, STATUS_TODO);

        // OR-of-tokens "rust cli" — two hits (Rust 重构 via title "rust",
        // 修复 cli 启动 bug via title "cli").
        let q = SearchQuery::new("rust cli");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 2);
        let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
        assert!(titles.iter().any(|t| t.contains("Rust")));
        assert!(titles.iter().any(|t| t.contains("cli")));

        // --limit override caps results.
        let mut q = SearchQuery::new("修复 时间线");
        q.limit = Some(1);
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert_eq!(hits.len(), 1);

        // Empty query → no hits.
        let q = SearchQuery::new("   ");
        let hits = search_for_search(&acc, &q).await.unwrap();
        assert!(hits.is_empty());

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    // ---- Soft delete (R024) ----

    /// Count rows directly in the table, bypassing every read guard. Used to
    /// assert the "row is retained" half of soft delete — the half a view
    /// cannot show.
    async fn raw_row_count(acc: &TodoAccount) -> i64 {
        let pool = open(acc).await.unwrap();
        sqlx::query("SELECT COUNT(*) AS n FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n")
    }

    /// The delete path had no test at all before this change (`todo delete` was
    /// never exercised), so this is also the regression net for it.
    #[tokio::test]
    async fn delete_hides_from_every_read_path_but_retains_the_row() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        let added = backend.add("要删掉的", None, None).await.unwrap();
        backend.add("保留的", None, None).await.unwrap();

        backend.delete(&added.id).await.unwrap();

        // Visible through no read path.
        let listed = backend.list(true).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "保留的");

        let q = SearchQuery::new("要删掉的");
        assert!(
            search_for_search(&acc, &q).await.unwrap().is_empty(),
            "deleted todo must not be searchable"
        );

        let from = chrono::Utc::now() - chrono::Duration::hours(1);
        let to = chrono::Utc::now() + chrono::Duration::hours(1);
        let entries = fetch_for_timeline(&acc, from, to).await.unwrap();
        assert!(
            entries.iter().all(|e| e.id != added.id),
            "deleted todo must not reach the timeline"
        );

        // ...but the row is still there.
        assert_eq!(
            raw_row_count(&acc).await,
            2,
            "soft delete retains the row; a hard DELETE would leave 1"
        );

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn deleted_todo_cannot_be_modified() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        let added = backend.add("任务", None, None).await.unwrap();
        backend.delete(&added.id).await.unwrap();

        let err = backend
            .set_status(&added.id, STATUS_DONE)
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    /// Deleting twice behaves like deleting a nonexistent id: the row is
    /// hidden, so it is "not found" rather than silently re-deleted.
    #[tokio::test]
    async fn deleting_twice_errors_like_a_missing_id() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        let added = backend.add("任务", None, None).await.unwrap();
        backend.delete(&added.id).await.unwrap();

        let err = backend.delete(&added.id).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn delete_missing_id_errors() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        let err = backend.delete("ghost").await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn delete_returns_title_for_the_event() {
        let acc = tmp_account();
        let backend = LocalTodoBackend::new(acc.clone());
        let added = backend.add("带标题的任务", None, None).await.unwrap();
        let deleted = backend.delete(&added.id).await.unwrap();
        assert_eq!(deleted.title, "带标题的任务");
        assert_eq!(deleted.id, added.id);
        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    /// Migration: a table created before `deleted_at` existed must gain the
    /// column on open. `memory/store.rs` is the counter-example — it declares
    /// the column only in CREATE TABLE and never migrates, so old databases
    /// silently lack it.
    #[tokio::test]
    async fn legacy_table_gains_deleted_at_column() {
        let acc = tmp_account();
        let path = acc.db_path.clone().unwrap();
        // Build the pre-soft-delete schema by hand.
        {
            let pool = connect(std::path::Path::new(&path)).await.unwrap();
            sqlx::query(
                "CREATE TABLE todos (\
                 id TEXT PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL, \
                 due TEXT, priority TEXT, created_at TEXT NOT NULL, \
                 updated_at TEXT NOT NULL DEFAULT '')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO todos (id, title, status, created_at, updated_at) \
                 VALUES ('legacy', '老任务', 'Todo', '2026-01-01T00:00:00Z', '')",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }

        // Opening through the backend migrates, and the legacy row is visible.
        let backend = LocalTodoBackend::new(acc.clone());
        let listed = backend.list(true).await.unwrap();
        assert_eq!(listed.len(), 1, "legacy row must survive the migration");
        assert_eq!(listed[0].title, "老任务");

        // And it can now be soft-deleted.
        backend.delete("legacy").await.unwrap();
        assert!(backend.list(true).await.unwrap().is_empty());
        assert_eq!(raw_row_count(&acc).await, 1);

        let _ = std::fs::remove_file(path);
    }
}
