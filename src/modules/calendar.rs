//! 日历模块（CalDAV）：login / calendars / list / add / delete。
//!
//! 流程：config.toml 存账户元数据（caldav_url/username）→ `everyday cal login` 存密码到
//! 系统密钥环 → `everyday cal calendars/list/add/delete` 自动读取密码连接 CalDAV。
//! 密码绝不落盘 config.toml。
//!
//! 技术栈：libdav 0.10（CalDAV 协议，request API）+ icalendar 0.17（iCalendar 解析/生成）
//! + hyper 1.x（HTTP，body=String 满足 libdav 的 HttpClient trait）+ hyper-rustls（ring
//!   TLS，webpki 根证书）+ tower-http（Basic Auth 中间件，覆盖式插 Authorization header）。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use http::Uri;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::TokioExecutor;
use icalendar::{Calendar, CalendarDateTime, Component, DatePerhapsTime, Event, EventLike};
use libdav::caldav::{FindCalendarHomeSet, FindCalendars, GetCalendarResources};
use libdav::dav::{Delete, GetProperty, PutResource, WebDavClient};
use libdav::names;
use libdav::{CalDavClient, caldav_service_for_url};
use tower_http::auth::AddAuthorization;

use crate::config::{CalendarAccount, Config};
use crate::error::{AgentError, Result};
use crate::modules::{parse_simple_args, ActionDoc, Executor};
use crate::output::Output;

/// hyper-rustls 的 HTTPS connector（webpki 根证书 + http1）。
type HttpsConnector =
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>;
/// 带 Basic Auth 的 hyper legacy client，body 类型为 String。
/// libdav 的 HttpClient blanket impl 要求 `Service<Request<String>, Response=Response<Incoming>>`，
/// 故 body 泛型参数固定为 String（http-body 1.0 实现了 `impl Body for String`）。
type HttpsClient = AddAuthorization<HyperClient<HttpsConnector, String>>;
/// CalDavClient 的具体类型（用 type alias 避免泛型签名传染）。
type CalDav = CalDavClient<HttpsClient>;

pub struct CalendarModule {
    config: Arc<Config>,
}

impl CalendarModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for CalendarModule {
    fn name(&self) -> &'static str {
        "cal"
    }

    fn description(&self) -> &'static str {
        "Calendar management (CalDAV): login, calendars, list, add, delete events."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("login", "Store CalDAV password in system keyring", "everyday cal login [--account NAME]"),
            ActionDoc::new("calendars", "List calendar collections", "everyday cal calendars [--account NAME]"),
            ActionDoc::new("list", "List events (default: today)", "everyday cal list [--today|--date YYYY-MM-DD] [--limit N] [--account NAME]"),
            ActionDoc::new("add", "Add an event", "everyday cal add --title T --start ISO --end ISO [--location L] [--description D] [--calendar HREF] [--account NAME]"),
            ActionDoc::new("delete", "Delete an event by href", "everyday cal delete --id HREF [--account NAME]"),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _) = parse_simple_args(args);
        let account = self
            .config
            .calendar_account(flags.get("account").map(|s| s.as_str()))?;

        // 未知 action 提前识别（坑10：避免空密码时优先报 AuthError 而非 UnknownAction）。
        match action {
            "login" => cal_login(account).await,
            "calendars" | "list" | "add" | "delete" => {
                let password = get_password(account)?;
                match action {
                    "calendars" => cal_calendars(account, &password).await,
                    "list" => cal_list(account, &password, &flags).await,
                    "add" => cal_add(account, &password, &flags).await,
                    "delete" => cal_delete(account, &password, &flags).await,
                    _ => unreachable!(),
                }
            }
            other => Err(AgentError::UnknownAction(format!("cal {other}"))),
        }
    }
}

// ============ keyring 凭证 ============

/// 从系统密钥环读取账户密码。
fn get_password(account: &CalendarAccount) -> Result<String> {
    let service = Config::keyring_service("cal", &account.name);
    let entry = keyring::Entry::new(&service, &account.username)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry.get_password().map_err(|e| {
        AgentError::Auth(format!(
            "no password in keyring for calendar account '{}': {e}. \
             Run `everyday cal login --account {}` to store it.",
            account.name, account.name
        ))
    })
}

