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
//! Fully independent from `timeline.db` / `ops-log.db`.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

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
        let uid: i64 = r.get(2);
        let size: Option<i64> = r.get(8);
        out.push(CachedEnvelope {
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
        });
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
}
