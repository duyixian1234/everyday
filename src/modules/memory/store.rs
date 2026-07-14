//! Storage layer for the `memory` module.
//!
//! Owns the SQLite schema, the single-instance path resolution, and the
//! connection helper. Per [K001](../../../docs/adr/K001-memory-module.md)
//! and [K004](../../../docs/adr/K004-memory-single-instance.md) the file
//! is `~/.config/everyday/memory.db`, no `account` column, shared across
//! all CLI invocations.
//!
//! Schema model: append-only versions of `(subject, predicate, object)`
//! triples; soft delete via `deleted_at`; current state derived via the
//! `current_state_view` window-function view (SQLite 3.25+).
//!
//! Indexes:
//! - `ix_memory_spo_created`  covers current-state lookup by `(s,p,o)` with
//!   `created_at DESC` for the partition order.
//! - `ix_memory_subject_created` powers `memory get <SUBJECT>` and the
//!   per-level BFS query in `graph`.
//! - `ix_memory_subject_predicate` powers `memory relation <S> <P>`.
//! - `ix_memory_created_at` powers `memory list` ordering.
//!
//! `history` does a full-table scan filtered by `(s,p,o)` — no index
//! needed since the predicate is unique per triple and the row count is
//! small for any single triple (typically 1-5 versions).

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::error::{AgentError, Result};

/// Table name (single source of truth).
pub const TABLE: &str = "memory";

/// Current-state view name (`MAX(created_at) WHERE deleted_at IS NULL`).
pub const VIEW_CURRENT: &str = "current_state_view";

/// Resolve the single-instance memory database path.
///
/// Per [K004](../../../docs/adr/K004-memory-single-instance.md) there is
/// exactly one `~/.config/everyday/memory.db` for the whole CLI. No
/// per-account override is exposed.
pub fn resolve_db_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("memory.db"))
}

/// Create the schema if it does not yet exist.
const CREATE_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS memory (\
    id TEXT PRIMARY KEY, \
    subject TEXT NOT NULL, \
    predicate TEXT NOT NULL, \
    object TEXT NOT NULL, \
    confidence REAL NOT NULL DEFAULT 1.0, \
    source TEXT, \
    created_at TEXT NOT NULL, \
    deleted_at TEXT)";

const CREATE_INDEX_SQLS: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS ix_memory_spo_created \
        ON memory(subject, predicate, object, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS ix_memory_subject_created \
        ON memory(subject, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS ix_memory_subject_predicate \
        ON memory(subject, predicate, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS ix_memory_created_at \
        ON memory(created_at DESC)",
];

/// Current-state view: the row with the latest `created_at` per
/// `(subject, predicate, object)` where `deleted_at IS NULL`. Uses the
/// `ROW_NUMBER() OVER (PARTITION BY ...)` window function (SQLite 3.25+).
///
/// This view is consumed by `memory get`, `memory relation`, `memory list`,
/// `memory graph`, and the `MemorySearchProvider` — the single source of
/// truth for "what is current".
const CREATE_VIEW_SQL: &str = "CREATE VIEW IF NOT EXISTS current_state_view AS \
    SELECT * FROM ( \
        SELECT \
            id, subject, predicate, object, confidence, source, created_at, deleted_at, \
            ROW_NUMBER() OVER ( \
                PARTITION BY subject, predicate, object \
                ORDER BY created_at DESC \
            ) AS rn \
        FROM memory \
        WHERE deleted_at IS NULL \
    ) WHERE rn = 1";

/// Open (creating if needed) the SQLite connection pool.
///
/// Mirrors `crate::modules::local::connect`: `create_if_missing(true)` +
/// `max_connections(1)` (CLI is short-lived, single connection suffices
/// and avoids SQLite write-concurrency locks).
pub async fn open() -> Result<SqlitePool> {
    let path = resolve_db_path()?;
    open_at(&path).await
}

/// Open the memory database at an explicit path. Used by tests that point
/// at a temp file. Always ensures schema and view exist.
pub async fn open_at(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::query(CREATE_TABLE_SQL).execute(&pool).await?;
    for ddl in CREATE_INDEX_SQLS {
        sqlx::query(ddl).execute(&pool).await?;
    }
    sqlx::query(CREATE_VIEW_SQL).execute(&pool).await?;
    Ok(pool)
}

/// Generate a short unique row ID with prefix `m`. See
/// [`crate::util::id::gen_id`] for uniqueness guarantees.
pub fn gen_id() -> String {
    crate::util::id::gen_id("m")
}

/// Validate the `--confidence` range. Returns the parsed `f64` on success.
///
/// Confidence must be a finite value in `[0.0, 1.0]`. Out of range or
/// non-numeric input is `AgentError::InvalidArgument` (per [K001](../../../docs/adr/K001-memory-module.md)
/// errors table).
pub fn parse_confidence(raw: &str) -> Result<f64> {
    let v: f64 = raw
        .parse()
        .map_err(|_| AgentError::InvalidArgument(format!("invalid --confidence '{raw}'")))?;
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err(AgentError::InvalidArgument(format!(
            "--confidence must be in [0.0, 1.0], got {raw}"
        )));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_db_path_is_everyday_memory_db() {
        let p = resolve_db_path().unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("everyday"));
        assert!(s.ends_with("memory.db"));
    }

    #[test]
    fn parse_confidence_accepts_in_range() {
        assert_eq!(parse_confidence("0").unwrap(), 0.0);
        assert_eq!(parse_confidence("1").unwrap(), 1.0);
        assert!((parse_confidence("0.5").unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parse_confidence_rejects_out_of_range() {
        assert!(parse_confidence("-0.1").is_err());
        assert!(parse_confidence("1.1").is_err());
        assert!(parse_confidence("abc").is_err());
    }

    #[tokio::test]
    async fn open_at_creates_schema_and_view() {
        let dir = std::env::temp_dir().join(format!("everyday-mem-test-{}", gen_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.db");
        let pool = open_at(&path).await.unwrap();

        // Table exists.
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, 1);

        // View exists.
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name='current_state_view'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, 1);

        // 4 indexes. (SQLite may also create an implicit index for the
        // PRIMARY KEY; on this SQLite version that row is not exposed via
        // sqlite_master as a separate index entry, so the count stays at
        // exactly 4. Use >= for forward-compat in case SQLite ever changes.)
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='memory'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(row.0 >= 4, "expected at least 4 indexes, got {}", row.0);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[tokio::test]
    async fn open_at_is_idempotent() {
        // Opening twice on the same path must not error (IF NOT EXISTS).
        let dir = std::env::temp_dir().join(format!("everyday-mem-test-{}", gen_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.db");
        let _ = open_at(&path).await.unwrap();
        let _ = open_at(&path).await.unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