/// 交互式输入密码并存入系统密钥环。
async fn cal_login(account: &CalendarAccount) -> Result<Output> {
    let service = Config::keyring_service("cal", &account.name);
    let entry = keyring::Entry::new(&service, &account.username)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    let username = account.username.clone();
    let account_name = account.name.clone();
    let password = tokio::task::spawn_blocking(move || {
        rpassword::prompt_password(format!("Password for {username}: "))
    })
    .await
    .map_err(|e| AgentError::Other(format!("join password prompt: {e}")))?
    .map_err(|e| AgentError::Other(format!("read password: {e}")))?;

    // 坑9：空密码校验。set_password("") 会成功，但 base64 成 "Basic Og==" 后服务端返 401。
    if password.is_empty() {
        return Err(AgentError::InvalidArgument("password cannot be empty".into()));
    }
    entry
        .set_password(&password)
        .map_err(|e| AgentError::Auth(format!("keyring set: {e}")))?;
    Ok(Output::text(format!(
        "password stored for calendar account '{account_name}'"
    )))
}

// ============ CalDAV 客户端构建 ============

/// 构建 CalDavClient：hyper + rustls(ring, webpki) + Basic Auth + well-known 探测。
///
/// **跳过** `bootstrap_via_service_discovery`（坑5：内部 fallback DNS SRV `_caldavs._tcp`，
/// 国内服务商不实现，远程 DNS 强制关闭连接 os error 10054）。改用 `find_context_path`
/// 只做 `/.well-known/caldav` 重定向探测（坑6：QQ 根 URL PROPFIND 返 404，well-known
/// 301 到 `/calendar/`，最多 5 跳）。探测失败静默降级用 base_url。
async fn build_client(account: &CalendarAccount, password: &str) -> Result<CalDav> {
    let base: Uri = account
        .caldav_url
        .parse()
        .map_err(|e| AgentError::InvalidArgument(format!("invalid caldav_url '{}': {e}", account.caldav_url)))?;
    let host = base
        .host()
        .ok_or_else(|| AgentError::InvalidArgument(format!("caldav_url missing host: {}", account.caldav_url)))?
        .to_string();
    let port = base
        .port_u16()
        .unwrap_or_else(|| if base.scheme_str() == Some("http") { 80 } else { 443 });

    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build();
    let https_client =
        HyperClient::builder(TokioExecutor::new()).build::<_, String>(https_connector);
    let auth_client = AddAuthorization::basic(https_client, &account.username, password);
    let mut webdav = WebDavClient::new(base, auth_client);

    // well-known 探测（RFC 6764 §5），跳过 SRV/TXT。
    let service = caldav_service_for_url(&webdav.base_url)
        .map_err(|e| AgentError::Network(format!("determine caldav service: {e}")))?;
    // 探测失败不致命：降级用 base_url（部分服务器 well-known 不可用但 base_url 直接可用）。
    if let Ok(Some(url)) = webdav.find_context_path(service, &host, port).await {
        // 重定向后的真实 context path（如 https://dav.qq.com:443/calendar/）。
        // base_url 是 pub 字段，直接覆盖（坑6）。
        webdav.base_url = url;
    }

    Ok(CalDavClient::new(webdav))
}

// ============ 日历发现 ============

/// 日历集合的展示信息。
struct CalendarInfo {
    href: String,
    name: Option<String>,
    colour: Option<String>,
}

