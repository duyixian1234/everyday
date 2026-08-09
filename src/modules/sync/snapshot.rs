//! Consistent snapshots and content hashes for sync (ADR D002).
//!
//! sqlx 0.8 defaults to WAL journal mode, so copying a `.db` file directly can
//! silently miss un-checkpointed WAL pages. Every SQLite file is snapshotted
//! with `VACUUM INTO` before hashing / upload — the snapshot is a consistent
//! whole-database copy that includes WAL content. `config.toml` is a plain
//! text file and is read directly (no snapshot needed).

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::sqlite::SqlitePoolOptions;

use crate::error::{AgentError, Result};

/// SQL-safe quote for a path in `VACUUM INTO '...'`: escape single quotes
/// and normalize backslashes to forward slashes (SQLite accepts both, and
/// forward slashes avoid string-escape ambiguity in the SQL literal).
fn sql_quote_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "''")
}

/// Create a consistent snapshot of a SQLite database via `VACUUM INTO`.
///
/// The snapshot is written to `<dir>/.sync-tmp/<rand>.db` and returned. The
/// caller owns the temp file (delete after upload). `VACUUM INTO` reads the
/// database including any pending WAL content and produces a single consistent
/// file (D002).
pub async fn snapshot_db(db_path: &Path, tmp_dir: &Path) -> Result<PathBuf> {
    if !db_path.exists() {
        return Err(AgentError::Io(format!(
            "snapshot source missing: {}",
            db_path.display()
        )));
    }
    std::fs::create_dir_all(tmp_dir)?;
    let tmp = tmp_dir.join(format!("{}.db", rand_suffix()));
    // VACUUM INTO creates the target file; it must not already exist.
    if tmp.exists() {
        std::fs::remove_file(&tmp)?;
    }
    let quoted = sql_quote_path(&tmp);
    let sql = format!("VACUUM INTO '{quoted}'");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(db_path))
        .await
        .map_err(|e| AgentError::Other(format!("open source db for snapshot: {e}")))?;
    sqlx::query(&sql)
        .execute(&pool)
        .await
        .map_err(|e| AgentError::Other(format!("VACUUM INTO snapshot failed: {e}")))?;
    pool.close().await;
    Ok(tmp)
}

/// SHA-256 hex digest of a file (streaming).
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| AgentError::Io(format!("open {} for hash: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| AgentError::Io(format!("read {} for hash: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// A unique-enough suffix for temp files: process id + high-resolution
/// timestamp (nanos since epoch). Not cryptographically random — collision
/// resistance across concurrent processes is all that matters here.
pub(crate) fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("sync-{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SQLite file with a known row, then snapshot and verify the row is
    /// present in the snapshot (covers the WAL-consistency guarantee).
    #[tokio::test]
    async fn snapshot_db_contains_committed_rows() {
        let tmp_dir = std::env::temp_dir().join(format!("everyday-snap-{}", rand_suffix()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let db_path = tmp_dir.join("source.db");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&db_path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t (v) VALUES ('wal-data')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let snap = snapshot_db(&db_path, &tmp_dir).await.unwrap();
        assert!(snap.exists());
        assert_ne!(snap, db_path);

        // Open the snapshot and check the row survived.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&snap))
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT v FROM t")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, "wal-data");
        pool.close().await;

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[tokio::test]
    async fn snapshot_missing_source_errors() {
        let tmp_dir = std::env::temp_dir().join(format!("everyday-snap-{}", rand_suffix()));
        let err = snapshot_db(&tmp_dir.join("nope.db"), &tmp_dir)
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "IoError");
    }

    #[test]
    fn sha256_file_is_deterministic_and_matches_known() {
        let tmp = std::env::temp_dir().join(format!("everyday-hash-{}", rand_suffix()));
        std::fs::write(&tmp, b"hello").unwrap();
        // sha256("hello") — independent check.
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert_eq!(sha256_file(&tmp).unwrap(), expected);
        // Re-read gives the same digest.
        assert_eq!(sha256_file(&tmp).unwrap(), expected);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn sql_quote_path_normalizes_backslashes_and_quotes() {
        let p = PathBuf::from(r"C:\Users\it's\everyday\a.db");
        assert_eq!(sql_quote_path(&p), "C:/Users/it''s/everyday/a.db");
    }
}
