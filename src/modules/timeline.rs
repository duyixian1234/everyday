//! Timeline module: unified event storage and query.
//!
//! CLI:
//! - `everyday timeline` (no action) = `today`
//! - `everyday timeline today|yesterday|week|month`
//! - `everyday timeline sync [--source S] [--since DATE]`
//! - `everyday timeline --from DATE --to DATE [--source S] [--account A] [--limit N] [--sync]`
//!
//! Core types:
//! - [`TimelineEvent`]: the unified event structure (immutable record).
//! - [`TimelineProvider`]: the data-pull trait for each source (stateless).
//! - [`SyncMode`]: append vs. window-refresh.
//! - [`TimeWindow`]: the sync time window.
//!
//! Submodules:
//! - [`store`]: timeline.db read/write.
//! - [`orchestrator`]: sync orchestrator.
//! - [`providers`]: per-source provider adapters.
//!
//! See [L001](../../docs/adr/L001-append-only-event-log.md) for the event log model.

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

// ============ core types ============

/// Unified event structure (immutable record).
///
/// Natural key: `(source, COALESCE(account, ''), ref_id, event_type, timestamp)`.
/// Used for sync idempotency: re-syncing the same window produces no duplicate rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    /// Source module: `mail` / `cal` / `rss` / `todo` / `note` / `bookmark`.
    pub source: String,
    /// Source account name (None for RSS).
    pub account: Option<String>,
    /// Event semantic type (e.g. `received` / `sent` / `created` / `completed` / `scheduled`).
    pub event_type: String,
    /// Event occurrence moment (RFC3339 UTC).
    pub timestamp: chrono::DateTime<Utc>,
    /// Event title.
    pub title: String,
    /// Event summary (may be empty).
    pub summary: String,
    /// Stable identifier of the entity referenced by the event in the source system.
    pub ref_id: String,
    /// Structured metadata (JSON object).
    pub metadata: serde_json::Value,
}

impl TimelineEvent {
    /// Create a new event; metadata defaults to an empty JSON object.
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

/// Sync time window.
#[derive(Debug, Clone)]
pub struct TimeWindow {
    /// Window start (UTC, inclusive).
    pub from: chrono::DateTime<Utc>,
    /// Window end (UTC, inclusive).
    pub to: chrono::DateTime<Utc>,
}

impl TimeWindow {
    pub fn new(from: chrono::DateTime<Utc>, to: chrono::DateTime<Utc>) -> Self {
        Self { from, to }
    }
}

/// Sync mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Idempotent append, de-duplicated by the natural key (`INSERT OR IGNORE`).
    Append,
    /// Delete old rows of the same source in the window first, then insert the current snapshot (only `cal`).
    WindowRefresh,
}

/// Per-source data-pull trait (stateless).
///
/// The provider only handles "given a window, return the snapshot of events in that
/// window and the sync mode". The watermark (`last_sync`) is managed by the orchestrator
/// in the `sync_state` table.
#[async_trait]
pub trait TimelineProvider: Send + Sync {
    /// Source identifier (`"mail"` / `"cal"` / ...).
    fn source(&self) -> &'static str;

    /// Account name (returns None for RSS and other accountless sources).
    fn account(&self) -> Option<&str>;

    /// Pull events within the given time window.
    ///
    /// Returns `(event list, sync mode)`.
    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)>;
}

/// Sync result of a single provider (collected by the orchestrator).
#[derive(Debug)]
pub struct ProviderSyncResult {
    pub source: String,
    pub account: Option<String>,
    pub events_count: usize,
    pub status: ProviderStatus,
}

/// Provider sync status.
#[derive(Debug)]
pub enum ProviderStatus {
    /// Success.
    Ok,
    /// Failure (with error message).
    Failed(String),
}

// ============ Executor impl ============

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
        // Query-style actions share the same flag set (no --account: it is a global flag injected by main).
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
            // No action or preset action -> query.
            "" | "today" | "yesterday" | "week" | "month" => {
                self.do_query(action, &flags, json_mode).await
            }
            other => Err(AgentError::UnknownAction(format!("timeline {other}"))),
        }
    }
}

impl TimelineModule {
    /// Run the `sync` subcommand.
    async fn do_sync(&self, flags: &std::collections::HashMap<String, String>) -> Result<Output> {
        let sources = parse_source_filter(flags.get("source"))?;
        let since = flags.get("since").and_then(|s| parse_date_to_utc(s, false));

        let output = orchestrator::run_sync(&self.config, &sources, since).await?;
        Ok(output.to_output(crate::util::json_mode::is_json()))
    }