/// 发现并返回所有日历集合。
///
/// 流程（RFC 5397 + RFC 4791）：current-user-principal → calendar-home-set → calendars。
/// 参照 libdav examples/find_calendars.rs。principal 或 home-set 发现失败（如 QQ 不支持
/// current-user-principal，PROPFIND 返 404）时降级用 base_url 作为 home set。
async fn list_all_calendars(caldav: &CalDav) -> Result<Vec<CalendarInfo>> {
    let home_sets: Vec<Uri> = match caldav.find_current_user_principal().await {
        Ok(Some(p)) => {
            // principal 找到 → 查 calendar-home-set；查询失败或为空则降级 base_url。
            match caldav.request(FindCalendarHomeSet::new(p.path())).await {
                Ok(resp) if !resp.home_sets.is_empty() => resp.home_sets,
                _ => vec![caldav.base_url().clone()],
            }
        }
        _ => vec![caldav.base_url().clone()], // principal 未找到或查询失败 → 降级 base_url
    };

    let mut out = Vec::new();
    for url in &home_sets {
        let resp = caldav
            .request(FindCalendars::new(url.path()))
            .await
            .map_err(|e| AgentError::Network(format!("find calendars: {e}")))?;
        for cal in resp.calendars {
            // DISPLAY_NAME / CALENDAR_COLOUR 取不到时降级为 None，不致命。
            let name = caldav
                .request(GetProperty::new(&cal.href, &names::DISPLAY_NAME))
                .await
                .ok()
                .and_then(|r| r.value);
            let colour = caldav
                .request(GetProperty::new(&cal.href, &names::CALENDAR_COLOUR))
                .await
                .ok()
                .and_then(|r| r.value);
            out.push(CalendarInfo {
                href: cal.href,
                name,
                colour,
            });
        }
    }
    Ok(out)
}

// ============ 动作实现 ============

/// `cal calendars`：列出当前用户的所有日历集合。
async fn cal_calendars(account: &CalendarAccount, password: &str) -> Result<Output> {
    let caldav = build_client(account, password).await?;
    let calendars = list_all_calendars(&caldav).await?;
    let rows = calendars
        .into_iter()
        .map(|c| vec![c.href, c.name.unwrap_or_default(), c.colour.unwrap_or_default()])
        .collect();
    Ok(Output::records(
        vec!["href".into(), "name".into(), "colour".into()],
        rows,
    ))
}

/// `cal list`：列出事件，默认今日；`--date YYYY-MM-DD` 指定日期。
///
/// 策略：用 `GetCalendarResources`（calendar-query REPORT）全量拉取每个日历的事件
/// （含 calendar-data），本地用 icalendar 解析 VEVENT，再按目标日期过滤。比服务端
/// time-range REPORT 更可靠（国内服务端 time-range 实现质量参差，可能返空）。
async fn cal_list(
    account: &CalendarAccount,
    password: &str,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let caldav = build_client(account, password).await?;
    let calendars = list_all_calendars(&caldav).await?;
    let limit: usize = flags.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);
    let target_date = match flags.get("date") {
        Some(d) => parse_date(d)?,
        None => chrono::Local::now().date_naive(), // 默认今日（--today 即默认行为）
    };

    let mut events: Vec<EventRow> = Vec::new();
    for cal in &calendars {
        let resp = match caldav.request(GetCalendarResources::new(&cal.href)).await {
            Ok(r) => r,
            Err(_) => continue, // 单个日历拉取失败不致命
        };
        for res in resp.resources {
            let content = match res.content {
                Ok(c) => c,
                Err(_) => continue,
            };
            // 解析 iCalendar，提取 VEVENT 并按日期过滤。
            if let Ok(parsed) = content.data.parse::<Calendar>() {
                for event in parsed.events() {
                    if let Some(row) = build_event_row(&res.href, event, &target_date) {
                        events.push(row);
                    }
                }
            }
        }
    }

    // 按开始时间（本地时间序）升序排列。
    events.sort_by_key(|e| e.sort_key);
    events.truncate(limit);

    let rows = events
        .into_iter()
        .map(|e| vec![e.href, e.start, e.end, e.summary, e.location])
        .collect();
    Ok(Output::records(
        vec!["href".into(), "start".into(), "end".into(), "summary".into(), "location".into()],
        rows,
    ))
}

