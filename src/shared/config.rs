//! Config loading and multi-account management.
//!
//! Config file: `~/.config/everyday/config.toml` (resolved cross-platform
//! via `dirs`). Each module supports multiple named accounts; the top-level
//! `default_account` selects the default account name.
//! **Secrets are never stored in the config file** — they live in the OS
//! keyring (see the security red line in [agents.md](../../agents.md)).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};

/// Macro: expands the 5 `X_account()` template methods
/// (mail/calendar/note/todo/bookmark).
///
/// Each method does three things:
/// 1. pick `override_name` > default > error
/// 2. find by name within the module's accounts
/// 3. not found → AccountNotFound
///
/// The 5 methods used to be ~75 lines of copy-paste; the macro
/// collapses that to 5 call sites.
/// See [R007](../../docs/adr/R007-config-account-macro.md).
macro_rules! impl_account_lookup {
    ($name:ident, $module:literal, $field:ident, $account:ty) => {
        #[doc = concat!("解析 ", $module, " 账户：优先 `override_name`，其次默认，最后报错。")]
        pub fn $name(&self, override_name: Option<&str>) -> Result<&$account> {
            let want = override_name.or(self.default_account.$field.as_deref());
            let name = want.ok_or_else(|| {
                AgentError::AccountNotFound(format!(
                    "no {} account specified and no default set in [default_account]",
                    $module
                ))
            })?;
            self.$field
                .accounts
                .iter()
                .find(|a| a.name == name)
                .ok_or_else(|| AgentError::AccountNotFound(format!("{} account '{name}'", $module)))
        }
    };
}

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Per-module default account name mapping.
    #[serde(default)]
    pub default_account: DefaultAccount,

    /// Mail module configuration.
    #[serde(default)]
    pub mail: MailConfig,

    /// Calendar module configuration.
    #[serde(default)]
    pub calendar: CalendarConfig,

    /// RSS module configuration.
    #[serde(default)]
    pub rss: RssConfig,

    /// Note module configuration.
    #[serde(default)]
    pub note: NoteConfig,

    /// Todo module configuration (Notion-backed).
    #[serde(default)]
    pub todo: TodoConfig,

    /// Bookmark module configuration.
    #[serde(default)]
    pub bookmark: BookmarkConfig,
}

/// Per-module default account names.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefaultAccount {
    /// Default mail account name.
    #[serde(default)]
    pub mail: Option<String>,
    /// Default calendar account name.
    #[serde(default)]
    pub calendar: Option<String>,

    /// Default note account name.
    #[serde(default)]
    pub note: Option<String>,

    /// Default todo account name.
    #[serde(default)]
    pub todo: Option<String>,

    /// Default bookmark account name.
    #[serde(default)]
    pub bookmark: Option<String>,
}

// ---- Mail ----

/// Mail module configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MailConfig {
    /// Named account list.
    #[serde(default)]
    pub accounts: Vec<MailAccount>,
}

/// A single mail account. Password is NOT stored here — it lives in the keyring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailAccount {
    /// Account name (e.g. `work`, `personal`).
    pub name: String,
    pub imap_host: String,
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    pub username: String,
    /// Optional: whether to use SSL/TLS.
    #[serde(default = "default_true")]
    pub tls: bool,
}

fn default_imap_port() -> u16 {
    993
}
fn default_smtp_port() -> u16 {
    587
}
fn default_true() -> bool {
    true
}

// ---- Calendar ----

/// Calendar module configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalendarConfig {
    #[serde(default)]
    pub accounts: Vec<CalendarAccount>,
}

/// A single calendar account (CalDAV).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarAccount {
    pub name: String,
    pub caldav_url: String,
    pub username: String,
    /// Calendar names to ignore for this account (matched case-insensitively
    /// against the display name).
    ///
    /// Config example (under `[[calendar.accounts]]`):
    /// `ignore_calendars = ["friend's birthday", "Tasks"]`
    /// Ignored calendars never appear in `cal calendars` / `cal list` / `cal add`.
    #[serde(default)]
    pub ignore_calendars: Vec<String>,
}

