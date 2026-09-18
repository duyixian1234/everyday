//! Calendar local event cache (Phase 27, ADR [C004](../../docs/adr/C004-cal-event-cache.md)).
//!
//! `cal` was the only remote-heavy module with no local cache: `mail` has
//! `mail_cache.db` ([M003](../../docs/adr/M003-envelope-cache.md)) and `rss`
//! has `rss-items.db`, but every `cal` call pulled CalDAV live. This cache
//! makes `cal list` and the `cal` search provider local-first, matching the
//! other six search providers.
//!
//! ## Snapshot replace, not append-only
//!
//! The mail cache is **append-only, never physically delete**
//! ([M003](../../docs/adr/M003-envelope-cache.md) §Retention K1). This cache
//! deliberately does the opposite: each refresh **replaces** a calendar's rows
//! wholesale.
//!
//! The reason is that calendar events are **mutable state**, not an append
//! log: they get rescheduled and deleted by the user. The timeline layer
//! already had to concede this ([L002](../../docs/adr/L002-calendar-window-refresh.md):
//! "Pure append is unusable for Calendar"). Copying K1 here would permanently
//! retain cancelled and rescheduled events — and unlike a stale mail envelope,
//! which is merely noise, a stale calendar row is **wrong information**: it
//! shows a meeting that no longer exists.
//!
//! A consequence worth stating: because replacement leaves no ghosts, this
//! cache needs no `gc` action. `mail gc` exists precisely because K1 *does*
//! produce ghosts ([M007](../../docs/adr/M007-mail-cache-gc.md)). If this
//! cache ever moves to incremental (ETag/sync-token) updates, ghosts become
//! possible and a `gc` would need to follow.
//!
//! ## Data model
//!
//! - `cal_events(account, calendar_href, uid, href, summary, location, start, end)`
//!   with PK `(account, calendar_href, uid)`: one row per event. The key is
//!   **UID**, not `href` — `cal add` derives the filename from a nanosecond
//!   timestamp, so a re-PUT changes the href while the UID is stable; keying
//!   on href would emit a phantom row on every edit.
//! - `cal_state(account, calendar_href, last_sync_at)` with PK
//!   `(account, calendar_href)`: per-calendar freshness watermark.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::{Executor, Row};

use crate::error::{AgentError, Result};

/// Cache freshness threshold, in seconds.
///
/// Hardcoded at 15 minutes and deliberately **not** exposed in config —
/// the same discipline the mail cache follows
/// ([M005](../../docs/adr/M005-staleness-auto-sync.md): "15 minutes is
/// hardcoded. Not exposed as a config.").
pub const STALENESS_THRESHOLD_SECS: i64 = 15 * 60;

const CREATE_CAL_EVENTS_SQL: &str = "CREATE TABLE IF NOT EXISTS cal_events (\
    account TEXT NOT NULL, \
    calendar_href TEXT NOT NULL, \
    uid TEXT NOT NULL, \
    href TEXT NOT NULL DEFAULT '', \
    summary TEXT NOT NULL DEFAULT '', \
    location TEXT NOT NULL DEFAULT '', \
    start TEXT NOT NULL DEFAULT '', \
    end TEXT NOT NULL DEFAULT '', \
    PRIMARY KEY (account, calendar_href, uid))";

const CREATE_CAL_STATE_SQL: &str = "CREATE TABLE IF NOT EXISTS cal_state (\
    account TEXT NOT NULL, \
    calendar_href TEXT NOT NULL, \
    last_sync_at TEXT NOT NULL, \
    PRIMARY KEY (account, calendar_href))";

const IX_CAL_EVENTS_ACCOUNT_SQL: &str =
    "CREATE INDEX IF NOT EXISTS ix_cal_events_account ON cal_events(account)";

/// One cached event row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedEvent {
    pub calendar_href: String,
    pub uid: String,
    pub href: String,
    pub summary: String,
    pub location: String,
    pub start: String,
    pub end: String,
}

/// Resolve the cal cache db path: `~/.config/everyday/cal-cache.db`.
fn cal_cache_db_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("cal-cache.db"))
}

/// Open the cal cache at an explicit path (creating if needed) and ensure the
/// tables exist. Used by [`open`] and by callers that point at a test db.
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
    pool.execute(CREATE_CAL_EVENTS_SQL).await?;
    pool.execute(CREATE_CAL_STATE_SQL).await?;
    pool.execute(IX_CAL_EVENTS_ACCOUNT_SQL).await?;
    Ok(pool)
}

/// Open the default cal cache (`~/.config/everyday/cal-cache.db`).
#[allow(dead_code)] // wired into the cached path in a later commit (t3/t4).
pub async fn open() -> Result<SqlitePool> {
    open_at(&cal_cache_db_path()?).await
}

