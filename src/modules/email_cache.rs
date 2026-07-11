//! mail 模块的本地 envelope 缓存层。
//!
//! 管理 `~/.config/everyday/mail_cache.db`，包含：
//! - `envelopes` 表：邮件摘要缓存，主键 `(account, folder, uid)`（IMAP UID folder-scoped）。
//! - `folder_state` 表：每文件夹水位元数据，主键 `(account, folder)`。
//!
//! 设计依据：`docs/adr/0011` (envelope 存储) + `0012` (UID 水位 + UIDVALIDITY) + `0013` (staleness)。
//!
//! 与 `timeline.db` / `ops-log.db` 完全独立。

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::error::{AgentError, Result};

// ============ 类型 ============

/// 单封邮件的 envelope 缓存行（主键 `(account, folder, uid)`）。
#[derive(Debug, Clone, Serialize)]
pub struct CachedEnvelope {
    pub account: String,
    pub folder: String,
    pub uid: u32,
    /// RFC3339 UTC 字符串（IMAP ENVELOPE.date 解析后转 UTC）。
    pub date: String,
    /// `mailbox@host`，已解码。
    pub from_addr: String,
    /// 已解码 MIME。
    pub subject: String,
    /// IMAP flags，空格分隔（`\Seen \Answered ...`）。
    pub flags: String,
    /// RFC 5322 Message-ID header，可能缺失。
    pub message_id: Option<String>,
    /// RFC822.SIZE in bytes。
    pub size: Option<i64>,
    /// 第一收件人 `mailbox@host`。
    pub to_addr: Option<String>,
    /// 本次写入时刻（RFC3339 UTC）。
    pub fetched_at: String,
}

/// 单文件夹水位（`folder_state` 行）。
#[derive(Debug, Clone)]
pub struct FolderState {
    pub uid_validity: u32,
    pub max_uid: u32,
    /// `None` 表示水位行存在但 `last_sync_at` 解析失败（DB 损坏 / 旧 schema），
    /// 按 stale 处理强制重新 sync；不再用 `Utc::now()` 静默掩盖（之前的实现
    /// 让"刚刚 sync 过"和"水位损坏"无法区分）。
    pub last_sync_at: Option<DateTime<Utc>>,
}

/// `mail list` 查询过滤条件。
#[derive(Debug, Clone, Default)]
pub struct EnvelopeQuery {
    /// `None` = 跨文件夹；`Some(name)` = 单文件夹。
    pub folder: Option<String>,
    /// true = `flags NOT LIKE '%\Seen%'`。
    pub unread_only: bool,
    /// `date >= since`。
    pub since: Option<DateTime<Utc>>,
    /// 截断条数；SQL 中字面拼接（与 timeline/store.rs::query_events 同处理）。
    pub limit: Option<usize>,
}

// ============ SQL 常量 ============

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

// ============ 连接 ============

fn mail_cache_db_path() -> Result<std::path::PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("mail_cache.db"))
}

/// 打开（必要时创建）mail_cache.db 连接池，并确保表和索引存在。
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

// ============ folder_state 操作 ============

/// 读单 folder 水位；不存在返回 `None`。
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

/// 事务：upsert envelopes + 更新水位 `max_uid` 与 `last_sync_at`。
/// 失败原子回滚——水位不会超前于实际 envelope（ADR 0012 强一致要求）。
///
/// `envelopes` 为空时仍更新 `last_sync_at`（`max_uid` 通过 `MAX()` 不变）。
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

/// UIDVALIDITY 失效处理：清空 folder 全部 envelope + 删水位行。
/// 下次 sync 将水位视为 0，回退全量 `UIDSEARCH UID 1:*`。
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

// ============ envelopes 查询 ============

/// 查询 envelopes：按 (account, 可选 folder, unread?, since?, limit) 过滤，全局按 `date DESC`。
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
        // IMAP \Seen 是已读标志；空格分隔的 flags 包含 \Seen 即已读。
        sql.push_str(" AND flags NOT LIKE '%\\Seen%'");
    }
    if let Some(since) = q.since {
        sql.push_str(" AND date >= ?");
        binds.push(since.to_rfc3339());
    }
    sql.push_str(" ORDER BY date DESC");
    if let Some(limit) = q.limit {
        // LIMIT 子句直接拼字面整数（与 timeline/store.rs 同处理；占位符在某些 sqlx 版本下对 LIMIT 不稳）。
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

// ============ 工具 ============

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    crate::util::datetime::parse_rfc3339(s)
}

/// staleness 阈值（ADR 0013：写死 15 分钟，无 flag / config）。
pub const STALENESS_THRESHOLD_SECS: i64 = 15 * 60;

