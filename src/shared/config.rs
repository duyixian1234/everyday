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

    /// Todo module configuration (local SQLite).
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
/// The `provider` field accepts `local`/`sqlite` (local SQLite). The remote
/// `notion` provider was removed in v0.13.0
/// ([R019](../../docs/adr/R019-remove-notion-provider.md)); existing configs
/// declaring it fail validation with a migration hint.
/// Credentials are never stored in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteAccount {
    /// Account name (e.g. `personal`, `work`).
    pub name: String,
    /// Backend provider: `local`/`sqlite` (local SQLite, default).
    #[serde(default = "default_provider")]
    pub provider: String,
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

/// Todo module configuration (local SQLite task database).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TodoConfig {
    /// Named account list.
    #[serde(default)]
    pub accounts: Vec<TodoAccount>,
}

/// Shared fields for a local-provider account.
///
/// `TodoAccount` and `BookmarkAccount` used to be byte-for-byte copies
/// (all 5 fields identical); this struct + type alias dedup them.
/// `NoteAccount` stays a separate type because its `default_page_id`
/// ("which page new notes go to") differs in meaning.
/// The Notion fields (`parent_page_id` / `default_database_id`) were removed
/// with the provider in v0.13.0
/// ([R019](../../docs/adr/R019-remove-notion-provider.md)).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAccount {
    /// Account name (e.g. `personal`, `work`).
    pub name: String,
    /// Backend provider: `local`/`sqlite` (local SQLite, default).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// SQLite file path for the local provider (only `local`/`sqlite`).
    /// Defaults to `~/.config/everyday/<module>-<account>.db`.
    #[serde(default)]
    pub db_path: Option<String>,
}

/// A single todo account.
///
/// Shares `LocalAccount` fields; the type alias keeps backward compat
/// (constructors using `TodoAccount { .. }` still work — zero fields are
/// filled by the Default impl).
pub type TodoAccount = LocalAccount;

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
/// Shares `LocalAccount` fields; the type alias keeps backward compat.
pub type BookmarkAccount = LocalAccount;

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
        /// Injected config subset for the module (P2b,
        /// [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
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

        // The subset reuses the single `AccountProvider` resolution algorithm
        // (P2a) instead of duplicating it — `resolve_account` delegates, so
        // "account resolution duplicates → 1" holds for subsets too.
        impl AccountProvider for $name {
            type Account = $account;
            fn module_name(&self) -> &'static str {
                $module_key
            }
            fn account_list(&self) -> &[Self::Account] {
                &self.accounts
            }
        }

        impl $name {
            /// Resolve the effective account: `--account` override > default > error.
            ///
            /// Delegates to the single [`AccountProvider::resolve_account`]
            /// algorithm (the module no longer needs the full `Config`).
            pub fn resolve_account(&self, override_name: Option<&str>) -> Result<&$account> {
                AccountProvider::resolve_account(
                    self,
                    override_name,
                    self.default_account.as_deref(),
                )
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
    "calendar"
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
    /// - invalid `provider` values (including the removed `notion`)
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
            validate_local_account("todo", a)?;
        }
        for a in &self.bookmark.accounts {
            validate_local_account("bookmark", a)?;
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
/// `LocalAccount`).
impl NamedAccount for LocalAccount {
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

/// Provider whitelist shared by note / todo / bookmark accounts.
///
/// `notion` was removed in v0.13.0 ([R019](../../docs/adr/R019-remove-notion-provider.md));
/// an existing `provider = "notion"` fails with a migration hint rather than silently
/// falling back to local (which would look like the user's data vanished while the CLI
/// reads/writes an empty local DB).
fn validate_provider_whitelist(module: &str, account: &str, provider: &str) -> Result<()> {
    if matches!(provider, "local" | "sqlite") {
        return Ok(());
    }
    if provider == "notion" {
        return Err(AgentError::Config(format!(
            "{module} account '{account}': provider 'notion' is no longer supported (removed in v0.13.0). \
             Migrate the account to the local provider: remove the provider field or set `provider = \"local\"`; \
             your Notion data is untouched on notion.so."
        )));
    }
    Err(AgentError::Config(format!(
        "{module} account '{account}': unknown provider '{provider}' (expected local|sqlite)"
    )))
}

fn validate_note_account(a: &NoteAccount) -> Result<()> {
    require_nonempty("note", &a.name, "name")?;
    validate_provider_whitelist("note", &a.name, &a.provider)
}

fn validate_local_account(module: &str, a: &LocalAccount) -> Result<()> {
    require_nonempty(module, &a.name, "name")?;
    validate_provider_whitelist(module, &a.name, &a.provider)
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
    fn validate_notion_provider_rejected() {
        // The notion provider was removed (v0.13.0); `provider = "notion"` must fail
        // with a migration hint.
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
        let msg = err.message();
        assert!(
            msg.contains("notion") && msg.contains("no longer supported"),
            "{msg}"
        );
    }

    #[test]
    fn validate_todo_notion_provider_rejected() {
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
    fn parses_note_account_config() {
        // Local-only account; legacy notion fields are ignored by serde
        // (no deny_unknown_fields), so old configs still parse.
        let cfg: Config = toml::from_str(
            r#"
[default_account]
note = "personal"

[[note.accounts]]
name = "personal"
provider = "local"
default_page_id = "page_xyz"
"#,
        )
        .unwrap();
        assert_eq!(cfg.note.accounts.len(), 1);
        assert_eq!(cfg.note.accounts[0].provider, "local");
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
    fn note_provider_explicit_notion_fails_validation() {
        // An explicit `provider = "notion"` now fails load-time validation.
        let cfg: Config = toml::from_str(
            r#"
[[note.accounts]]
name = "x"
provider = "notion"
"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn resolves_default_note_account() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
note = "personal"

[[note.accounts]]
name = "personal"
"#,
        )
        .unwrap();
        let acc = cfg
            .note
            .resolve_account(None, cfg.default_account.note.as_deref())
            .unwrap();
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
provider = "local"
db_path = "C:/data/todo.db"
"#,
        )
        .unwrap();
        assert_eq!(cfg.todo.accounts.len(), 1);
        assert_eq!(cfg.todo.accounts[0].provider, "local");
        assert_eq!(
            cfg.todo.accounts[0].db_path.as_deref(),
            Some("C:/data/todo.db")
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
    fn todo_provider_explicit_notion_fails_validation() {
        let cfg: Config = toml::from_str(
            r#"
[[todo.accounts]]
name = "x"
provider = "notion"
"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn resolves_default_todo_account() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
todo = "personal"

[[todo.accounts]]
name = "personal"
"#,
        )
        .unwrap();
        let acc = cfg
            .todo
            .resolve_account(None, cfg.default_account.todo.as_deref())
            .unwrap();
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
provider = "local"
"#,
        )
        .unwrap();
        assert_eq!(cfg.bookmark.accounts.len(), 1);
        assert_eq!(cfg.bookmark.accounts[0].provider, "local");
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