/// Replace every cached row for one calendar with `events`, and stamp the
/// calendar's freshness watermark — in a single transaction.
///
/// Delete-then-insert **is** the snapshot semantics (see the module docs): a
/// cancelled or rescheduled event simply does not come back, so no ghost row
/// survives. Running both statements in one transaction means a failed refresh
/// leaves the previous snapshot intact rather than an empty calendar
/// ([M004](../../docs/adr/M004-uid-watermark-sync.md) writes rows and advances
/// the watermark together for the same reason).
#[allow(dead_code)] // wired into the cached path in a later commit (t3/t4).
pub async fn replace_calendar(
    pool: &SqlitePool,
    account: &str,
    calendar_href: &str,
    events: &[CachedEvent],
    synced_at: DateTime<Utc>,
) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM cal_events WHERE account = ?1 AND calendar_href = ?2")
        .bind(account)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    for e in events {
        sqlx::query(
            "INSERT OR REPLACE INTO cal_events \
             (account, calendar_href, uid, href, summary, location, start, end) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(account)
        .bind(calendar_href)
        .bind(&e.uid)
        .bind(&e.href)
        .bind(&e.summary)
        .bind(&e.location)
        .bind(&e.start)
        .bind(&e.end)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "INSERT OR REPLACE INTO cal_state (account, calendar_href, last_sync_at) \
         VALUES (?1, ?2, ?3)",
    )
    .bind(account)
    .bind(calendar_href)
    .bind(synced_at.to_rfc3339())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Read every cached event for an account, across all its calendars.
#[allow(dead_code)] // wired into the cached path in a later commit (t3/t4).
pub async fn query_events(pool: &SqlitePool, account: &str) -> Result<Vec<CachedEvent>> {
    let rows = sqlx::query(
        "SELECT calendar_href, uid, href, summary, location, start, end \
         FROM cal_events WHERE account = ?1",
    )
    .bind(account)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| CachedEvent {
            calendar_href: r.get("calendar_href"),
            uid: r.get("uid"),
            href: r.get("href"),
            summary: r.get("summary"),
            location: r.get("location"),
            start: r.get("start"),
            end: r.get("end"),
        })
        .collect())
}

/// The oldest `last_sync_at` across an account's calendars.
///
/// `None` when the account has never been synced — which is always stale, so a
/// fresh user gets a refresh on first query.
#[allow(dead_code)] // wired into the cached path in a later commit (t3/t4).
pub async fn oldest_sync(pool: &SqlitePool, account: &str) -> Result<Option<DateTime<Utc>>> {
    let row = sqlx::query("SELECT MIN(last_sync_at) AS oldest FROM cal_state WHERE account = ?1")
        .bind(account)
        .fetch_one(pool)
        .await?;
    let oldest: Option<String> = row.try_get("oldest").unwrap_or(None);
    Ok(oldest.and_then(|s| {
        DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }))
}

