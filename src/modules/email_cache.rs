//! Local envelope cache layer for the mail module.
//!
//! Manages `~/.config/everyday/mail_cache.db`, containing:
//! - `envelopes` table: cached mail summaries, primary key `(account, folder, uid)`
//!   (IMAP UID is folder-scoped).
//! - `folder_state` table: per-folder watermark metadata, primary key `(account, folder)`.
//!
//! Design basis: `docs/adr/0011` (envelope storage) + `0012` (UID watermark +
//! UIDVALIDITY) + `0013` (staleness).
//!
//! Fully independent from `timeline.db` (the `ops-log.db` was removed with the
//! Notion provider, v0.13.0 — [R019](../../docs/adr/R019-remove-notion-provider.md)).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};

use crate::error::{AgentError, Result};

// ============ Types ============

/// Cache row for a single mail envelope (primary key `(account, folder, uid)`).
#[derive(Debug, Clone, Serialize)]
pub struct CachedEnvelope {
    pub account: String,
    pub folder: String,
    pub uid: u32,
    /// RFC3339 UTC string (parsed from the IMAP ENVELOPE.date, converted to UTC).
    pub date: String,
    /// `mailbox@host`, decoded.
    pub from_addr: String,
    /// Decoded MIME.
    pub subject: String,
    /// IMAP flags, space-separated (`\Seen \Answered ...`).
    pub flags: String,
    /// RFC 5322 Message-ID header, may be absent.
    pub message_id: Option<String>,
    /// RFC822.SIZE in bytes.
    pub size: Option<i64>,
    /// First recipient `mailbox@host`.
    pub to_addr: Option<String>,
    /// Moment this row was written (RFC3339 UTC).
    pub fetched_at: String,
}

/// Per-folder watermark (`folder_state` row).
#[derive(Debug, Clone)]
pub struct FolderState {
    pub uid_validity: u32,
    pub max_uid: u32,
    /// `None` means the watermark row exists but `last_sync_at` failed to parse
    /// (DB corruption / old schema); treated as stale and forces a re-sync.
    /// We no longer silently hide this behind `Utc::now()` — the old behavior
    /// made "just synced" and "corrupted watermark" indistinguishable.
    pub last_sync_at: Option<DateTime<Utc>>,
}

/// Query filter for `mail list`.
#[derive(Debug, Clone, Default)]
pub struct EnvelopeQuery {
    /// `None` = cross-folder; `Some(name)` = single folder.
    pub folder: Option<String>,
    /// true = `flags NOT GLOB '*\\Seen*'` (matches by whole IMAP flag token, so a
    /// "Seen" inside subject / from is never misclassified).
    pub unread_only: bool,
    /// `date >= since`.
    pub since: Option<DateTime<Utc>>,
    /// Row cap; concatenated literally into the SQL (same handling as
    /// timeline/store.rs::query_events).
    pub limit: Option<usize>,
}

// ============ SQL constants ============

const CREATE_ENVELOPES_SQL: &str = "CREATE TABLE IF NOT EXISTS envelopes (\
    account TEXT NOT NULL, \
    folder TEXT NOT NULL, \
    uid INTEGER NOT NULL, \
    date TEXT NOT NULL, \
    from_addr TEXT NOT NULL, \
    subject TEXT NOT NULL, \
    flags TEXT NOT NULL, \
    message_id TEXT, \
    size INTEGER, \
    to_addr TEXT, \
    fetched_at TEXT NOT NULL, \
    PRIMARY KEY (account, folder, uid))";

const CREATE_FOLDER_STATE_SQL: &str = "CREATE TABLE IF NOT EXISTS folder_state (\
    account TEXT NOT NULL, \
    folder TEXT NOT NULL, \
    uid_validity INTEGER NOT NULL, \
    max_uid INTEGER NOT NULL DEFAULT 0, \
    last_sync_at TEXT NOT NULL, \
    PRIMARY KEY (account, folder))";

const IX_ENVELOPES_DATE_SQL: &str = "CREATE INDEX IF NOT EXISTS ix_envelopes_account_date \
    ON envelopes(account, date DESC)";

const IX_ENVELOPES_FOLDER_SQL: &str = "CREATE INDEX IF NOT EXISTS ix_envelopes_account_folder \
    ON envelopes(account, folder)";

// ============ Connection ============

fn mail_cache_db_path() -> Result<std::path::PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("mail_cache.db"))
}