    /// Run a query (preset or custom range).
    async fn do_query(
        &self,
        preset: &str,
        flags: &std::collections::HashMap<String, String>,
        json_mode: bool,
    ) -> Result<Output> {
        // --sync: run a sync first, then query.
        // Sync failures are no longer silently swallowed by `let _ =` (previously this
        // meant users could see stale data without realising the sync had failed).
        // The sync error now bubbles up to do_query and ultimately to main.rs's
        // finalize, which renders it as an error output.
        if flags.contains_key("sync") {
            let sources = parse_source_filter(flags.get("source"))?;
            orchestrator::run_sync(&self.config, &sources, None).await?;
        }

        // Resolve the time range.
        // Previously `--from` alone (no `--to`) was silently ignored and fell back to
        // the preset; an invalid `--from 2026-07-99` was silently swallowed, leading
        // users to think they had data [L013](../../docs/adr/L013-from-explicit-error.md).
        // `resolve_query_range` now handles all combinations explicitly and reports errors.
        let (from_utc, to_utc) = resolve_query_range(
            preset,
            flags.get("from").map(String::as_str),
            flags.get("to").map(String::as_str),
            flags.get("since").map(String::as_str),
        )?;

        // Build query params.
        let sources = parse_source_filter(flags.get("source"))?;
        let account = flags.get("account").cloned();
        // Previously `--limit` parse failures silently fell back to 100 (looks like 100
        // rows of results when actually parsing failed). Switched to explicit error
        // to avoid silent bugs like "I asked for -1 rows but got 100".
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

// ============ time helpers ============

/// Parse `--source mail,cal` into `["mail", "cal"]`.
/// Known source-ID list (ADRs [L001](../../docs/adr/L001-append-only-event-log.md)
/// -[L009](../../docs/adr/L009-best-effort-sync.md)).
///
/// `--source` is validated when parsed; previously unknown sources were silently
/// dropped (the `events` table's `source IN (...)` query naturally returns empty),
/// so users saw "no events" and assumed it was a data problem. Now it errors explicitly.
pub const KNOWN_SOURCES: &[&str] = &["mail", "cal", "rss", "todo", "note", "bookmark"];

fn parse_source_filter(raw: Option<&String>) -> Result<Vec<String>> {
    parse_source_list(raw, KNOWN_SOURCES)
}

/// Shared helper: parse a comma-separated source/module allow-list,
/// validating each entry against `known`. Empty / None -> empty Vec.
/// Used by timeline's --source and search's --module (S006).
pub fn parse_source_list(raw: Option<&String>, known: &[&str]) -> Result<Vec<String>> {
    let Some(s) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for token in s.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !known.contains(&token) {
            return Err(AgentError::InvalidArgument(format!(
                "unknown module '{token}', expected one of: {}",
                known.join(", ")
            )));
        }
        out.push(token.to_string());
    }
    Ok(out)
}

/// Parse a date string `YYYY-MM-DD` into a NaiveDate.
fn parse_date_str(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
        AgentError::InvalidArgument(format!("invalid date '{s}', expected YYYY-MM-DD"))
    })
}

/// Convert the local date's 00:00:00 to a UTC DateTime.
///
/// Returns None at DST ambiguous / non-existent intervals (callers must decide the
/// fallback rather than panic). `and_hms_opt(0,0,0)` is always Some on ordinary
/// dates, but `from_local_datetime` is None on a spring-forward gap.
fn local_to_utc_start(date: NaiveDate) -> Option<chrono::DateTime<Utc>> {
    let ndt = date.and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&ndt)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Convert the local date's 23:59:59 to a UTC DateTime.
fn local_to_utc_end(date: NaiveDate) -> Option<chrono::DateTime<Utc>> {
    let ndt = date.and_hms_opt(23, 59, 59)?;
    Local
        .from_local_datetime(&ndt)
        .latest()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parse `--since YYYY-MM-DD` into a UTC DateTime (00:00 local of that date -> UTC).
fn parse_date_to_utc(s: &str, end_of_day: bool) -> Option<chrono::DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    if end_of_day {
        local_to_utc_end(date)
    } else {
        local_to_utc_start(date)
    }
}

/// Parse `--since` into a UTC DateTime (query-path-only, **preserves sub-day precision**).
///
/// Thin wrapper around [`crate::util::datetime::parse_since`]; kept as
/// a method-like entry point for backward compat in timeline's existing
/// callers.
fn parse_since_utc(s: &str) -> Result<chrono::DateTime<Utc>> {
    crate::util::datetime::parse_since(s)
}

/// Resolve the query time range -> (from_utc, to_utc).
///
/// Priority: `--from`/`--to` (either given) > `--since` > preset.
///
/// Previously `--from` alone (no `--to`) was silently ignored and fell back to the
/// preset; an invalid `--from 2026-07-99` was silently swallowed, so users got
/// "fake data" from the preset range [L013](../../docs/adr/L013-from-explicit-error.md).
/// Now both sides are handled independently:
/// - `--from` only: `to` defaults to `now()` (events from that date up to now).
/// - `--to` only: `from` defaults to the preset start (00:00 local date -> UTC).
/// - Both given: when `from > to`, error explicitly to avoid silently returning empty.
/// - Either date fails to parse (e.g. `2026-07-99`): explicit `InvalidArgument`.
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

