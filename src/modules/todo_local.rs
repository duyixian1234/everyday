//! Local SQLite provider for the `todo` module.
//!
//! Parity implementation of `list` / `add` / `start` / `complete` semantics
//! alongside the Notion provider [T001](../../docs/adr/T001-notion-todo-module.md),
//! with data persisted in the account's local SQLite file. The local provider needs
//! no credentials (credentials are owned by the `auth` module), and `init-db`
//! only creates the table and reports its path.
//!
//! Output shape (column names / JSON keys) is deliberately kept identical to
//! the Notion version in `todo.rs` [F005](../../docs/adr/F005-default-provider-local.md),
//! so an Agent can switch providers without changing its parsing logic.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};
use sqlx::Row;

use crate::config::{Config, TodoAccount};
use crate::error::{AgentError, Result};
use crate::modules::local::{connect, mode_json, resolve_db_path};
use crate::output::Output;
use crate::search::{Hit, SearchQuery, Searchable};

/// Status option names (kept identical to the Notion provider).
const STATUS_TODO: &str = "Todo";
pub const STATUS_IN_PROGRESS: &str = "In Progress";
pub const STATUS_DONE: &str = "Done";

/// Table creation SQL: the todos table (includes the `updated_at` column used for timeline incremental pulls).
const CREATE_SQL: &str = "CREATE TABLE IF NOT EXISTS todos (\
    id TEXT PRIMARY KEY, \
    title TEXT NOT NULL, \
    status TEXT NOT NULL, \
    due TEXT, \
    priority TEXT, \
    created_at TEXT NOT NULL, \
    updated_at TEXT NOT NULL DEFAULT '')";

/// Open a connection and ensure the table exists (including the `updated_at` column migration).
async fn open(account: &TodoAccount) -> Result<sqlx::SqlitePool> {
    let path = resolve_db_path("todo", &account.name, account.db_path.as_deref())?;
    let pool = connect(&path).await?;
    sqlx::query(CREATE_SQL).execute(&pool).await?;
    // Migration: older tables lack the `updated_at` column; add it idempotently.
    let has_col: Option<(i64,)> =
        sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info('todos') WHERE name='updated_at'")
            .fetch_optional(&pool)
            .await?;
    if let Some((count,)) = has_col
        && count == 0
    {
        sqlx::query("ALTER TABLE todos ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
            .execute(&pool)
            .await?;
    }
    Ok(pool)
}

/// Generate a short unique ID (todo prefix `t`; see [`crate::util::id::gen_id`]).
fn gen_id() -> String {
    crate::util::id::gen_id("t")
}

// ============ actions ============

/// `todo init-db` (local): create the table and report the database path.
pub async fn init_db(account: &TodoAccount) -> Result<Output> {
    let path = resolve_db_path("todo", &account.name, account.db_path.as_deref())?;
    let _ = open(account).await?;
    let path_str = path.to_string_lossy().to_string();
    if mode_json() {
        Ok(Output::Json(
            json!({ "account": account.name, "db_path": path_str, "provider": "local" }),
        ))
    } else {
        Ok(Output::text(format!(
            "initialized local todo database for account '{}'\n{}",
            account.name, path_str
        )))
    }
}

/// `todo list [--all]` (local): list tasks, filtering done by default, ordered by due ascending (nulls last).
pub async fn list(account: &TodoAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let pool = open(account).await?;
    let show_all = flags.contains_key("all");

    // Order by due ascending, nulls last; tie-break by created_at.
    let sql = if show_all {
        "SELECT id, title, status, due, priority FROM todos \
         ORDER BY (due IS NULL), due ASC, created_at ASC"
    } else {
        "SELECT id, title, status, due, priority FROM todos \
         WHERE status <> ?1 ORDER BY (due IS NULL), due ASC, created_at ASC"
    };

    let rows = if show_all {
        sqlx::query(sql).fetch_all(&pool).await?
    } else {
        sqlx::query(sql).bind(STATUS_DONE).fetch_all(&pool).await?
    };

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "title": r.get::<String, _>("title"),
                "status": r.get::<String, _>("status"),
                "due": r.get::<Option<String>, _>("due"),
                "priority": r.get::<Option<String>, _>("priority"),
            })
        })
        .collect();

    if mode_json() {
        Ok(Output::Json(Value::Array(items)))
    } else {
        let table_rows = rows
            .iter()
            .map(|r| {
                vec![
                    r.get::<String, _>("id"),
                    r.get::<String, _>("title"),
                    r.get::<String, _>("status"),
                    r.get::<Option<String>, _>("due").unwrap_or_default(),
                    r.get::<Option<String>, _>("priority").unwrap_or_default(),
                ]
            })
            .collect();
        Ok(Output::records(
            vec![
                "id".into(),
                "title".into(),
                "status".into(),
                "due".into(),
                "priority".into(),
            ],
            table_rows,
        ))
    }
}

