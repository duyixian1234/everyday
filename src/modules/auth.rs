//! Top-level credential lifecycle module (Phase 12).
//!
//! Consolidates all credential/login logic that used to live in five separate
//! modules (`mail` / `cal` / `note` / `todo` / `bookmark`) into one owner.
//! See [R013](../../docs/adr/R013-auth-module-consolidation.md) (consolidation),
//! [R014](../../docs/adr/R014-auth-verify-opt-in.md) (verify is opt-in),
//! [R015](../../docs/adr/R015-auth-credential-io.md) (non-interactive input).
//!
//! - `login`  — stores only by default; `--verify` stores + verifies in one call.
//! - `logout` — deletes the stored credential from the OS keyring.
//! - `verify` — reads the already-stored credential and authenticates (no re-prompt).
//! - `list`   — enumerates config accounts and probes keyring state (stored/missing/not_required).
//!
//! The keyring service string `everyday/<module>/<account>` is frozen (F002);
//! only the keyring *user* selection (account username vs `"token"`) is centralized here.
//! The Notion `Token` strategy was removed in v0.13.0
//! ([R019](../../docs/adr/R019-remove-notion-provider.md)).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use serde_json::json;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::calendar;
use crate::modules::email;
use crate::modules::{Executor, ModuleArgSpec, Output};
use crate::util::args::parse_simple_args;

/// Credential strategy for a (module, account) pair.
///
/// Derived purely from `Config` — no per-module declaration. See
/// [R013](../../docs/adr/R013-auth-module-consolidation.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStrategy {
    /// username/password (mail, cal). keyring user = account username.
    Password,
    /// No credential (note/todo/bookmark local/sqlite provider, rss).
    None,
}

/// Resolve the credential strategy for a (module, account) from `Config` only.
///
/// `config` is unused since v0.13.0 (no provider-based strategy remains); the
/// parameter is kept for signature stability.
pub fn resolve_strategy(_config: &Config, module: &str, _account: &str) -> Result<AuthStrategy> {
    match module {
        "mail" | "cal" | "webdav" => Ok(AuthStrategy::Password),
        "note" | "todo" | "bookmark" => Ok(AuthStrategy::None),
        "rss" => Ok(AuthStrategy::None),
        other => Err(AgentError::InvalidArgument(format!(
            "unknown module for auth: '{other}'"
        ))),
    }
}

/// Resolve the keyring username for a password-strategy module.
fn username_for(config: &Config, module: &str, account: &str) -> Result<String> {
    match module {
        "mail" => Ok(config.mail_account(Some(account))?.username.clone()),
        "cal" => Ok(config.calendar_account(Some(account))?.username.clone()),
        "webdav" => Ok(config.webdav_account(Some(account))?.username.clone()),
        other => Err(AgentError::InvalidArgument(format!(
            "module '{other}' has no password/username credential"
        ))),
    }
}

/// Resolve (keyring service, keyring user) for a (module, account, strategy).
fn keyring_target(
    config: &Config,
    module: &str,
    account: &str,
    strategy: &AuthStrategy,
) -> Result<(String, String)> {
    let service = Config::keyring_service(module, account);
    let user = match strategy {
        AuthStrategy::Password => username_for(config, module, account)?,
        AuthStrategy::None => {
            return Err(AgentError::Auth(format!(
                "module '{module}' account '{account}' requires no credential (local/sqlite or rss)"
            )));
        }
    };
    Ok((service, user))
}

/// Store a credential in the OS keyring (strategy derived from `Config`).
pub fn store_credential(config: &Config, module: &str, account: &str, secret: &str) -> Result<()> {
    let strategy = resolve_strategy(config, module, account)?;
    let (service, user) = keyring_target(config, module, account, &strategy)?;
    let entry = keyring::Entry::new(&service, &user)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry
        .set_password(secret)
        .map_err(|e| AgentError::Auth(format!("keyring set: {e}")))?;
    Ok(())
}