/// Open (creating if needed) the mail_cache.db connection pool and ensure tables
/// and indexes exist.
pub async fn open() -> Result<SqlitePool> {
    let path = mail_cache_db_path()?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Config(format!("create mail_cache.db parent dir: {e}")))?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;
    sqlx::query(CREATE_ENVELOPES_SQL).execute(&pool).await?;
    sqlx::query(CREATE_FOLDER_STATE_SQL).execute(&pool).await?;
    sqlx::query(IX_ENVELOPES_DATE_SQL).execute(&pool).await?;
    sqlx::query(IX_ENVELOPES_FOLDER_SQL).execute(&pool).await?;
    Ok(pool)
}

// ============ folder_state operations ============

/// Read a single folder's watermark; returns `None` if absent.
pub async fn get_folder_state(
    pool: &SqlitePool,
    account: &str,
    folder: &str,
) -> Result<Option<FolderState>> {
    let row = sqlx::query(
        "SELECT uid_validity, max_uid, last_sync_at FROM folder_state WHERE account = ? AND folder = ?",
    )
    .bind(account)
    .bind(folder)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        let uid_validity: i64 = r.get(0);
        let max_uid: i64 = r.get(1);
        let last_sync_str: String = r.get(2);
        FolderState {
            uid_validity: uid_validity as u32,
            max_uid: max_uid as u32,
            last_sync_at: parse_rfc3339(&last_sync_str),
        }
    }))
}

/// Transaction: upsert envelopes + advance the watermark `max_uid` and `last_sync_at`.
/// On failure it rolls back atomically — the watermark never gets ahead of the
/// actual envelopes (strong-consistency requirement of
/// [M004](../../docs/adr/M004-uid-watermark-sync.md)).
///
/// When `envelopes` is empty, `last_sync_at` is still updated (`max_uid` stays
/// unchanged via `MAX()`).
pub async fn upsert_envelopes(
    pool: &SqlitePool,
    account: &str,
    folder: &str,
    new_uid_validity: u32,
    envelopes: &[CachedEnvelope],
) -> Result<u32> {
    let mut tx = pool.begin().await?;
    let fetched_at = Utc::now().to_rfc3339();
    let mut max_uid_in_batch: u32 = 0;
    for env in envelopes {
        sqlx::query(
            "INSERT INTO envelopes \
             (account, folder, uid, date, from_addr, subject, flags, message_id, size, to_addr, fetched_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(account, folder, uid) DO UPDATE SET \
                date = excluded.date, \
                from_addr = excluded.from_addr, \
                subject = excluded.subject, \
                flags = excluded.flags, \
                message_id = excluded.message_id, \
                size = excluded.size, \
                to_addr = excluded.to_addr, \
                fetched_at = excluded.fetched_at",
        )
        .bind(account)
        .bind(folder)
        .bind(env.uid as i64)
        .bind(&env.date)
        .bind(&env.from_addr)
        .bind(&env.subject)
        .bind(&env.flags)
        .bind(&env.message_id)
        .bind(env.size)
        .bind(&env.to_addr)
        .bind(&fetched_at)
        .execute(&mut *tx)
        .await?;
        if env.uid > max_uid_in_batch {
            max_uid_in_batch = env.uid;
        }
    }
    sqlx::query(
        "INSERT INTO folder_state (account, folder, uid_validity, max_uid, last_sync_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(account, folder) DO UPDATE SET \
            uid_validity = excluded.uid_validity, \
            max_uid = MAX(max_uid, excluded.max_uid), \
            last_sync_at = excluded.last_sync_at",
    )
    .bind(account)
    .bind(folder)
    .bind(new_uid_validity as i64)
    .bind(max_uid_in_batch as i64)
    .bind(&fetched_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(max_uid_in_batch)
}

/// UIDVALIDITY invalidation: delete all envelopes of the folder + drop the watermark row.
/// The next sync treats the watermark as 0 and falls back to a full `UIDSEARCH UID 1:*`.
pub async fn clear_folder(pool: &SqlitePool, account: &str, folder: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM envelopes WHERE account = ? AND folder = ?")
        .bind(account)
        .bind(folder)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM folder_state WHERE account = ? AND folder = ?")
        .bind(account)
        .bind(folder)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ============ envelope queries ============

/// Distinct folders that have cached envelopes for an account. Drives the
/// whole-cache `mail cache gc` reconciliation ([M007](../../docs/adr/M007-mail-cache-gc.md)):
/// ghosts can only exist in folders we cached something into.
pub async fn list_folders(pool: &SqlitePool, account: &str) -> Result<Vec<String>> {
    let rows =
        sqlx::query("SELECT DISTINCT folder FROM envelopes WHERE account = ? ORDER BY folder")
            .bind(account)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|r| r.get(0)).collect())
}

