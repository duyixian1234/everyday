//! Calendar module (CalDAV): login / calendars / list / add / delete.
//!
//! Flow: config.toml holds account metadata (caldav_url/username) → `everyday cal login`
//! stores the password in the system keyring → `everyday cal calendars/list/add/delete`
//! auto-reads the password to connect to CalDAV. The password never touches config.toml.
//!
//! Stack: libdav 0.10 (CalDAV protocol, request API) + icalendar 0.17 (iCalendar
//! parse/generate) + hyper 1.x (HTTP, body=String to satisfy libdav's HttpClient trait)
//! + hyper-rustls (ring TLS, webpki roots) + tower-http (Basic Auth middleware that
//!   overwrites the Authorization header).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
use crate::modules::{Executor, parse_simple_args};
use crate::output::Output;
use crate::search::{Hit, SearchQuery, Searchable};

/// hyper-rustls HTTPS connector (webpki roots + http1).
type HttpsConnector =
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>;
/// hyper legacy client with Basic Auth, body type `String`.
/// libdav's HttpClient blanket impl requires `Service<Request<String>, Response=Response<Incoming>>`,
/// so the body generic is pinned to `String` (http-body 1.0 implements `impl Body for String`).
type HttpsClient = AddAuthorization<HyperClient<HttpsConnector, String>>;
/// Concrete type of CalDavClient (type alias to avoid leaking generics into signatures).
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
    fn description(&self) -> &'static str {
        "Calendar management (CalDAV): login, calendars, list, add, delete events."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "login",
                description: "保存 CalDAV 凭证到系统 keyring",
                usage: "everyday cal login [--account NAME]",
                args: &[],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "calendars",
                description: "列出日历集",
                usage: "everyday cal calendars [--account NAME]",
                args: &[],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "list",
                description: "列出日历事件",
                usage: "everyday cal list [--today|--date YYYY-MM-DD|--all] [--limit N] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "today",
                        help: "仅今日",
                        kind: ArgKind::Bool,
                    },
                    ArgSpec {
                        name: "date",
                        help: "指定日期 YYYY-MM-DD",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "all",
                        help: "返回全部（默认今天及未来）",
                        kind: ArgKind::Bool,
                    },
                    ArgSpec {
                        name: "limit",
                        help: "条数上限",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "add",
                description: "新增事件",
                usage: "everyday cal add --title T --start ISO --end ISO [--location L] [--description D] [--calendar HREF] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "title",
                        help: "标题",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "start",
                        help: "开始时间（RFC3339 或 YYYY-MM-DDTHH:MM:SS）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "end",
                        help: "结束时间",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "location",
                        help: "地点",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "description",
                        help: "描述",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "calendar",
                        help: "目标日历 href 或显示名",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "delete",
                description: "删除事件",
                usage: "everyday cal delete --id HREF [--account NAME]",
                args: &[ArgSpec {
                    name: "id",
                    help: "事件 href",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "cal",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _) = parse_simple_args(args);
        let account = self
            .config
            .calendar_account(flags.get("account").map(|s| s.as_str()))?;

        // Recognize an unknown action early (pitfall 10: avoid surfacing AuthError instead
        // of UnknownAction when the password is empty).
        // The ignored-calendar list belongs to the account ([[calendar.accounts]]'s
        // ignore_calendars).
        let ignored = &account.ignore_calendars;
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

// ============ keyring credentials ============

/// Read the account password from the system keyring.
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

/// Prompt for the password interactively and store it in the system keyring.
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

    // Pitfall 9: empty-password guard. set_password("") succeeds, but base64-encodes
    // to "Basic Og==" and the server then returns 401.
    if password.is_empty() {
        return Err(AgentError::InvalidArgument(
            "password cannot be empty".into(),
        ));
    }
    entry
        .set_password(&password)
        .map_err(|e| AgentError::Auth(format!("keyring set: {e}")))?;
    Ok(Output::text(format!(
        "password stored for calendar account '{account_name}'"
    )))
}

// ============ CalDAV client construction ============

/// Build a CalDavClient: hyper + rustls(ring, webpki) + Basic Auth + well-known discovery.
///
/// We **skip** `bootstrap_via_service_discovery` (pitfall 5: its internal DNS SRV
/// `_caldavs._tcp` fallback is not implemented by domestic providers, and the remote
/// DNS forcibly closes the connection with os error 10054). Instead we use
/// `find_context_path` for only the `/.well-known/caldav` redirect probe (pitfall 6: QQ's
/// root URL PROPFIND returns 404, well-known 301-redirects to `/calendar/`, up to 5 hops).
/// On probe failure we silently fall back to base_url. See
/// [C001](../../docs/adr/C001-caldav-stack.md).
async fn build_client(account: &CalendarAccount, password: &str) -> Result<CalDav> {
    let base: Uri = account.caldav_url.parse().map_err(|e| {
        AgentError::InvalidArgument(format!("invalid caldav_url '{}': {e}", account.caldav_url))
    })?;
    let host = base
        .host()
        .ok_or_else(|| {
            AgentError::InvalidArgument(format!("caldav_url missing host: {}", account.caldav_url))
        })?
        .to_string();
    let port = base.port_u16().unwrap_or_else(|| {
        if base.scheme_str() == Some("http") {
            80
        } else {
            443
        }
    });

    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build();
    let https_client =
        HyperClient::builder(TokioExecutor::new()).build::<_, String>(https_connector);
    let auth_client = AddAuthorization::basic(https_client, &account.username, password);
    let mut webdav = WebDavClient::new(base, auth_client);

    // well-known discovery (RFC 6764 §5), skipping SRV/TXT.
    let service = caldav_service_for_url(&webdav.base_url)
        .map_err(|e| AgentError::Network(format!("determine caldav service: {e}")))?;
    // Discovery failure is non-fatal: fall back to base_url (some servers expose no
    // usable well-known endpoint even though base_url works directly).
    if let Ok(Some(url)) = webdav.find_context_path(service, &host, port).await {
        // The real context path after redirection (e.g. https://dav.qq.com:443/calendar/).
        // base_url is a pub field, overwrite it directly (pitfall 6).
        webdav.base_url = url;
    }

    Ok(CalDavClient::new(webdav))
}

// ============ Calendar discovery ============

/// Display info for a calendar collection.
struct CalendarInfo {
    href: String,
    name: Option<String>,
    colour: Option<String>,
}

/// Discover and return all calendar collections (filtering out calendars whose
/// displayname matches an entry in `ignored`).
///
/// Flow (RFC 5397 + RFC 4791): current-user-principal → calendar-home-set → calendars.
/// Based on libdav's examples/find_calendars.rs. When principal or home-set discovery
/// fails (e.g. QQ doesn't support current-user-principal and PROPFIND returns 404), we
/// fall back to base_url as the home set.
async fn list_all_calendars(caldav: &CalDav, ignored: &[String]) -> Result<Vec<CalendarInfo>> {
    let home_sets: Vec<Uri> = match caldav.find_current_user_principal().await {
        Ok(Some(p)) => {
            // principal found → query calendar-home-set; on failure or empty, fall back to base_url.
            match caldav.request(FindCalendarHomeSet::new(p.path())).await {
                Ok(resp) if !resp.home_sets.is_empty() => resp.home_sets,
                _ => vec![caldav.base_url().clone()],
            }
        }
        _ => vec![caldav.base_url().clone()], // principal not found or query failed → fall back to base_url
    };

    let mut out = Vec::new();
    // One PROPFIND Depth:1 to fetch displayname + color + resourcetype.
    // QQ quirk: a Depth:0 displayname query on a single calendar returns 404, but a
    // Depth:1 batch query from the home set returns 200.
    // Based on the Python caldav library's get_calendars() implementation.
    let props = [
        &names::DISPLAY_NAME,
        &names::CALENDAR_COLOUR,
        &names::RESOURCETYPE,
    ];
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
            Err(_) => continue, // a single home-set query failure is non-fatal
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
            // Keep only calendar collections (resourcetype contains C:calendar).
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
            // Filter out ignored calendars (case-insensitive displayname match).
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

// ============ Action implementations ============

/// `cal calendars`: list all of the user's calendar collections (decoded href + unnamed placeholder).
async fn cal_calendars(
    account: &CalendarAccount,
    password: &str,
    ignored: &[String],
) -> Result<Output> {
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

/// `cal list`: list events. By default returns events from all calendars; `--today`
/// limits to today, `--date YYYY-MM-DD` limits to a given date.
///
/// Strategy: use `GetCalendarResources` (calendar-query REPORT) to pull every calendar's
/// events in full (with calendar-data), parse VEVENTs locally with icalendar, then filter
/// by optional date. This is more reliable than a server-side time-range REPORT (domestic
/// servers vary in time-range quality and may return empty). See
/// [C002](../../docs/adr/C002-full-pull-local-filter.md).
async fn cal_list(
    account: &CalendarAccount,
    password: &str,
    flags: &HashMap<String, String>,
    ignored: &[String],
) -> Result<Output> {
    let caldav = build_client(account, password).await?;
    let calendars = list_all_calendars(&caldav, ignored).await?;
    let limit: usize = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    // Default: today and future; --all: everything; --today: today only; --date YYYY-MM-DD: that day.
    let today = chrono::Local::now().date_naive();
    let (exact_date, min_date): (Option<chrono::NaiveDate>, Option<chrono::NaiveDate>) =
        if flags.contains_key("all") {
            (None, None)
        } else if flags.contains_key("today") {
            (Some(today), None)
        } else if let Some(d) = flags.get("date") {
            (Some(parse_date(d)?), None)
        } else {
            (None, Some(today)) // default: today and the future
        };

    let mut events: Vec<EventRow> = Vec::new();
    for cal in &calendars {
        let resp = match caldav.request(GetCalendarResources::new(&cal.href)).await {
            Ok(r) => r,
            Err(_) => continue, // a single calendar's fetch failure is non-fatal
        };
        for res in resp.resources {
            let content = match res.content {
                Ok(c) => c,
                Err(_) => continue,
            };
            // Parse iCalendar, extract VEVENTs, and apply date filtering.
            if let Ok(parsed) = content.data.parse::<Calendar>() {
                for event in parsed.events() {
                    if let Some(row) = build_event_row(&res.href, event) {
                        let d = row.sort_key.date();
                        let keep =
                            exact_date.is_none_or(|e| d == e) && min_date.is_none_or(|m| d >= m);
                        if keep {
                            events.push(row);
                        }
                    }
                }
            }
        }
    }

    // Sort: future events first (ascending start time, nearest future first), past events
    // after (descending, nearest past first). This keeps a flood of past events (e.g. contact
    // birthdays) from filling the limit and hiding upcoming events.
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
        vec![
            "路径".into(),
            "开始".into(),
            "结束".into(),
            "主题".into(),
            "地点".into(),
        ],
        rows,
    ))
}

/// `cal add`: add an event. Build a VEVENT with icalendar and PUT it to the target calendar.
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

    // Build the VEVENT. Event's builder methods return &mut Self, so we create it owned,
    // chain, then call .done() at the end.
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
    // icalendar's fmt_write emits CRLF via write_crlf!, but normalization guarantees the
    // whole body is CRLF (required by CalDAV).
    let ics = normalize_crlf(&calendar.to_string());

    let caldav = build_client(account, password).await?;
    let calendars = list_all_calendars(&caldav, ignored).await?;

    // Pick the target calendar: --calendar HREF or name match, default to the first.
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

    // Build the new href: <calendar_href>/<timestamp>.ics. UID is auto-generated by icalendar.
    let new_href = format!(
        "{}{}.ics",
        ensure_trailing_slash(&target.href),
        event_filename()
    );

    let resp = caldav
        .request(PutResource::new(&new_href).create(ics, "text/calendar; charset=utf-8"))
        .await
        .map_err(|e| AgentError::Network(format!("put event: {e}")))?;

    Ok(Output::text(format!(
        "event added: {new_href} (etag: {})",
        resp.etag.unwrap_or_else(|| "n/a".into())
    )))
}

