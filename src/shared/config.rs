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

/// Single source of truth for account resolution (P2a, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
///
/// Every account-bearing module config (`MailConfig`, `CalendarConfig`,
/// `NoteConfig`, `TodoConfig`, `BookmarkConfig`) implements this. Resolution
/// order is always: `--account` override > `[default_account]` > error.
///
/// The `default_name` is passed in by the caller because the `[default_account]`
/// section lives on the top-level `Config`, not on the per-module sections.
/// The default implementation in [`AccountProvider::resolve_account`] is the
/// one shared resolution algorithm — previously each module duplicated it via
/// the `impl_account_lookup!` macro ([R007](../../docs/adr/R007-config-account-macro.md)).
pub trait AccountProvider {
    /// The module's account type.
    type Account: NamedAccount;

    /// Module key used in error messages (e.g. `"mail"`).
    fn module_name(&self) -> &'static str;

    /// The module's account list.
    fn account_list(&self) -> &[Self::Account];

    /// Resolve `override_name` > `default_name` > error, returning the account.
    fn resolve_account<'a>(
        &'a self,
        override_name: Option<&str>,
        default_name: Option<&'a str>,
    ) -> Result<&'a Self::Account> {
        let want = override_name.or(default_name);
        let name = want.ok_or_else(|| {
            AgentError::AccountNotFound(format!(
                "no {} account specified and no default set in [default_account]",
                self.module_name()
            ))
        })?;
        self.account_list()
            .iter()
            .find(|a| a.name() == name)
            .ok_or_else(|| {
                AgentError::AccountNotFound(format!("{} account '{name}'", self.module_name()))
            })
    }
}

impl AccountProvider for MailConfig {
    type Account = MailAccount;
    fn module_name(&self) -> &'static str {
        "mail"
    }
    fn account_list(&self) -> &[MailAccount] {
        &self.accounts
    }
}

impl AccountProvider for CalendarConfig {
    type Account = CalendarAccount;
    fn module_name(&self) -> &'static str {
        "calendar"
    }
    fn account_list(&self) -> &[CalendarAccount] {
        &self.accounts
    }
}

impl AccountProvider for NoteConfig {
    type Account = NoteAccount;
    fn module_name(&self) -> &'static str {
        "note"
    }
    fn account_list(&self) -> &[NoteAccount] {
        &self.accounts
    }
}

impl AccountProvider for TodoConfig {
    type Account = TodoAccount;
    fn module_name(&self) -> &'static str {
        "todo"
    }
    fn account_list(&self) -> &[TodoAccount] {
        &self.accounts
    }
}

impl AccountProvider for BookmarkConfig {
    type Account = BookmarkAccount;
    fn module_name(&self) -> &'static str {
        "bookmark"
    }
    fn account_list(&self) -> &[BookmarkAccount] {
        &self.accounts
    }
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

// ---- Module config subsets (P2b, [F012](../../docs/adr/F012-architecture-deepening-phase.md)) ----
//
// Each business module receives only its own config section at construction,
// not the full `Config`. This removes hidden dependencies: a module can be
// built (and tested) with a minimal subset instead of a full global config.
// The subset owns account resolution (`override > default > error`), so
// modules no longer call `Config::X_account()` themselves.
//
// `timeline` / `search` / `auth` keep `Arc<Config>`: they are cross-module
// orchestrators that genuinely need every section.

macro_rules! impl_module_config {
    ($name:ident, $config:ident, $account:ty, $default_field:ident, $module_key:literal) => {
        /// Injected config subset for the module (P2b).
        #[derive(Debug, Clone, Default)]
        pub struct $name {
            /// Named account list (mirror of the module's `[[X.accounts]]`).
            pub accounts: Vec<$account>,
            /// Default account name (from `[default_account].X`).
            pub default_account: Option<String>,
        }

        impl From<&Config> for $name {
            fn from(c: &Config) -> Self {
                Self {
                    accounts: c.$config.accounts.clone(),
                    default_account: c.default_account.$default_field.clone(),
                }
            }
        }

        impl $name {
            /// Resolve the effective account: `--account` override > default > error.
            ///
            /// Same resolution algorithm as [`AccountProvider::resolve_account`],
            /// applied to the subset (the module no longer needs the full `Config`).
            pub fn resolve_account(&self, override_name: Option<&str>) -> Result<&$account> {
                let want = override_name.or(self.default_account.as_deref());
                let name = want.ok_or_else(|| {
                    AgentError::AccountNotFound(format!(
                        "no {} account specified and no default set in [default_account]",
                        $module_key
                    ))
                })?;
                self.accounts
                    .iter()
                    .find(|a| a.name() == name)
                    .ok_or_else(|| {
                        AgentError::AccountNotFound(format!("{} account '{name}'", $module_key))
                    })
            }
        }
    };
}