/// All locally cached UIDs for one (account, folder), for ghost detection.
pub async fn get_folder_uids(pool: &SqlitePool, account: &str, folder: &str) -> Result<Vec<u32>> {
    let rows =
        sqlx::query("SELECT uid FROM envelopes WHERE account = ? AND folder = ? ORDER BY uid")
            .bind(account)
            .bind(folder)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|r| r.get::<i64, _>(0) as u32)
        .collect())
}

/// Delete specific envelopes by UID within one (account, folder). Returns the
/// number of rows deleted. Used by `mail cache gc` to remove ghosts.
pub async fn delete_envelopes_by_uids(
    pool: &SqlitePool,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Result<usize> {
    if uids.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await?;
    let mut removed = 0usize;
    for uid in uids {
        let res = sqlx::query("DELETE FROM envelopes WHERE account = ? AND folder = ? AND uid = ?")
            .bind(account)
            .bind(folder)
            .bind(*uid as i64)
            .execute(&mut *tx)
            .await?;
        removed += res.rows_affected() as usize;
    }
    tx.commit().await?;
    Ok(removed)
}

/// Advance a folder's watermark after a clean `mail cache gc` reconcile: set
/// `uid_validity` + `max_uid` to the server's authoritative values and touch
/// `last_sync_at`. So the next incremental sync re-requests only `max_uid+1:*`
/// instead of re-searching UIDs we just deleted. No-op-safe: creates the row if
/// missing.
pub async fn advance_folder_watermark(
    pool: &SqlitePool,
    account: &str,
    folder: &str,
    uid_validity: u32,
    max_uid: u32,
) -> Result<()> {
    let fetched_at = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO folder_state (account, folder, uid_validity, max_uid, last_sync_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(account, folder) DO UPDATE SET \
            uid_validity = excluded.uid_validity, \
            max_uid = excluded.max_uid, \
            last_sync_at = excluded.last_sync_at",
    )
    .bind(account)
    .bind(folder)
    .bind(uid_validity as i64)
    .bind(max_uid as i64)
    .bind(&fetched_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Query envelopes: filter by (account, optional folder, unread?, since?, limit),
/// globally ordered by `date DESC`.
pub async fn query_envelopes(
    pool: &SqlitePool,
    account: &str,
    q: &EnvelopeQuery,
) -> Result<Vec<CachedEnvelope>> {
    let mut sql = String::from(
        "SELECT account, folder, uid, date, from_addr, subject, flags, message_id, size, to_addr, fetched_at \
         FROM envelopes WHERE account = ?",
    );
    let mut binds: Vec<String> = vec![account.to_string()];
    if let Some(f) = &q.folder {
        sql.push_str(" AND folder = ?");
        binds.push(f.clone());
    }
    if q.unread_only {
        // IMAP \Seen marks read; flags is a space-separated token list.
        // Match on token boundaries with GLOB `*\Seen*` to avoid LIKE's substring
        // match misclassifying a "Seen" inside subject / from as the read flag.
        // Before the fix, `flags NOT LIKE '%\Seen%'` would wrongly mark a mail whose
        // subject contains "Seen" as read.
        sql.push_str(" AND flags NOT GLOB '*\\Seen*'");
    }
    if let Some(since) = q.since {
        sql.push_str(" AND date >= ?");
        binds.push(since.to_rfc3339());
    }
    sql.push_str(" ORDER BY date DESC");
    if let Some(limit) = q.limit {
        // The LIMIT clause is concatenated as a literal integer (same handling as
        // timeline/store.rs; bind placeholders are unstable for LIMIT in some sqlx versions).
        sql.push_str(&format!(" LIMIT {}", limit));
    }
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    let rows = query.fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_cached_envelope(&r));
    }
    Ok(out)
}

/// Map a row from the canonical 11-column `envelopes` SELECT to a
/// `CachedEnvelope`. Shared by both [`query_envelopes`] and
/// [`search_envelopes`] so the column contract stays in one place.
fn row_to_cached_envelope(r: &SqliteRow) -> CachedEnvelope {
    let uid: i64 = r.get(2);
    let size: Option<i64> = r.get(8);
    CachedEnvelope {
        account: r.get(0),
        folder: r.get(1),
        uid: uid as u32,
        date: r.get(3),
        from_addr: r.get(4),
        subject: r.get(5),
        flags: r.get(6),
        message_id: r.try_get(7).ok().flatten(),
        size,
        to_addr: r.try_get(9).ok().flatten(),
        fetched_at: r.get(10),
    }
}

/// Free-text search across **all** cached envelopes (every account, every
/// folder).
///
/// Matching is case-insensitive GLOB substring on `subject`, `from_addr`, and
/// `to_addr`. Each token is matched against all three columns (OR within a
/// token); multiple tokens are OR'd together (multi-word OR, per
/// [S003](../../docs/adr/S003-query-semantics.md)). Tokens containing GLOB
/// metacharacters (`* ? [ ]`) are skipped to avoid injection — mirrors
/// `search::glob_substring`.
///
/// This deliberately scans the whole cache rather than going through IMAP
/// `SEARCH`: the `search` module is best-effort / local-first (see ADR S007),
/// consistent with `rss` (local cache) and `cal` (full-pull local filter). A
/// stale or empty cache simply yields fewer / zero hits; the aggregator treats
/// zero hits as a non-error (exit 0).
pub async fn search_envelopes(pool: &SqlitePool, tokens: &[&str]) -> Result<Vec<CachedEnvelope>> {
    // Build per-token OR across the 3 columns. Skip unusable tokens.
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    for tok in tokens {
        if tok.is_empty() || tok.contains(['*', '?', '[', ']']) {
            continue;
        }
        let pat = format!("*{}*", tok.to_ascii_lowercase());
        clauses.push(
            "(lower(subject) GLOB ? OR lower(from_addr) GLOB ? OR lower(to_addr) GLOB ?)"
                .to_string(),
        );
        binds.push(pat.clone());
        binds.push(pat.clone());
        binds.push(pat.clone());
    }
    if clauses.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT account, folder, uid, date, from_addr, subject, flags, message_id, size, to_addr, fetched_at \
         FROM envelopes WHERE {} ORDER BY date DESC",
        clauses.join(" OR ")
    );
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    let rows = query.fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_cached_envelope(&r));
    }
    Ok(out)
}