// ---- RSS ----

/// RSS module configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RssConfig {
    /// Subscription feed list.
    #[serde(default)]
    pub feeds: Vec<RssFeed>,
}

/// A single RSS feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssFeed {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub category: Option<String>,
}

// ---- Note ----

/// Note module configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoteConfig {
    /// Named account list.
    #[serde(default)]
    pub accounts: Vec<NoteAccount>,
}

/// A single note account.
///
/// The `provider` field accepts `local`/`sqlite` (local SQLite, **default**)
/// and `notion` (remote Notion). Reserved for future backends
/// (e.g. `obsidian` local dir, `feishu` docs).
/// Credentials (Notion Integration Token) are never stored in the config
/// file — they live in the keyring (service = `everyday/note/<account>`).
/// See [F005](../../docs/adr/F005-default-provider-local.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteAccount {
    /// Account name (e.g. `personal`, `work`).
    pub name: String,
    /// Backend provider: `local`/`sqlite` (local SQLite, default) or
    /// `notion` (remote Notion).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Default database ID: used when `note create` omits `--db`.
    #[serde(default)]
    pub default_database_id: Option<String>,
    /// Default page ID: used when `note append`/`note read` omit page_id.
    #[serde(default)]
    pub default_page_id: Option<String>,
    /// SQLite file path for the local provider (only `local`/`sqlite`).
    /// Defaults to `~/.config/everyday/note-<account>.db`.
    #[serde(default)]
    pub db_path: Option<String>,
}

fn default_provider() -> String {
    "local".to_string()
}

// ---- Todo ----

/// Todo module configuration (Notion-backed task database).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TodoConfig {
    /// Named account list.
    #[serde(default)]
    pub accounts: Vec<TodoAccount>,
}

/// Shared fields for a Notion + local provider account.
///
/// `TodoAccount` and `BookmarkAccount` used to be byte-for-byte copies
/// (all 5 fields identical); this struct + type alias dedup them.
/// `NoteAccount` stays a separate type because its `default_page_id`
/// ("which page new notes go to") differs in meaning from the
/// `parent_page_id` ("which page the DB is built under at init-db")
/// used by todo/bookmark.
/// See [R010](../../docs/adr/R010-notion-local-account.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotionLocalAccount {
    /// Account name (e.g. `personal`, `work`).
    pub name: String,
    /// Backend provider: `local`/`sqlite` (local SQLite, default) or
    /// `notion` (remote Notion).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Parent page ID when creating the database (non-secret, on-disk).
    #[serde(default)]
    pub parent_page_id: Option<String>,
    /// Default database ID (filled back after `init-db`; explicit
    /// `--db` when absent).
    #[serde(default)]
    pub default_database_id: Option<String>,
    /// SQLite file path for the local provider (only `local`/`sqlite`).
    /// Defaults to `~/.config/everyday/<module>-<account>.db`.
    #[serde(default)]
    pub db_path: Option<String>,
}

/// A single todo account.
///
/// Shares `NotionLocalAccount` fields; the type alias keeps
/// backward compat (constructors using `TodoAccount { .. }` still
/// work — zero fields are filled by the Default impl).
pub type TodoAccount = NotionLocalAccount;

// ---- Bookmark ----

/// Bookmark module configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookmarkConfig {
    /// Named account list.
    #[serde(default)]
    pub accounts: Vec<BookmarkAccount>,
}

/// A single bookmark account.
///
/// Shares `NotionLocalAccount` fields; the type alias keeps
/// backward compat.
pub type BookmarkAccount = NotionLocalAccount;

// ---- Load / Save ----

