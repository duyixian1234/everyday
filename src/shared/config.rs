//! Config loading and multi-account management.
//!
//! Config file: `~/.config/everyday/config.toml` (resolved cross-platform
//! via `dirs`). Each module supports multiple named accounts; the top-level
//! `default_account` selects the default account name.
//! **Secrets are never stored in the config file** — they live in the OS
//! keyring (see the security red line in [agents.md](../../agents.md)).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use toml_edit::DocumentMut;

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

impl AccountProvider for WebdavConfig {
    type Account = WebdavAccount;
    fn module_name(&self) -> &'static str {
        "webdav"
    }
    fn account_list(&self) -> &[WebdavAccount] {
        &self.accounts
    }
}

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Per-module default account name mapping.
    #[serde(default)]
    pub default_account: DefaultAccount,

    /// Auth module configuration (credential lifecycle).
    #[serde(default)]
    pub auth: AuthConfig,

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

    /// WebDAV device-sync configuration (cross-device file sync, ADR D001–D003).
    #[serde(default)]
    pub webdav: WebdavConfig,

    /// Daemon auto-sync configuration (resident process, ADR F016).
    #[serde(default)]
    pub daemon: DaemonConfig,

    /// User-defined executable tasks, keyed by task name (ADR F017).
    #[serde(default)]
    pub tasks: HashMap<String, TaskConfig>,
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

    /// Default webdav sync account name.
    #[serde(default)]
    pub webdav: Option<String>,
}

/// Auth module configuration (credential lifecycle).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Allow credentials to be read from environment variables when the OS
    /// keyring backend is unavailable (R020). **Default off — opt-in only.**
    ///
    /// When enabled, the read chain becomes `keyring → env → error` with
    /// variable names `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD`. The equivalent
    /// environment switch `EVERYDAY_ENV_CREDENTIALS=1` also activates the
    /// fallback for call sites that hold no `Config` (P2b config subsets).
    #[serde(default)]
    pub env_credentials: bool,
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

// ---- WebDAV ----

/// WebDAV device-sync configuration (ADR D001).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebdavConfig {
    /// Named account list (each account is one sync namespace).
    #[serde(default)]
    pub accounts: Vec<WebdavAccount>,
}

/// A single WebDAV sync account.
///
/// The application password (not the login password) lives in the OS keyring
/// under `everyday/webdav/<name>` — never in the config file ([F002]).
/// `url` is the remote directory (e.g. `https://dav.jianguoyun.com/dav/everyday`);
/// synced files are PUT/GET under `{url}/{name}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebdavAccount {
    /// Account name (e.g. `personal`).
    pub name: String,
    /// WebDAV remote directory URL (RFC 4918).
    pub url: String,
    /// WebDAV username (e.g. your Jianguoyun email).
    pub username: String,
    /// Best-effort push after write commands (opt-in, default off — D003).
    #[serde(default)]
    pub auto_sync: bool,
}

// ---- Daemon ----

/// Daemon auto-sync configuration (ADR F016).
///
/// Controls the resident `everyday daemon run` process: whether it may start,
/// how long between sync cycles, and which sources each cycle syncs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Whether the daemon is allowed to run. `daemon run` refuses to start
    /// when disabled (a service-manager restart loop must not spin an empty
    /// process). Default: true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Seconds to sleep after one sync cycle completes (sleep-after-completion
    /// semantics — no tick catch-up). Default: 900 (15 minutes). Must be >= 1.
    #[serde(default = "default_interval_seconds")]
    pub interval_seconds: u64,
    /// Source whitelist for each cycle. Empty = all sources. Values are the
    /// timeline source names (`mail` / `cal` / `rss` / `todo` / `note` /
    /// `bookmark`); a whitelisted source turns on both its timeline provider
    /// and its cache action (mail cache / rss cache, when applicable).
    #[serde(default)]
    pub sources: Vec<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_seconds: 900,
            sources: Vec::new(),
        }
    }
}

fn default_interval_seconds() -> u64 {
    900
}

// ---- User-defined tasks ----

