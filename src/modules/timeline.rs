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
use crate::modules::{Executor, parse_simple_args};
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
    fn description(&self) -> &'static str {
        "Unified event timeline: query and sync across all sources."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        // 查询类 action 共享同一组 flag（不含 --account：它是全局 flag，由 main 注入）。
        static QUERY_ARGS: &[ArgSpec] = &[
            ArgSpec {
                name: "from",
                help: "起始日期 YYYY-MM-DD",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "to",
                help: "结束日期 YYYY-MM-DD",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "since",
                help: "相对起点：YYYY-MM-DD 或 30m/2h/1d/7d",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "source",
                help: "来源过滤：mail,cal,rss,todo,note,bookmark（逗号分隔）",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "limit",
                help: "条数上限（默认 100）",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "sync",
                help: "查询前先同步一次",
                kind: ArgKind::Bool,
            },
        ];
        static SYNC_ARGS: &[ArgSpec] = &[
            ArgSpec {
                name: "source",
                help: "来源过滤（逗号分隔）",
                kind: ArgKind::Value,
            },
            ArgSpec {
                name: "since",
                help: "仅同步该日期之后的事件 YYYY-MM-DD",
                kind: ArgKind::Value,
            },
        ];
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "today",
                description: "今天的事件",
                usage: "everyday timeline today [--source S] [--account A] [--limit N] [--sync]",
                args: QUERY_ARGS,
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "yesterday",
                description: "昨天的事件",
                usage: "everyday timeline yesterday [--source S] [--account A] [--limit N] [--sync]",
                args: QUERY_ARGS,
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "week",
                description: "本周（周一-周日）的事件",
                usage: "everyday timeline week [--source S] [--account A] [--limit N] [--sync]",
                args: QUERY_ARGS,
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "month",
                description: "本月的事件",
                usage: "everyday timeline month [--source S] [--account A] [--limit N] [--sync]",
                args: QUERY_ARGS,
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "sync",
                description: "同步各来源事件到 timeline",
                usage: "everyday timeline sync [--source mail,cal] [--since 2026-01-01]",
                args: SYNC_ARGS,
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "timeline",
            description: self.description(),
            actions: ACTIONS,
        }
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
        let sources = parse_source_filter(flags.get("source"))?;
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
        // sync 失败不再 `let _ =` 静默吞（之前会让用户拿到旧数据却不知道 sync 挂了）。
        // 这里让 sync 错误冒泡到 do_query，最终由 main.rs 的 finalize 渲染为错误输出。
        if flags.contains_key("sync") {
            let sources = parse_source_filter(flags.get("source"))?;
            orchestrator::run_sync(&self.config, &sources, None).await?;
        }

        // 解析时间范围。
        // 之前 `--from` 单独给定(无 `--to`)会被静默忽略并回退到 preset,
        // 无效的 `--from 2026-07-99` 也被静默吞掉,用户以为拿到了数据。
        // `resolve_query_range` 现在显式处理所有组合并报错。
        let (from_utc, to_utc) = resolve_query_range(
            preset,
            flags.get("from").map(String::as_str),
            flags.get("to").map(String::as_str),
            flags.get("since").map(String::as_str),
        )?;

        // 构造查询参数。
        let sources = parse_source_filter(flags.get("source"))?;
        let account = flags.get("account").cloned();
        // --limit 解析失败之前静默回退 100（看似 100 行结果其实是 parse 失败）。
        // 改为显式报错，避免"我请求 -1 行却拿到 100 行"这类无声 bug。
        let limit: usize = match flags.get("limit") {
            Some(s) => s.parse().map_err(|_| {
                AgentError::InvalidArgument(format!(
                    "invalid --limit '{s}', expected non-negative integer"
                ))
            })?,
            None => 100,
        };

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
/// 已知 source ID 列表（ADR 0001-0009）。
///
/// `--source` 解析时校验；未知 source 之前被静默丢弃（`events` 表 SQL
/// `source IN (...)` 自然返空），用户看到"0 events"以为是数据问题。
/// 现在显式报错。
const KNOWN_SOURCES: &[&str] = &["mail", "cal", "rss", "todo", "note", "bookmark"];

