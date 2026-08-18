//! SQLite persistence for task executions and cron next-due state.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sqlx::Row;

use crate::error::{AgentError, Result};
use crate::modules::local::connect;

const CREATE_RUNS_SQL: &str = "CREATE TABLE IF NOT EXISTS task_runs (\
    id TEXT PRIMARY KEY,\
    task_name TEXT NOT NULL,\
    command TEXT NOT NULL,\
    args TEXT NOT NULL,\
    extra_args TEXT,\
    resolved_args TEXT NOT NULL,\
    allow_extra_args INTEGER NOT NULL,\
    timeout_secs INTEGER NOT NULL,\
    capture_output INTEGER NOT NULL,\
    cwd TEXT NOT NULL,\
    status TEXT NOT NULL,\
    exit_code INTEGER,\
    timed_out INTEGER NOT NULL,\
    stdout TEXT,\
    stderr TEXT,\
    started_at TEXT NOT NULL,\
    duration_ms INTEGER NOT NULL)";

const CREATE_RUNS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_task_runs_name ON task_runs(task_name, started_at DESC)";

const CREATE_SCHEDULE_SQL: &str = "CREATE TABLE IF NOT EXISTS task_schedule_state (\
    task_name TEXT PRIMARY KEY,\
    next_due_at TEXT NOT NULL)";

/// One durable task execution record.
#[derive(Debug, Clone, Serialize)]
pub struct TaskRunRecord {
    pub id: String,
    pub task_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub extra_args: Option<Vec<String>>,
    pub resolved_args: Vec<String>,
    pub allow_extra_args: bool,
    pub timeout_secs: u64,
    pub capture_output: bool,
    pub cwd: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub started_at: String,
    pub duration_ms: u64,
}

/// Single-connection task database.
#[derive(Clone)]
pub struct TaskStore {
    pool: sqlx::SqlitePool,
}

impl TaskStore {
    /// Open the fixed `~/.config/everyday/task.db`.
    pub async fn open_default() -> Result<Self> {
        Self::open_path(&task_db_path()?).await
    }

    /// Open an explicit path (used by tests).
    pub async fn open_path(path: &Path) -> Result<Self> {
        let pool = connect(path).await?;
        sqlx::query(CREATE_RUNS_SQL).execute(&pool).await?;
        sqlx::query(CREATE_RUNS_INDEX_SQL).execute(&pool).await?;
        sqlx::query(CREATE_SCHEDULE_SQL).execute(&pool).await?;
        Ok(Self { pool })
    }