/// `cal add`：添加事件。用 icalendar 构造 VEVENT，PUT 到目标日历。
async fn cal_add(
    account: &CalendarAccount,
    password: &str,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("--title <text> is required".into()))?;
    let start = flags.get("start").ok_or_else(|| {
        AgentError::InvalidArgument("--start <ISO> is required (e.g. 2026-07-09T14:00:00Z)".into())
    })?;
    let end = flags
        .get("end")
        .ok_or_else(|| AgentError::InvalidArgument("--end <ISO> is required".into()))?;

    let start_dt = parse_datetime(start)?;
    let end_dt = parse_datetime(end)?;

    // 构造 VEVENT。Event 的 builder 方法返回 &mut Self，故先 owned 再链式最后 .done()。
    let mut event = Event::new();
    event.summary(title).starts(start_dt).ends(end_dt);
    if let Some(loc) = flags.get("location") {
        event.location(loc);
    }
    if let Some(desc) = flags.get("description") {
        event.description(desc);
    }
    let event = event.done();

    let calendar = Calendar::new().push(event).done();
    // icalendar 的 fmt_write 用 write_crlf! 输出 CRLF，但归一化确保整体 CRLF（CalDAV 要求）。
    let ics = normalize_crlf(&calendar.to_string());

    let caldav = build_client(account, password).await?;
    let calendars = list_all_calendars(&caldav).await?;

    // 选目标日历：--calendar HREF 或 name 匹配，默认第一个。
    let target = if let Some(h) = flags.get("calendar") {
        calendars
            .into_iter()
            .find(|c| c.href == *h || c.name.as_deref() == Some(h.as_str()))
            .ok_or_else(|| AgentError::InvalidArgument(format!("calendar '{h}' not found")))?
    } else {
        calendars.into_iter().next().ok_or_else(|| {
            AgentError::Other("no calendar collection found for this account".into())
        })?
    };

    // 生成新 href：<calendar_href>/<timestamp>.ics。UID 由 icalendar 自动生成。
    let new_href = format!("{}{}.ics", ensure_trailing_slash(&target.href), event_filename());

    let resp = caldav
        .request(PutResource::new(&new_href).create(ics, "text/calendar; charset=utf-8"))
        .await
        .map_err(|e| AgentError::Network(format!("put event: {e}")))?;

    Ok(Output::text(format!(
        "event added: {new_href} (etag: {})",
        resp.etag.unwrap_or_else(|| "n/a".into())
    )))
}

/// `cal delete`：按 href 删除事件（无条件 force 删除）。
async fn cal_delete(
    account: &CalendarAccount,
    password: &str,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let href = flags.get("id").ok_or_else(|| {
        AgentError::InvalidArgument("--id <href> is required (get href from `cal list`)".into())
    })?;
    let caldav = build_client(account, password).await?;
    caldav
        .request(Delete::new(href).force())
        .await
        .map_err(|e| AgentError::Network(format!("delete event: {e}")))?;
    Ok(Output::text(format!("deleted: {href}")))
}

// ============ 辅助函数 ============

/// 单条事件展示行 + 排序键。
struct EventRow {
    href: String,
    start: String,
    end: String,
    summary: String,
    location: String,
    sort_key: chrono::NaiveDateTime,
}

/// 从解析出的 VEVENT 构造展示行；事件开始日期 != 目标日期时返回 None（过滤）。
fn build_event_row(
    href: &str,
    event: &Event,
    target_date: &chrono::NaiveDate,
) -> Option<EventRow> {
    let start_dpt = event.get_start()?;
    let start_ndt = date_perhaps_time_to_naive(&start_dpt)?;
    // 过滤：事件开始日期 == 目标日期。
    if start_ndt.date() != *target_date {
        return None;
    }
    let end_str = event
        .get_end()
        .as_ref()
        .map(format_date_perhaps_time)
        .unwrap_or_default();
    Some(EventRow {
        href: href.to_string(),
        start: format_date_perhaps_time(&start_dpt),
        end: end_str,
        summary: event.get_summary().unwrap_or("").to_string(),
        location: event.get_location().unwrap_or("").to_string(),
        sort_key: start_ndt,
    })
}