/// One user-defined command runnable manually or by the daemon scheduler.
///
/// `command` is an executable path/name, never a shell expression. `args` is
/// whitespace-split into argv; arguments containing spaces are intentionally
/// unsupported in v1 (ADR F017).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Executable file or path.
    pub command: String,
    /// Optional whitespace-separated configured arguments.
    #[serde(default)]
    pub args: String,
    /// Whether `task run <name> -- ...` may append runtime arguments.
    #[serde(default)]
    pub allow_extra_args: bool,
    /// Execution timeout in seconds. Zero disables the timeout.
    #[serde(default = "default_task_timeout_secs")]
    pub timeout_secs: u64,
    /// Persist captured stdout/stderr for manual runs.
    #[serde(default)]
    pub capture_output: bool,
    /// Optional standard five-field cron expression in local time.
    #[serde(default)]
    pub schedule: Option<String>,
}

fn default_task_timeout_secs() -> u64 {
    60
}

/// Validate one task entry independently of the full config.
pub(crate) fn validate_task_config(name: &str, task: &TaskConfig) -> Result<()> {
    let mut chars = name.chars();
    let valid_name = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid_name {
        return Err(AgentError::InvalidArgument(format!(
            "invalid task name `{name}`; expected ^[A-Za-z0-9][A-Za-z0-9_-]*$"
        )));
    }
    if task.command.trim().is_empty() {
        return Err(AgentError::InvalidArgument(format!(
            "task `{name}` command must not be empty"
        )));
    }
    if let Some(schedule) = task.schedule.as_deref()
        && !schedule.trim().is_empty()
    {
        validate_task_schedule(schedule)?;
    }
    Ok(())
}

/// Validate standard five-field cron syntax.
pub(crate) fn validate_task_schedule(expression: &str) -> Result<()> {
    let trimmed = expression.trim();
    if trimmed.split_whitespace().count() != 5 {
        return Err(AgentError::InvalidArgument(
            "task schedule must contain exactly 5 cron fields: min hour dom mon dow".into(),
        ));
    }
    croner::Cron::from_str(trimmed)
        .map(|_| ())
        .map_err(|e| AgentError::InvalidArgument(format!("invalid task schedule: {e}")))
}

// ---- Config editor (ADR R022) ----
//
// `ConfigEditor` is the single writer of `config.toml`. Both `config set`
// (`set_dotted`) and `task add`/`remove` (`insert_task`/`remove_task`) route
// through it so that every config mutation preserves hand-written comments and
// is persisted atomically (temp + rename). Previously `config set` re-serialised
// the file with `toml::to_string_pretty`, dropping comments, while the task
// module wrote its own `toml_edit` traversal (ADR F017 §"Config write lossiness").

/// Comment-preserving editor for the canonical config file (ADR R022).
///
/// All mutations go through a `toml_edit::DocumentMut` round-trip, so
/// hand-written comments survive, and are persisted atomically (write temp +
/// rename), matching `daemon/state.rs` / `sync/state.rs`.
#[derive(Debug)]
pub struct ConfigEditor {
    path: PathBuf,
}

impl ConfigEditor {
    /// Open an editor bound to the canonical config path.
    pub fn open() -> Result<Self> {
        Ok(Self {
            path: Config::config_path()?,
        })
    }

    /// Open an editor bound to an explicit path (used by tests).
    #[allow(dead_code)] // test seam mirroring TaskStore::open_path
    pub fn open_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Set a dotted path, coercing the raw string to bool / int / float /
    /// string. Known paths are validated at write time; unknown paths pass
    /// through (preserving incremental multi-step setup).
    pub fn set_dotted(&self, path: &str, raw_value: &str) -> Result<()> {
        validate_dotted_set(path, raw_value)?;
        let val = parse_value(raw_value);
        self.edit(|doc| set_dotted(doc, path, val).map(|_| true))?;
        Ok(())
    }

    /// Insert a typed `[tasks.<name>]` entry. Errors if the task already exists.
    pub fn insert_task(&self, name: &str, task: &TaskConfig) -> Result<()> {
        validate_task_config(name, task)?;
        self.edit(|doc| insert_task_into(doc, name, task))?;
        Ok(())
    }

    /// Remove a `[tasks.<name>]` entry. Returns whether it existed.
    pub fn remove_task(&self, name: &str) -> Result<bool> {
        self.edit(|doc| remove_task_from(doc, name))
    }

    /// Load the document, run `f`, and persist atomically if the closure
    /// reported a change.
    fn edit(&self, f: impl FnOnce(&mut DocumentMut) -> Result<bool>) -> Result<bool> {
        let text = if self.path.exists() {
            std::fs::read_to_string(&self.path)?
        } else {
            String::new()
        };
        let mut doc = DocumentMut::from_str(&text)
            .map_err(|e| AgentError::Config(format!("failed to parse config for edit: {e}")))?;
        let changed = f(&mut doc)?;
        if changed {
            save_document(&self.path, &doc)?;
        }
        Ok(changed)
    }
}