/// Resolve a preset time range -> (from_local, to_local) local dates.
fn resolve_preset(preset: &str) -> Result<(NaiveDate, NaiveDate)> {
    let today = Local::now().date_naive();
    match preset {
        "" | "today" => Ok((today, today)),
        "yesterday" => {
            let y = today - Duration::days(1);
            Ok((y, y))
        }
        "week" => {
            // ISO 8601: Monday is the first day (Mon=1).
            let weekday = today.weekday().num_days_from_monday();
            let monday = today - Duration::days(weekday as i64);
            let sunday = monday + Duration::days(6);
            Ok((monday, sunday))
        }
        "month" => {
            let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
                .ok_or_else(|| AgentError::Other("invalid month start".into()))?;
            // The 1st of next month - 1 day = the last day of this month.
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
        // Before the fix: `--source bogus` silently returned an empty list; the SQL
        // `source IN ()` returned 0 rows, and users saw "no events" and assumed
        // it was a data problem. Now it explicitly reports an UnknownSource error.
        let err = parse_source_filter(Some(&"bogus".to_string())).unwrap_err();
        assert!(err.message().contains("bogus"));
        assert!(err.message().contains("mail")); // error message lists the valid sources
    }

    #[test]
    fn parse_source_filter_rejects_one_bad_among_good() {
        // A single unknown source in the comma-separated list rejects the whole list.
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
        // 00:00 local of that date -> UTC; under a local baseline it must be before current UTC.
        let now = Utc::now();
        assert!(dt < now, "date-form since must be in the past");
        assert!(dt < now + chrono::Duration::days(1));
    }

    #[test]
    fn parse_since_duration_days_subtracts() {
        // 1d: now - 1d converted to UTC, must fall in [now - 2d, now]
        let now = Utc::now();
        let dt = parse_since_utc("1d").unwrap();
        assert!(dt < now);
        assert!(dt > now - chrono::Duration::days(2));
    }

    #[test]
    fn parse_since_duration_minutes_subtracts() {
        let now = Utc::now();
        let dt = parse_since_utc("30m").unwrap();
        // 30m ago ~= now - 30min, allow 1 minute drift
        let diff = now - dt;
        assert!(diff >= chrono::Duration::minutes(29));
        assert!(diff <= chrono::Duration::minutes(31));
    }

    #[test]
    fn parse_since_invalid_errors() {
        assert!(parse_since_utc("30x").is_err());
        assert!(parse_since_utc("not-a-thing").is_err());
        assert!(parse_since_utc("2026/07/11").is_err()); // wrong format
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
        // `--from` alone is now respected, no longer silently falling back to the preset.
        let expected_from =
            local_to_utc_start(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()).unwrap();
        let (from, to) = resolve_query_range("today", Some("2000-01-01"), None, None).unwrap();
        assert_eq!(from, expected_from);
        assert!(to > from); // to defaults to now(), which is necessarily after the year 2000
    }

    #[test]
    fn resolve_query_range_from_alone_invalid_errors() {
        // Previously `--from 2026-07-99` alone was silently ignored and fell back to
        // the preset range. Now it must explicitly error, with no silent fallback.
        let err = resolve_query_range("today", Some("2026-07-99"), None, None).unwrap_err();
        assert!(err.message().contains("2026-07-99"));
        assert!(err.message().contains("YYYY-MM-DD"));
    }

    #[test]
    fn resolve_query_range_inverted_from_to_errors() {
        // `--from` later than `--to` previously returned empty results silently; now it errors explicitly.
        let err =
            resolve_query_range("today", Some("2026-07-01"), Some("2026-06-01"), None).unwrap_err();
        assert!(err.message().contains("later than"));
    }

    #[test]
    fn resolve_query_range_to_alone_defaults_from_preset() {
        // With only `--to`, from takes the preset start (00:00 local -> UTC) and to takes 23:59:59 of that date.
        let today = Local::now().date_naive();
        let expected_from = local_to_utc_start(today).unwrap();
        let expected_to = local_to_utc_end(NaiveDate::from_ymd_opt(2030, 12, 31).unwrap()).unwrap();
        let (from, to) = resolve_query_range("today", None, Some("2030-12-31"), None).unwrap();
        assert_eq!(from, expected_from);
        assert_eq!(to, expected_to);
    }

    #[test]
    fn resolve_query_range_since_still_works() {
        // The `--since` path is not affected by the `--from`/`--to` refactor.
        let now = Utc::now();
        let (from, to) = resolve_query_range("today", None, None, Some("30m")).unwrap();
        let diff = to - from;
        assert!(diff >= chrono::Duration::minutes(29));
        assert!(diff <= chrono::Duration::minutes(31));
        assert!(to > now - chrono::Duration::minutes(1));
    }
}
