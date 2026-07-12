//! TimelineProvider adapters for each source.
//!
//! Each adapter holds the account config of its module and calls the module's
//! `fetch_for_timeline` function to convert the module's native data into a
//! [`TimelineEvent`].
//!
//! Dependency direction: timeline -> each module (one-way)
//! [L004](../../../docs/adr/L004-timeline-provider-pull-only.md).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;

use crate::config::{
    BookmarkAccount, CalendarAccount, Config, MailAccount, NoteAccount, TodoAccount,
};
use crate::error::Result;
use crate::modules::bookmark::local as bookmark_local;
use crate::modules::calendar;
use crate::modules::email;
use crate::modules::note::local as note_local;
use crate::modules::rss;
use crate::modules::timeline::{SyncMode, TimeWindow, TimelineEvent, TimelineProvider};
use crate::modules::todo::local as todo_local;

// ============ Mail ============

/// Mail timeline provider (IMAP pull).
pub struct MailProvider {
    config: Arc<Config>,
    account: MailAccount,
}

impl MailProvider {
    pub fn new(config: Arc<Config>, account: MailAccount) -> Self {
        Self { config, account }
    }
}

#[async_trait]
impl TimelineProvider for MailProvider {
    fn source(&self) -> &'static str {
        "mail"
    }
    fn account(&self) -> Option<&str> {
        Some(&self.account.name)
    }

    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries =
            email::fetch_for_timeline(&self.config, &self.account, window.from, window.to).await?;
        let events: Vec<TimelineEvent> = entries
            .iter()
            .map(|e| {
                let timestamp = parse_mail_date(&e.date).unwrap_or_else(Utc::now);
                let ref_id = format!("{}:{}", self.account.name, e.uid);
                let summary = format!("From: {}\nFolder: {}", e.from, e.folder);
                let metadata = json!({
                    "from": e.from,
                    "folder": e.folder,
                });
                TimelineEvent::new(
                    "mail",
                    Some(&self.account.name),
                    "received",
                    timestamp,
                    &e.subject,
                    &summary,
                    &ref_id,
                    metadata,
                )
            })
            .collect();
        Ok((events, SyncMode::Append))
    }
}

/// Parse an RFC2822 mail date into a UTC DateTime.
fn parse_mail_date(s: &str) -> Option<DateTime<Utc>> {
    let cleaned = s.split('(').next().unwrap_or("").trim_end();
    chrono::DateTime::parse_from_rfc2822(cleaned)
        .ok()
        .or_else(|| chrono::DateTime::parse_from_rfc2822(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

// ============ Calendar ============

/// Calendar timeline provider (CalDAV pull, window-refresh mode).
pub struct CalProvider {
    config: Arc<Config>,
    account: CalendarAccount,
    ignored: Vec<String>,
}

impl CalProvider {
    pub fn new(config: Arc<Config>, account: CalendarAccount, ignored: Vec<String>) -> Self {
        Self {
            config,
            account,
            ignored,
        }
    }
}

#[async_trait]
impl TimelineProvider for CalProvider {
    fn source(&self) -> &'static str {
        "cal"
    }
    fn account(&self) -> Option<&str> {
        Some(&self.account.name)
    }

    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries =
            calendar::fetch_for_timeline(&self.config, &self.account, &self.ignored).await?;
        let events: Vec<TimelineEvent> = entries
            .iter()
            .filter_map(|e| {
                let timestamp = parse_naive_dt(&e.start)?;
                // Window filtering: CalDAV returns all calendar events at once, so they must be
                // trimmed to the window passed by the orchestrator ([watermark, now+7d]) to match
                // the WindowRefresh semantics [L002](../../../docs/adr/L002-calendar-window-refresh.md).
                // In WindowRefresh mode the orchestrator issues DELETE WHERE timestamp BETWEEN
                // window.from AND window.to - events outside the window should not appear in events at all.
                if timestamp < window.from || timestamp > window.to {
                    return None;
                }
                let summary = format!("{} - {}", e.start, e.end);
                let metadata = json!({
                    "calendar": e.href,
                    "location": e.location,
                    "start": e.start,
                    "end": e.end,
                });
                Some(TimelineEvent::new(
                    "cal",
                    Some(&self.account.name),
                    "scheduled",
                    timestamp,
                    &e.summary,
                    &summary,
                    &e.uid,
                    metadata,
                ))
            })
            .collect();
        Ok((events, SyncMode::WindowRefresh))
    }
}

/// Parse a NaiveDateTime string (e.g. "2026-07-11 14:00:00") into a UTC DateTime.
/// Assumes the local timezone (CalDAV timestamps are mostly local floating times).
///
/// Returns None at DST boundaries (spring-forward gap / fall-back ambiguous),
/// where the old `.unwrap()` would panic.
fn parse_naive_dt(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC3339 parsing (for timezone-bearing values).
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Try NaiveDateTime parsing (floating time, treated as local timezone).
    let formats = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d"];
    for fmt in &formats {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            // earliest() handles DST ambiguity (takes the earlier offset).
            // None only occurs at a spring-forward gap - this NaiveDateTime does not exist locally.
            return chrono::Local
                .from_local_datetime(&ndt)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc));
        }
        if let Ok(nd) = chrono::NaiveDate::parse_from_str(s, fmt) {
            let ndt = nd.and_hms_opt(0, 0, 0)?;
            return chrono::Local
                .from_local_datetime(&ndt)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc));
        }
    }
    None
}

