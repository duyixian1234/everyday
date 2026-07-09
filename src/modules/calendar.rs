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
use libdav::caldav::{FindCalendarHomeSet, GetCalendarResources};
use libdav::dav::{Delete, Propfind, PutResource, WebDavClient};
use libdav::names;
use libdav::{CalDavClient, Depth, caldav_service_for_url};
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
            ActionDoc::new("list", "List events (default: today & future)", "everyday cal list [--today|--date YYYY-MM-DD|--all] [--limit N] [--account NAME]"),
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
        let ignored = &self.config.calendar.ignored_names;
        match action {
            "login" => cal_login(account).await,
            "calendars" | "list" | "add" | "delete" => {
                let password = get_password(account)?;
                match action {
                    "calendars" => cal_calendars(account, &password, ignored).await,
                    "list" => cal_list(account, &password, &flags, ignored).await,
                    "add" => cal_add(account, &password, &flags, ignored).await,
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

/// 发现并返回所有日历集合（过滤掉 `ignored` 中按 displayname 匹配的日历）。
///
/// 流程（RFC 5397 + RFC 4791）：current-user-principal → calendar-home-set → calendars。
/// 参照 libdav examples/find_calendars.rs。principal 或 home-set 发现失败（如 QQ 不支持
/// current-user-principal，PROPFIND 返 404）时降级用 base_url 作为 home set。
async fn list_all_calendars(caldav: &CalDav, ignored: &[String]) -> Result<Vec<CalendarInfo>> {
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
    // 一次 PROPFIND Depth:1 查 displayname + color + resourcetype。
    // QQ quirk: 对单日历 Depth:0 查 displayname 返 404，但从 home set Depth:1 批量查返 200。
    // 参照 Python caldav 库 get_calendars() 的实现。
    let props = [&names::DISPLAY_NAME, &names::CALENDAR_COLOUR, &names::RESOURCETYPE];
    for url in &home_sets {
        let resp = match caldav
            .request(
                Propfind::new(url.path())
                    .with_properties(&props)
                    .with_depth(Depth::One),
            )
            .await
        {
            Ok(r) => r,
            Err(_) => continue, // 单个 home set 查询失败不致命
        };
        let doc = match resp.xml_tree() {
            Ok(d) => d,
            Err(_) => continue,
        };
        for response in doc
            .root_element()
            .descendants()
            .filter(|n| n.tag_name() == names::RESPONSE)
        {
            let href = response
                .descendants()
                .find(|n| n.tag_name() == names::HREF)
                .and_then(|n| n.text())
                .unwrap_or("")
                .to_string();
            // 只保留 calendar 集合（resourcetype 含 C:calendar）。
            let is_calendar = response
                .descendants()
                .find(|n| n.tag_name() == names::RESOURCETYPE)
                .map(|rt| rt.descendants().any(|n| n.tag_name() == names::CALENDAR))
                .unwrap_or(false);
            if !is_calendar {
                continue;
            }
            let name = response
                .descendants()
                .find(|n| n.tag_name() == names::DISPLAY_NAME)
                .and_then(|n| n.text())
                .map(|s| s.to_string());
            let colour = response
                .descendants()
                .find(|n| n.tag_name() == names::CALENDAR_COLOUR)
                .and_then(|n| n.text())
                .map(|s| s.to_string());
            // 过滤忽略的日历（按 displayname 不区分大小写匹配）。
            if let Some(ref n) = name
                && ignored.iter().any(|ig| ig.eq_ignore_ascii_case(n))
            {
                continue;
            }
            out.push(CalendarInfo { href, name, colour });
        }
    }
    Ok(out)
}

// ============ 动作实现 ============

/// `cal calendars`：列出当前用户的所有日历集合（中文列名 + href 解码 + 无名占位）。
async fn cal_calendars(account: &CalendarAccount, password: &str, ignored: &[String]) -> Result<Output> {
    let caldav = build_client(account, password).await?;
    let calendars = list_all_calendars(&caldav, ignored).await?;
    let rows = calendars
        .into_iter()
        .map(|c| {
            vec![
                percent_decode(&c.href),
                c.name.unwrap_or_else(|| "未命名".into()),
                c.colour.unwrap_or_default(),
            ]
        })
        .collect();
    Ok(Output::records(
        vec!["路径".into(), "名称".into(), "颜色".into()],
        rows,
    ))
}

/// `cal list`：列出事件，默认返回所有日历的所有日程；`--today` 限今日，`--date YYYY-MM-DD` 限指定日期。
///
/// 策略：用 `GetCalendarResources`（calendar-query REPORT）全量拉取每个日历的事件
/// （含 calendar-data），本地用 icalendar 解析 VEVENT，再按可选日期过滤。比服务端
/// time-range REPORT 更可靠（国内服务端 time-range 实现质量参差，可能返空）。
async fn cal_list(
    account: &CalendarAccount,
    password: &str,
    flags: &HashMap<String, String>,
    ignored: &[String],
) -> Result<Output> {
    let caldav = build_client(account, password).await?;
    let calendars = list_all_calendars(&caldav, ignored).await?;
    let limit: usize = flags.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);
    // 默认返回今天及未来；--all 返回所有；--today 限今天；--date YYYY-MM-DD 限指定日期。
    let today = chrono::Local::now().date_naive();
    let (exact_date, min_date): (Option<chrono::NaiveDate>, Option<chrono::NaiveDate>) =
        if flags.contains_key("all") {
            (None, None)
        } else if flags.contains_key("today") {
            (Some(today), None)
        } else if let Some(d) = flags.get("date") {
            (Some(parse_date(d)?), None)
        } else {
            (None, Some(today)) // 默认：今天及未来
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
                    if let Some(row) = build_event_row(&res.href, event) {
                        let d = row.sort_key.date();
                        let keep = exact_date.is_none_or(|e| d == e)
                            && min_date.is_none_or(|m| d >= m);
                        if keep {
                            events.push(row);
                        }
                    }
                }
            }
        }
    }

    // 排序：未来事件优先（开始时间升序，最近未来在前），过去事件在后（降序，最近过去在前）。
    // 避免大量历史事件（如联系人生日）占满 limit 导致看不到未来日程。
    let now = chrono::Local::now().naive_local();
    events.sort_by(|a, b| {
        let (af, bf) = (a.sort_key >= now, b.sort_key >= now);
        match (af, bf) {
            (true, true) => a.sort_key.cmp(&b.sort_key),
            (false, false) => b.sort_key.cmp(&a.sort_key),
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
        }
    });
    events.truncate(limit);

    let rows = events
        .into_iter()
        .map(|e| vec![e.href, e.start, e.end, e.summary, e.location])
        .collect();
    Ok(Output::records(
        vec!["路径".into(), "开始".into(), "结束".into(), "主题".into(), "地点".into()],
        rows,
    ))
}