    /// Persist one completed execution.
    pub async fn insert_run(&self, run: &TaskRunRecord) -> Result<()> {
        let args = serde_json::to_string(&run.args)?;
        let extra_args = run
            .extra_args
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let resolved_args = serde_json::to_string(&run.resolved_args)?;
        sqlx::query(
            "INSERT INTO task_runs (id, task_name, command, args, extra_args, resolved_args,\
             allow_extra_args, timeout_secs, capture_output, cwd, status, exit_code, timed_out,\
             stdout, stderr, started_at, duration_ms)\
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )
        .bind(&run.id)
        .bind(&run.task_name)
        .bind(&run.command)
        .bind(args)
        .bind(extra_args)
        .bind(resolved_args)
        .bind(run.allow_extra_args)
        .bind(i64::try_from(run.timeout_secs).unwrap_or(i64::MAX))
        .bind(run.capture_output)
        .bind(&run.cwd)
        .bind(&run.status)
        .bind(run.exit_code)
        .bind(run.timed_out)
        .bind(&run.stdout)
        .bind(&run.stderr)
        .bind(&run.started_at)
        .bind(i64::try_from(run.duration_ms).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Return newest runs for one task. `limit=0` means unlimited.
    pub async fn history(&self, task_name: &str, limit: usize) -> Result<Vec<TaskRunRecord>> {
        let rows = if limit == 0 {
            sqlx::query(
                "SELECT * FROM task_runs WHERE task_name = ?1 ORDER BY started_at DESC, id DESC",
            )
            .bind(task_name)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT * FROM task_runs WHERE task_name = ?1\
                 ORDER BY started_at DESC, id DESC LIMIT ?2",
            )
            .bind(task_name)
            .bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(row_to_run).collect()
    }

    /// Load a persisted UTC next-due timestamp.
    pub async fn next_due(&self, task_name: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
        let row = sqlx::query("SELECT next_due_at FROM task_schedule_state WHERE task_name = ?1")
            .bind(task_name)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| {
            let raw: String = r.get("next_due_at");
            chrono::DateTime::parse_from_rfc3339(&raw)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| AgentError::Other(format!("invalid task next_due_at `{raw}`: {e}")))
        })
        .transpose()
    }

    /// Insert or replace one task's UTC next-due timestamp.
    pub async fn set_next_due(
        &self,
        task_name: &str,
        next_due: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO task_schedule_state(task_name, next_due_at) VALUES (?1, ?2)\
             ON CONFLICT(task_name) DO UPDATE SET next_due_at = excluded.next_due_at",
        )
        .bind(task_name)
        .bind(next_due.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove obsolete cron state while retaining execution history.
    pub async fn clear_schedule(&self, task_name: &str) -> Result<()> {
        sqlx::query("DELETE FROM task_schedule_state WHERE task_name = ?1")
            .bind(task_name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Fixed task database path.
pub fn task_db_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("task.db"))
}

fn row_to_run(row: &sqlx::sqlite::SqliteRow) -> Result<TaskRunRecord> {
    let args_raw: String = row.get("args");
    let extra_raw: Option<String> = row.get("extra_args");
    let resolved_raw: String = row.get("resolved_args");
    Ok(TaskRunRecord {
        id: row.get("id"),
        task_name: row.get("task_name"),
        command: row.get("command"),
        args: serde_json::from_str(&args_raw)?,
        extra_args: extra_raw
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?,
        resolved_args: serde_json::from_str(&resolved_raw)?,
        allow_extra_args: row.get("allow_extra_args"),
        timeout_secs: u64::try_from(row.get::<i64, _>("timeout_secs")).unwrap_or(0),
        capture_output: row.get("capture_output"),
        cwd: row.get("cwd"),
        status: row.get("status"),
        exit_code: row.get("exit_code"),
        timed_out: row.get("timed_out"),
        stdout: row.get("stdout"),
        stderr: row.get("stderr"),
        started_at: row.get("started_at"),
        duration_ms: u64::try_from(row.get::<i64, _>("duration_ms")).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("task-store-{}-{name}.db", std::process::id()))
    }

    #[tokio::test]
    async fn insert_and_history_round_trip() {
        let path = test_path("history");
        let _ = std::fs::remove_file(&path);
        let store = TaskStore::open_path(&path).await.unwrap();
        let run = TaskRunRecord {
            id: "tk-test-1".into(),
            task_name: "build".into(),
            command: "tool".into(),
            args: vec!["a".into()],
            extra_args: Some(vec!["b".into()]),
            resolved_args: vec!["a".into(), "b".into()],
            allow_extra_args: true,
            timeout_secs: 60,
            capture_output: true,
            cwd: "cwd".into(),
            status: "success".into(),
            exit_code: Some(0),
            timed_out: false,
            stdout: Some("out".into()),
            stderr: Some("err".into()),
            started_at: "2026-08-18T00:00:00Z".into(),
            duration_ms: 12,
        };
        store.insert_run(&run).await.unwrap();
        let rows = store.history("build", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].resolved_args, vec!["a", "b"]);
        assert_eq!(rows[0].stdout.as_deref(), Some("out"));
        drop(store);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn schedule_state_upserts_and_clears() {
        let path = test_path("schedule");
        let _ = std::fs::remove_file(&path);
        let store = TaskStore::open_path(&path).await.unwrap();
        let due = chrono::DateTime::parse_from_rfc3339("2026-08-18T01:02:03Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        store.set_next_due("x", due).await.unwrap();
        assert_eq!(store.next_due("x").await.unwrap(), Some(due));
        store.clear_schedule("x").await.unwrap();
        assert!(store.next_due("x").await.unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_file(&path);
    }
}