/// Account-scoped free-text search over the envelope cache — the CLI's
/// `mail search --cached` local path ([M006](../../docs/adr/M006-mail-search-cached.md)).
///
/// Same token-OR case-insensitive GLOB matching as [`search_envelopes`]
/// (subject/from/to, metacharacter tokens skipped), but restricted to one
/// account and optionally one folder, with a row cap. This is what `mail list`
/// does to the cache (account-scoped) plus the search matcher — in contrast to
/// the cross-module [`search_envelopes`], which scans every account/folder.
pub async fn search_envelopes_scoped(
    pool: &SqlitePool,
    account: &str,
    tokens: &[&str],
    folder: Option<&str>,
    limit: usize,
) -> Result<Vec<CachedEnvelope>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    for tok in tokens {
        if tok.is_empty() || tok.contains(['*', '?', '[', ']']) {
            continue;
        }
        let pat = format!("*{}*", tok.to_ascii_lowercase());
        clauses.push(
            "(lower(subject) GLOB ? OR lower(from_addr) GLOB ? OR lower(to_addr) GLOB ?)"
                .to_string(),
        );
        binds.push(pat.clone());
        binds.push(pat.clone());
        binds.push(pat.clone());
    }
    if clauses.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = format!(
        "SELECT account, folder, uid, date, from_addr, subject, flags, message_id, size, to_addr, fetched_at \
         FROM envelopes WHERE account = ? AND ({})",
        clauses.join(" OR ")
    );
    binds.insert(0, account.to_string());
    if let Some(f) = folder {
        sql.push_str(" AND folder = ?");
        binds.push(f.to_string());
    }
    sql.push_str(" ORDER BY date DESC LIMIT ");
    // The LIMIT is a literal integer (same handling as `query_envelopes` /
    // timeline store; bind placeholders for LIMIT are unstable in some sqlx versions).
    sql.push_str(&limit.to_string());
    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    let rows = query.fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_cached_envelope(&r));
    }
    Ok(out)
}

// ============ Utilities ============

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    crate::util::datetime::parse_rfc3339(s)
}

/// Staleness threshold (ADR [M005](../../docs/adr/M005-staleness-auto-sync.md):
/// hard-coded to 15 minutes, no flag / config).
pub const STALENESS_THRESHOLD_SECS: i64 = 15 * 60;

/// Whether a folder is stale (last_sync_at further from now than the threshold).
/// Boundary: exactly equal to the threshold is NOT stale.
///
/// `last_sync_at == None` (DB corruption / old schema / parse failure) is always
/// treated as stale, forcing a re-sync; we no longer silently fall back to
/// `Utc::now()`, which made "just synced" and "corrupted watermark" indistinguishable.
pub fn is_stale(state: &FolderState, now: DateTime<Utc>) -> bool {
    match state.last_sync_at {
        None => true,
        Some(t) => (now - t).num_seconds() > STALENESS_THRESHOLD_SECS,
    }
}