/// `cal delete`: delete an event by href (force delete, unconditional).
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

// ============ Helper functions ============

/// A single event's display row + sort key.
struct EventRow {
    href: String,
    start: String,
    end: String,
    summary: String,
    location: String,
    sort_key: chrono::NaiveDateTime,
}

/// Build a display row from a parsed VEVENT (no date filtering; filtering is done by
/// the caller `cal_list`).
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

/// Convert [`DatePerhapsTime`] to [`chrono::NaiveDateTime`] for sorting/filtering
/// (local-time order).
///
/// - `Date` variant pads 00:00:00 (all-day event).
/// - `Utc` takes naive_utc (normalized to the UTC instant).
/// - `Floating` / `WithTimezone` take the naive part (local time, more intuitive for
///   "today's events").
///
/// We don't enable icalendar's `chrono-tz` feature, so we avoid `try_into_utc` and use
/// NaiveDateTime for local-time sorting, which is sufficient for single-day events and
/// matches user expectations better.
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

/// Format a [`DatePerhapsTime`] as a human-readable string.
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

/// Parse a `YYYY-MM-DD` date.
fn parse_date(s: &str) -> Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
        AgentError::InvalidArgument(format!("invalid date '{s}' (expected YYYY-MM-DD): {e}"))
    })
}