/// 把 [`DatePerhapsTime`] 转为 [`chrono::NaiveDateTime`] 用于排序/过滤（本地时间序）。
///
/// - `Date` 变体拼 00:00:00（全天事件）。
/// - `Utc` 取 naive_utc（统一到 UTC 时刻）。
/// - `Floating` / `WithTimezone` 取 naive 部分（本地时间，对"今日事件"更直观）。
///
/// 不启用 icalendar 的 `chrono-tz` feature，故不用 `try_into_utc`；用 NaiveDateTime
/// 做本地时间排序对单日事件足够且更符合用户预期。
fn date_perhaps_time_to_naive(dpt: &DatePerhapsTime) -> Option<chrono::NaiveDateTime> {
    match dpt {
        DatePerhapsTime::Date(d) => d.and_hms_opt(0, 0, 0),
        DatePerhapsTime::DateTime(CalendarDateTime::Utc(dt)) => Some(dt.naive_utc()),
        DatePerhapsTime::DateTime(CalendarDateTime::Floating(dt)) => Some(*dt),
        DatePerhapsTime::DateTime(CalendarDateTime::WithTimezone { date_time, .. }) => {
            Some(*date_time)
        }
    }
}

/// 格式化 [`DatePerhapsTime`] 为人类可读字符串。
fn format_date_perhaps_time(dpt: &DatePerhapsTime) -> String {
    match dpt {
        DatePerhapsTime::Date(d) => d.format("%Y-%m-%d").to_string(),
        DatePerhapsTime::DateTime(CalendarDateTime::Utc(dt)) => {
            dt.format("%Y-%m-%d %H:%M").to_string()
        }
        DatePerhapsTime::DateTime(CalendarDateTime::Floating(dt)) => {
            dt.format("%Y-%m-%d %H:%M").to_string()
        }
        DatePerhapsTime::DateTime(CalendarDateTime::WithTimezone { date_time, .. }) => {
            date_time.format("%Y-%m-%d %H:%M").to_string()
        }
    }
}

/// 解析 `YYYY-MM-DD` 日期。
fn parse_date(s: &str) -> Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
        AgentError::InvalidArgument(format!("invalid date '{s}' (expected YYYY-MM-DD): {e}"))
    })
}

/// 解析日期时间，兼容三种形式：
/// - `2026-07-09T14:00:00Z`（UTC，RFC3339）
/// - `2026-07-09T14:00:00+08:00`（带偏移，RFC3339）
/// - `2026-07-09T14:00:00`（无时区，按 UTC 处理）
fn parse_datetime(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // 无时区后缀：按 UTC 解析（NaiveDateTime::and_utc 返 DateTime<Utc>，非 Option）。
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(AgentError::InvalidArgument(format!(
        "invalid datetime '{s}' (expected RFC3339 like 2026-07-09T14:00:00Z or 2026-07-09T14:00:00)"
    )))
}

