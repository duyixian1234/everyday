//! 各 source 的 TimelineProvider adapter。
//!
//! 每个 adapter 持有对应模块的账户配置，调用模块的 `fetch_for_timeline` 函数，
//! 将模块原生数据转换为 [`TimelineEvent`]。
//!
//! 依赖方向：timeline → 各模块（单向）。

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;

use crate::config::{
    BookmarkAccount, CalendarAccount, Config, MailAccount, NoteAccount, TodoAccount,
};
use crate::error::Result;
use crate::modules::bookmark_local;
use crate::modules::calendar;
use crate::modules::email;
use crate::modules::note_local;
use crate::modules::rss;
use crate::modules::timeline::{SyncMode, TimeWindow, TimelineEvent, TimelineProvider};
use crate::modules::todo_local;

// ============ Mail ============

/// Mail timeline provider（IMAP 拉取）。
pub struct MailProvider {
    account: MailAccount,
}

impl MailProvider {
    pub fn new(account: MailAccount) -> Self {
        Self { account }
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
        let entries = email::fetch_for_timeline(&self.account, window.from, window.to).await?;
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

/// 解析 RFC2822 邮件日期为 UTC DateTime。
fn parse_mail_date(s: &str) -> Option<DateTime<Utc>> {
    let cleaned = s.split('(').next().unwrap_or("").trim_end();
    chrono::DateTime::parse_from_rfc2822(cleaned)
        .ok()
        .or_else(|| chrono::DateTime::parse_from_rfc2822(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

// ============ Calendar ============

/// Calendar timeline provider（CalDAV 拉取，窗口刷新模式）。
pub struct CalProvider {
    account: CalendarAccount,
    ignored: Vec<String>,
}

impl CalProvider {
    pub fn new(account: CalendarAccount, ignored: Vec<String>) -> Self {
        Self { account, ignored }
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

    async fn sync(&self, _window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)> {
        let entries = calendar::fetch_for_timeline(&self.account, &self.ignored).await?;
        let events: Vec<TimelineEvent> = entries
            .iter()
            .filter_map(|e| {
                let timestamp = parse_naive_dt(&e.start)?;
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

/// 解析 NaiveDateTime 字符串（如 "2026-07-11 14:00:00"）为 UTC DateTime。
/// 假设本地时区（CalDAV 返回的时间多为本地浮动时间）。
///
/// 在 DST 边界（spring-forward gap / fall-back ambiguous）返回 None，
/// 之前用 .unwrap() 会 panic。
fn parse_naive_dt(s: &str) -> Option<DateTime<Utc>> {
    // 尝试 RFC3339 解析（带时区的情况）。
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // 尝试 NaiveDateTime 解析（浮动时间，按本地时区处理）。
    let formats = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d"];
    for fmt in &formats {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            // earliest() 处理 DST ambiguous（取较早的 offset）。
            // None 仅发生在 spring-forward gap —— 该 NaiveDateTime 在本地不存在。
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

/// RSS timeline provider（HTTP 抓取，追加模式）。
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

/// Todo timeline provider（本地 SQLite 拉取）。
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
            // created 事件
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
            // 若 updated_at 有值且与 created_at 不同，生成状态变化事件。
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

/// Note timeline provider（本地 SQLite 拉取）。
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

/// Bookmark timeline provider（本地 SQLite 拉取）。
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

/// 把 ops-log.db 中某 module 的行投影为 TimelineEvent。
///
/// 由 notion 账户的 todo/note/bookmark 写入路径（AOP hook）→ ops-log。
/// 这里把 ops-log 转成 timeline events,使这些操作能在 `timeline list` 中可见。
///
/// ADR 0007 不查 notion API，timeline 通过此 provider 看本地 ops-log。
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
        // ops-log 单 provider 覆盖该 module 的所有 account，
        // 通过事件自身的 `account` 字段区分；sync_state 行 account = ""。
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

// ============ 辅助 ============

/// 解析 RFC3339 时间字符串。
fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

// ============ Provider Registry ============

/// 构建 TimelineProvider 列表（遍历 config 中所有已配置的账户）。
///
/// - Mail：每个 mail 账户一个 MailProvider。
/// - Cal：每个 calendar 账户一个 CalProvider。
/// - RSS：单个 RssProvider（无账户概念）。
/// - Todo/Note/Bookmark：
///   - local provider 账户 → 单独的 XxxProvider（直拉 SQLite）。
///   - notion provider 账户 → 注册 [`OpsLogProvider`]（投影 ops-log.db）。
///
/// 两者可共存（不同账户不同 provider）。
pub fn build_providers(config: &Arc<Config>) -> Vec<Box<dyn TimelineProvider>> {
    let mut providers: Vec<Box<dyn TimelineProvider>> = Vec::new();

    // Mail
    for acc in &config.mail.accounts {
        providers.push(Box::new(MailProvider::new(acc.clone())));
    }

    // Calendar
    for acc in &config.calendar.accounts {
        providers.push(Box::new(CalProvider::new(
            acc.clone(),
            acc.ignore_calendars.clone(),
        )));
    }

    // RSS（无账户，单个）
    if !config.rss.feeds.is_empty() {
        providers.push(Box::new(RssProvider::new(config.clone())));
    }

    // Todo：local 账户走直拉，notion 账户走 ops-log 投影。
    let has_notion_todo = config
        .todo
        .accounts
        .iter()
        .any(|a| !crate::modules::local::is_local_provider(&a.provider));
    for acc in &config.todo.accounts {
        if crate::modules::local::is_local_provider(&acc.provider) {
            providers.push(Box::new(TodoProvider::new(acc.clone())));
        }
    }
    if has_notion_todo {
        providers.push(Box::new(OpsLogProvider::new("todo")));
    }

    // Note：同上。
    let has_notion_note = config
        .note
        .accounts
        .iter()
        .any(|a| !crate::modules::local::is_local_provider(&a.provider));
    for acc in &config.note.accounts {
        if crate::modules::local::is_local_provider(&acc.provider) {
            providers.push(Box::new(NoteProvider::new(acc.clone())));
        }
    }
    if has_notion_note {
        providers.push(Box::new(OpsLogProvider::new("note")));
    }

    // Bookmark：同上。
    let has_notion_bookmark = config
        .bookmark
        .accounts
        .iter()
        .any(|a| !crate::modules::local::is_local_provider(&a.provider));
    for acc in &config.bookmark.accounts {
        if crate::modules::local::is_local_provider(&acc.provider) {
            providers.push(Box::new(BookmarkProvider::new(acc.clone())));
        }
    }
    if has_notion_bookmark {
        providers.push(Box::new(OpsLogProvider::new("bookmark")));
    }

    providers
}

/// 按来源过滤 providers（`--source mail,cal` 只保留 mail 和 cal 的 provider）。
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