/// `todo add --title T [--due DATE] [--priority P]` (local): create a new task.
pub async fn add(account: &TodoAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --title <title>".into()))?;
    let pool = open(account).await?;
    let id = gen_id();
    let due = flags.get("due").cloned();
    let priority = flags.get("priority").cloned();
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

    if mode_json() {
        Ok(Output::Json(
            json!({ "id": id, "title": title, "status": STATUS_TODO }),
        ))
    } else {
        Ok(Output::text(format!("added todo '{title}' (id={id})")))
    }
}

/// `todo start/complete <id>` (local): update the task status.
pub async fn set_status(
    account: &TodoAccount,
    id: Option<&String>,
    status: &str,
) -> Result<Output> {
    let id = id.ok_or_else(|| AgentError::InvalidArgument(format!("`{status}` requires <id>")))?;
    let pool = open(account).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let res = sqlx::query("UPDATE todos SET status = ?1, updated_at = ?2 WHERE id = ?3")
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
    if mode_json() {
        Ok(Output::Json(json!({ "id": id, "status": status })))
    } else {
        Ok(Output::text(format!("set todo {id} -> status '{status}'")))
    }
}

/// `todo delete <id>` (local): physically delete a task.
///
/// SELECT the title first, then DELETE; `rows_affected == 0` is treated as
/// "id not found" and reported as an error. The extra read lets the ops-log
/// delete event carry the title, matching the Notion version's convention
/// [T002](../../docs/adr/T002-todo-delete-action.md).
pub async fn delete(account: &TodoAccount, id: Option<&String>) -> Result<Output> {
    let id = id.ok_or_else(|| AgentError::InvalidArgument("`delete` requires <id>".into()))?;
    let pool = open(account).await?;
    let row = sqlx::query("SELECT title FROM todos WHERE id = ?1")
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
    let res = sqlx::query("DELETE FROM todos WHERE id = ?1")
        .bind(id)
        .execute(&pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AgentError::InvalidArgument(format!(
            "no todo with id '{id}' in local database"
        )));
    }
    if mode_json() {
        Ok(Output::Json(
            json!({ "id": id, "title": title, "status": "deleted" }),
        ))
    } else {
        Ok(Output::text(format!("deleted todo '{title}' (id={id})")))
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
/// not the full transfer history [L001](../../docs/adr/L001-append-only-event-log.md).
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
        "SELECT id, title, status, due, priority, created_at, updated_at FROM todos \
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
/// ([S004](../../docs/adr/S004-execution-model.md)).
const SEARCH_PER_MODULE_CAP: usize = 50;

/// Cross-module search (Phase 11): return todo hits whose `title` matches
/// the query (OR over tokens, case-insensitive GLOB).
///
/// `ts` is `updated_at` (UTC, RFC3339) — the module's primary edit time
/// ([S005](../../docs/adr/S005-time-semantics-scope.md)); falls back to
/// `created_at` when `updated_at` is the default empty string (untouched
/// after add).
///
/// Notion accounts are skipped in v1 (live-fetch-on-search rejected by
/// [S005](../../docs/adr/S005-time-semantics-scope.md)).
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
        "SELECT id, title, status, created_at, updated_at FROM todos \
         WHERE {where_clause} \
         ORDER BY (CASE WHEN updated_at = '' THEN created_at ELSE updated_at END) DESC, id ASC \
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
            parent_page_id: None,
            default_database_id: None,
            db_path: Some(file.to_string_lossy().to_string()),
        }
    }

    #[tokio::test]
    async fn add_list_and_status_roundtrip() {
        let acc = tmp_account();
        let mut flags = HashMap::new();
        flags.insert("title".into(), "写代码".into());
        flags.insert("due".into(), "2026-07-15".into());
        add(&acc, &flags).await.unwrap();

        let pool = open(&acc).await.unwrap();
        let rows = sqlx::query("SELECT id, status FROM todos")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let id: String = rows[0].get("id");
        assert_eq!(rows[0].get::<String, _>("status"), STATUS_TODO);

        set_status(&acc, Some(&id), STATUS_DONE).await.unwrap();
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
        let err = set_status(&acc, Some(&"ghost".to_string()), STATUS_DONE)
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    /// Cross-module search (Phase 11): OR-of-tokens, GLOB on title, snippet
    /// carries the status, ts prefers updated_at over created_at.
    #[tokio::test]
    async fn search_for_search_matches_title_with_or() {
        let acc = tmp_account();
        let mut f1 = HashMap::new();
        f1.insert("title".into(), "Rust 重构".into());
        add(&acc, &f1).await.unwrap();
        let mut f2 = HashMap::new();
        f2.insert("title".into(), "修复 cli 启动 bug".into());
        add(&acc, &f2).await.unwrap();
        let mut f3 = HashMap::new();
        f3.insert("title".into(), "时间线文档".into());
        add(&acc, &f3).await.unwrap();

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
}