/// Parse a date-time, accepting three forms:
/// - `2026-07-09T14:00:00Z` (UTC, RFC3339)
/// - `2026-07-09T14:00:00+08:00` (with offset, RFC3339)
/// - `2026-07-09T14:00:00` (no timezone, treated as UTC)
fn parse_datetime(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // No timezone suffix: parse as UTC (NaiveDateTime::and_utc returns DateTime<Utc>, not Option).
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(AgentError::InvalidArgument(format!(
        "invalid datetime '{s}' (expected RFC3339 like 2026-07-09T14:00:00Z or 2026-07-09T14:00:00)"
    )))
}

/// Normalize line endings to CRLF: first collapse `\r\n` and `\r` to `\n`, then turn `\n` into `\r\n`.
///
/// icalendar's `fmt_write` already emits CRLF via `write_crlf!`, but property values may embed
/// bare `\n`/`\r`; normalization guarantees consistently CRLF-terminated lines (required by
/// CalDAV / RFC 5545).
fn normalize_crlf(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

/// Ensure an href ends with `/` (used when composing event hrefs).
fn ensure_trailing_slash(s: &str) -> String {
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

/// Decode percent-encoded strings (e.g. `%40` → `@`, `%20` → space) for display of calendar hrefs.
///
/// Invalid `%XX` sequences are preserved verbatim. Hand-rolled minimal implementation with no extra deps.
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

/// Convert a single hexadecimal digit to its numeric value.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Generate an event filename (nanosecond timestamp; unique enough for a single-user scenario).
fn event_filename() -> String {
    let now = chrono::Utc::now();
    let nanos = now.timestamp_nanos_opt().unwrap_or(0);
    format!("{nanos:x}")
}

// ============ Timeline data fetching ============

/// Raw calendar event data for Timeline ingestion.
pub struct CalTimelineEntry {
    pub href: String,
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub start: String,
    pub end: String,
}

/// Timeline ingestion: fetch all calendar events from CalDAV.
///
/// Cal uses a window-refresh model ([C003](../../docs/adr/C003-cal-provider-window-filter.md)), so this
/// returns the current snapshot; the orchestrator deletes old rows inside the window and re-inserts.
pub async fn fetch_for_timeline(
    account: &CalendarAccount,
    ignored: &[String],
) -> Result<Vec<CalTimelineEntry>> {
    let password = get_password(account)?;
    let caldav = build_client(account, &password).await?;
    let calendars = list_all_calendars(&caldav, ignored).await?;

    let mut entries = Vec::new();
    for cal in &calendars {
        let resp = match caldav.request(GetCalendarResources::new(&cal.href)).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        for res in resp.resources {
            let content = match res.content {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Ok(parsed) = content.data.parse::<Calendar>() {
                for event in parsed.events() {
                    let start_dpt = match event.get_start() {
                        Some(s) => s,
                        None => continue,
                    };
                    let start_str = format_date_perhaps_time(&start_dpt);
                    let end_str = event
                        .get_end()
                        .as_ref()
                        .map(format_date_perhaps_time)
                        .unwrap_or_default();
                    let summary = event.get_summary().unwrap_or("").to_string();
                    let location = event.get_location().unwrap_or("").to_string();
                    // VEVENT UID is used as ref_id (per the iCalendar standard).
                    let uid = event.get_uid().unwrap_or(&res.href).to_string();
                    entries.push(CalTimelineEntry {
                        href: res.href.clone(),
                        uid,
                        summary,
                        location,
                        start: start_str,
                        end: end_str,
                    });
                }
            }
        }
    }
    Ok(entries)
}

// ============ Cross-module search (Phase 11) ============

/// Per-module hard cap, enforced inside the provider
/// ([S004](../../docs/adr/S004-execution-model.md)).
const SEARCH_PER_MODULE_CAP: usize = 50;

/// Cross-module search (Phase 11): full-pull every event from CalDAV and
/// GLOB-filter by summary / location / description (OR-of-tokens,
/// case-insensitive via lower()).
///
/// Calendar is the only non-append source; ADR [S005](../../docs/adr/S005-time-semantics-scope.md)
/// pins its primary `ts` to **event start time** so future events surface
/// naturally in `ts desc` ordering.
///
/// Per [C002](../../docs/adr/C002-full-pull-local-filter.md) the full-pull
/// is intentional (no server-side time-range filter), so the local
/// GLOB is the only filter step. Network failure surfaces as `Err`
/// which the aggregator captures into a `SearchWarning`.
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub async fn search_for_search(
    account: &CalendarAccount,
    ignored: &[String],
    q: &SearchQuery,
) -> Result<Vec<Hit>> {
    let tokens: Vec<&str> = q.tokens();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let entries = fetch_for_timeline(account, ignored).await?;

    // Build the local GLOB filter (no SQL; iterate in memory).
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();
    for t in &tokens {
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        for col in ["summary", "location", "start"] {
            params.push(format!("*{lower}*"));
            conds.push(col.to_string());
        }
    }
    if conds.is_empty() {
        return Ok(Vec::new());
    }

    // Per-row GLOB match over summary / location / start (description is
    // not stored in CalTimelineEntry, so use summary+location+start).
    let mut hits = Vec::new();
    for e in &entries {
        let summary_lc = e.summary.to_ascii_lowercase();
        let location_lc = e.location.to_ascii_lowercase();
        let start_lc = e.start.to_ascii_lowercase();
        let matches_any = tokens.iter().any(|t| {
            let lower = t.to_ascii_lowercase();
            summary_lc.contains(&lower) || location_lc.contains(&lower) || start_lc.contains(&lower)
        });
        if !matches_any {
            continue;
        }
        // ts = event start time ([S005](../../docs/adr/S005-time-semantics-scope.md)).
        let ts = parse_naive_dt_to_utc(&e.start);
        // snippet = "<start> @ <location>" — short, contextual.
        let snippet = if e.location.is_empty() {
            e.start.clone()
        } else {
            format!("{} @ {}", e.start, e.location)
        };
        hits.push(Hit {
            module: "cal",
            account: Some(account.name.clone()),
            id: e.uid.clone(),
            title: e.summary.clone(),
            snippet,
            url: None,
            ts,
            kind: "event",
        });
    }

    // Sort: ts desc (None last), then stable id asc.
    hits.sort_by(|a, b| match (a.ts, b.ts) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    if hits.len() > SEARCH_PER_MODULE_CAP {
        hits.truncate(SEARCH_PER_MODULE_CAP);
    }
    let _ = conds;
    let _ = params;
    Ok(hits)
}

/// Convert a calendar date string to a UTC DateTime. Mirrors the helper
/// in providers.rs; duplicated here to avoid leaking the timeline module's
/// internal helper. Returns None when parsing fails (DST boundary or
/// unexpected format).
fn parse_naive_dt_to_utc(s: &str) -> Option<DateTime<Utc>> {
    use chrono::TimeZone;
    // RFC3339 first.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    let formats = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d"];
    for fmt in &formats {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return chrono::Local
                .from_local_datetime(&ndt)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc));
        }
        if let Ok(nd) = chrono::NaiveDate::parse_from_str(s, fmt)
            && let Some(ndt) = nd.and_hms_opt(0, 0, 0)
        {
            return chrono::Local
                .from_local_datetime(&ndt)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc));
        }
    }
    None
}

