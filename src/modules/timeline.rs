//! Timeline 模块：统一事件存储与查询。
//!
//! CLI:
//! - `everyday timeline`（无 action）= `today`
//! - `everyday timeline today|yesterday|week|month`
//! - `everyday timeline sync [--source S] [--since DATE]`
//! - `everyday timeline --from DATE --to DATE [--source S] [--account A] [--limit N] [--sync]`
//!
//! 核心类型：
//! - [`TimelineEvent`]：统一事件结构（不可变记录）。
//! - [`TimelineProvider`]：各 source 的数据拉取 trait（无状态）。
//! - [`SyncMode`]：追加 vs 窗口刷新。
//! - [`TimeWindow`]：同步时间窗口。
//!
//! 子模块：
//! - [`store`]：timeline.db 读写。
//! - [`orchestrator`]：sync 编排器。
//! - [`providers`]：各 source 的 provider adapter。

pub mod orchestrator;
pub mod providers;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor, parse_simple_args};
use crate::output::Output;

// ============ 核心类型 ============

/// 统一事件结构（不可变记录）。
///
/// 自然键：`(source, COALESCE(account, ''), ref_id, event_type, timestamp)`。
/// 用于同步幂等：相同窗口重复同步不产生重复行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    /// 来源模块：`mail` / `cal` / `rss` / `todo` / `note` / `bookmark`。
    pub source: String,
    /// 来源账户名（RSS 为 None）。
    pub account: Option<String>,
    /// 事件语义类型（如 `received` / `sent` / `created` / `completed` / `scheduled`）。
    pub event_type: String,
    /// 事件发生时刻（RFC3339 UTC）。
    pub timestamp: chrono::DateTime<Utc>,
    /// 事件标题。
    pub title: String,
    /// 事件摘要（可能为空）。
    pub summary: String,
    /// 事件引用的实体在来源系统中的稳定标识。
    pub ref_id: String,
    /// 结构化元数据（JSON 对象）。
    pub metadata: serde_json::Value,
}

impl TimelineEvent {
    /// 创建一个新事件，metadata 默认为空 JSON 对象。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: &str,
        account: Option<&str>,
        event_type: &str,
        timestamp: chrono::DateTime<Utc>,
        title: &str,
        summary: &str,
        ref_id: &str,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            source: source.to_string(),
            account: account.map(|s| s.to_string()),
            event_type: event_type.to_string(),
            timestamp,
            title: title.to_string(),
            summary: summary.to_string(),
            ref_id: ref_id.to_string(),
            metadata,
        }
    }
}

/// 同步时间窗口。
#[derive(Debug, Clone)]
pub struct TimeWindow {
    /// 窗口起始（UTC，含）。
    pub from: chrono::DateTime<Utc>,
    /// 窗口结束（UTC，含）。
    pub to: chrono::DateTime<Utc>,
}

impl TimeWindow {
    pub fn new(from: chrono::DateTime<Utc>, to: chrono::DateTime<Utc>) -> Self {
        Self { from, to }
    }
}

/// 同步模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// 幂等追加，靠自然键去重（`INSERT OR IGNORE`）。
    Append,
    /// 先删窗口内同 source 旧行，再插入当前快照（仅 `cal`）。
    WindowRefresh,
}

/// 各 source 的数据拉取 trait（无状态）。
///
/// Provider 只负责"给我一个窗口，我返回这个窗口内的事件快照与同步模式"。
/// 水位（last_sync）由编排器在 `sync_state` 表中管理。
#[async_trait]
pub trait TimelineProvider: Send + Sync {
    /// 来源标识（`"mail"` / `"cal"` / ...）。
    fn source(&self) -> &'static str;

    /// 账户名（RSS 等无账户概念返回 None）。
    fn account(&self) -> Option<&str>;

    /// 拉取指定时间窗口内的事件。
    ///
    /// 返回 `(事件列表, 同步模式)`。
    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)>;
}

/// 单个 provider 的同步结果（编排器收集用）。
#[derive(Debug)]
pub struct ProviderSyncResult {
    pub source: String,
    pub account: Option<String>,
    pub events_count: usize,
    pub status: ProviderStatus,
}