/// Is the cache stale for this account?
///
/// `None` (never synced) is always stale. Otherwise the cache is stale once the
/// oldest calendar's watermark is older than
/// [`STALENESS_THRESHOLD_SECS`].
#[allow(dead_code)] // wired into the cached path in a later commit (t3/t4).
pub async fn is_stale(pool: &SqlitePool, account: &str, now: DateTime<Utc>) -> Result<bool> {
    match oldest_sync(pool, account).await? {
        None => Ok(true),
        Some(last) => Ok(now - last > Duration::seconds(STALENESS_THRESHOLD_SECS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build a fresh test pool pointed at a temp db. Returns (pool, db_path).
    async fn tmp_pool() -> (SqlitePool, PathBuf) {
        let file = std::env::temp_dir().join(format!(
            "everyday-cal-cache-test-{}.db",
            crate::util::id::gen_id("cc")
        ));
        (open_at(&file).await.unwrap(), file)
    }

    fn ev(uid: &str, summary: &str) -> CachedEvent {
        CachedEvent {
            calendar_href: "/cal/work/".into(),
            uid: uid.into(),
            href: format!("/cal/work/{uid}.ics"),
            summary: summary.into(),
            location: "A 座".into(),
            start: "2026-08-05 10:00".into(),
            end: "2026-08-05 11:00".into(),
        }
    }

    #[tokio::test]
    async fn replace_then_query_returns_rows() {
        let (pool, path) = tmp_pool().await;
        let now = Utc::now();
        replace_calendar(&pool, "work", "/cal/work/", &[ev("a", "晨会")], now)
            .await
            .unwrap();

        let rows = query_events(&pool, "work").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uid, "a");
        assert_eq!(rows[0].summary, "晨会");
        std::fs::remove_file(path).ok();
    }

    /// The load-bearing property of snapshot replace: a rescheduled or
    /// cancelled event does **not** survive a refresh. This is exactly what an
    /// append-only cache (mail's K1) would get wrong.
    #[tokio::test]
    async fn snapshot_replace_leaves_no_ghost_rows() {
        let (pool, path) = tmp_pool().await;
        let now = Utc::now();
        replace_calendar(
            &pool,
            "work",
            "/cal/work/",
            &[ev("a", "晨会"), ev("b", "已取消的会议")],
            now,
        )
        .await
        .unwrap();
        assert_eq!(query_events(&pool, "work").await.unwrap().len(), 2);

        // Refresh where "b" was cancelled and "a" was rescheduled.
        let mut rescheduled = ev("a", "晨会（已改期）");
        rescheduled.start = "2026-08-05 15:00".into();
        replace_calendar(&pool, "work", "/cal/work/", &[rescheduled], now)
            .await
            .unwrap();

        let rows = query_events(&pool, "work").await.unwrap();
        assert_eq!(rows.len(), 1, "cancelled event must not survive");
        assert_eq!(rows[0].uid, "a");
        assert_eq!(rows[0].start, "2026-08-05 15:00", "reschedule must land");
        std::fs::remove_file(path).ok();
    }

    /// Re-keying on a changed href must not duplicate: the UID is the identity.
    #[tokio::test]
    async fn same_uid_new_href_does_not_duplicate() {
        let (pool, path) = tmp_pool().await;
        let now = Utc::now();
        replace_calendar(&pool, "work", "/cal/work/", &[ev("a", "晨会")], now)
            .await
            .unwrap();

        let mut moved = ev("a", "晨会");
        moved.href = "/cal/work/9999-deadbeef.ics".into();
        replace_calendar(&pool, "work", "/cal/work/", &[moved], now)
            .await
            .unwrap();

        let rows = query_events(&pool, "work").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].href, "/cal/work/9999-deadbeef.ics");
        std::fs::remove_file(path).ok();
    }

    /// Two calendars of one account are replaced independently.
    #[tokio::test]
    async fn replace_is_scoped_per_calendar() {
        let (pool, path) = tmp_pool().await;
        let now = Utc::now();
        replace_calendar(&pool, "work", "/cal/work/", &[ev("a", "晨会")], now)
            .await
            .unwrap();
        let mut personal = ev("p", "生日");
        personal.calendar_href = "/cal/personal/".into();
        replace_calendar(&pool, "work", "/cal/personal/", &[personal], now)
            .await
            .unwrap();

        // Refreshing the work calendar must not touch personal's rows.
        replace_calendar(&pool, "work", "/cal/work/", &[], now)
            .await
            .unwrap();

        let rows = query_events(&pool, "work").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uid, "p");
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn never_synced_is_stale() {
        let (pool, path) = tmp_pool().await;
        assert!(is_stale(&pool, "work", Utc::now()).await.unwrap());
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn fresh_sync_is_not_stale_and_old_sync_is() {
        let (pool, path) = tmp_pool().await;
        let now = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();

        replace_calendar(&pool, "work", "/cal/work/", &[ev("a", "晨会")], now)
            .await
            .unwrap();
        assert!(!is_stale(&pool, "work", now).await.unwrap());

        // One second under the threshold is still fresh.
        let just_under = now + Duration::seconds(STALENESS_THRESHOLD_SECS - 1);
        assert!(!is_stale(&pool, "work", just_under).await.unwrap());

        // Past the threshold it is stale.
        let past = now + Duration::seconds(STALENESS_THRESHOLD_SECS + 1);
        assert!(is_stale(&pool, "work", past).await.unwrap());
        std::fs::remove_file(path).ok();
    }

    /// The watermark is the **oldest** calendar's, so one stale calendar makes
    /// the whole account stale (a refresh covers all calendars anyway).
    #[tokio::test]
    async fn staleness_uses_oldest_calendar() {
        let (pool, path) = tmp_pool().await;
        let t0 = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
        let t1 = t0 + Duration::seconds(STALENESS_THRESHOLD_SECS * 2);

        replace_calendar(&pool, "work", "/cal/work/", &[ev("a", "晨会")], t0)
            .await
            .unwrap();
        let mut personal = ev("p", "生日");
        personal.calendar_href = "/cal/personal/".into();
        replace_calendar(&pool, "work", "/cal/personal/", &[personal], t1)
            .await
            .unwrap();

        // personal is fresh, but work is stale -> the account is stale.
        assert!(is_stale(&pool, "work", t1).await.unwrap());
        std::fs::remove_file(path).ok();
    }

    #[tokio::test]
    async fn accounts_are_isolated() {
        let (pool, path) = tmp_pool().await;
        let now = Utc::now();
        replace_calendar(&pool, "work", "/cal/work/", &[ev("a", "晨会")], now)
            .await
            .unwrap();
        replace_calendar(&pool, "home", "/cal/home/", &[ev("h", "家务")], now)
            .await
            .unwrap();

        assert_eq!(query_events(&pool, "work").await.unwrap().len(), 1);
        assert_eq!(query_events(&pool, "home").await.unwrap().len(), 1);
        assert_eq!(query_events(&pool, "work").await.unwrap()[0].uid, "a");
        std::fs::remove_file(path).ok();
    }
}