/// 归一化换行为 CRLF：先把 `\r\n` 和 `\r` 都归一到 `\n`，再把 `\n` 转 `\r\n`。
///
/// icalendar 的 `fmt_write` 已用 `write_crlf!` 输出 CRLF，但 property 值内部可能混入
/// 裸 `\n`/`\r`，归一化确保整体 CRLF（CalDAV/RFC 5545 要求 CRLF 行结束）。
fn normalize_crlf(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

/// 确保 href 以 `/` 结尾（用于拼接事件 href）。
fn ensure_trailing_slash(s: &str) -> String {
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

/// 生成事件文件名（纳秒时间戳，单用户场景足够唯一）。
fn event_filename() -> String {
    let now = chrono::Utc::now();
    let nanos = now.timestamp_nanos_opt().unwrap_or(0);
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn parse_date_valid() {
        assert_eq!(
            parse_date("2026-07-09").unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap()
        );
    }

    #[test]
    fn parse_date_invalid() {
        assert!(parse_date("2026/07/09").is_err());
        assert!(parse_date("not-a-date").is_err());
    }

    #[test]
    fn parse_datetime_accepts_utc_z() {
        let dt = parse_datetime("2026-07-09T14:00:00Z").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap());
    }

    #[test]
    fn parse_datetime_accepts_offset() {
        // +08:00 → UTC 06:00
        let dt = parse_datetime("2026-07-09T14:00:00+08:00").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 7, 9, 6, 0, 0).unwrap());
    }

    #[test]
    fn parse_datetime_accepts_naive_as_utc() {
        let dt = parse_datetime("2026-07-09T14:00:00").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap());
    }

    #[test]
    fn parse_datetime_rejects_garbage() {
        assert!(parse_datetime("yesterday").is_err());
    }

    #[test]
    fn normalize_crlf_handles_all_forms() {
        assert_eq!(normalize_crlf("a\nb"), "a\r\nb");
        assert_eq!(normalize_crlf("a\r\nb"), "a\r\nb");
        assert_eq!(normalize_crlf("a\rb"), "a\r\nb");
        assert_eq!(normalize_crlf("a\r\nb\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn ensure_trailing_slash_works() {
        assert_eq!(ensure_trailing_slash("/cal/"), "/cal/");
        assert_eq!(ensure_trailing_slash("/cal"), "/cal/");
        assert_eq!(ensure_trailing_slash(""), "/");
    }

    #[test]
    fn date_perhaps_time_variants_to_naive() {
        let nd = chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
        let ndt = nd.and_hms_opt(14, 0, 0).unwrap();
        let utc = Utc.from_utc_datetime(&ndt);

        assert_eq!(
            date_perhaps_time_to_naive(&DatePerhapsTime::Date(nd)),
            Some(nd.and_hms_opt(0, 0, 0).unwrap())
        );
        assert_eq!(
            date_perhaps_time_to_naive(&DatePerhapsTime::DateTime(CalendarDateTime::Utc(utc))),
            Some(ndt)
        );
        assert_eq!(
            date_perhaps_time_to_naive(&DatePerhapsTime::DateTime(CalendarDateTime::Floating(ndt))),
            Some(ndt)
        );
        assert_eq!(
            date_perhaps_time_to_naive(&DatePerhapsTime::DateTime(CalendarDateTime::WithTimezone {
                date_time: ndt,
                tzid: "Asia/Shanghai".into()
            })),
            Some(ndt)
        );
    }

    #[test]
    fn build_event_row_filters_by_date() {
        let dt = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        let event = Event::new()
            .summary("meeting")
            .starts(dt)
            .ends(dt + chrono::Duration::hours(1))
            .done();

        let target = chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
        let row = build_event_row("/cal/ev.ics", &event, &target).expect("should match today");
        assert_eq!(row.summary, "meeting");
        assert_eq!(row.href, "/cal/ev.ics");
        assert!(row.start.contains("2026-07-09"));

        let other = chrono::NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        assert!(build_event_row("/cal/ev.ics", &event, &other).is_none());
    }

    #[test]
    fn icalendar_event_roundtrip() {
        let dt = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        let event = Event::new()
            .summary("测试会议")
            .starts(dt)
            .ends(dt + chrono::Duration::hours(1))
            .done();
        let cal = Calendar::new().push(event).done();
        let ics = cal.to_string();

        // 序列化结果含 VEVENT 与 SUMMARY，且为 CRLF 行结束。
        assert!(ics.contains("BEGIN:VEVENT"));
        assert!(ics.contains("SUMMARY:测试会议"));
        assert!(ics.contains("\r\n"));

        // 解析回来，摘要与开始时间一致。
        let parsed: Calendar = ics.parse().expect("parse ics");
        let ev = parsed.events().next().expect("one event");
        assert_eq!(ev.get_summary(), Some("测试会议"));
        let start = ev.get_start().expect("has start");
        let start_ndt = date_perhaps_time_to_naive(&start).unwrap();
        assert_eq!(start_ndt, dt.naive_utc());
    }

    #[test]
    fn event_filename_is_nonempty_hex() {
        let name = event_filename();
        assert!(!name.is_empty());
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
