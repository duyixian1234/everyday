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

        // Deletions (R024): a soft-deleted todo is hidden from every read path,
        // including `fetch_for_timeline`, so without its own event the timeline
        // would freeze it at its last pre-deletion state — showing `In Progress`
        // for a task the user deleted. That is wrong information, not missing
        // data. `TodoDeletedEntry` reads past the current-state view to report
        // it.
        let deletions =
            todo_local::fetch_deletions_for_timeline(&self.account, window.from, window.to).await?;
        for d in &deletions {
            let deleted_ts = parse_rfc3339(&d.deleted_at).unwrap_or_else(Utc::now);
            events.push(TimelineEvent::new(
                "todo",
                Some(&self.account.name),
                "deleted",
                deleted_ts,
                &d.title,
                "",
                &d.id,
                json!({}),
            ));
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

// ============ helpers ============

/// Parse an RFC3339 time string.
fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    crate::util::datetime::parse_rfc3339(s)
}

// ============ Provider Registry ============

/// Build the TimelineProvider list (iterating over all configured accounts in config).
///
/// - Mail: one MailProvider per mail account.
/// - Cal: one CalProvider per calendar account.
/// - RSS: a single RssProvider (no account concept).
/// - Todo/Note/Bookmark: one local provider per account (local-only since
///   v0.13.0 — the Notion `OpsLogProvider` and the `add_dual_providers!`
///   macro were removed with the provider, [R019](../../../docs/adr/R019-remove-notion-provider.md)).
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

    // Todo / Note / Bookmark (local only)
    for acc in &config.todo.accounts {
        providers.push(Box::new(TodoProvider::new(acc.clone())));
    }
    for acc in &config.note.accounts {
        providers.push(Box::new(NoteProvider::new(acc.clone())));
    }
    for acc in &config.bookmark.accounts {
        providers.push(Box::new(BookmarkProvider::new(acc.clone())));
    }

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
    use crate::modules::todo::backend::TodoBackend;

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
    /// End-to-end against a **real** SQLite DB, which no test did before: the
    /// three local `fetch_for_timeline` functions had zero coverage, so the
    /// `deleted` branch had no baseline to be written against.
    #[tokio::test]
    async fn todo_provider_emits_created_and_deleted_events() {
        let path = std::env::temp_dir().join(format!(
            "everyday-todo-provider-test-{}.db",
            crate::util::id::gen_id("t")
        ));
        let account = crate::config::TodoAccount {
            name: "test".into(),
            provider: "local".into(),
            db_path: Some(path.to_string_lossy().to_string()),
        };

        // Seed through the real backend so the schema/migration path runs.
        let backend = crate::modules::todo::local::LocalTodoBackend::new(account.clone());
        let keep = backend.add("保留的任务", None, None).await.unwrap();
        let doomed = backend.add("要删的任务", None, None).await.unwrap();
        backend.delete(&doomed.id).await.unwrap();

        let provider = TodoProvider {
            account: account.clone(),
        };
        let window = TimeWindow {
            from: Utc::now() - chrono::Duration::hours(1),
            to: Utc::now() + chrono::Duration::hours(1),
        };
        let (events, mode) = provider.sync(&window).await.unwrap();

        assert!(matches!(mode, SyncMode::Append));

        // The live todo yields `created`.
        assert!(
            events
                .iter()
                .any(|e| e.event_type == "created" && e.ref_id == keep.id),
            "a live todo must produce a created event"
        );

        // The deleted one yields `deleted`, and **not** a stale live event.
        let deleted: Vec<_> = events
            .iter()
            .filter(|e| e.ref_id == doomed.id && e.event_type == "deleted")
            .collect();
        assert_eq!(deleted.len(), 1, "the deletion must be reported");
        assert_eq!(deleted[0].title, "要删的任务");
        assert!(
            !events
                .iter()
                .any(|e| e.ref_id == doomed.id && e.event_type != "deleted"),
            "a soft-deleted todo must not still look live"
        );

        let _ = std::fs::remove_file(path);
    }

    /// With nothing deleted, no `deleted` event is invented.
    #[tokio::test]
    async fn todo_provider_emits_no_deleted_event_without_a_deletion() {
        let path = std::env::temp_dir().join(format!(
            "everyday-todo-provider-test-{}.db",
            crate::util::id::gen_id("t")
        ));
        let account = crate::config::TodoAccount {
            name: "test".into(),
            provider: "local".into(),
            db_path: Some(path.to_string_lossy().to_string()),
        };
        let backend = crate::modules::todo::local::LocalTodoBackend::new(account.clone());
        backend.add("正常任务", None, None).await.unwrap();

        let provider = TodoProvider {
            account: account.clone(),
        };
        let window = TimeWindow {
            from: Utc::now() - chrono::Duration::hours(1),
            to: Utc::now() + chrono::Duration::hours(1),
        };
        let (events, _mode) = provider.sync(&window).await.unwrap();

        assert!(
            events.iter().all(|e| e.event_type != "deleted"),
            "no deletion means no deleted event"
        );
        let _ = std::fs::remove_file(path);
    }
}
