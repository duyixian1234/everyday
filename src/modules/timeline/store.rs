//! Timeline 的 SQLite 存储层。
//!
//! 管理 `~/.config/everyday/timeline.db`，包含：
//! - `events` 表：不可变事件日志，自然键幂等去重。
//! - `sync_state` 表：各 (source, account) 的同步水位。
//!
//! 所有 timestamp 存 UTC RFC3339 字符串（字典序 = 时间序）。

use std::path::PathBuf;

use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::error::{AgentError, Result};
use crate::modules::timeline::{SyncMode, TimelineEvent};

// ============ SQL 常量 ============

const CREATE_EVENTS_SQL: &str = "CREATE TABLE IF NOT EXISTS events (\
    id TEXT PRIMARY KEY, \
    source TEXT NOT NULL, \
    account TEXT NOT NULL DEFAULT '', \
    event_type TEXT NOT NULL, \
    timestamp TEXT NOT NULL, \
    title TEXT NOT NULL, \
    summary TEXT NOT NULL DEFAULT '', \
    ref_id TEXT NOT NULL, \
    metadata TEXT NOT NULL DEFAULT '{}', \
    created_at TEXT NOT NULL)";

const CREATE_SYNC_STATE_SQL: &str = "CREATE TABLE IF NOT EXISTS sync_state (\
    source TEXT NOT NULL, \
    account TEXT NOT NULL DEFAULT '', \
    last_sync TEXT NOT NULL, \
    first_sync_done INTEGER NOT NULL DEFAULT 0, \
    PRIMARY KEY (source, account))";

const UX_EVENTS_NATURAL_SQL: &str = "CREATE UNIQUE INDEX IF NOT EXISTS ux_events_natural \
    ON events(source, account, ref_id, event_type, timestamp)";

const IX_EVENTS_TIME_SOURCE_SQL: &str = "CREATE INDEX IF NOT EXISTS ix_events_time_source \
    ON events(timestamp, source)";

// ============ 连接 ============

/// 解析 timeline.db 路径：`~/.config/everyday/timeline.db`。
fn timeline_db_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir.join("everyday").join("timeline.db"))
}

/// 打开（必要时创建）timeline.db 连接池，并确保表和索引存在。
pub async fn open() -> Result<SqlitePool> {
    let path = timeline_db_path()?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::query(CREATE_EVENTS_SQL).execute(&pool).await?;
    sqlx::query(CREATE_SYNC_STATE_SQL).execute(&pool).await?;
    sqlx::query(UX_EVENTS_NATURAL_SQL).execute(&pool).await?;
    sqlx::query(IX_EVENTS_TIME_SOURCE_SQL)
        .execute(&pool)
        .await?;
    Ok(pool)
}

// ============ 事件写入 ============