/// Read a stored credential from the OS keyring.
///
/// Modules call this instead of their own `get_password` / `get_token`.
pub fn get_credential(config: &Config, module: &str, account: &str) -> Result<String> {
    let strategy = resolve_strategy(config, module, account)?;
    let (service, user) = keyring_target(config, module, account, &strategy)?;
    // Delegate to the shared reader; `keyring_target` already rejects
    // `AuthStrategy::None` (module requires no credential).
    get_credential_for(module, account, &service, &user)
}

/// Read a stored credential given an explicit keyring user (P2b,
/// [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
///
/// Business modules that receive only their config **subset** (not the full
/// `Config`) call this: they already know the keyring user from the resolved
/// account (`username` for mail/cal).
/// The keyring service name is a pure function of `(module, account)`.
pub fn get_credential_with_user(module: &str, account: &str, user: &str) -> Result<String> {
    let service = Config::keyring_service(module, account);
    get_credential_for(module, account, &service, user)
}

/// Shared keyring read: `(service, user)` → stored password (or an auth error
/// with a login hint). Single error-formatting path for both credential readers.
fn get_credential_for(module: &str, account: &str, service: &str, user: &str) -> Result<String> {
    let entry = keyring::Entry::new(service, user)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry.get_password().map_err(|e| {
        AgentError::Auth(format!(
            "no credential in keyring for {module} account '{account}': {e}. \
             Run `everyday auth login --module {module} --account {account}` to store it."
        ))
    })
}

/// Delete a stored credential from the OS keyring.
pub fn delete_credential(config: &Config, module: &str, account: &str) -> Result<()> {
    let strategy = resolve_strategy(config, module, account)?;
    let (service, user) = keyring_target(config, module, account, &strategy)?;
    let entry = keyring::Entry::new(&service, &user)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry
        .delete_password()
        .map_err(|e| AgentError::Auth(format!("keyring delete: {e}")))?;
    Ok(())
}

/// Default account name for a module (from `[default_account]`), if set.
fn default_account_name(config: &Config, module: &str) -> Option<String> {
    match module {
        "mail" => config.default_account.mail.clone(),
        "cal" => config.default_account.calendar.clone(),
        "note" => config.default_account.note.clone(),
        "todo" => config.default_account.todo.clone(),
        "bookmark" => config.default_account.bookmark.clone(),
        "webdav" => config.default_account.webdav.clone(),
        _ => None,
    }
}

/// All configured account names for a module (empty for modules without accounts).
fn list_accounts(config: &Config, module: &str) -> Vec<String> {
    match module {
        "mail" => config
            .mail
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect(),
        "cal" => config
            .calendar
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect(),
        "note" => config
            .note
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect(),
        "todo" => config
            .todo
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect(),
        "bookmark" => config
            .bookmark
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect(),
        "webdav" => config
            .webdav
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// Prompt for a secret on a TTY (falls back to this when no `--password`/`--token`).
///
/// Secrets are never read from the environment (R015). The prompt does not echo
/// the secret back to the terminal.
async fn prompt_secret(prompt: &str) -> Result<String> {
    let prompt = prompt.to_string();
    let s = tokio::task::spawn_blocking(move || rpassword::prompt_password(prompt))
        .await
        .map_err(|e| AgentError::Other(format!("join secret prompt: {e}")))?
        .map_err(|e| AgentError::Other(format!("read secret: {e}")))?;
    Ok(s)
}

/// Top-level credential lifecycle module.
pub struct AuthModule {
    config: Arc<Config>,
}

impl AuthModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

// ============ service layer (P1 wiring, [F012](../../docs/adr/F012-architecture-deepening-phase.md)) ============

/// Result of `login`: the confirmation message plus whether verification ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginReceipt {
    /// Human-readable confirmation ("credential stored for mail account 'work'"; may
    /// be suffixed "; verified" when `verify` was requested and succeeded).
    pub message: String,
    /// `true` when `--verify` was requested and the credential verified OK.
    pub verified: bool,
}

/// Result of `verify`: whether the module requires a credential at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Module/account needs no credential (local/sqlite provider, rss).
    NotRequired,
    /// Credential read + authenticated against the external service.
    Verified,
}

/// One row of `auth list`: module / account / keyring state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRow {
    pub module: String,
    pub account: String,
    pub status: String,
}