impl_module_config!(MailModuleConfig, mail, MailAccount, mail, "mail");
impl_module_config!(
    CalendarModuleConfig,
    calendar,
    CalendarAccount,
    calendar,
    "cal"
);
impl_module_config!(NoteModuleConfig, note, NoteAccount, note, "note");
impl_module_config!(TodoModuleConfig, todo, TodoAccount, todo, "todo");
impl_module_config!(
    BookmarkModuleConfig,
    bookmark,
    BookmarkAccount,
    bookmark,
    "bookmark"
);

/// RSS module config subset (no accounts; just the feed list).
#[derive(Debug, Clone, Default)]
pub struct RssModuleConfig {
    /// Subscription feed list.
    pub feeds: Vec<RssFeed>,
}

impl From<&Config> for RssModuleConfig {
    fn from(c: &Config) -> Self {
        Self {
            feeds: c.rss.feeds.clone(),
        }
    }
}

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
        cfg.validate()?;
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

    /// Semantic validation at load time (P2c, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
    ///
    /// Catches errors that would otherwise surface later as confusing runtime
    /// failures, while the config is still fresh in the user's mind:
    /// - `[default_account]` names that reference undefined accounts
    /// - empty required fields (hosts, usernames, feed URLs)
    /// - invalid `provider` values
    /// - malformed Notion page/database IDs
    ///
    /// An empty or default config (no accounts) is valid — modules without
    /// accounts (rss) or with local-only defaults (note/todo/bookmark) must
    /// keep working with zero configuration.
    pub fn validate(&self) -> Result<()> {
        validate_default_account(
            "mail",
            self.default_account.mail.as_deref(),
            &self.mail.accounts,
        )?;
        validate_default_account(
            "calendar",
            self.default_account.calendar.as_deref(),
            &self.calendar.accounts,
        )?;
        validate_default_account(
            "note",
            self.default_account.note.as_deref(),
            &self.note.accounts,
        )?;
        validate_default_account(
            "todo",
            self.default_account.todo.as_deref(),
            &self.todo.accounts,
        )?;
        validate_default_account(
            "bookmark",
            self.default_account.bookmark.as_deref(),
            &self.bookmark.accounts,
        )?;

        for a in &self.mail.accounts {
            require_nonempty("mail", &a.name, "name")?;
            require_nonempty("mail", &a.imap_host, "imap_host")?;
            require_nonempty("mail", &a.smtp_host, "smtp_host")?;
            require_nonempty("mail", &a.username, "username")?;
        }
        for a in &self.calendar.accounts {
            require_nonempty("calendar", &a.name, "name")?;
            require_nonempty("calendar", &a.caldav_url, "caldav_url")?;
            require_nonempty("calendar", &a.username, "username")?;
        }
        for f in &self.rss.feeds {
            require_nonempty("rss", &f.name, "name")?;
            require_nonempty("rss", &f.url, "url")?;
        }
        for a in &self.note.accounts {
            validate_note_account(a)?;
        }
        for a in &self.todo.accounts {
            validate_notion_local_account("todo", a)?;
        }
        for a in &self.bookmark.accounts {
            validate_notion_local_account("bookmark", a)?;
        }
        Ok(())
    }

    // ---- Config subsets (P2b) ----
    //
    // Business modules receive their own section at construction instead of the
    // full `Config`. These extractors are how `ModuleRegistry::build` slices the
    // config. See [F012](../../docs/adr/F012-architecture-deepening-phase.md).

    /// Mail module subset.
    pub fn mail_module_config(&self) -> MailModuleConfig {
        MailModuleConfig::from(self)
    }
    /// Calendar module subset.
    pub fn calendar_module_config(&self) -> CalendarModuleConfig {
        CalendarModuleConfig::from(self)
    }
    /// RSS module subset.
    pub fn rss_module_config(&self) -> RssModuleConfig {
        RssModuleConfig::from(self)
    }
    /// Note module subset.
    pub fn note_module_config(&self) -> NoteModuleConfig {
        NoteModuleConfig::from(self)
    }
    /// Todo module subset.
    pub fn todo_module_config(&self) -> TodoModuleConfig {
        TodoModuleConfig::from(self)
    }
    /// Bookmark module subset.
    pub fn bookmark_module_config(&self) -> BookmarkModuleConfig {
        BookmarkModuleConfig::from(self)
    }

    // ---- Account lookup ----
    //
    // Backward-compat accessors. All five delegate to the unified
    // `AccountProvider` implementation (single resolution algorithm).
    // See [F012](../../docs/adr/F012-architecture-deepening-phase.md) P2a.

    /// Resolve the mail account: `override_name` > default > error.
    pub fn mail_account(&self, override_name: Option<&str>) -> Result<&MailAccount> {
        self.mail
            .resolve_account(override_name, self.default_account.mail.as_deref())
    }

    /// Resolve the calendar account: `override_name` > default > error.
    pub fn calendar_account(&self, override_name: Option<&str>) -> Result<&CalendarAccount> {
        self.calendar
            .resolve_account(override_name, self.default_account.calendar.as_deref())
    }

    /// Resolve the note account: `override_name` > default > error.
    pub fn note_account(&self, override_name: Option<&str>) -> Result<&NoteAccount> {
        self.note
            .resolve_account(override_name, self.default_account.note.as_deref())
    }

    /// Resolve the todo account: `override_name` > default > error.
    pub fn todo_account(&self, override_name: Option<&str>) -> Result<&TodoAccount> {
        self.todo
            .resolve_account(override_name, self.default_account.todo.as_deref())
    }

    /// Resolve the bookmark account: `override_name` > default > error.
    pub fn bookmark_account(&self, override_name: Option<&str>) -> Result<&BookmarkAccount> {
        self.bookmark
            .resolve_account(override_name, self.default_account.bookmark.as_deref())
    }

    /// Resolve the effective account **name** for a module by key
    /// (`mail` / `cal` / `note` / `todo` / `bookmark`): `override_name` >
    /// default > error. String-level entry point for code that only needs the
    /// name (e.g. timeline / ops-log accounting).
    pub fn resolve_account_name(
        &self,
        module: &str,
        override_name: Option<&str>,
    ) -> Result<String> {
        match module {
            "mail" => Ok(self.mail_account(override_name)?.name.clone()),
            "calendar" | "cal" => Ok(self.calendar_account(override_name)?.name.clone()),
            "note" => Ok(self.note_account(override_name)?.name.clone()),
            "todo" => Ok(self.todo_account(override_name)?.name.clone()),
            "bookmark" => Ok(self.bookmark_account(override_name)?.name.clone()),
            other => Err(AgentError::InvalidArgument(format!(
                "unknown account module '{other}'"
            ))),
        }
    }

    /// keyring service-name convention: `everyday/<module>/<account>`.
    /// See [F002](../../docs/adr/F002-multi-account-keyring.md).
    pub fn keyring_service(module: &str, account: &str) -> String {
        format!("everyday/{module}/{account}")
    }
}

