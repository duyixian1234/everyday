//! Local SQLite provider for the `todo` module.
//!
//! Parity implementation of `list` / `add` / `start` / `complete` semantics
//! alongside the Notion provider [T001](../../docs/adr/T001-notion-todo-module.md),
//! with data persisted in the account's local SQLite file. `login` is a no-op
//! for the local provider (no credentials), and `init-db` only creates the
//! table and reports its path.
//!
//! Output shape (column names / JSON keys) is deliberately kept identical to
//! the Notion version in `todo.rs` [F005](../../docs/adr/F005-default-provider-local.md),
//! so an Agent can switch providers without changing its parsing logic.

use std::collections::HashMap;

use serde_json::{Value, json};
use sqlx::Row;

use crate::config::TodoAccount;
use crate::error::{AgentError, Result};
use crate::modules::local::{connect, mode_json, resolve_db_path};
use crate::output::Output;

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

/// `todo login` (local): the local provider needs no credentials.
pub fn login(account: &TodoAccount) -> Result<Output> {
    Ok(Output::text(format!(
        "todo account '{}' uses the local sqlite provider; no login required",
        account.name
    )))
}

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
}