/// Parse a raw string into the most appropriate toml_edit value type.
fn parse_value(raw: &str) -> toml_edit::Value {
    if raw == "true" {
        return toml_edit::Value::from(true);
    }
    if raw == "false" {
        return toml_edit::Value::from(false);
    }
    if let Ok(n) = raw.parse::<i64>() {
        return toml_edit::Value::from(n);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return toml_edit::Value::from(f);
    }
    toml_edit::Value::from(raw.to_string())
}

/// Validate a dotted `config set` against registered per-path rules. Unknown
/// paths pass through unchanged.
fn validate_dotted_set(path: &str, raw_value: &str) -> Result<()> {
    // Registered: `tasks.<name>` — validate as a task entry (name / command /
    // cron). Only `command` and `schedule` are meaningfully settable via a
    // single dotted value; validate those when present.
    if let Some(rest) = path.strip_prefix("tasks.") {
        if let Some(task_name) = rest.split('.').next() {
            if task_name.is_empty() {
                return Err(AgentError::InvalidArgument(format!(
                    "invalid task path `{path}`"
                )));
            }
            // Reuse the same name + cron checks `task add` uses.
            validate_task_name(task_name)?;
            if let Some(field) = rest
                .strip_prefix(task_name)
                .and_then(|s| s.strip_prefix('.'))
            {
                match field {
                    "command" => {
                        if raw_value.trim().is_empty() {
                            return Err(AgentError::InvalidArgument(
                                "task command must not be empty".into(),
                            ));
                        }
                    }
                    "schedule" => validate_task_schedule(raw_value)?,
                    _ => {}
                }
            }
        }
    } else if path == "daemon.interval_seconds" {
        let n: i64 = raw_value.parse().map_err(|_| {
            AgentError::InvalidArgument("daemon.interval_seconds must be an integer".to_string())
        })?;
        if n < 1 {
            return Err(AgentError::InvalidArgument(
                "daemon.interval_seconds must be >= 1".into(),
            ));
        }
    }
    Ok(())
}

fn validate_task_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid_name = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid_name {
        return Err(AgentError::InvalidArgument(format!(
            "invalid task name `{name}`; expected ^[A-Za-z0-9][A-Za-z0-9_-]*$"
        )));
    }
    Ok(())
}

/// Persist a document atomically (temp file + rename). Mirrors the
/// `daemon/state.rs` / `sync/state.rs` pattern.
fn save_document(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&tmp, doc.to_string())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ── dotted-path upsert (toml_edit) ──────────────────────────────────────

/// Ensure `cur` is an array (inline array OR array-of-tables) long enough for
/// `need` elements, auto-extending with empty tables. Mirrors the old
/// `toml::Value` `ensure_array_len`.
fn ensure_array_len(cur: &mut toml_edit::Item, need: usize) -> Result<()> {
    match cur {
        toml_edit::Item::Value(toml_edit::Value::Array(arr)) => {
            while arr.len() < need {
                arr.push(toml_edit::Value::InlineTable(toml_edit::InlineTable::new()));
            }
            Ok(())
        }
        toml_edit::Item::ArrayOfTables(aot) => {
            while aot.len() < need {
                aot.push(toml_edit::Table::new());
            }
            Ok(())
        }
        _ => Err(AgentError::InvalidArgument("not an array".into())),
    }
}