// ---- Shared account traits & validation helpers ----

/// An account that has a name — the common denominator for account lookup and
/// load-time validation across the five account-bearing modules.
pub(crate) trait NamedAccount {
    fn name(&self) -> &str;
}

impl NamedAccount for MailAccount {
    fn name(&self) -> &str {
        &self.name
    }
}
impl NamedAccount for CalendarAccount {
    fn name(&self) -> &str {
        &self.name
    }
}
impl NamedAccount for NoteAccount {
    fn name(&self) -> &str {
        &self.name
    }
}
/// Covers both `TodoAccount` and `BookmarkAccount` (type aliases of
/// `NotionLocalAccount`).
impl NamedAccount for NotionLocalAccount {
    fn name(&self) -> &str {
        &self.name
    }
}

/// `[default_account].X` must reference a defined account of that module.
fn validate_default_account<T: NamedAccount>(
    module: &str,
    default_name: Option<&str>,
    accounts: &[T],
) -> Result<()> {
    if let Some(name) = default_name
        && !accounts.iter().any(|a| a.name() == name)
    {
        return Err(AgentError::Config(format!(
            "[default_account].{module} = \"{name}\" but no {module} account with that name is defined"
        )));
    }
    Ok(())
}

fn require_nonempty(module: &str, value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(AgentError::Config(format!(
            "{module} config: {field} must not be empty"
        )));
    }
    Ok(())
}