/// Login request: explicit secret (from `--password`/`--token`) or `None` to
/// prompt interactively on the TTY ([R015](../../docs/adr/R015-auth-credential-io.md)).
#[derive(Debug, Clone)]
pub struct AuthLoginRequest<'a> {
    pub module: &'a str,
    pub account: &'a str,
    pub secret: Option<&'a str>,
    pub verify: bool,
}

/// Auth service trait: domain methods, no `Output` in sight (P1).
///
/// `ConfigAuthBackend` talks to the keyring + external services;
/// `testkit::MockAuthBackend` (tests) returns fixed data. `dispatch` is the
/// only place that maps CLI args → service calls → `Output`.
#[async_trait]
pub trait AuthBackend: Send + Sync {
    /// Resolve the account name for a module: explicit override or the
    /// configured default account ([F002](../../docs/adr/F002-multi-account-keyring.md)).
    /// Returns `InvalidArgument` when neither exists.
    fn resolve_account(&self, module: &str, account: Option<&str>) -> Result<String>;
    /// Store a credential (interactive prompt when `secret` is `None`).
    async fn login(&self, req: &AuthLoginRequest<'_>) -> Result<LoginReceipt>;
    /// Delete a stored credential; returns the confirmation message.
    async fn logout(&self, module: &str, account: &str) -> Result<String>;
    /// Verify a stored credential (no re-prompt).
    async fn verify(&self, module: &str, account: &str) -> Result<VerifyOutcome>;
    /// Enumerate configured accounts and probe keyring state.
    async fn list(&self, module: Option<&str>) -> Result<Vec<CredentialRow>>;
}

/// Real backend: holds the full `Config` (auth is a cross-module
/// orchestrator, [F012 P2b](../../docs/adr/F012-architecture-deepening-phase.md)).
pub struct ConfigAuthBackend {
    config: Arc<Config>,
}

impl ConfigAuthBackend {
    fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Read the stored credential and authenticate against the external service.
    ///
    /// Reuses the modules' existing connection primitives (R013): `email::imap_connect`,
    /// `calendar::cal_verify`. The `None` strategy short-circuits.
    /// Internal helper — distinct from the trait's `verify` (which reports
    /// `VerifyOutcome`), so it is named `verify_credential`.
    async fn verify_credential(&self, module: &str, account: &str) -> Result<()> {
        let secret = get_credential(&self.config, module, account)?;
        match module {
            "mail" => {
                let acc = self.config.mail_account(Some(account))?;
                let _ = email::imap_connect(acc, &secret).await?;
            }
            "cal" => {
                let acc = self.config.calendar_account(Some(account))?;
                calendar::cal_verify(acc, &secret).await?;
            }
            "webdav" => {
                let acc = self.config.webdav_account(Some(account))?;
                verify_webdav_remote(&acc.url, &acc.username, &secret).await?;
            }
            other => {
                return Err(AgentError::InvalidArgument(format!(
                    "module '{other}' does not support verification"
                )));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl AuthBackend for ConfigAuthBackend {
    fn resolve_account(&self, module: &str, account: Option<&str>) -> Result<String> {
        account
            .map(|a| a.to_string())
            .or_else(|| default_account_name(&self.config, module))
            .ok_or_else(|| {
                AgentError::InvalidArgument(format!(
                    "requires --account <name> (or a default account for module '{module}')"
                ))
            })
    }

    async fn login(&self, req: &AuthLoginRequest<'_>) -> Result<LoginReceipt> {
        let module = req.module;
        let account = req.account;
        let strategy = resolve_strategy(&self.config, module, account)?;
        let secret = match strategy {
            AuthStrategy::None => {
                return Err(AgentError::Auth(format!(
                    "module '{module}' account '{account}' requires no credential (local/sqlite or rss); nothing to store"
                )));
            }
            AuthStrategy::Password => {
                if let Some(p) = req.secret {
                    p.to_string()
                } else {
                    let username = username_for(&self.config, module, account)?;
                    prompt_secret(&format!("Password for {username}: ")).await?
                }
            }
        };
        if secret.trim().is_empty() {
            return Err(AgentError::InvalidArgument(
                "credential must not be empty".into(),
            ));
        }
        store_credential(&self.config, module, account, secret.trim())?;
        let mut verified = false;
        if req.verify {
            self.verify_credential(module, account).await?;
            verified = true;
        }
        let mut message = format!("credential stored for {module} account '{account}'");
        if verified {
            message.push_str("; verified");
        }
        Ok(LoginReceipt { message, verified })
    }

    async fn logout(&self, module: &str, account: &str) -> Result<String> {
        let strategy = resolve_strategy(&self.config, module, account)?;
        if matches!(strategy, AuthStrategy::None) {
            return Err(AgentError::Auth(format!(
                "module '{module}' account '{account}' requires no credential; nothing to remove"
            )));
        }
        delete_credential(&self.config, module, account)?;
        Ok(format!(
            "credential removed for {module} account '{account}'"
        ))
    }

    async fn verify(&self, module: &str, account: &str) -> Result<VerifyOutcome> {
        let strategy = resolve_strategy(&self.config, module, account)?;
        match strategy {
            AuthStrategy::None => Ok(VerifyOutcome::NotRequired),
            _ => {
                self.verify_credential(module, account).await?;
                Ok(VerifyOutcome::Verified)
            }
        }
    }

    async fn list(&self, module: Option<&str>) -> Result<Vec<CredentialRow>> {
        let modules: Vec<&str> = match module {
            Some(m) => vec![m],
            None => vec!["mail", "cal", "note", "todo", "bookmark", "webdav"],
        };
        let mut rows = Vec::new();
        for m in &modules {
            for acc_name in list_accounts(&self.config, m) {
                let strategy = resolve_strategy(&self.config, m, &acc_name)?;
                let status = match strategy {
                    AuthStrategy::None => "not_required".to_string(),
                    _ => match get_credential(&self.config, m, &acc_name) {
                        Ok(_) => "stored".to_string(),
                        Err(_) => "missing".to_string(),
                    },
                };
                rows.push(CredentialRow {
                    module: m.to_string(),
                    account: acc_name,
                    status,
                });
            }
        }
        Ok(rows)
    }
}

/// Build the auth backend for the current config.
pub fn for_config(config: &Arc<Config>) -> Box<dyn AuthBackend> {
    Box::new(ConfigAuthBackend::new(config.clone()))
}

/// Verify a WebDAV credential by PROPFINDing the remote directory: HTTP 401/403
/// means the application password is wrong. Inlined here (not in the sync
/// module) to keep the dependency direction auth → modules one-way — sync
/// reads credentials via `auth::get_credential_with_user`, so auth must not
/// depend back on sync.
async fn verify_webdav_remote(url: &str, username: &str, secret: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AgentError::Network(format!("build http client: {e}")))?;
    let method = reqwest::Method::from_bytes(b"PROPFIND")
        .map_err(|e| AgentError::Other(format!("invalid method token: {e}")))?;
    let resp = client
        .request(method, url)
        .header("Depth", "0")
        .basic_auth(username, Some(secret))
        .send()
        .await
        .map_err(|e| AgentError::Network(format!("webdav verify: {e}")))?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(AgentError::Auth(format!(
            "webdav authentication rejected ({status}); check the application password"
        )));
    }
    if status == 207 || (200..300).contains(&status) {
        return Ok(());
    }
    Err(AgentError::Other(format!(
        "webdav verify failed: HTTP {status}"
    )))
}

/// CLI dispatch: parse args → call the [`AuthBackend`] service method →
/// render to `Output` (P1 wiring, [F012](../../docs/adr/F012-architecture-deepening-phase.md)).
///
/// This is the only function in the auth module that touches `Output` for
/// actions; service methods are output-free and directly testable via
/// [`testkit::MockAuthBackend`].
async fn dispatch(backend: &dyn AuthBackend, action: &str, args: &[String]) -> Result<Output> {
    let (flags, _positional) = parse_simple_args(args);
    let module_opt = flags.get("module").cloned();
    match action {
        "list" => {
            let rows = backend.list(module_opt.as_deref()).await?;
            render_list(rows)
        }
        "login" | "logout" | "verify" => {
            let module = module_opt.ok_or_else(|| {
                AgentError::InvalidArgument(format!("auth {action} requires --module <module>"))
            })?;
            let account =
                backend.resolve_account(&module, flags.get("account").map(String::as_str))?;
            match action {
                "login" => {
                    let secret = flags.get("password").map(String::as_str);
                    let verify = flags.get("verify").map(|v| v == "true").unwrap_or(false);
                    let receipt = backend
                        .login(&AuthLoginRequest {
                            module: &module,
                            account: &account,
                            secret,
                            verify,
                        })
                        .await?;
                    Ok(Output::text(receipt.message))
                }
                "logout" => {
                    let msg = backend.logout(&module, &account).await?;
                    Ok(Output::text(msg))
                }
                "verify" => {
                    let outcome = backend.verify(&module, &account).await?;
                    match outcome {
                        VerifyOutcome::NotRequired => Ok(Output::text(format!(
                            "{module} account '{account}' requires no credential (not_required)"
                        ))),
                        VerifyOutcome::Verified => Ok(Output::text(format!(
                            "{module} account '{account}' verified"
                        ))),
                    }
                }
                _ => unreachable!(),
            }
        }
        other => Err(AgentError::UnknownAction(format!("auth {other}"))),
    }
}

/// Render `auth list` rows to Json or a text table.
fn render_list(rows: Vec<CredentialRow>) -> Result<Output> {
    if crate::util::json_mode::is_json() {
        let arr: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "module": r.module,
                    "account": r.account,
                    "status": r.status,
                })
            })
            .collect();
        Ok(Output::Json(json!(arr)))
    } else {
        let tbl: Vec<Vec<String>> = rows
            .iter()
            .map(|r| vec![r.module.clone(), r.account.clone(), r.status.clone()])
            .collect();
        Ok(Output::records(
            vec!["module".into(), "account".into(), "status".into()],
            tbl,
        ))
    }
}