// ============ RSS ============

/// RSS timeline provider (HTTP fetch, append mode).
pub struct RssProvider {
    config: Arc<Config>,
}

impl RssProvider {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl TimelineProvider for RssProvider {
    fn source(&self) -> &'static str {
        "rss"
    }
    fn account(&self) -> Option<&str> {
        None
    }

    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries = rss::fetch_for_timeline(&self.config, window.from, window.to).await?;
        let events: Vec<TimelineEvent> = entries
            .iter()
            .filter_map(|e| {
                let timestamp = e.published?;
                let ref_id = if e.guid.is_empty() {
                    e.link.clone()
                } else {
                    e.guid.clone()
                };
                let metadata = json!({
                    "feed": e.feed_name,
                    "url": e.feed_url,
                    "link": e.link,
                    "author": e.author,
                });
                Some(TimelineEvent::new(
                    "rss",
                    None,
                    "published",
                    timestamp,
                    &e.title,
                    &e.summary,
                    &ref_id,
                    metadata,
                ))
            })
            .collect();
        Ok((events, SyncMode::Append))
    }
}

// ============ Todo (local) ============

/// Todo timeline provider (local SQLite pull).
pub struct TodoProvider {
    account: TodoAccount,
}

impl TodoProvider {
    pub fn new(account: TodoAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl TimelineProvider for TodoProvider {
    fn source(&self) -> &'static str {
        "todo"
    }
    fn account(&self) -> Option<&str> {
        Some(&self.account.name)
    }

    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries = todo_local::fetch_for_timeline(&self.account, window.from, window.to).await?;
        let mut events = Vec::new();
        for e in &entries {
            let created_ts = parse_rfc3339(&e.created_at).unwrap_or_else(Utc::now);
            // created event
            events.push(TimelineEvent::new(
                "todo",
                Some(&self.account.name),
                "created",
                created_ts,
                &e.title,
                "",
                &e.id,
                json!({
                    "status": e.status,
                    "due": e.due,
                    "priority": e.priority,
                }),
            ));
            // If updated_at is present and differs from created_at, emit a status-change event.
            if !e.updated_at.is_empty()
                && let Some(updated_ts) = parse_rfc3339(&e.updated_at)
                && updated_ts != created_ts
            {
                let event_type = match e.status.as_str() {
                    "In Progress" => "started",
                    "Done" => "completed",
                    _ => "updated",
                };
                events.push(TimelineEvent::new(
                    "todo",
                    Some(&self.account.name),
                    event_type,
                    updated_ts,
                    &e.title,
                    "",
                    &e.id,
                    json!({
                        "status": e.status,
                        "due": e.due,
                        "priority": e.priority,
                    }),
                ));
            }
        }
        Ok((events, SyncMode::Append))
    }
}

// ============ Note (local) ============

/// Note timeline provider (local SQLite pull).
pub struct NoteProvider {
    account: NoteAccount,
}

