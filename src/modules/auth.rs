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

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use serde_json::json;

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::calendar;
use crate::modules::email;
use crate::modules::{Executor, ModuleArgSpec, Output};
use crate::shared::keyring_user::KEYRING_USER;
use crate::shared::notion_client::NotionClient;
use crate::util::args::parse_simple_args;

/// Credential strategy for a (module, account) pair.
///
/// Derived purely from `Config` — no per-module declaration. See
/// [R013](../../docs/adr/R013-auth-module-consolidation.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStrategy {
    /// username/password (mail, cal). keyring user = account username.
    Password,
    /// Notion Integration Token (note/todo/bookmark with provider = notion).
    /// keyring user = `KEYRING_USER` ("token").
    Token,
    /// No credential (note/todo/bookmark local/sqlite provider, rss).
    None,
}

/// Resolve the credential strategy for a (module, account) from `Config` only.
pub fn resolve_strategy(config: &Config, module: &str, account: &str) -> Result<AuthStrategy> {
    match module {
        "mail" | "cal" => Ok(AuthStrategy::Password),
        "note" | "todo" | "bookmark" => {
            let provider = provider_of(config, module, account)?;
            if crate::modules::local::is_local_provider(&provider) {
                Ok(AuthStrategy::None)
            } else {
                Ok(AuthStrategy::Token)
            }
        }
        "rss" => Ok(AuthStrategy::None),
        other => Err(AgentError::InvalidArgument(format!(
            "unknown module for auth: '{other}'"
        ))),
    }
}

/// Resolve the configured provider string for a Notion-family account.
fn provider_of(config: &Config, module: &str, account: &str) -> Result<String> {
    match module {
        "note" => Ok(config.note_account(Some(account))?.provider.clone()),
        "todo" => Ok(config.todo_account(Some(account))?.provider.clone()),
        "bookmark" => Ok(config.bookmark_account(Some(account))?.provider.clone()),
        _ => Err(AgentError::InvalidArgument(format!(
            "module '{module}' has no notion provider"
        ))),
    }
}