/// Set a dotted path, matching the old `toml::Value` upsert behaviour:
/// - non-numeric segments walk/create real `[table]` sections (or descend into
///   an array element's inline table)
/// - numeric segments walk arrays (`Item::Value(Array)` or `ArrayOfTables`),
///   auto-extended with empty tables
/// - the final numeric segment sets an array element; the final non-numeric
///   segment sets a table key.
fn set_dotted(doc: &mut DocumentMut, path: &str, val: toml_edit::Value) -> Result<()> {
    let segs: Vec<&str> = path.split('.').collect();
    if segs.is_empty() {
        return Err(AgentError::InvalidArgument("empty path".into()));
    }
    let (last, rest) = segs.split_last().unwrap();

    let mut cur = doc.as_item_mut(); // container Item holding the next segment
    for seg in rest {
        if let Ok(idx) = seg.parse::<usize>() {
            ensure_array_len(cur, idx + 1)?;
            cur = &mut cur[idx]; // Item::IndexMut<usize> — requires an array
        } else if cur.as_table_mut().is_some() {
            let table = cur.as_table_mut().unwrap(); // real [table] section
            if !table.contains_key(seg) {
                table.insert(seg, toml_edit::Item::Table(toml_edit::Table::new()));
            }
            cur = table.get_mut(seg).expect("key just ensured");
        } else if cur.is_none() || matches!(cur.as_value(), Some(toml_edit::Value::InlineTable(_)))
        {
            cur = &mut cur[seg]; // array element (inline table) or unset slot
        } else {
            return Err(AgentError::InvalidArgument(format!(
                "path segment `{seg}` is not a table or array"
            )));
        }
    }

    if let Ok(idx) = last.parse::<usize>() {
        ensure_array_len(cur, idx + 1)?;
        cur[idx] = toml_edit::Item::Value(val);
    } else {
        cur[last] = toml_edit::Item::Value(val);
    }
    Ok(())
}

// ── typed task insert / remove (toml_edit) ─────────────────────────────