#[async_trait]
impl Executor for AuthModule {
    fn description(&self) -> &'static str {
        "Credential lifecycle (login/logout/verify/list) for all modules."
    }

    fn module_arg_spec(&self) -> ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ModuleArgSpec};
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!(
                "login",
                "保存凭据到系统 keyring（默认只存；--verify 显式验证）",
                "everyday auth login --module <mod> [--account NAME] [--password PWD] [--verify]",
                &[
                    flag!("module", "目标模块（mail/cal/webdav）"),
                    flag!("password", "密码（mail/cal/webdav，非交互）"),
                    flag!("verify", "存后显式验证凭据", Bool),
                ]
            ),
            cli_action!(
                "logout",
                "从 keyring 删除凭据",
                "everyday auth logout --module <mod> [--account NAME]",
                &[flag!("module", "目标模块")]
            ),
            cli_action!(
                "verify",
                "读取已存凭据并验证（不重新输入）",
                "everyday auth verify --module <mod> [--account NAME]",
                &[flag!("module", "目标模块")]
            ),
            cli_action!(
                "list",
                "枚举账户并探测 keyring 状态（stored/missing/not_required）",
                "everyday auth list [--module <mod>]",
                &[flag!("module", "目标模块（省略则全部）")]
            ),
        ];
        ModuleArgSpec {
            name: "auth",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(
        &self,
        action: &str,
        args: &[String],
        _ctx: &crate::shared::request_context::RequestContext,
    ) -> Result<Output> {
        let backend = for_config(&self.config);
        dispatch(&*backend, action, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::config::Config;
    use toml;

    fn test_config() -> Config {
        let s = r#"
[default_account]
note = "local1"
mail = "m1"

[[mail.accounts]]
name = "m1"
imap_host = "imap.example.com"
smtp_host = "smtp.example.com"
username = "me@example.com"

[[note.accounts]]
name = "local1"
provider = "local"

[[todo.accounts]]
name = "local1"
provider = "local"

[[bookmark.accounts]]
name = "local1"
provider = "local"

[[rss.feeds]]
name = "hn"
url = "https://hnrss.org/frontpage"

[[webdav.accounts]]
name = "wd1"
url = "https://dav.jianguoyun.com/dav/everyday"
username = "wd@example.com"
"#;
        toml::from_str(s).unwrap()
    }

    #[test]
    fn resolve_strategy_password_modules() {
        let c = test_config();
        assert_eq!(
            resolve_strategy(&c, "mail", "m1").unwrap(),
            AuthStrategy::Password
        );
        assert_eq!(
            resolve_strategy(&c, "cal", "m1").unwrap(),
            AuthStrategy::Password
        );
    }

    #[test]
    fn resolve_strategy_local_none() {
        let c = test_config();
        assert_eq!(
            resolve_strategy(&c, "note", "local1").unwrap(),
            AuthStrategy::None
        );
        assert_eq!(
            resolve_strategy(&c, "todo", "local1").unwrap(),
            AuthStrategy::None
        );
        assert_eq!(
            resolve_strategy(&c, "bookmark", "local1").unwrap(),
            AuthStrategy::None
        );
    }

    #[test]
    fn resolve_strategy_rss_none() {
        let c = test_config();
        assert_eq!(
            resolve_strategy(&c, "rss", "hn").unwrap(),
            AuthStrategy::None
        );
    }

    #[test]
    fn resolve_strategy_webdav_password() {
        let c = test_config();
        assert_eq!(
            resolve_strategy(&c, "webdav", "wd1").unwrap(),
            AuthStrategy::Password
        );
    }

    #[test]
    fn webdav_username_resolves_from_config() {
        let c = test_config();
        assert_eq!(username_for(&c, "webdav", "wd1").unwrap(), "wd@example.com");
    }

    #[test]
    fn webdav_keyring_target_uses_username() {
        let c = test_config();
        let (service, user) = keyring_target(&c, "webdav", "wd1", &AuthStrategy::Password).unwrap();
        assert_eq!(service, "everyday/webdav/wd1");
        assert_eq!(user, "wd@example.com");
    }

    #[test]
    fn resolve_strategy_unknown_module_errors() {
        let c = test_config();
        assert!(resolve_strategy(&c, "bogus", "x").is_err());
    }

    #[test]
    fn get_credential_missing_is_error() {
        // No keyring write happens; a missing entry (or unavailable backend) must
        // surface as an Auth error, not panic.
        let c = test_config();
        let err = get_credential(&c, "mail", "m1").unwrap_err();
        assert_eq!(err.type_name(), "AuthError");
    }

    #[tokio::test]
    async fn verify_none_strategy_short_circuits() {
        let backend = ConfigAuthBackend::new(Arc::new(test_config()));
        let out = backend.verify("note", "local1").await.unwrap();
        assert_eq!(out, VerifyOutcome::NotRequired);
    }

    #[tokio::test]
    async fn list_reports_three_states() {
        let backend = ConfigAuthBackend::new(Arc::new(test_config()));
        let rows = backend.list(None).await.unwrap();
        assert!(
            rows.iter()
                .any(|r| r.module == "note" && r.account == "local1" && r.status == "not_required")
        );
        // mail/m1 has no stored credential in this environment → "missing"
        assert!(
            rows.iter()
                .any(|r| r.module == "mail" && r.account == "m1" && r.status == "missing")
        );
    }

    #[tokio::test]
    async fn logout_none_strategy_errors() {
        let backend = ConfigAuthBackend::new(Arc::new(test_config()));
        let err = backend.logout("note", "local1").await.unwrap_err();
        assert_eq!(err.type_name(), "AuthError");
    }

    #[tokio::test]
    async fn resolve_account_falls_back_to_default() {
        let backend = ConfigAuthBackend::new(Arc::new(test_config()));
        // test_config sets [default_account] note = "local1".
        assert_eq!(backend.resolve_account("note", None).unwrap(), "local1");
        // Explicit override wins.
        assert_eq!(
            backend.resolve_account("todo", Some("local1")).unwrap(),
            "local1"
        );
        // No default + no override → error.
        assert!(backend.resolve_account("bookmark", None).is_err());
    }

    // ============ P1 dispatch tests (Mock backend) ============

    /// Test-only in-memory backend for dispatch tests.
    pub(crate) mod testkit {
        use super::super::*;
        use std::sync::Mutex;

        /// `(module, account, secret, verify)` of the last `login` call.
        type LoginCall = (String, String, Option<String>, bool);

        #[derive(Default)]
        pub struct MockAuthBackend {
            /// Rows returned by `list`.
            pub rows: Vec<CredentialRow>,
            /// Outcome returned by `verify`.
            pub verify_outcome: Option<VerifyOutcome>,
            /// Last `login` request (module/account/secret/verify).
            pub last_login: Mutex<Option<LoginCall>>,
        }

        #[async_trait]
        impl AuthBackend for MockAuthBackend {
            fn resolve_account(&self, _module: &str, account: Option<&str>) -> Result<String> {
                account
                    .map(|a| a.to_string())
                    .ok_or_else(|| AgentError::InvalidArgument("missing account".into()))
            }

            async fn login(&self, req: &AuthLoginRequest<'_>) -> Result<LoginReceipt> {
                *self.last_login.lock().unwrap() = Some((
                    req.module.to_string(),
                    req.account.to_string(),
                    req.secret.map(|s| s.to_string()),
                    req.verify,
                ));
                Ok(LoginReceipt {
                    message: format!(
                        "credential stored for {} account '{}'",
                        req.module, req.account
                    ),
                    verified: req.verify,
                })
            }

            async fn logout(&self, _module: &str, account: &str) -> Result<String> {
                Ok(format!("credential removed for {account}"))
            }

            async fn verify(&self, _module: &str, _account: &str) -> Result<VerifyOutcome> {
                Ok(self.verify_outcome.unwrap_or(VerifyOutcome::Verified))
            }

            async fn list(&self, _module: Option<&str>) -> Result<Vec<CredentialRow>> {
                Ok(self.rows.clone())
            }
        }
    }

    use testkit::MockAuthBackend;

    #[tokio::test]
    async fn dispatch_login_forwards_secret_and_verify() {
        let mock = MockAuthBackend::default();
        let out = dispatch(
            &mock,
            "login",
            &[
                "--module".to_string(),
                "mail".to_string(),
                "--account".to_string(),
                "m1".to_string(),
                "--password".to_string(),
                "hunter2".to_string(),
                "--verify".to_string(),
                "true".to_string(),
            ],
        )
        .await
        .unwrap();
        let (module, account, secret, verify) = mock.last_login.lock().unwrap().clone().unwrap();
        assert_eq!(module, "mail");
        assert_eq!(account, "m1");
        assert_eq!(secret.as_deref(), Some("hunter2"));
        assert!(verify);
        if let Output::Text(s) = out {
            assert!(s.contains("credential stored for mail account 'm1'"));
        } else {
            panic!("expected Text output");
        }
    }

    #[tokio::test]
    async fn dispatch_verify_not_required_renders_text() {
        let mock = MockAuthBackend {
            verify_outcome: Some(VerifyOutcome::NotRequired),
            ..Default::default()
        };
        let out = dispatch(
            &mock,
            "verify",
            &[
                "--module".to_string(),
                "note".to_string(),
                "--account".to_string(),
                "local1".to_string(),
            ],
        )
        .await
        .unwrap();
        if let Output::Text(s) = out {
            assert!(s.contains("not_required"));
        } else {
            panic!("expected Text output");
        }
    }

    #[tokio::test]
    async fn dispatch_list_renders_json_rows() {
        let mock = MockAuthBackend {
            rows: vec![CredentialRow {
                module: "note".into(),
                account: "local1".into(),
                status: "not_required".into(),
            }],
            ..Default::default()
        };
        crate::util::json_mode::set_json_mode(true);
        let out = dispatch(&mock, "list", &[]).await.unwrap();
        crate::util::json_mode::set_json_mode(false);
        if let Output::Json(v) = out {
            assert_eq!(v[0]["module"], "note");
            assert_eq!(v[0]["status"], "not_required");
        } else {
            panic!("expected Json output");
        }
    }

    #[tokio::test]
    async fn dispatch_login_requires_module() {
        let mock = MockAuthBackend::default();
        let err = dispatch(&mock, "login", &[]).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn dispatch_unknown_action_errors() {
        let mock = MockAuthBackend::default();
        let err = dispatch(&mock, "bogus", &[]).await.unwrap_err();
        assert_eq!(err.type_name(), "UnknownAction");
    }
}