/// Resolve the keyring username for a password-strategy module.
fn username_for(config: &Config, module: &str, account: &str) -> Result<String> {
    match module {
        "mail" => Ok(config.mail_account(Some(account))?.username.clone()),
        "cal" => Ok(config.calendar_account(Some(account))?.username.clone()),
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
        AuthStrategy::Token => KEYRING_USER.to_string(),
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
    let entry = keyring::Entry::new(&service, &user)
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

    async fn cmd_login(
        &self,
        module: &str,
        account: &str,
        flags: &HashMap<String, String>,
    ) -> Result<Output> {
        let strategy = resolve_strategy(&self.config, module, account)?;
        let secret = match strategy {
            AuthStrategy::None => {
                return Err(AgentError::Auth(format!(
                    "module '{module}' account '{account}' requires no credential (local/sqlite or rss); nothing to store"
                )));
            }
            AuthStrategy::Password => {
                if let Some(p) = flags.get("password") {
                    p.clone()
                } else {
                    let username = username_for(&self.config, module, account)?;
                    prompt_secret(&format!("Password for {username}: ")).await?
                }
            }
            AuthStrategy::Token => {
                if let Some(t) = flags.get("token") {
                    t.clone()
                } else {
                    prompt_secret(&format!(
                        "Paste Notion Integration Token (ntn_...) for {module} account '{account}': "
                    ))
                    .await?
                }
            }
        };
        if secret.trim().is_empty() {
            return Err(AgentError::InvalidArgument(
                "credential must not be empty".into(),
            ));
        }
        store_credential(&self.config, module, account, secret.trim())?;
        let mut msg = format!("credential stored for {module} account '{account}'");
        if flags.get("verify").map(|v| v == "true").unwrap_or(false) {
            self.verify(module, account).await?;
            msg.push_str("; verified");
        }
        Ok(Output::text(msg))
    }

    async fn cmd_logout(&self, module: &str, account: &str) -> Result<Output> {
        let strategy = resolve_strategy(&self.config, module, account)?;
        if matches!(strategy, AuthStrategy::None) {
            return Err(AgentError::Auth(format!(
                "module '{module}' account '{account}' requires no credential; nothing to remove"
            )));
        }
        delete_credential(&self.config, module, account)?;
        Ok(Output::text(format!(
            "credential removed for {module} account '{account}'"
        )))
    }

    async fn cmd_verify(&self, module: &str, account: &str) -> Result<Output> {
        let strategy = resolve_strategy(&self.config, module, account)?;
        match strategy {
            AuthStrategy::None => Ok(Output::text(format!(
                "{module} account '{account}' requires no credential (not_required)"
            ))),
            _ => {
                self.verify(module, account).await?;
                Ok(Output::text(format!(
                    "{module} account '{account}' verified"
                )))
            }
        }
    }

    async fn cmd_list(&self, module: Option<&str>) -> Result<Output> {
        let modules: Vec<&str> = match module {
            Some(m) => vec![m],
            None => vec!["mail", "cal", "note", "todo", "bookmark"],
        };
        let mut rows: Vec<Value> = Vec::new();
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
                rows.push(json!({ "module": m, "account": acc_name, "status": status }));
            }
        }
        if crate::util::json_mode::is_json() {
            Ok(Output::Json(json!(rows)))
        } else {
            let tbl: Vec<Vec<String>> = rows
                .iter()
                .map(|r| {
                    vec![
                        r["module"].as_str().unwrap_or("").to_string(),
                        r["account"].as_str().unwrap_or("").to_string(),
                        r["status"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect();
            Ok(Output::records(
                vec!["module".into(), "account".into(), "status".into()],
                tbl,
            ))
        }
    }

    /// Read the stored credential and authenticate against the external service.
    ///
    /// Reuses the modules' existing connection primitives (R013): `email::imap_connect`,
    /// `calendar::cal_verify`, `NotionClient`. The `None` strategy short-circuits.
    async fn verify(&self, module: &str, account: &str) -> Result<()> {
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
            "note" | "todo" | "bookmark" => {
                let client = NotionClient::new(secret)?;
                client.get::<Value>("/users/me").await?;
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
impl Executor for AuthModule {
    fn description(&self) -> &'static str {
        "Credential lifecycle (login/logout/verify/list) for all modules."
    }

    fn module_arg_spec(&self) -> ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "login",
                description: "保存凭据到系统 keyring（默认只存；--verify 显式验证）",
                usage: "everyday auth login --module <mod> [--account NAME] [--password PWD | --token TOK] [--verify]",
                args: &[
                    ArgSpec {
                        name: "module",
                        help: "目标模块（mail/cal/note/todo/bookmark）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "password",
                        help: "密码（mail/cal，非交互）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "token",
                        help: "Notion 集成令牌（note/todo/bookmark）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "verify",
                        help: "存后显式验证凭据",
                        kind: ArgKind::Bool,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "logout",
                description: "从 keyring 删除凭据",
                usage: "everyday auth logout --module <mod> [--account NAME]",
                args: &[ArgSpec {
                    name: "module",
                    help: "目标模块",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "verify",
                description: "读取已存凭据并验证（不重新输入）",
                usage: "everyday auth verify --module <mod> [--account NAME]",
                args: &[ArgSpec {
                    name: "module",
                    help: "目标模块",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "list",
                description: "枚举账户并探测 keyring 状态（stored/missing/not_required）",
                usage: "everyday auth list [--module <mod>]",
                args: &[ArgSpec {
                    name: "module",
                    help: "目标模块（省略则全部）",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "auth",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _positional) = parse_simple_args(args);
        let module_opt = flags.get("module").cloned();
        match action {
            "list" => self.cmd_list(module_opt.as_deref()).await,
            "login" | "logout" | "verify" => {
                let module = module_opt.ok_or_else(|| {
                    AgentError::InvalidArgument(format!("auth {action} requires --module <module>"))
                })?;
                let account = flags
                    .get("account")
                    .cloned()
                    .or_else(|| default_account_name(&self.config, &module))
                    .ok_or_else(|| {
                        AgentError::InvalidArgument(format!(
                            "auth {action} requires --account <name> (or a default account for module '{module}')"
                        ))
                    })?;
                match action {
                    "login" => self.cmd_login(&module, &account, &flags).await,
                    "logout" => self.cmd_logout(&module, &account).await,
                    "verify" => self.cmd_verify(&module, &account).await,
                    _ => unreachable!(),
                }
            }
            other => Err(AgentError::UnknownAction(format!("auth {other}"))),
        }
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

[[note.accounts]]
name = "remote1"
provider = "notion"

[[todo.accounts]]
name = "local1"
provider = "local"

[[todo.accounts]]
name = "remote1"
provider = "notion"

[[bookmark.accounts]]
name = "local1"
provider = "local"

[[bookmark.accounts]]
name = "remote1"
provider = "notion"

[[rss.feeds]]
name = "hn"
url = "https://hnrss.org/frontpage"
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
    fn resolve_strategy_notion_token() {
        let c = test_config();
        assert_eq!(
            resolve_strategy(&c, "note", "remote1").unwrap(),
            AuthStrategy::Token
        );
        assert_eq!(
            resolve_strategy(&c, "todo", "remote1").unwrap(),
            AuthStrategy::Token
        );
        assert_eq!(
            resolve_strategy(&c, "bookmark", "remote1").unwrap(),
            AuthStrategy::Token
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
        let m = AuthModule::new(Arc::new(test_config()));
        let out = m.cmd_verify("note", "local1").await.unwrap();
        if let Output::Text(s) = out {
            assert!(s.contains("not_required"));
        } else {
            panic!("expected Text output");
        }
    }

    #[tokio::test]
    async fn list_reports_three_states() {
        let m = AuthModule::new(Arc::new(test_config()));
        let out = m.cmd_list(None).await.unwrap();
        if let Output::Records { rows, .. } = out {
            assert!(
                rows.iter()
                    .any(|r| r[0] == "note" && r[1] == "local1" && r[2] == "not_required")
            );
            // mail/m1 has no stored credential in this environment → "missing"
            assert!(
                rows.iter()
                    .any(|r| r[0] == "mail" && r[1] == "m1" && r[2] == "missing")
            );
        } else {
            panic!("expected Records output");
        }
    }

    #[tokio::test]
    async fn logout_none_strategy_errors() {
        let m = AuthModule::new(Arc::new(test_config()));
        let err = m.cmd_logout("note", "local1").await.unwrap_err();
        assert_eq!(err.type_name(), "AuthError");
    }
}