fn parse_source_filter(raw: Option<&String>) -> Result<Vec<String>> {
    let Some(s) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for token in s.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !KNOWN_SOURCES.contains(&token) {
            return Err(AgentError::InvalidArgument(format!(
                "unknown --source '{token}', expected one of: {}",
                KNOWN_SOURCES.join(", ")
            )));
        }
        out.push(token.to_string());
    }
    Ok(out)
}

/// 解析日期字符串 `YYYY-MM-DD` 为 NaiveDate。
fn parse_date_str(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
        AgentError::InvalidArgument(format!("invalid date '{s}', expected YYYY-MM-DD"))
    })
}

/// 把本地日期的 00:00:00 转为 UTC DateTime。
///
/// 在 DST 模糊/不存在区间返回 None（调用方应决定如何兜底，
/// 而不是 panic）。`and_hms_opt(0,0,0)` 在普通日期上始终 Some，
/// 但 `from_local_datetime` 在春令时 gap 上是 None。
fn local_to_utc_start(date: NaiveDate) -> Option<chrono::DateTime<Utc>> {
    let ndt = date.and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&ndt)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
}

/// 把本地日期的 23:59:59 转为 UTC DateTime。
fn local_to_utc_end(date: NaiveDate) -> Option<chrono::DateTime<Utc>> {
    let ndt = date.and_hms_opt(23, 59, 59)?;
    Local
        .from_local_datetime(&ndt)
        .latest()
        .map(|dt| dt.with_timezone(&Utc))
}