impl NoteProvider {
    pub fn new(account: NoteAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl TimelineProvider for NoteProvider {
    fn source(&self) -> &'static str {
        "note"
    }
    fn account(&self) -> Option<&str> {
        Some(&self.account.name)
    }

    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries = note_local::fetch_for_timeline(&self.account, window.from, window.to).await?;
        let mut events = Vec::new();
        for e in &entries {
            let created_ts = parse_rfc3339(&e.created_at).unwrap_or_else(Utc::now);
            events.push(TimelineEvent::new(
                "note",
                Some(&self.account.name),
                "created",
                created_ts,
                &e.title,
                "",
                &e.id,
                json!({}),
            ));
            if e.updated_at != e.created_at
                && let Some(updated_ts) = parse_rfc3339(&e.updated_at)
            {
                events.push(TimelineEvent::new(
                    "note",
                    Some(&self.account.name),
                    "updated",
                    updated_ts,
                    &e.title,
                    "",
                    &e.id,
                    json!({}),
                ));
            }
        }
        Ok((events, SyncMode::Append))
    }
}

// ============ Bookmark (local) ============

/// Bookmark timeline provider (local SQLite pull).
pub struct BookmarkProvider {
    account: BookmarkAccount,
}

impl BookmarkProvider {
    pub fn new(account: BookmarkAccount) -> Self {
        Self { account }
    }
}

#[async_trait]
impl TimelineProvider for BookmarkProvider {
    fn source(&self) -> &'static str {
        "bookmark"
    }
    fn account(&self) -> Option<&str> {
        Some(&self.account.name)
    }

    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries =
            bookmark_local::fetch_for_timeline(&self.account, window.from, window.to).await?;
        let events: Vec<TimelineEvent> = entries
            .iter()
            .filter_map(|e| {
                let timestamp = parse_rfc3339(&e.created_at)?;
                let metadata = json!({
                    "url": e.url,
                    "tags": e.tags,
                });
                Some(TimelineEvent::new(
                    "bookmark",
                    Some(&self.account.name),
                    "added",
                    timestamp,
                    &e.title,
                    &e.url,
                    &e.id,
                    metadata,
                ))
            })
            .collect();
        Ok((events, SyncMode::Append))
    }
}

// ============ Ops-log Provider ============

/// Project a module's rows in ops-log.db into TimelineEvent.
///
/// Written by the todo/note/bookmark write paths of Notion accounts (AOP hook) -> ops-log.
/// Here ops-log is turned into timeline events so these operations become visible in
/// `timeline list`.
///
/// Per ADR [L007](../../../docs/adr/L007-notion-ops-log.md) we do not query the Notion API;
/// the timeline views the local ops-log through this provider
/// [L010](../../../docs/adr/L010-ops-log-provider.md).
pub struct OpsLogProvider {
    module: &'static str,
}

impl OpsLogProvider {
    pub fn new(module: &'static str) -> Self {
        debug_assert!(
            matches!(module, "todo" | "note" | "bookmark"),
            "OpsLogProvider only supports logged modules"
        );
        Self { module }
    }
}

#[async_trait]
impl TimelineProvider for OpsLogProvider {
    fn source(&self) -> &'static str {
        self.module
    }
    fn account(&self) -> Option<&str> {
        // The single ops-log provider covers all accounts of this module,
        // distinguished by the event's own `account` field; the sync_state row has account = "".
        None
    }

    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries =
            crate::ops_log::fetch_ops_log_for_timeline(self.module, window.from, Some(window.to))
                .await?;
        let events: Vec<TimelineEvent> = entries
            .iter()
            .filter_map(|e| {
                let timestamp = parse_rfc3339(&e.occurred_at)?;
                Some(TimelineEvent::new(
                    self.module,
                    Some(&e.account),
                    &e.action,
                    timestamp,
                    &e.title,
                    "",
                    &e.ref_id,
                    e.metadata.clone(),
                ))
            })
            .collect();
        Ok((events, SyncMode::Append))
    }
}

// ============ helpers ============

/// Parse an RFC3339 time string.
fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    crate::util::datetime::parse_rfc3339(s)
}

// ============ Provider Registry ============