/// Notion page/database IDs are 32 hex chars, optionally hyphenated
/// (8-4-4-4-12 form). `local`/`sqlite` providers never require them.
fn valid_notion_id(id: &str) -> bool {
    let compact = id.replace('-', "");
    compact.len() == 32 && compact.chars().all(|c| c.is_ascii_hexdigit())
}

/// Provider whitelist shared by note / todo / bookmark accounts.
fn validate_provider_whitelist(module: &str, account: &str, provider: &str) -> Result<()> {
    if !matches!(provider, "local" | "sqlite" | "notion") {
        return Err(AgentError::Config(format!(
            "{module} account '{account}': unknown provider '{provider}' (expected local|sqlite|notion)"
        )));
    }
    Ok(())
}

/// Notion-ID format check for a list of `(field, value)` pairs; only applied
/// when the account's provider is notion.
fn validate_notion_ids(module: &str, account: &str, ids: &[(&str, &Option<String>)]) -> Result<()> {
    for (field, id) in ids {
        if let Some(id) = id
            && !valid_notion_id(id)
        {
            return Err(AgentError::Config(format!(
                "{module} account '{account}': {field} '{}' is not a valid Notion ID (expected 32 hex chars)",
                id
            )));
        }
    }
    Ok(())
}

fn validate_note_account(a: &NoteAccount) -> Result<()> {
    require_nonempty("note", &a.name, "name")?;
    validate_provider_whitelist("note", &a.name, &a.provider)?;
    if a.provider == "notion" {
        validate_notion_ids(
            "note",
            &a.name,
            &[
                ("default_database_id", &a.default_database_id),
                ("default_page_id", &a.default_page_id),
            ],
        )?;
    }
    Ok(())
}