/// provider 同步状态。
#[derive(Debug)]
pub enum ProviderStatus {
    /// 成功。
    Ok,
    /// 失败（含错误消息）。
    Failed(String),
}

// ============ Executor 实现 ============

pub struct TimelineModule {
    config: Arc<Config>,
}

impl TimelineModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for TimelineModule {
    fn name(&self) -> &'static str {
        "timeline"
    }

    fn description(&self) -> &'static str {
        "Unified event timeline: query and sync across all sources."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new(
                "today",
                "Show today's events (default)",
                "everyday timeline today [--source S] [--account A] [--limit N] [--sync]",
            ),
            ActionDoc::new(
                "yesterday",
                "Show yesterday's events",
                "everyday timeline yesterday [--source S] [--account A] [--limit N]",
            ),
            ActionDoc::new(
                "week",
                "Show this week's events (Mon-Sun)",
                "everyday timeline week [--source S] [--account A] [--limit N]",
            ),
            ActionDoc::new(
                "month",
                "Show this month's events",
                "everyday timeline month [--source S] [--account A] [--limit N]",
            ),
            ActionDoc::new(
                "sync",
                "Sync events from all sources into timeline.db",
                "everyday timeline sync [--source mail,cal] [--since 2026-01-01]",
            ),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _positional) = parse_simple_args(args);
        let json_mode = crate::util::json_mode::is_json();

        match action {
            "sync" => self.do_sync(&flags).await,
            // 无 action 或预设动作 → 查询。
            "" | "today" | "yesterday" | "week" | "month" => {
                self.do_query(action, &flags, json_mode).await
            }
            other => Err(AgentError::UnknownAction(format!("timeline {other}"))),
        }
    }
}

impl TimelineModule {
    /// 执行 sync 子命令。
    async fn do_sync(&self, flags: &std::collections::HashMap<String, String>) -> Result<Output> {
        let sources = parse_source_filter(flags.get("source"));
        let since = flags.get("since").and_then(|s| parse_date_to_utc(s, false));

        let output = orchestrator::run_sync(&self.config, &sources, since).await?;
        Ok(output.to_output(crate::util::json_mode::is_json()))
    }

    /// 执行查询（预设或自定义范围）。
    async fn do_query(
        &self,
        preset: &str,
        flags: &std::collections::HashMap<String, String>,
        json_mode: bool,
    ) -> Result<Output> {
        // --sync：查询前先同步一次。
        if flags.contains_key("sync") {
            let sources = parse_source_filter(flags.get("source"));
            let _ = orchestrator::run_sync(&self.config, &sources, None).await;
        }

        // 解析时间范围。
        // 优先级:`--from` + `--to` > `--since` > preset。
        //
        // 三种独立路径:
        // - `--from` + `--to` 同时给:用它们 (NaiveDate 粒度,unambiguous)
        // - `--since` 单给:`parse_since_utc` 拿 UTC 起点,保留 sub-day 精度;
        //   to = now() (相对时长的自然终点)
        // - 都缺:纯 preset
        let (from_utc, to_utc) = if let (Some(f), Some(t)) = (flags.get("from"), flags.get("to")) {
            let f_d = parse_date_str(f)?;
            let t_d = parse_date_str(t)?;
            (local_to_utc_start(f_d), local_to_utc_end(t_d))
        } else if let Some(s) = flags.get("since") {
            let from = parse_since_utc(s)?;
            (from, Utc::now())
        } else {
            let (f_l, t_l) = resolve_preset(preset)?;
            (local_to_utc_start(f_l), local_to_utc_end(t_l))
        };

        // 构造查询参数。
        let sources = parse_source_filter(flags.get("source"));
        let account = flags.get("account").cloned();
        let limit: usize = flags
            .get("limit")
            .and_then(|s| s.parse().ok())
            .unwrap_or(100);

        let pool = store::open().await?;
        let params = store::QueryParams {
            from: Some(from_utc),
            to: Some(to_utc),
            sources,
            account,
            limit,
        };
        let rows = store::query_events(&pool, &params).await?;

        if rows.is_empty() {
            return if json_mode {
                Ok(Output::Json(serde_json::Value::Array(vec![])))
            } else {
                Ok(Output::text("no events"))
            };
        }

        if json_mode {
            Ok(Output::Json(store::rows_to_json(&rows)))
        } else {
            let (headers, table_rows) = store::rows_to_table_rows(&rows);
            Ok(Output::records(headers, table_rows))
        }
    }
}