/// folder 是否 stale（last_sync_at 距 now > 阈值）。边界：恰 = 阈值不 stale。
///
/// `last_sync_at == None`（DB 损坏 / 旧 schema / 解析失败）一律视为 stale，
/// 强制重新 sync；不再静默用 `Utc::now()` 兜底导致"刚 sync 过"和"水位损坏"
/// 表现一样。
pub fn is_stale(state: &FolderState, now: DateTime<Utc>) -> bool {
    match state.last_sync_at {
        None => true,
        Some(t) => (now - t).num_seconds() > STALENESS_THRESHOLD_SECS,
    }
}

// ============ 测试 ============

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
        // 恰 900 秒（阈值）— 不 stale（要求严格 >）
        let state = FolderState {
            uid_validity: 1,
            max_uid: 100,
            last_sync_at: Some(now - chrono::Duration::seconds(900)),
        };
        assert!(
            !is_stale(&state, now),
            "exactly at threshold should not be stale"
        );
        // 901 秒 — stale
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
        // chrono::DateTime::to_rfc3339 对 UTC 输出 "+00:00" 而非 "Z"，
        // 仅校验语义等价（同一时刻），不强求字面。
        let expected = chrono::DateTime::parse_from_rfc3339(s).unwrap();
        assert_eq!(dt, Some(expected.with_timezone(&chrono::Utc)));
    }

    #[test]
    fn parse_rfc3339_invalid_returns_none() {
        // 修复：之前 parse_rfc3339 失败时静默回退到 Utc::now()，
        // 让"刚 sync 过"和"水位损坏"无法区分。
        // 改为 None，由调用方决定如何处理（is_stale 直接视为 stale）。
        assert!(parse_rfc3339("not a date").is_none());
    }

    #[test]
    fn stale_when_last_sync_at_corrupt() {
        // None 强制 stale —— DB 损坏时不再被静默用 now() 掩盖。
        let now = Utc::now();
        let state = FolderState {
            uid_validity: 1,
            max_uid: 100,
            last_sync_at: None,
        };
        assert!(is_stale(&state, now));
    }

    // ============ SQL 集成测试 ============
    //
    // 用临时文件 SQLite 测事务原子性 / UIDVALIDITY 失效 / K1 ghost / flags 过滤。
    // 不依赖网络（仅 SQLite + sqlx）。

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    /// 创建临时 mail_cache 测试库（含 schema），返回连接池。
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
        // fetched_at 由 upsert 写入，应非空
        assert!(!rows[0].fetched_at.is_empty());
    }

    #[tokio::test]
    async fn upsert_empty_advances_last_sync_only() {
        let pool = tmp_pool().await;
        // 先写入一个 envelope 把水位抬到 100
        let envs = vec![sample_envelope(100, "\\Seen")];
        upsert_envelopes(&pool, "acc", "INBOX", 7, &envs)
            .await
            .unwrap();
        let before = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();

        // 空 batch 再 upsert：max_uid 不变，last_sync_at 前进
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
        // 首次写
        upsert_envelopes(&pool, "acc", "INBOX", 1, &[sample_envelope(50, "\\Seen")])
            .await
            .unwrap();
        // 再写同 (account, folder, uid) 但不同 subject/flags —— 应 UPDATE
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
        // 模拟 sync 路径：水位有效 → 检查 UIDVALIDITY → 不一致 → clear + 全量
        let pool = tmp_pool().await;
        upsert_envelopes(&pool, "acc", "INBOX", 100, &[sample_envelope(10, "")])
            .await
            .unwrap();

        // server 端重建文件夹 → UIDVALIDITY 变到 200
        let local = get_folder_state(&pool, "acc", "INBOX")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(local.uid_validity, 100);
        if local.uid_validity != 200 {
            clear_folder(&pool, "acc", "INBOX").await.unwrap();
        }
        // 重新全量
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
    async fn ghost_envelope_persists_after_k1_no_cleanup() {
        // K1: sync 只追加不删除。Server 端删除/移动的邮件，本地 envelope 仍留着。
        let pool = tmp_pool().await;
        // 模拟 server 端有 3 封邮件，本地全部缓存
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

        // 模拟下一次 sync：水位 3 → UIDSEARCH UID 4:* → 0 个新邮件
        // 但我们不主动清理 uid=1,2,3（K1）
        let next_max = upsert_envelopes(&pool, "acc", "INBOX", 1, &[])
            .await
            .unwrap();
        assert_eq!(next_max, 0, "empty batch returns 0 max_uid_in_batch");

        // 本地仍有 3 个 envelope（ghost）—— 默认 list 仍能列出
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