/// 批量写入事件。
///
/// - `SyncMode::Append`：`INSERT OR IGNORE`，靠自然键去重。
/// - `SyncMode::WindowRefresh`：先删除该窗口内同 source 的旧行，再插入。
pub async fn insert_events(
    pool: &SqlitePool,
    events: &[TimelineEvent],
    mode: SyncMode,
    window_from: DateTime<Utc>,
    window_to: DateTime<Utc>,
) -> Result<usize> {
    if events.is_empty() {
        // WindowRefresh 模式即使无新事件也要清理窗口内旧行。
        if mode == SyncMode::WindowRefresh {
            delete_window_events(pool, "cal", window_from, window_to).await?;
        }
        return Ok(0);
    }

    let now = Utc::now().to_rfc3339();

    if mode == SyncMode::WindowRefresh {
        let source = &events[0].source;
        delete_window_events(pool, source, window_from, window_to).await?;
    }

    let mut count = 0usize;
    for ev in events {
        let id = crate::util::id::gen_id("ev");
        let account = ev.account.as_deref();
        let metadata_str = serde_json::to_string(&ev.metadata).unwrap_or_else(|_| "{}".into());
        let ts = ev.timestamp.to_rfc3339();

        sqlx::query(
            "INSERT OR IGNORE INTO events \
             (id, source, account, event_type, timestamp, title, summary, ref_id, metadata, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(&id)
        .bind(&ev.source)
        .bind(account.unwrap_or(""))
        .bind(&ev.event_type)
        .bind(&ts)
        .bind(&ev.title)
        .bind(&ev.summary)
        .bind(&ev.ref_id)
        .bind(&metadata_str)
        .bind(&now)
        .execute(pool)
        .await?;
        count += 1;
    }
    Ok(count)
}

/// 删除指定 source 在时间窗口内的所有事件（WindowRefresh 用）。
async fn delete_window_events(
    pool: &SqlitePool,
    source: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<()> {
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    sqlx::query("DELETE FROM events WHERE source = ?1 AND timestamp BETWEEN ?2 AND ?3")
        .bind(source)
        .bind(&from_str)
        .bind(&to_str)
        .execute(pool)
        .await?;
    Ok(())
}

// ============ 事件查询 ============

/// 查询参数。
#[derive(Debug, Clone, Default)]
pub struct QueryParams {
    /// UTC 时间下限（含）。
    pub from: Option<DateTime<Utc>>,
    /// UTC 时间上限（含）。
    pub to: Option<DateTime<Utc>>,
    /// 来源过滤（IN 列表）。
    pub sources: Vec<String>,
    /// 账户过滤。
    pub account: Option<String>,
    /// 结果上限（0 = 无限制）。
    pub limit: usize,
}

/// 查询结果行。
#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub id: String,
    pub source: String,
    pub account: Option<String>,
    pub event_type: String,
    pub timestamp: String,
    pub title: String,
    pub summary: String,
    pub ref_id: String,
    pub metadata: Value,
}

/// 按条件查询事件，按 timestamp 降序。
pub async fn query_events(pool: &SqlitePool, params: &QueryParams) -> Result<Vec<EventRow>> {
    let mut sql = String::from(
        "SELECT id, source, account, event_type, timestamp, title, summary, ref_id, metadata \
         FROM events WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();
    let mut idx = 1usize;

    if let Some(from) = params.from {
        sql.push_str(&format!(" AND timestamp >= ?{idx}"));
        binds.push(from.to_rfc3339());
        idx += 1;
    }
    if let Some(to) = params.to {
        sql.push_str(&format!(" AND timestamp <= ?{idx}"));
        binds.push(to.to_rfc3339());
        idx += 1;
    }
    if !params.sources.is_empty() {
        let placeholders: Vec<String> = (0..params.sources.len())
            .map(|_| {
                let p = format!("?{idx}");
                idx += 1;
                p
            })
            .collect();
        sql.push_str(&format!(" AND source IN ({})", placeholders.join(",")));
        binds.extend(params.sources.iter().cloned());
    }
    if let Some(ref account) = params.account {
        sql.push_str(&format!(" AND account = ?{idx}"));
        binds.push(account.clone());
    }

    sql.push_str(" ORDER BY timestamp DESC");

    if params.limit > 0 {
        // LIMIT 接字面整数（SQLite + sqlx 可靠），不作为占位符绑定。
        sql.push_str(&format!(" LIMIT {}", params.limit));
    }

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }

    let rows = q.fetch_all(pool).await?;
    let result: Vec<EventRow> = rows
        .iter()
        .map(|r| EventRow {
            id: r.get("id"),
            source: r.get("source"),
            account: r.get("account"),
            event_type: r.get("event_type"),
            timestamp: r.get("timestamp"),
            title: r.get("title"),
            summary: r.get("summary"),
            ref_id: r.get("ref_id"),
            metadata: serde_json::from_str(&r.get::<String, _>("metadata")).unwrap_or(Value::Null),
        })
        .collect();
    Ok(result)
}

/// 将 EventRow 序列化为 JSON 数组。
pub fn rows_to_json(rows: &[EventRow]) -> Value {
    let arr: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "source": r.source,
                "account": r.account,
                "event_type": r.event_type,
                "timestamp": r.timestamp,
                "title": r.title,
                "summary": r.summary,
                "ref_id": r.ref_id,
                "metadata": r.metadata,
            })
        })
        .collect();
    Value::Array(arr)
}

/// 将 EventRow 渲染为表格行（time 本地化显示）。
pub fn rows_to_table_rows(rows: &[EventRow]) -> (Vec<String>, Vec<Vec<String>>) {
    let headers = vec![
        "time".to_string(),
        "source".to_string(),
        "type".to_string(),
        "title".to_string(),
    ];
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let local_time = parse_rfc3339(&r.timestamp)
                .map(|dt| dt.with_timezone(&Local).format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| r.timestamp.clone());
            vec![
                local_time,
                r.source.clone(),
                r.event_type.clone(),
                r.title.clone(),
            ]
        })
        .collect();
    (headers, table_rows)
}

/// 解析 RFC3339 时间字符串。
fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    crate::util::datetime::parse_rfc3339(s)
}

// ============ 水位管理 ============

/// 读取某 (source, account) 的水位。
/// 返回 None 表示尚未同步过。
pub async fn get_watermark(
    pool: &SqlitePool,
    source: &str,
    account: Option<&str>,
) -> Result<Option<Watermark>> {
    let row = sqlx::query(
        "SELECT last_sync, first_sync_done FROM sync_state \
         WHERE source = ?1 AND account = ?2",
    )
    .bind(source)
    .bind(account.unwrap_or(""))
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let last_sync_str: String = r.get("last_sync");
            let first_done: i64 = r.get("first_sync_done");
            let last_sync = parse_rfc3339(&last_sync_str);
            Ok(Some(Watermark {
                last_sync,
                first_sync_done: first_done != 0,
            }))
        }
        None => Ok(None),
    }
}

/// 水位信息。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Watermark {
    /// 上次同步时间（可能为 None 如果解析失败）。
    pub last_sync: Option<DateTime<Utc>>,
    /// 是否已完成首次同步。
    pub first_sync_done: bool,
}