/// Macro: register a local/notion dual-provider module (shared by todo/note/bookmark).
///
/// Pattern:
/// 1. Push one local Provider for each local account.
/// 2. If a Notion account exists, push a single [`OpsLogProvider`].
///
/// The `local_providers` expression must evaluate to `&[Account]`, and `provider_for`
/// is a constructor expression like `TodoProvider::new(acc.clone())`.
macro_rules! add_dual_providers {
    ($providers:expr, $module:literal, $local_providers:expr, $provider_for:expr) => {{
        let local_providers: &[_] = $local_providers;
        let mut has_notion = false;
        for acc in local_providers {
            if crate::modules::local::is_local_provider(&acc.provider) {
                $providers.push(Box::new($provider_for(acc.clone())));
            } else {
                has_notion = true;
            }
        }
        if has_notion {
            $providers.push(Box::new(OpsLogProvider::new($module)));
        }
    }};
}

/// Build the TimelineProvider list (iterating over all configured accounts in config).
///
/// - Mail: one MailProvider per mail account.
/// - Cal: one CalProvider per calendar account.
/// - RSS: a single RssProvider (no account concept).
/// - Todo/Note/Bookmark: local accounts use the local provider; Notion accounts share a
///   single [`OpsLogProvider`] (projecting ops-log.db)
///   [L010](../../../docs/adr/L010-ops-log-provider.md). The two can coexist.
pub fn build_providers(config: &Arc<Config>) -> Vec<Box<dyn TimelineProvider>> {
    let mut providers: Vec<Box<dyn TimelineProvider>> = Vec::new();

    // Mail
    for acc in &config.mail.accounts {
        providers.push(Box::new(MailProvider::new(config.clone(), acc.clone())));
    }

    // Calendar
    for acc in &config.calendar.accounts {
        providers.push(Box::new(CalProvider::new(
            config.clone(),
            acc.clone(),
            acc.ignore_calendars.clone(),
        )));
    }

    // RSS (no account, single)
    if !config.rss.feeds.is_empty() {
        providers.push(Box::new(RssProvider::new(config.clone())));
    }

    // Todo / Note / Bookmark: local/notion dual-provider pattern.
    add_dual_providers!(providers, "todo", &config.todo.accounts, TodoProvider::new);
    add_dual_providers!(providers, "note", &config.note.accounts, NoteProvider::new);
    add_dual_providers!(
        providers,
        "bookmark",
        &config.bookmark.accounts,
        BookmarkProvider::new
    );

    providers
}

/// Filter providers by source (`--source mail,cal` keeps only mail and cal providers).
pub fn filter_providers(
    providers: Vec<Box<dyn TimelineProvider>>,
    sources: &[String],
) -> Vec<Box<dyn TimelineProvider>> {
    if sources.is_empty() {
        return providers;
    }
    providers
        .into_iter()
        .filter(|p| sources.iter().any(|s| s == p.source()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rfc3339_valid() {
        let dt = parse_rfc3339("2026-07-11T06:30:00+00:00");
        assert!(dt.is_some());
    }

    #[test]
    fn parse_rfc3339_invalid() {
        assert!(parse_rfc3339("not a date").is_none());
    }

    #[test]
    fn parse_naive_dt_rfc3339() {
        let dt = parse_naive_dt("2026-07-11T14:00:00+08:00");
        assert!(dt.is_some());
    }

    #[test]
    fn parse_naive_dt_simple() {
        let dt = parse_naive_dt("2026-07-11 14:00:00");
        assert!(dt.is_some());
    }

    #[test]
    fn parse_naive_dt_date_only() {
        let dt = parse_naive_dt("2026-07-11");
        assert!(dt.is_some());
    }

    #[test]
    fn parse_mail_date_rfc2822() {
        let dt = parse_mail_date("Sat, 11 Jul 2026 14:30:00 +0800");
        assert!(dt.is_some());
    }

    #[test]
    fn build_providers_empty_config() {
        let config = Arc::new(Config::default());
        let providers = build_providers(&config);
        assert!(providers.is_empty());
    }

    #[test]
    fn filter_providers_by_source() {
        let config = Arc::new(Config {
            rss: crate::config::RssConfig {
                feeds: vec![crate::config::RssFeed {
                    name: "test".into(),
                    url: "https://example.com/feed".into(),
                    category: None,
                }],
            },
            ..Default::default()
        });
        let providers = build_providers(&config);
        assert_eq!(providers.len(), 1);

        let filtered = filter_providers(providers, &["mail".to_string()]);
        assert!(filtered.is_empty());
    }
}