// ============ 时间工具 ============

/// 解析 `--source mail,cal` 为 `["mail", "cal"]`。
fn parse_source_filter(raw: Option<&String>) -> Vec<String> {
    match raw {
        None => Vec::new(),
        Some(s) => s
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}

/// 解析日期字符串 `YYYY-MM-DD` 为 NaiveDate。
fn parse_date_str(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
        AgentError::InvalidArgument(format!("invalid date '{s}', expected YYYY-MM-DD"))
    })
}

/// 把本地日期的 00:00:00 转为 UTC DateTime。
fn local_to_utc_start(date: NaiveDate) -> chrono::DateTime<Utc> {
    let local_dt = Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .unwrap();
    local_dt.with_timezone(&Utc)
}

/// 把本地日期的 23:59:59 转为 UTC DateTime。
fn local_to_utc_end(date: NaiveDate) -> chrono::DateTime<Utc> {
    let local_dt = Local
        .from_local_datetime(&date.and_hms_opt(23, 59, 59).unwrap())
        .unwrap();
    local_dt.with_timezone(&Utc)
}

/// 把 `--since YYYY-MM-DD` 解析为 UTC DateTime（该日期 00:00 本地 → UTC）。
fn parse_date_to_utc(s: &str, end_of_day: bool) -> Option<chrono::DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    if end_of_day {
        Some(local_to_utc_end(date))
    } else {
        Some(local_to_utc_start(date))
    }
}

/// 把 `--since` 解析为 UTC DateTime（query 路径专用，**保留 sub-day 精度**）。
///
/// 接受：
/// - `YYYY-MM-DD` 日期：该日 00:00 本地 = UTC 起点（粒度 1 天）。
/// - 相对时长 `30m` / `2h` / `1d` / `7d`：当前本地时间 - 时长,转 UTC（粒度 1 分钟）。
///
/// 这样 `timeline today --since 30m` 真正只回 30 分钟内事件,而不是全天。
fn parse_since_utc(s: &str) -> Result<chrono::DateTime<Utc>> {
    let s = s.trim();
    // 1. 日期
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(local_to_utc_start(d));
    }
    // 2. 相对时长
    if s.len() >= 2 {
        let (num, unit) = s.split_at(s.len() - 1);
        if let Ok(n) = num.parse::<u64>() {
            let now_local = Local::now();
            let dt = match unit {
                "m" => now_local - chrono::Duration::minutes(n as i64),
                "h" => now_local - chrono::Duration::hours(n as i64),
                "d" => now_local - chrono::Duration::days(n as i64),
                _ => {
                    return Err(AgentError::InvalidArgument(format!(
                        "invalid --since '{s}', expected YYYY-MM-DD or 30m/2h/1d"
                    )));
                }
            };
            return Ok(dt.with_timezone(&Utc));
        }
    }
    Err(AgentError::InvalidArgument(format!(
        "invalid --since '{s}', expected YYYY-MM-DD or 30m/2h/1d"
    )))
}