impl Config {
    /// Return the canonical config file path.
    pub fn config_path() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
        Ok(dir.join("everyday").join("config.toml"))
    }

    /// Load from an explicit path.
    pub fn load_from(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let cfg: Self = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// Load from the default path; missing file → default config (no error).
    pub fn load_or_default() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load_from(&path)
    }

    // ---- Account lookup ----

    // The five `X_account()` methods used to copy-paste this ~15-line
    // template. Factored into a macro: expands at compile time,
    // zero runtime cost.
    // See [R007](../../docs/adr/R007-config-account-macro.md).
    impl_account_lookup!(mail_account, "mail", mail, MailAccount);
    impl_account_lookup!(calendar_account, "calendar", calendar, CalendarAccount);
    impl_account_lookup!(note_account, "note", note, NoteAccount);
    impl_account_lookup!(todo_account, "todo", todo, TodoAccount);
    impl_account_lookup!(bookmark_account, "bookmark", bookmark, BookmarkAccount);

    /// keyring service-name convention: `everyday/<module>/<account>`.
    /// See [F002](../../docs/adr/F002-multi-account-keyring.md).
    pub fn keyring_service(module: &str, account: &str) -> String {
        format!("everyday/{module}/{account}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[default_account]
mail = "work"
calendar = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
smtp_host = "smtp.example.com"
username = "me@example.com"

[[mail.accounts]]
name = "personal"
imap_host = "imap.gmail.com"
smtp_host = "smtp.gmail.com"
username = "me@gmail.com"

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/me"
username = "me"
ignore_calendars = ["好友生日", "Tasks"]

[[rss.feeds]]
name = "hackernews"
url = "https://hnrss.org/frontpage"
"#;

    #[test]
    fn parses_multi_account_config() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert_eq!(cfg.mail.accounts.len(), 2);
        assert_eq!(cfg.mail.accounts[0].name, "work");
        assert_eq!(cfg.mail.accounts[0].imap_port, 993); // default
        assert_eq!(cfg.calendar.accounts.len(), 1);
        assert_eq!(
            cfg.calendar.accounts[0].ignore_calendars,
            vec!["好友生日", "Tasks"]
        );
        assert_eq!(cfg.rss.feeds.len(), 1);
    }

    #[test]
    fn ignore_calendars_default_empty() {
        let cfg: Config = toml::from_str(
            "[[calendar.accounts]]\nname = \"x\"\ncaldav_url = \"u\"\nusername = \"u\"\n",
        )
        .unwrap();
        assert!(cfg.calendar.accounts[0].ignore_calendars.is_empty());
    }

    #[test]
    fn resolves_default_mail_account() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let acc = cfg.mail_account(None).unwrap();
        assert_eq!(acc.name, "work");
    }

    #[test]
    fn resolves_overridden_account() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let acc = cfg.mail_account(Some("personal")).unwrap();
        assert_eq!(acc.username, "me@gmail.com");
    }

    #[test]
    fn missing_account_errors() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let err = cfg.mail_account(Some("nonexistent")).unwrap_err();
        assert_eq!(err.type_name(), "AccountNotFound");
    }

    #[test]
    fn no_default_and_no_override_errors() {
        let cfg = Config::default();
        let err = cfg.mail_account(None).unwrap_err();
        assert_eq!(err.type_name(), "AccountNotFound");
    }

    #[test]
    fn empty_file_yields_default() {
        let tmp = std::env::temp_dir().join("everyday_empty_test.toml");
        std::fs::write(&tmp, "   \n").unwrap();
        let cfg = Config::load_from(&tmp).unwrap();
        assert!(cfg.mail.accounts.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let tmp = std::env::temp_dir().join("everyday_roundtrip_test.toml");
        let _ = std::fs::remove_file(&tmp);
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let text = toml::to_string_pretty(&cfg).unwrap();
        std::fs::write(&tmp, &text).unwrap();
        let reloaded = Config::load_from(&tmp).unwrap();
        assert_eq!(reloaded.mail.accounts.len(), 2);
        assert_eq!(reloaded.default_account.mail.as_deref(), Some("work"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn keyring_service_format() {
        assert_eq!(
            Config::keyring_service("mail", "work"),
            "everyday/mail/work"
        );
    }

    #[test]
    fn parses_note_account_config() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
note = "personal"

[[note.accounts]]
name = "personal"
provider = "notion"
default_database_id = "db_abc"
default_page_id = "page_xyz"
"#,
        )
        .unwrap();
        assert_eq!(cfg.note.accounts.len(), 1);
        assert_eq!(cfg.note.accounts[0].provider, "notion");
        assert_eq!(
            cfg.note.accounts[0].default_database_id.as_deref(),
            Some("db_abc")
        );
        assert_eq!(
            cfg.note.accounts[0].default_page_id.as_deref(),
            Some("page_xyz")
        );
    }

    #[test]
    fn note_provider_defaults_to_local() {
        let cfg: Config = toml::from_str(
            r#"
[[note.accounts]]
name = "x"
"#,
        )
        .unwrap();
        assert_eq!(cfg.note.accounts[0].provider, "local");
    }

    #[test]
    fn note_provider_explicit_notion_preserved() {
        // Backward-compat: an explicit `provider = "notion"` must be
        // preserved verbatim.
        let cfg: Config = toml::from_str(
            r#"
[[note.accounts]]
name = "x"
provider = "notion"
"#,
        )
        .unwrap();
        assert_eq!(cfg.note.accounts[0].provider, "notion");
    }

    #[test]
    fn resolves_default_note_account() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
note = "personal"

[[note.accounts]]
name = "personal"
provider = "notion"
"#,
        )
        .unwrap();
        let acc = cfg.note_account(None).unwrap();
        assert_eq!(acc.name, "personal");
    }

    #[test]
    fn parses_todo_account_config() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