/// 把 `--since YYYY-MM-DD` 解析为 UTC DateTime（该日期 00:00 本地 → UTC）。
fn parse_date_to_utc(s: &str, end_of_day: bool) -> Option<chrono::DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    if end_of_day {
        local_to_utc_end(date)
    } else {
        local_to_utc_start(date)
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
        return local_to_utc_start(d).ok_or_else(|| {
            AgentError::InvalidArgument(format!("invalid --since '{s}': DST gap on date"))
        });
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

/// 解析查询时间范围 → (from_utc, to_utc)。
///
/// 优先级：`--from`/`--to`(任一给定) > `--since` > preset。
///
/// 之前 `--from` 单独给定(无 `--to`)会被静默忽略并回退到 preset,
/// 无效的 `--from 2026-07-99` 也被静默吞掉,用户拿到 preset 范围的"假数据"。
/// 现在独立处理两边:
/// - 仅 `--from`：to 默认 `now()`(查从该日起到现在的事件)
/// - 仅 `--to`：from 默认 preset 起点(本地日 00:00 → UTC)
/// - 二者皆给：from > to 时显式报错,避免静默返回空结果
/// - 任一日期解析失败(如 `2026-07-99`)：显式 `InvalidArgument`
fn resolve_query_range(
    preset: &str,
    from: Option<&str>,
    to: Option<&str>,
    since: Option<&str>,
) -> Result<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> {
    if from.is_some() || to.is_some() {
        let from_utc = match from {
            Some(f) => {
                let f_d = parse_date_str(f)?;
                local_to_utc_start(f_d).ok_or_else(|| {
                    AgentError::InvalidArgument(format!(
                        "--from {f} falls in DST spring-forward gap in local timezone"
                    ))
                })?
            }
            None => {
                let (f_l, _) = resolve_preset(preset)?;
                local_to_utc_start(f_l).ok_or_else(|| {
                    AgentError::InvalidArgument(format!(
                        "preset '{preset}' start falls in DST spring-forward gap"
                    ))
                })?
            }
        };
        let to_utc = match to {
            Some(t) => {
                let t_d = parse_date_str(t)?;
                local_to_utc_end(t_d).ok_or_else(|| {
                    AgentError::InvalidArgument(format!(
                        "--to {t} falls in DST spring-forward gap in local timezone"
                    ))
                })?
            }
            None => Utc::now(),
        };
        if let (Some(f), Some(t)) = (from, to)
            && from_utc > to_utc
        {
            return Err(AgentError::InvalidArgument(format!(
                "--from {f} is later than --to {t}"
            )));
        }
        Ok((from_utc, to_utc))
    } else if let Some(s) = since {
        Ok((parse_since_utc(s)?, Utc::now()))
    } else {
        let (f_l, t_l) = resolve_preset(preset)?;
        let from_utc = local_to_utc_start(f_l).ok_or_else(|| {
            AgentError::InvalidArgument(format!(
                "preset '{preset}' start falls in DST spring-forward gap"
            ))
        })?;
        let to_utc = local_to_utc_end(t_l).ok_or_else(|| {
            AgentError::InvalidArgument(format!(
                "preset '{preset}' end falls in DST spring-forward gap"
            ))
        })?;
        Ok((from_utc, to_utc))
    }
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
        let result =
            parse_source_filter(Some(&"mail, cal , rss".to_string())).expect("known sources");
        assert_eq!(result, vec!["mail", "cal", "rss"]);
    }

    #[test]
    fn parse_source_filter_none() {
        assert!(parse_source_filter(None).expect("none is empty").is_empty());
    }

    #[test]
    fn parse_source_filter_rejects_unknown() {
        // 修复前：`--source bogus` 静默返空 list，SQL `source IN ()` 返 0 行，
        // 用户看到 "no events" 以为是数据问题。
        // 现在显式报 UnknownSource 错误。
        let err = parse_source_filter(Some(&"bogus".to_string())).unwrap_err();
        assert!(err.message().contains("bogus"));
        assert!(err.message().contains("mail")); // 错误里列出合法 source
    }

    #[test]
    fn parse_source_filter_rejects_one_bad_among_good() {
        // 逗号分隔的 source 列表中只要有一个未知，整个列表拒绝。
        let err = parse_source_filter(Some(&"mail,bogus,rss".to_string())).unwrap_err();
        assert!(err.message().contains("bogus"));
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

    #[test]
    fn resolve_query_range_from_alone_valid() {
        // `--from` 单独给定现在也应被采信,不再静默回退 preset。
        let expected_from =
            local_to_utc_start(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()).unwrap();
        let (from, to) = resolve_query_range("today", Some("2000-01-01"), None, None).unwrap();
        assert_eq!(from, expected_from);
        assert!(to > from); // to 默认 now(),必然晚于 2000 年
    }

    #[test]
    fn resolve_query_range_from_alone_invalid_errors() {
        // 之前 `--from 2026-07-99` 单独给定会被静默忽略,回退到 preset 范围。
        // 现在必须显式报错,不再静默 fallback。
        let err = resolve_query_range("today", Some("2026-07-99"), None, None).unwrap_err();
        assert!(err.message().contains("2026-07-99"));
        assert!(err.message().contains("YYYY-MM-DD"));
    }

    #[test]
    fn resolve_query_range_inverted_from_to_errors() {
        // `--from` 晚于 `--to` 时静默返回空结果,现在显式报错。
        let err =
            resolve_query_range("today", Some("2026-07-01"), Some("2026-06-01"), None).unwrap_err();
        assert!(err.message().contains("later than"));
    }

    #[test]
    fn resolve_query_range_to_alone_defaults_from_preset() {
        // 仅 `--to` 时,from 取 preset 起点(本地日 00:00 → UTC),to 取该日 23:59:59。
        let today = Local::now().date_naive();
        let expected_from = local_to_utc_start(today).unwrap();
        let expected_to = local_to_utc_end(NaiveDate::from_ymd_opt(2030, 12, 31).unwrap()).unwrap();
        let (from, to) = resolve_query_range("today", None, Some("2030-12-31"), None).unwrap();
        assert_eq!(from, expected_from);
        assert_eq!(to, expected_to);
    }

    #[test]
    fn resolve_query_range_since_still_works() {
        // `--since` 路径不受 `--from`/`--to` 重构影响。
        let now = Utc::now();
        let (from, to) = resolve_query_range("today", None, None, Some("30m")).unwrap();
        let diff = to - from;
        assert!(diff >= chrono::Duration::minutes(29));
        assert!(diff <= chrono::Duration::minutes(31));
        assert!(to > now - chrono::Duration::minutes(1));
    }
}