/// 更新水位。
pub async fn set_watermark(
    pool: &SqlitePool,
    source: &str,
    account: Option<&str>,
    last_sync: DateTime<Utc>,
    first_sync_done: bool,
) -> Result<()> {
    let last_sync_str = last_sync.to_rfc3339();
    let first_flag = if first_sync_done { 1 } else { 0 };
    sqlx::query(
        "INSERT INTO sync_state (source, account, last_sync, first_sync_done) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(source, account) DO UPDATE SET \
         last_sync = excluded.last_sync, first_sync_done = excluded.first_sync_done",
    )
    .bind(source)
    .bind(account.unwrap_or(""))
    .bind(&last_sync_str)
    .bind(first_flag)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::timeline::{SyncMode, TimelineEvent};
    use chrono::Utc;
    use serde_json::json;

    async fn tmp_pool() -> SqlitePool {
        let file = std::env::temp_dir().join(format!(
            "everyday-timeline-test-{}.db",
            crate::util::id::gen_id("tl")
        ));
        let path = file.to_string_lossy().to_string();
        let opts = SqliteConnectOptions::new()
            .filename(&file)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(CREATE_EVENTS_SQL).execute(&pool).await.unwrap();
        sqlx::query(CREATE_SYNC_STATE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(UX_EVENTS_NATURAL_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(IX_EVENTS_TIME_SOURCE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let _ = path; // keep for cleanup
        pool
    }

    #[tokio::test]
    async fn insert_and_query_roundtrip() {
        let pool = tmp_pool().await;
        let now = Utc::now();
        let events = vec![
            TimelineEvent::new(
                "todo",
                Some("personal"),
                "created",
                now,
                "写周报",
                "",
                "t1",
                json!({"status": "Todo"}),
            ),
            TimelineEvent::new(
                "rss",
                None,
                "published",
                now,
                "Rust 1.95",
                "summary",
                "guid-1",
                json!({"feed": "hackernews"}),
            ),
        ];
        let n = insert_events(&pool, &events, SyncMode::Append, now, now)
            .await
            .unwrap();
        assert_eq!(n, 2);

        let rows = query_events(
            &pool,
            &QueryParams {
                limit: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn insert_dedup_by_natural_key() {
        let pool = tmp_pool().await;
        let now = Utc::now();
        let ev = TimelineEvent::new(
            "todo",
            Some("personal"),
            "created",
            now,
            "买咖啡",
            "",
            "t1",
            json!({}),
        );
        insert_events(&pool, std::slice::from_ref(&ev), SyncMode::Append, now, now)
            .await
            .unwrap();
        // 重复插入 → INSERT OR IGNORE，不增加行数。
        let n = insert_events(&pool, &[ev], SyncMode::Append, now, now)
            .await
            .unwrap();
        assert_eq!(n, 1); // 返回尝试插入的行数（SQL INSERT OR IGNORE 不报错）

        let rows = query_events(
            &pool,
            &QueryParams {
                limit: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1); // 实际只有 1 行
    }

    #[tokio::test]
    async fn window_refresh_deletes_old() {
        let pool = tmp_pool().await;
        let t1 = Utc::now() - chrono::Duration::hours(2);
        let t2 = Utc::now() - chrono::Duration::hours(1);

        let old = vec![TimelineEvent::new(
            "cal",
            Some("personal"),
            "scheduled",
            t1,
            "旧会议",
            "",
            "evt-old",
            json!({}),
        )];
        insert_events(&pool, &old, SyncMode::WindowRefresh, t1, t1)
            .await
            .unwrap();

        let new = vec![TimelineEvent::new(
            "cal",
            Some("personal"),
            "scheduled",
            t2,
            "新会议",
            "",
            "evt-new",
            json!({}),
        )];
        // 窗口覆盖 t1..t2，旧 t1 行会被删除。
        insert_events(&pool, &new, SyncMode::WindowRefresh, t1, t2)
            .await
            .unwrap();

        let rows = query_events(
            &pool,
            &QueryParams {
                sources: vec!["cal".into()],
                limit: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "新会议");
    }

    #[tokio::test]
    async fn watermark_roundtrip() {
        let pool = tmp_pool().await;
        let now = Utc::now();

        // 初始无水位。
        let wm = get_watermark(&pool, "mail", Some("work")).await.unwrap();
        assert!(wm.is_none());

        set_watermark(&pool, "mail", Some("work"), now, true)
            .await
            .unwrap();
        let wm = get_watermark(&pool, "mail", Some("work"))
            .await
            .unwrap()
            .unwrap();
        assert!(wm.first_sync_done);
        assert!(wm.last_sync.is_some());
    }

    #[tokio::test]
    async fn watermark_rss_null_account() {
        let pool = tmp_pool().await;
        let now = Utc::now();
        set_watermark(&pool, "rss", None, now, true).await.unwrap();
        let wm = get_watermark(&pool, "rss", None).await.unwrap().unwrap();
        assert!(wm.first_sync_done);
    }
}