/// 解析预设时间范围 → (from_local, to_local) 本地日期。
fn resolve_preset(preset: &str) -> Result<(NaiveDate, NaiveDate)> {
    let today = Local::now().date_naive();
    match preset {
        "" | "today" => Ok((today, today)),
        "yesterday" => {
            let y = today - Duration::days(1);
            Ok((y, y))
        }
        "week" => {
            // ISO 8601: 周一为首日（Mon=1）。
            let weekday = today.weekday().num_days_from_monday();
            let monday = today - Duration::days(weekday as i64);
            let sunday = monday + Duration::days(6);
            Ok((monday, sunday))
        }
        "month" => {
            let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
                .ok_or_else(|| AgentError::Other("invalid month start".into()))?;
            // 下月 1 号 - 1 天 = 本月最后一天。
            let next_month = if today.month() == 12 {
                NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
            };
            let last = next_month.ok_or_else(|| AgentError::Other("invalid month end".into()))?
                - Duration::days(1);
            Ok((first, last))
        }
        other => Err(AgentError::UnknownAction(format!("timeline {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn timeline_event_new_basic() {
        let now = Utc::now();
        let ev = TimelineEvent::new(
            "todo",
            Some("personal"),
            "created",
            now,
            "买咖啡",
            "",
            "t1",
            json!({"status": "Todo"}),
        );
        assert_eq!(ev.source, "todo");
        assert_eq!(ev.account.as_deref(), Some("personal"));
        assert_eq!(ev.event_type, "created");
        assert_eq!(ev.ref_id, "t1");
        assert_eq!(ev.metadata["status"], "Todo");
    }

    #[test]
    fn timeline_event_rss_no_account() {
        let ev = TimelineEvent::new(
            "rss",
            None,
            "published",
            Utc::now(),
            "Rust 1.95",
            "summary",
            "guid-123",
            json!({"feed": "hackernews"}),
        );
        assert!(ev.account.is_none());
    }

    #[test]
    fn time_window_new() {
        let from = Utc::now();
        let to = from + Duration::days(7);
        let w = TimeWindow::new(from, to);
        assert_eq!(w.from, from);
        assert_eq!(w.to, to);
    }

    #[test]
    fn parse_source_filter_splits_commas() {
        let result = parse_source_filter(Some(&"mail, cal , rss".to_string()));
        assert_eq!(result, vec!["mail", "cal", "rss"]);
    }

    #[test]
    fn parse_source_filter_none() {
        assert!(parse_source_filter(None).is_empty());
    }

    #[test]
    fn parse_date_str_valid() {
        assert!(parse_date_str("2026-07-11").is_ok());
    }

    #[test]
    fn parse_date_str_invalid() {
        assert!(parse_date_str("not a date").is_err());
    }

    #[test]
    fn parse_since_date_form_returns_utc() {
        let dt = parse_since_utc("2026-07-11").unwrap();
        // 该日 00:00 本地 → UTC,本地基准下应早于当前 UTC。
        let now = Utc::now();
        assert!(dt < now, "date-form since must be in the past");
        assert!(dt < now + chrono::Duration::days(1));
    }

    #[test]
    fn parse_since_duration_days_subtracts() {
        // 1d：now - 1d 转 UTC,必须 [now - 2d, now] 之间
        let now = Utc::now();
        let dt = parse_since_utc("1d").unwrap();
        assert!(dt < now);
        assert!(dt > now - chrono::Duration::days(2));
    }

    #[test]
    fn parse_since_duration_minutes_subtracts() {
        let now = Utc::now();
        let dt = parse_since_utc("30m").unwrap();
        // 30m ago ≈ now - 30min,允许 1 分钟漂移
        let diff = now - dt;
        assert!(diff >= chrono::Duration::minutes(29));
        assert!(diff <= chrono::Duration::minutes(31));
    }

    #[test]
    fn parse_since_invalid_errors() {
        assert!(parse_since_utc("30x").is_err());
        assert!(parse_since_utc("not-a-thing").is_err());
        assert!(parse_since_utc("2026/07/11").is_err()); // 错格式
    }

    #[test]
    fn resolve_preset_today() {
        let (from, to) = resolve_preset("today").unwrap();
        let today = Local::now().date_naive();
        assert_eq!(from, today);
        assert_eq!(to, today);
    }

    #[test]
    fn resolve_preset_yesterday() {
        let (from, to) = resolve_preset("yesterday").unwrap();
        let today = Local::now().date_naive();
        assert_eq!(from, today - Duration::days(1));
        assert_eq!(to, today - Duration::days(1));
    }

    #[test]
    fn resolve_preset_week_returns_mon_to_sun() {
        let (from, to) = resolve_preset("week").unwrap();
        let weekday = Local::now().date_naive().weekday().num_days_from_monday();
        let monday = Local::now().date_naive() - Duration::days(weekday as i64);
        assert_eq!(from, monday);
        assert_eq!(to, monday + Duration::days(6));
    }

    #[test]
    fn resolve_preset_month_first_to_last() {
        let (from, to) = resolve_preset("month").unwrap();
        assert_eq!(from.day(), 1);
        assert!(to >= from);
    }

    #[test]
    fn parse_date_to_utc_start() {
        let dt = parse_date_to_utc("2026-07-11", false);
        assert!(dt.is_some());
    }
}