todo = "personal"

[[todo.accounts]]
name = "personal"
provider = "notion"
parent_page_id = "page_xyz"
default_database_id = "db_abc"
"#,
        )
        .unwrap();
        assert_eq!(cfg.todo.accounts.len(), 1);
        assert_eq!(cfg.todo.accounts[0].provider, "notion");
        assert_eq!(
            cfg.todo.accounts[0].parent_page_id.as_deref(),
            Some("page_xyz")
        );
        assert_eq!(
            cfg.todo.accounts[0].default_database_id.as_deref(),
            Some("db_abc")
        );
    }

    #[test]
    fn todo_provider_defaults_to_local() {
        let cfg: Config = toml::from_str(
            r#"
[[todo.accounts]]
name = "x"
"#,
        )
        .unwrap();
        assert_eq!(cfg.todo.accounts[0].provider, "local");
    }

    #[test]
    fn todo_provider_explicit_notion_preserved() {
        // Backward-compat: an explicit `provider = "notion"` must be
        // preserved verbatim.
        let cfg: Config = toml::from_str(
            r#"
[[todo.accounts]]
name = "x"
provider = "notion"
"#,
        )
        .unwrap();
        assert_eq!(cfg.todo.accounts[0].provider, "notion");
    }

    #[test]
    fn resolves_default_todo_account() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
todo = "personal"

[[todo.accounts]]
name = "personal"
provider = "notion"
"#,
        )
        .unwrap();
        let acc = cfg.todo_account(None).unwrap();
        assert_eq!(acc.name, "personal");
    }

    #[test]
    fn parses_bookmark_account_config() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
bookmark = "personal"

[[bookmark.accounts]]
name = "personal"
provider = "notion"
parent_page_id = "page_xyz"
default_database_id = "db_abc"
"#,
        )
        .unwrap();
        assert_eq!(cfg.bookmark.accounts.len(), 1);
        assert_eq!(cfg.bookmark.accounts[0].provider, "notion");
        assert_eq!(
            cfg.bookmark.accounts[0].parent_page_id.as_deref(),
            Some("page_xyz")
        );
        assert_eq!(
            cfg.bookmark.accounts[0].default_database_id.as_deref(),
            Some("db_abc")
        );
    }

    #[test]
    fn bookmark_provider_defaults_to_local() {
        let cfg: Config = toml::from_str(
            r#"
[[bookmark.accounts]]
name = "x"
"#,
        )
        .unwrap();
        assert_eq!(cfg.bookmark.accounts[0].provider, "local");
    }
}