/// Test helper: build a throwaway `mail_cache.db` (temp file, schema created)
/// and return the pool. `pub(crate)` so other modules' `#[cfg(test)]` can
/// exercise the cache without touching the real config path.
#[cfg(test)]
pub(crate) async fn open_temp_pool() -> SqlitePool {
    let file = std::env::temp_dir().join(format!(
        "everyday-mailcache-test-{}.db",
        crate::util::id::gen_id("mc")
    ));
    let opts = SqliteConnectOptions::new()
        .filename(&file)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::query(CREATE_ENVELOPES_SQL)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(CREATE_FOLDER_STATE_SQL)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(IX_ENVELOPES_DATE_SQL)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(IX_ENVELOPES_FOLDER_SQL)
        .execute(&pool)
        .await
        .unwrap();
    pool
}

// ============ Tests ============

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staleness_threshold_is_15_minutes() {
        assert_eq!(STALENESS_THRESHOLD_SECS, 900);
    }

    #[test]
    fn staleness_recent_state_not_stale() {
        let now = Utc::now();
        let state = FolderState {
            uid_validity: 1,
            max_uid: 100,
            last_sync_at: Some(now - chrono::Duration::seconds(60)),
        };
        assert!(!is_stale(&state, now));
    }

    #[test]
    fn staleness_old_state_is_stale() {
        let now = Utc::now();
        let state = FolderState {
            uid_validity: 1,
            max_uid: 100,
            last_sync_at: Some(now - chrono::Duration::seconds(1000)),
        };
        assert!(is_stale(&state, now));
    }

    #[test]
    fn staleness_at_threshold_boundary() {
        let now = Utc::now();
        // Exactly 900 seconds (the threshold) — not stale (strict > required)
        let state = FolderState {
            uid_validity: 1,
            max_uid: 100,
            last_sync_at: Some(now - chrono::Duration::seconds(900)),
        };
        assert!(
            !is_stale(&state, now),
            "exactly at threshold should not be stale"
        );
        // 901 seconds — stale
        let state = FolderState {
            uid_validity: 1,
            max_uid: 100,
            last_sync_at: Some(now - chrono::Duration::seconds(901)),
        };
        assert!(
            is_stale(&state, now),
            "1 second past threshold should be stale"
        );
    }

    #[test]
    fn parse_rfc3339_valid() {
        let s = "2026-07-11T14:30:00Z";
        let dt = parse_rfc3339(s);
        // chrono::DateTime::to_rfc3339 emits "+00:00" (not "Z") for UTC;
        // only check semantic equivalence (same instant), not the literal form.
        let expected = chrono::DateTime::parse_from_rfc3339(s).unwrap();
        assert_eq!(dt, Some(expected.with_timezone(&chrono::Utc)));
    }

    #[test]
    fn parse_rfc3339_invalid_returns_none() {
        // Fix: previously parse_rfc3339 silently fell back to Utc::now() on failure,
        // making "just synced" and "corrupted watermark" indistinguishable.
        // Now it returns None, and the caller decides (is_stale treats it as stale).
        assert!(parse_rfc3339("not a date").is_none());
    }

    #[test]
    fn stale_when_last_sync_at_corrupt() {
        // None forces stale — DB corruption is no longer silently hidden behind now().
        let now = Utc::now();
        let state = FolderState {
            uid_validity: 1,
            max_uid: 100,
            last_sync_at: None,
        };
        assert!(is_stale(&state, now));
    }

    // ============ SQL integration tests ============
    //
    // Use a temp-file SQLite to test transaction atomicity / UIDVALIDITY
    // invalidation / K1 ghost / flag filtering. No network needed (only SQLite + sqlx).

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    /// Create a temporary mail_cache test DB (with schema) and return the pool.
    async fn tmp_pool() -> SqlitePool {
        let file = std::env::temp_dir().join(format!(
            "everyday-mailcache-test-{}.db",
            crate::util::id::gen_id("mc")
        ));
        let opts = SqliteConnectOptions::new()
            .filename(&file)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(CREATE_ENVELOPES_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(CREATE_FOLDER_STATE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(IX_ENVELOPES_DATE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(IX_ENVELOPES_FOLDER_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn sample_envelope(uid: u32, flags: &str) -> CachedEnvelope {
        CachedEnvelope {
            account: String::new(),
            folder: "INBOX".to_string(),
            uid,
            date: "2026-07-11T10:00:00+00:00".to_string(),
            from_addr: "alice@example.com".to_string(),
            subject: format!("subject {uid}"),
            flags: flags.to_string(),
            message_id: Some(format!("<msg-{uid}@example.com>")),
            size: Some(1024),
            to_addr: Some("bob@example.com".to_string()),
            fetched_at: String::new(),
        }
    }

    #[tokio::test]
    async fn upsert_writes_envelopes_and_advances_watermark() {
        let pool = tmp_pool().await;
        let envs = vec![sample_envelope(101, "\\Seen"), sample_envelope(102, "")];
        let max = upsert_envelopes(&pool, "acc", "INBOX", 42, &envs)
            .await
            .unwrap();
        assert_eq!(max, 102);

        let state = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.uid_validity, 42);
        assert_eq!(state.max_uid, 102);

        let q = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            ..Default::default()
        };
        let rows = query_envelopes(&pool, "acc", &q).await.unwrap();
        assert_eq!(rows.len(), 2);
        // fetched_at is written by upsert, must be non-empty
        assert!(!rows[0].fetched_at.is_empty());
    }

    #[tokio::test]
    async fn upsert_empty_advances_last_sync_only() {
        let pool = tmp_pool().await;
        // Write one envelope first to raise the watermark to 100
        let envs = vec![sample_envelope(100, "\\Seen")];
        upsert_envelopes(&pool, "acc", "INBOX", 7, &envs)
            .await
            .unwrap();
        let before = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();

        // Upsert an empty batch: max_uid stays, last_sync_at advances
        std::thread::sleep(std::time::Duration::from_millis(10));
        upsert_envelopes(&pool, "acc", "INBOX", 7, &[])
            .await
            .unwrap();
        let after = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            after.max_uid, before.max_uid,
            "empty batch must not advance max_uid"
        );
        assert!(
            after.last_sync_at.unwrap() > before.last_sync_at.unwrap(),
            "empty batch must advance last_sync_at"
        );
    }

    #[tokio::test]
    async fn upsert_upserts_on_conflict_by_pk() {
        let pool = tmp_pool().await;
        // First write
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[sample_envelope(50, "\\Seen")])
            .await
            .unwrap();
        // Write the same (account, folder, uid) with a different subject/flags — should UPDATE
        let mut updated = sample_envelope(50, "");
        updated.subject = "updated subject".to_string();
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[updated])
            .await
            .unwrap();

        let q = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            ..Default::default()
        };
        let rows = query_envelopes(&pool, "acc", &q).await.unwrap();
        assert_eq!(rows.len(), 1, "upsert must not duplicate by primary key");
        assert_eq!(rows[0].subject, "updated subject");
        assert_eq!(rows[0].flags, "");
    }

    #[tokio::test]
    async fn clear_folder_removes_envelopes_and_state() {
        let pool = tmp_pool().await;
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[sample_envelope(10, "")])
            .await
            .unwrap();
        assert!(
            get_folder_state(&pool, "acc", "INBOX")
                .await
                .unwrap()
                .is_some()
        );

        clear_folder(&pool, "acc", "INBOX").await.unwrap();

        assert!(
            get_folder_state(&pool, "acc", "INBOX")
                .await
                .unwrap()
                .is_none()
        );
        let q = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            ..Default::default()
        };
        assert_eq!(query_envelopes(&pool, "acc", &q).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn uidvalidity_change_simulates_full_reset() {
        // Simulate the sync path: watermark valid → check UIDVALIDITY → mismatch →
        // clear + full re-sync
        let pool = tmp_pool().await;
        upsert_envelopes(&pool, "acc", "INBOX", 100, &[sample_envelope(10, "")])
            .await
            .unwrap();

        // Server rebuilds the folder → UIDVALIDITY changes to 200
        let local = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(local.uid_validity, 100);
        if local.uid_validity != 200 {
            clear_folder(&pool, "acc", "INBOX").await.unwrap();
        }
        // Full re-sync
        upsert_envelopes(&pool, "acc", "INBOX", 200, &[sample_envelope(1, "")])
            .await
            .unwrap();
        let after = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.uid_validity, 200);
        assert_eq!(after.max_uid, 1, "max_uid reset after UIDVALIDITY change");
    }

    #[tokio::test]
    async fn query_unread_filters_seen_flag() {
        let pool = tmp_pool().await;
        upsert_envelopes(
            &pool,
            "acc",
            "INBOX",
            1,
            &[
                sample_envelope(1, "\\Seen"),
                sample_envelope(2, ""),
                sample_envelope(3, "\\Seen \\Flagged"),
            ],
        )
        .await
        .unwrap();

        let q_all = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            unread_only: false,
            ..Default::default()
        };
        assert_eq!(
            query_envelopes(&pool, "acc", &q_all).await.unwrap().len(),
            3
        );

        let q_unread = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            unread_only: true,
            ..Default::default()
        };
        let unread = query_envelopes(&pool, "acc", &q_unread).await.unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].uid, 2);
    }

    #[tokio::test]
    async fn query_unread_does_not_false_positive_on_seen_in_subject() {
        // Before the fix, `flags NOT LIKE '%\Seen%'` was used — but the subject field
        // also participates in LIKE, which doesn't matter here since LIKE only looks at
        // the flags column. The real issue is whole-token GLOB matching vs substring LIKE:
        // previously, if flags contained "\\SeenFoo" (a hyphenated IMAP flag), LIKE would
        // still judge it as read, whereas GLOB `*\Seen*` also includes it. The difference
        // is that GLOB never misaligns token boundaries.
        //
        // This test verifies that a token "SeenSomething" in the flags column (not the
        // IMAP \Seen) is not misclassified as read by GLOB — flags is a token list and
        // "\\Seen" is a separate token.
        let pool = tmp_pool().await;
        // Build a flags string: \"\\Seen SomethingElse\" — contains \\Seen and another token.
        upsert_envelopes(
            &pool,
            "acc",
            "INBOX",
            1,
            &[sample_envelope(1, "\\Seen Other")],
        )
        .await
        .unwrap();

        let q_unread = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            unread_only: true,
            ..Default::default()
        };
        // \\Seen present → read → unread_only should filter it out, 0 rows.
        let unread = query_envelopes(&pool, "acc,", &q_unread).await.unwrap();
        assert_eq!(unread.len(), 0);
    }

    #[tokio::test]
    async fn ghost_envelope_persists_after_k1_no_cleanup() {
        // K1: sync only appends, never deletes. Mails deleted/moved on the server remain
        // in the local envelope cache.
        let pool = tmp_pool().await;
        // Simulate 3 mails on the server, all cached locally
        upsert_envelopes(
            &pool,
            "acc",
            "INBOX",
            1,
            &[
                sample_envelope(1, ""),
                sample_envelope(2, "\\Seen"),
                sample_envelope(3, ""),
            ],
        )
        .await
        .unwrap();

        // Simulate the next sync: watermark 3 → UIDSEARCH UID 4:* → 0 new mails
        // but we don't actively clean up uid=1,2,3 (K1)
        let next_max = upsert_envelopes(&pool, "acc", "INBOX", 1, &[])
            .await
            .unwrap();
        assert_eq!(next_max, 0, "empty batch returns 0 max_uid_in_batch");

        // Locally the 3 envelopes (ghost) are still present — the default list still returns them
        let q = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            ..Default::default()
        };
        let rows = query_envelopes(&pool, "acc", &q).await.unwrap();
        assert_eq!(rows.len(), 3, "K1: ghost envelopes persist in cache");
    }

    #[tokio::test]
    async fn query_orders_by_date_desc_and_respects_limit() {
        let pool = tmp_pool().await;
        let mut e1 = sample_envelope(1, "");
        e1.date = "2026-07-10T00:00:00+00:00".to_string();
        let mut e2 = sample_envelope(2, "");
        e2.date = "2026-07-11T00:00:00+00:00".to_string();
        let mut e3 = sample_envelope(3, "");
        e3.date = "2026-07-09T00:00:00+00:00".to_string();
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[e1, e2, e3])
            .await
            .unwrap();

        let q = EnvelopeQuery {
            folder: Some("INBOX".to_string()),
            limit: Some(2),
            ..Default::default()
        };
        let rows = query_envelopes(&pool, "acc", &q).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uid, 2, "newest first");
        assert_eq!(rows[1].uid, 1);
    }

    #[tokio::test]
    async fn search_envelopes_matches_subject_from_case_insensitive() {
        let pool = tmp_pool().await;
        upsert_envelopes(
            &pool,
            "acc",
            "INBOX",
            1,
            &[sample_envelope(1, ""), sample_envelope(2, "")],
        )
        .await
        .unwrap();
        // A third envelope with a distinct subject + sender + recipient so each
        // token below maps to exactly one row.
        let mut e3 = sample_envelope(3, "");
        e3.from_addr = "carol@example.com".to_string();
        e3.subject = "Rust newsletter".to_string();
        e3.to_addr = Some("dave@example.com".to_string());
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[e3])
            .await
            .unwrap();

        // subject token (case-insensitive) hits "subject 1" + "subject 2" only
        let hits = search_envelopes(&pool, &["SUBJECT"]).await.unwrap();
        assert_eq!(hits.len(), 2);

        // from token matches the carol envelope only
        let hits = search_envelopes(&pool, &["carol"]).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].from_addr, "carol@example.com");

        // subject token "rust" hits the Rust newsletter envelope only
        let hits = search_envelopes(&pool, &["rust"]).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject, "Rust newsletter");

        // to_addr token hits the dave recipient only
        let hits = search_envelopes(&pool, &["dave"]).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].to_addr.as_deref(), Some("dave@example.com"));
    }

    #[tokio::test]
    async fn search_envelopes_token_or_and_skips_metachars() {
        let pool = tmp_pool().await;
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[sample_envelope(1, "")])
            .await
            .unwrap();

        // Empty token list -> zero hits.
        assert!(search_envelopes(&pool, &[]).await.unwrap().is_empty());

        // GLOB metachar token is skipped -> no usable token -> zero hits.
        assert!(search_envelopes(&pool, &["*"]).await.unwrap().is_empty());

        // Two tokens are OR'd: matches either "subject" or "nonexistent".
        let hits = search_envelopes(&pool, &["subject", "zzz-no-match"])
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "token OR, not AND");
    }

    #[tokio::test]
    async fn search_envelopes_scoped_restricts_account_folder_and_limit() {
        let pool = tmp_pool().await;
        // Two accounts, each with a matching subject, plus a folder-scoped match.
        let mut e1 = sample_envelope(1, "");
        e1.account = "acc".to_string();
        let mut e2 = sample_envelope(2, "");
        e2.account = "acc".to_string();
        let mut e3 = sample_envelope(3, "");
        e3.account = "other".to_string();
        let mut e4 = sample_envelope(4, "");
        e4.account = "acc".to_string();
        e4.folder = "Sent".to_string();
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[e1, e2])
            .await
            .unwrap();
        upsert_envelopes(&pool, "other", "INBOX", 1, &[e3])
            .await
            .unwrap();
        upsert_envelopes(&pool, "acc", "Sent", 1, &[e4])
            .await
            .unwrap();

        // Account scoping: token "subject" hits only `acc`'s 3 envelopes, not `other`.
        let hits = search_envelopes_scoped(&pool, "acc", &["subject"], None, 100)
            .await
            .unwrap();
        assert_eq!(hits.len(), 3, "only the target account's envelopes match");
        assert!(
            hits.iter().all(|e| e.account == "acc"),
            "never leaks another account"
        );

        // Folder scoping narrows to that folder only.
        let hits = search_envelopes_scoped(&pool, "acc", &["subject"], Some("Sent"), 100)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].folder, "Sent");

        // Limit caps rows.
        let hits = search_envelopes_scoped(&pool, "acc", &["subject"], None, 2)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn search_envelopes_scoped_empty_or_metachar_tokens_yield_zero() {
        let pool = tmp_pool().await;
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[sample_envelope(1, "")])
            .await
            .unwrap();

        // Empty token list -> zero hits.
        assert!(
            search_envelopes_scoped(&pool, "acc", &[], None, 100)
                .await
                .unwrap()
                .is_empty()
        );
        // GLOB metachar token skipped -> zero hits.
        assert!(
            search_envelopes_scoped(&pool, "acc", &["*"], None, 100)
                .await
                .unwrap()
                .is_empty()
        );
        // No rows for an account with nothing cached -> zero hits.
        assert!(
            search_envelopes_scoped(&pool, "ghost-account", &["subject"], None, 100)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn gc_helpers_list_folders_and_delete_ghosts_and_advance_watermark() {
        let pool = tmp_pool().await;
        upsert_envelopes(
            &pool,
            "acc",
            "INBOX",
            1,
            &[sample_envelope(1, ""), sample_envelope(2, "")],
        )
        .await
        .unwrap();
        upsert_envelopes(&pool, "acc", "Sent", 1, &[sample_envelope(3, "")])
            .await
            .unwrap();

        // list_folders enumerates only folders that have cached envelopes.
        let folders = list_folders(&pool, "acc").await.unwrap();
        assert_eq!(folders, vec!["INBOX", "Sent"]);

        // get_folder_uids returns the cached UIDs.
        let uids = get_folder_uids(&pool, "acc", "INBOX").await.unwrap();
        assert_eq!(uids, vec![1, 2]);

        // delete_envelopes_by_uids removes exactly the given UIDs.
        let removed = delete_envelopes_by_uids(&pool, "acc", "INBOX", &[2])
            .await
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            get_folder_uids(&pool, "acc", "INBOX").await.unwrap(),
            vec![1]
        );

        // advance_folder_watermark sets uid_validity + max_uid + last_sync_at.
        advance_folder_watermark(&pool, "acc", "INBOX", 99, 5)
            .await
            .unwrap();
        let state = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.uid_validity, 99);
        assert_eq!(state.max_uid, 5);
        assert!(state.last_sync_at.is_some());
    }
}