/// Insert a `[tasks.<name>]` table. Returns false if it already existed.
fn insert_task_into(doc: &mut DocumentMut, name: &str, task: &TaskConfig) -> Result<bool> {
    if !doc.as_table().contains_key("tasks") {
        doc["tasks"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let tasks = doc["tasks"]
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("`tasks` must be a TOML table".into()))?;
    if tasks.contains_key(name) {
        return Err(AgentError::InvalidArgument(format!(
            "task `{name}` already exists"
        )));
    }

    let mut table = toml_edit::Table::new();
    table.set_implicit(false);
    table["command"] = toml_edit::value(&task.command);
    if !task.args.is_empty() {
        table["args"] = toml_edit::value(&task.args);
    }
    table["allow_extra_args"] = toml_edit::value(task.allow_extra_args);
    table["timeout_secs"] = toml_edit::value(i64::try_from(task.timeout_secs).unwrap_or(i64::MAX));
    table["capture_output"] = toml_edit::value(task.capture_output);
    if let Some(schedule) = task.schedule.as_deref()
        && !schedule.trim().is_empty()
    {
        table["schedule"] = toml_edit::value(schedule);
    }
    tasks.insert(name, toml_edit::Item::Table(table));
    Ok(true)
}

/// Remove a `[tasks.<name>]` entry. Returns whether it existed.
fn remove_task_from(doc: &mut DocumentMut, name: &str) -> Result<bool> {
    let Some(tasks) = doc.get_mut("tasks").and_then(toml_edit::Item::as_table_mut) else {
        return Ok(false);
    };
    Ok(tasks.remove(name).is_some())
}

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
        validate_default_account(
            "webdav",
            self.default_account.webdav.as_deref(),
            &self.webdav.accounts,
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
        for a in &self.webdav.accounts {
            require_nonempty("webdav", &a.name, "name")?;
            require_nonempty("webdav", &a.url, "url")?;
            require_nonempty("webdav", &a.username, "username")?;
        }
        if self.daemon.interval_seconds == 0 {
            return Err(AgentError::Config(
                "daemon.interval_seconds must be >= 1".into(),
            ));
        }
        for (name, task) in &self.tasks {
            validate_task_config(name, task).map_err(|e| AgentError::Config(e.message()))?;
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

    /// Resolve the webdav sync account: `override_name` > default > error.
    pub fn webdav_account(&self, override_name: Option<&str>) -> Result<&WebdavAccount> {
        self.webdav
            .resolve_account(override_name, self.default_account.webdav.as_deref())
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
impl NamedAccount for WebdavAccount {
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

    // ---- Auth config (R020 env fallback) ----

    #[test]
    fn auth_env_credentials_defaults_false() {
        // Opt-in only: absent `[auth] env_credentials` must be false.
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert!(!cfg.auth.env_credentials);
    }

    #[test]
    fn auth_env_credentials_parses_true() {
        let cfg: Config = toml::from_str(
            r#"
[auth]
env_credentials = true
"#,
        )
        .unwrap();
        assert!(cfg.auth.env_credentials);
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

    // ---- WebDAV device sync (ADR D001) ----

    #[test]
    fn parses_webdav_account() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
webdav = "personal"

[[webdav.accounts]]
name = "personal"
url = "https://dav.jianguoyun.com/dav/everyday"
username = "me@example.com"
"#,
        )
        .unwrap();
        assert_eq!(cfg.webdav.accounts.len(), 1);
        assert_eq!(
            cfg.webdav.accounts[0].url,
            "https://dav.jianguoyun.com/dav/everyday"
        );
        assert_eq!(cfg.webdav.accounts[0].username, "me@example.com");
        // auto_sync defaults to off (opt-in, D003).
        assert!(!cfg.webdav.accounts[0].auto_sync);
        assert_eq!(cfg.default_account.webdav.as_deref(), Some("personal"));
    }

    #[test]
    fn parses_webdav_auto_sync_true() {
        let cfg: Config = toml::from_str(
            r#"
[[webdav.accounts]]
name = "p"
url = "https://dav.example.com/x"
username = "u"
auto_sync = true
"#,
        )
        .unwrap();
        assert!(cfg.webdav.accounts[0].auto_sync);
    }

    #[test]
    fn webdav_default_has_no_accounts() {
        let cfg = Config::default();
        assert!(cfg.webdav.accounts.is_empty());
        assert!(cfg.default_account.webdav.is_none());
    }

    #[test]
    fn resolves_default_webdav_account() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
webdav = "personal"

[[webdav.accounts]]
name = "personal"
url = "https://dav.example.com/x"
username = "u"
"#,
        )
        .unwrap();
        let acc = cfg.webdav_account(None).unwrap();
        assert_eq!(acc.name, "personal");
    }

    #[test]
    fn resolves_overridden_webdav_account() {
        let cfg: Config = toml::from_str(
            r#"
[[webdav.accounts]]
name = "a"
url = "https://dav.example.com/a"
username = "u1"

[[webdav.accounts]]
name = "b"
url = "https://dav.example.com/b"
username = "u2"
"#,
        )
        .unwrap();
        let acc = cfg.webdav_account(Some("b")).unwrap();
        assert_eq!(acc.username, "u2");
    }

    #[test]
    fn missing_webdav_account_errors() {
        let cfg = Config::default();
        let err = cfg.webdav_account(None).unwrap_err();
        assert_eq!(err.type_name(), "AccountNotFound");
    }

    #[test]
    fn validate_webdav_required_fields() {
        let cfg: Config = toml::from_str(
            r#"
[[webdav.accounts]]
name = "x"
url = ""
username = "u"
"#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.type_name(), "ConfigError");
        assert!(err.message().contains("url"), "{}", err.message());
    }

    #[test]
    fn validate_webdav_default_account_must_exist() {
        let cfg: Config = toml::from_str(
            r#"
[default_account]
webdav = "ghost"
"#,
        )
        .unwrap();
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.type_name(), "ConfigError");
        assert!(err.message().contains("webdav"), "{}", err.message());
    }

    // ---- Daemon config (ADR F016) ----

    #[test]
    fn daemon_defaults_when_section_missing() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.daemon.enabled);
        assert_eq!(cfg.daemon.interval_seconds, 900);
        assert!(cfg.daemon.sources.is_empty());
    }

    #[test]
    fn daemon_parses_explicit_values() {
        let cfg: Config = toml::from_str(
            r#"
[daemon]
enabled = false
interval_seconds = 60
sources = ["mail", "rss"]
"#,
        )
        .unwrap();
        assert!(!cfg.daemon.enabled);
        assert_eq!(cfg.daemon.interval_seconds, 60);
        assert_eq!(cfg.daemon.sources, vec!["mail", "rss"]);
    }

    #[test]
    fn daemon_partial_section_keeps_defaults() {
        // A partial `[daemon]` section must not reset unset fields to zero.
        let cfg: Config = toml::from_str("[daemon]\ninterval_seconds = 30\n").unwrap();
        assert!(cfg.daemon.enabled, "enabled must keep its default `true`");
        assert_eq!(cfg.daemon.interval_seconds, 30);
        assert!(cfg.daemon.sources.is_empty());
    }

    #[test]
    fn validate_daemon_interval_zero_rejected() {
        let cfg: Config = toml::from_str("[daemon]\ninterval_seconds = 0\n").unwrap();
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.type_name(), "ConfigError");
        assert!(
            err.message().contains("interval_seconds"),
            "{}",
            err.message()
        );
    }

    // ---- Task config (ADR F017) ----

    #[test]
    fn task_defaults_and_schedule_parse() {
        let cfg: Config = toml::from_str(
            r#"
[tasks.build]
command = "cargo"
args = "check --all-targets"
schedule = "*/5 * * * *"
"#,
        )
        .unwrap();
        cfg.validate().unwrap();
        let task = &cfg.tasks["build"];
        assert_eq!(task.timeout_secs, 60);
        assert!(!task.allow_extra_args);
        assert!(!task.capture_output);
    }

    #[test]
    fn invalid_task_config_is_rejected() {
        let invalid_name: Config =
            toml::from_str("[tasks.\"bad name\"]\ncommand = \"echo\"\n").unwrap();
        assert!(invalid_name.validate().is_err());

        let invalid_cron: Config =
            toml::from_str("[tasks.x]\ncommand = \"echo\"\nschedule = \"* * * *\"\n").unwrap();
        assert!(invalid_cron.validate().is_err());
    }

    // ---- Config editor (ADR R022) ----

    fn editor_path(name: &str) -> std::path::PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("config-editor-{}-{name}.toml", std::process::id()))
    }

    fn helper_task() -> TaskConfig {
        TaskConfig {
            command: "echo".into(),
            args: "hello".into(),
            allow_extra_args: false,
            timeout_secs: 10,
            capture_output: true,
            schedule: Some("*/5 * * * *".into()),
        }
    }

    #[test]
    fn insert_task_preserves_comments_and_remove_is_independent() {
        let path = editor_path("comments");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# keep me\n[daemon]\nenabled = true # inline\n").unwrap();
        let editor = ConfigEditor::open_path(path.clone());
        editor.insert_task("hello", &helper_task()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"));
        assert!(text.contains("enabled = true # inline"));
        assert!(text.contains("[tasks.hello]"));
        assert!(editor.remove_task("hello").unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"));
        assert!(!text.contains("[tasks.hello]"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn insert_duplicate_task_is_rejected() {
        let path = editor_path("duplicate");
        std::fs::write(&path, "[tasks.x]\ncommand = \"echo\"\n").unwrap();
        let editor = ConfigEditor::open_path(path.clone());
        assert!(editor.insert_task("x", &helper_task()).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn set_dotted_preserves_comments_and_creates_tables() {
        let path = editor_path("dotted");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "# top comment\n[daemon]\n# keep me\ninterval_seconds = 900\n",
        )
        .unwrap();
        let editor = ConfigEditor::open_path(path.clone());
        editor.set_dotted("daemon.interval_seconds", "60").unwrap();
        editor.set_dotted("default_account.mail", "work").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# top comment"));
        assert!(text.contains("# keep me"));
        assert!(text.contains("interval_seconds = 60"));
        assert!(text.contains("default_account"));
        // Re-parse and verify the value round-trips.
        let cfg: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.daemon.interval_seconds, 60);
        assert_eq!(cfg.default_account.mail.as_deref(), Some("work"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn set_dotted_extends_array_by_index() {
        let path = editor_path("array");
        std::fs::write(&path, "[[mail.accounts]]\nname = \"personal\"\n").unwrap();
        let editor = ConfigEditor::open_path(path.clone());
        editor.set_dotted("mail.accounts.1.name", "work").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("[[mail.accounts]]").count(), 2, "{text}");
        assert!(text.contains("name = \"personal\""));
        assert!(text.contains("name = \"work\""));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn set_dotted_validates_task_and_daemon_paths() {
        let path = editor_path("validate");
        std::fs::write(&path, "[tasks.x]\ncommand = \"echo\"\n").unwrap();
        let editor = ConfigEditor::open_path(path.clone());
        // Empty command rejected.
        assert!(editor.set_dotted("tasks.x.command", "  ").is_err());
        // Invalid cron rejected.
        assert!(editor.set_dotted("tasks.x.schedule", "* * * *").is_err());
        // Valid cron accepted.
        assert!(editor.set_dotted("tasks.x.schedule", "*/5 * * * *").is_ok());
        // daemon.interval_seconds >= 1.
        assert!(editor.set_dotted("daemon.interval_seconds", "0").is_err());
        assert!(editor.set_dotted("daemon.interval_seconds", "30").is_ok());
        // Unknown path passes through.
        assert!(editor.set_dotted("some.unknown.key", "value").is_ok());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn remove_missing_task_returns_false_without_writing() {
        let path = editor_path("remove-missing");
        std::fs::write(&path, "# keep\n").unwrap();
        let editor = ConfigEditor::open_path(path.clone());
        assert!(!editor.remove_task("nope").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# keep\n");
        let _ = std::fs::remove_file(path);
    }
}