/// Provider adapter: implements [`Searchable`] for one calendar account.
///
/// One provider per account. The full-pull is a network call;
/// transient failures are captured by the aggregator as `SearchWarning`.
#[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
pub struct CalSearchProvider {
    account: CalendarAccount,
    ignored: Vec<String>,
}

impl CalSearchProvider {
    /// Construct from a configured calendar account + its ignore list.
    #[allow(dead_code)] // public API: wired into SearchRegistry in a later commit.
    pub fn new(account: CalendarAccount, ignored: Vec<String>) -> Self {
        Self { account, ignored }
    }
}

#[async_trait]
impl Searchable for CalSearchProvider {
    fn module_name(&self) -> &'static str {
        "cal"
    }

    async fn search(&self, q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>> {
        if q.raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        search_for_search(&self.account, &self.ignored, q).await
    }
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
        assert_eq!(
            percent_decode("/calendar/duyixian1234%40qq.com"),
            "/calendar/duyixian1234@qq.com"
        );
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("no-encoding"), "no-encoding");
        // Invalid %XX sequences are preserved verbatim.
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
            date_perhaps_time_to_naive(&DatePerhapsTime::DateTime(
                CalendarDateTime::WithTimezone {
                    date_time: ndt,
                    tzid: "Asia/Shanghai".into()
                }
            )),
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

        // Serialized output contains a VEVENT and SUMMARY, and uses CRLF line endings.
        assert!(ics.contains("BEGIN:VEVENT"));
        assert!(ics.contains("SUMMARY:测试会议"));
        assert!(ics.contains("\r\n"));

        // Round-trip parse: summary and start time must match.
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

    /// `parse_naive_dt_to_utc` parses the formats produced by
    /// `format_date_perhaps_time` (RFC3339 / "%Y-%m-%d %H:%M" / date-only).
    #[test]
    fn parse_naive_dt_to_utc_handles_calendar_formats() {
        use chrono::Local;

        // RFC3339 (with offset) — fixed point: 14:00 +08:00 == 06:00 UTC.
        let dt = parse_naive_dt_to_utc("2026-07-09T14:00:00+08:00").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 7, 9, 6, 0, 0).unwrap());

        // Floating (no offset) — local interpretation. The date may
        // shift across the UTC day boundary depending on the local
        // timezone offset, so we verify against the *local* date instead.
        let dt = parse_naive_dt_to_utc("2026-07-09 14:00:00").unwrap();
        let local = dt.with_timezone(&Local);
        assert_eq!(
            local.format("%Y-%m-%d %H:%M").to_string(),
            "2026-07-09 14:00"
        );

        // Date-only: midnight local; the UTC date may shift but the local
        // date should round-trip.
        let dt = parse_naive_dt_to_utc("2026-07-09").unwrap();
        let local = dt.with_timezone(&Local);
        assert_eq!(local.format("%Y-%m-%d").to_string(), "2026-07-09");

        // Garbage.
        assert!(parse_naive_dt_to_utc("not a date").is_none());
    }

    /// Local GLOB filter behavior — verified with hand-built CalTimelineEntry
    /// samples (no network). The full-pull path is exercised end-to-end by
    /// `fetch_for_timeline` tests; here we focus on the in-memory GLOB.
    #[test]
    fn cal_search_glob_matches_summary_location_start() {
        // Replicate the in-memory GLOB filter from `search_for_search`.
        fn glob_match(entries: &[CalTimelineEntry], tokens: &[&str]) -> Vec<String> {
            entries
                .iter()
                .filter(|e| {
                    let summary_lc = e.summary.to_ascii_lowercase();
                    let location_lc = e.location.to_ascii_lowercase();
                    let start_lc = e.start.to_ascii_lowercase();
                    tokens.iter().any(|t| {
                        let lower = t.to_ascii_lowercase();
                        summary_lc.contains(&lower)
                            || location_lc.contains(&lower)
                            || start_lc.contains(&lower)
                    })
                })
                .map(|e| e.uid.clone())
                .collect()
        }

        let entries = vec![
            CalTimelineEntry {
                href: "/cal/a.ics".into(),
                uid: "a".into(),
                summary: "Rust 1.95 release party".into(),
                location: "线上".into(),
                start: "2026-07-09 14:00:00".into(),
                end: "2026-07-09 15:00:00".into(),
            },
            CalTimelineEntry {
                href: "/cal/b.ics".into(),
                uid: "b".into(),
                summary: "weekly sync".into(),
                location: "office".into(),
                start: "2026-07-10 10:00:00".into(),
                end: "2026-07-10 11:00:00".into(),
            },
            CalTimelineEntry {
                href: "/cal/c.ics".into(),
                uid: "c".into(),
                summary: "team lunch".into(),
                location: "office".into(),
                start: "2026-07-12 12:00:00".into(),
                end: "2026-07-12 13:00:00".into(),
            },
        ];

        // Single token "rust" -> only event a (summary).
        assert_eq!(glob_match(&entries, &["rust"]), vec!["a".to_string()]);

        // OR-of-tokens "rust sync" -> a (rust) + b (sync).
        let mut hits = glob_match(&entries, &["rust", "sync"]);
        hits.sort();
        assert_eq!(hits, vec!["a".to_string(), "b".to_string()]);

        // Location match: "office" -> b + c.
        let mut hits = glob_match(&entries, &["office"]);
        hits.sort();
        assert_eq!(hits, vec!["b".to_string(), "c".to_string()]);

        // Case-insensitive: "RUST" matches a.
        assert_eq!(glob_match(&entries, &["RUST"]), vec!["a".to_string()]);
    }
}