fn validate_notion_local_account(module: &str, a: &NotionLocalAccount) -> Result<()> {
    require_nonempty(module, &a.name, "name")?;
    validate_provider_whitelist(module, &a.name, &a.provider)?;
    if a.provider == "notion" {
        validate_notion_ids(
            module,
            &a.name,
            &[
                ("parent_page_id", &a.parent_page_id),
                ("default_database_id", &a.default_database_id),
            ],
        )?;
    }
    Ok(())
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

    // ---- P2c: load-time semantic validation (F012) ----

    #[test]
    fn validate_ok_for_sample_config() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_empty_default_config_ok() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn validate_default_account_must_exist() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
mail = "ghost"
"#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.type_name(), "ConfigError");
        assert!(
            err.message().contains("[default_account].mail"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn validate_mail_required_fields() {
        let cfg: Config = toml::from_str(
            r#"
[[mail.accounts]]
name = "x"
imap_host = ""
smtp_host = "s"
username = "u"
"#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.type_name(), "ConfigError");
        assert!(err.message().contains("imap_host"), "{}", err.message());
    }

    #[test]
    fn validate_calendar_url_required() {
        let cfg: Config = toml::from_str(
            r#"
[[calendar.accounts]]
name = "x"
caldav_url = ""
username = "u"
"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rss_feed_url_required() {
        let cfg: Config = toml::from_str(
            r#"
[[rss.feeds]]
name = "x"
url = ""
"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_provider_unknown() {
        let cfg: Config = toml::from_str(
            r#"
[[note.accounts]]
name = "x"
provider = "bogus"
"#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(err.message().contains("provider"), "{}", err.message());
    }

    #[test]
    fn validate_local_provider_allows_empty_ids() {
        let cfg: Config = toml::from_str(
            r#"
[[note.accounts]]
name = "x"
"#,
        )
        .unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_notion_id_malformed() {
        let cfg: Config = toml::from_str(
            r#"
[[note.accounts]]
name = "x"
provider = "notion"
default_database_id = "not-a-notion-id"
"#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(
            err.message().contains("default_database_id"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn validate_notion_id_hyphenated_ok() {
        let cfg: Config = toml::from_str(
            r#"
[[note.accounts]]
name = "x"
provider = "notion"
default_database_id = "01234567-89ab-cdef-0123-456789abcdef"
"#,
        )
        .unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_todo_notion_id_malformed() {
        let cfg: Config = toml::from_str(
            r#"
[[todo.accounts]]
name = "x"
provider = "notion"
parent_page_id = "short"
"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn load_from_invalid_config_errors() {
        let tmp = std::env::temp_dir().join("everyday_invalid_test.toml");
        std::fs::write(
            &tmp,
            r#"
[default_account]
mail = "ghost"
"#,
        )
        .unwrap();
        let err = Config::load_from(&tmp).unwrap_err();
        assert_eq!(err.type_name(), "ConfigError");
        let _ = std::fs::remove_file(&tmp);
    }

    // ---- P2a: AccountProvider trait (F012) ----

    #[test]
    fn account_provider_resolves_default() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let acc = cfg
            .mail
            .resolve_account(None, cfg.default_account.mail.as_deref())
            .unwrap();
        assert_eq!(acc.name, "work");
    }

    #[test]
    fn account_provider_resolves_override_over_default() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let acc = cfg
            .mail
            .resolve_account(Some("personal"), cfg.default_account.mail.as_deref())
            .unwrap();
        assert_eq!(acc.name, "personal");
    }

    #[test]
    fn account_provider_errors_when_missing() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let err = cfg
            .mail
            .resolve_account(Some("nope"), cfg.default_account.mail.as_deref())
            .unwrap_err();
        assert_eq!(err.type_name(), "AccountNotFound");
    }

    #[test]
    fn account_provider_errors_no_default() {
        let cfg = Config::default();
        let err = cfg.mail.resolve_account(None, None).unwrap_err();
        assert_eq!(err.type_name(), "AccountNotFound");
    }

    #[test]
    fn account_provider_works_for_all_modules() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
calendar = "personal"
note = "n1"
todo = "t1"
bookmark = "b1"

[[calendar.accounts]]
name = "personal"
caldav_url = "u"
username = "u"

[[note.accounts]]
name = "n1"

[[todo.accounts]]
name = "t1"

[[bookmark.accounts]]
name = "b1"
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.calendar
                .resolve_account(None, cfg.default_account.calendar.as_deref())
                .unwrap()
                .name,
            "personal"
        );
        assert_eq!(
            cfg.note
                .resolve_account(None, cfg.default_account.note.as_deref())
                .unwrap()
                .name,
            "n1"
        );
        assert_eq!(
            cfg.todo
                .resolve_account(None, cfg.default_account.todo.as_deref())
                .unwrap()
                .name,
            "t1"
        );
        assert_eq!(
            cfg.bookmark
                .resolve_account(None, cfg.default_account.bookmark.as_deref())
                .unwrap()
                .name,
            "b1"
        );
    }

    #[test]
    fn resolve_account_name_returns_string() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert_eq!(cfg.resolve_account_name("mail", None).unwrap(), "work");
        // "cal" alias resolves the calendar module.
        assert_eq!(cfg.resolve_account_name("cal", None).unwrap(), "personal");
    }

    #[test]
    fn resolve_account_name_unknown_module_errors() {
        let cfg = Config::default();
        let err = cfg.resolve_account_name("bogus", None).unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
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