/// `cal add`：添加事件。用 icalendar 构造 VEVENT，PUT 到目标日历。
async fn cal_add(
    account: &CalendarAccount,
    password: &str,
    flags: &HashMap<String, String>,
    ignored: &[String],
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
    let calendars = list_all_calendars(&caldav, ignored).await?;

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

/// 从解析出的 VEVENT 构造展示行（不做日期过滤，过滤由调用方 `cal_list` 处理）。
fn build_event_row(href: &str, event: &Event) -> Option<EventRow> {
    let start_dpt = event.get_start()?;
    let start_ndt = date_perhaps_time_to_naive(&start_dpt)?;
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

/// 对 percent-encoded 字符串做解码（如 `%40` → `@`、`%20` → 空格），用于展示日历 href。
///
/// 非法 `%XX` 序列原样保留。无额外依赖，手写最小实现。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 单个十六进制字符转数值。
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
    fn percent_decode_common_sequences() {
        assert_eq!(percent_decode("/calendar/duyixian1234%40qq.com"), "/calendar/duyixian1234@qq.com");
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("no-encoding"), "no-encoding");
        // 非法 %XX 原样保留。
        assert_eq!(percent_decode("a%ZZb"), "a%ZZb");
        assert_eq!(percent_decode("a%4"), "a%4");
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
    fn build_event_row_constructs_row() {
        let dt = Utc.with_ymd_and_hms(2026, 7, 9, 14, 0, 0).unwrap();
        let event = Event::new()
            .summary("meeting")
            .starts(dt)
            .ends(dt + chrono::Duration::hours(1))
            .done();

        let row = build_event_row("/cal/ev.ics", &event).expect("should build row");
        assert_eq!(row.summary, "meeting");
        assert_eq!(row.href, "/cal/ev.ics");
        assert!(row.start.contains("2026-07-09"));
        assert_eq!(row.sort_key, dt.naive_utc());
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
