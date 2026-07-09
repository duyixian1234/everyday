//! 配置加载与多账户管理。
//!
//! 配置文件路径：`~/.config/everyday/config.toml`（跨平台经 `dirs` 解析）。
//! 每个模块支持多个命名账户，顶层 `default_account` 指定默认账户名。
//! **密码绝不存配置文件**，走系统密钥环（见 `agents.md` 安全红线）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};

/// 顶层配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// 各模块的默认账户名映射。
    #[serde(default)]
    pub default_account: DefaultAccount,

    /// 邮件模块配置。
    #[serde(default)]
    pub mail: MailConfig,

    /// 日历模块配置。
    #[serde(default)]
    pub calendar: CalendarConfig,

    /// RSS 模块配置。
    #[serde(default)]
    pub rss: RssConfig,
}

/// 各模块默认账户名。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefaultAccount {
    /// 默认邮件账户名。
    #[serde(default)]
    pub mail: Option<String>,
    /// 默认日历账户名。
    #[serde(default)]
    pub calendar: Option<String>,
}

// ---- 邮件 ----

/// 邮件模块配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MailConfig {
    /// 命名账户列表。
    #[serde(default)]
    pub accounts: Vec<MailAccount>,
}

/// 单个邮件账户。密码不存这里，走 keyring。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailAccount {
    /// 账户名（如 `work`、`personal`）。
    pub name: String,
    pub imap_host: String,
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    pub username: String,
    /// 可选：是否使用 SSL/TLS。
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

// ---- 日历 ----

/// 日历模块配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalendarConfig {
    #[serde(default)]
    pub accounts: Vec<CalendarAccount>,
}

/// 单个日历账户（CalDAV）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarAccount {
    pub name: String,
    pub caldav_url: String,
    pub username: String,
    /// 该账户需要忽略的日历名称（按 displayname 匹配，不区分大小写）。
    ///
    /// 配置示例（写在 `[[calendar.accounts]]` 下）：
    /// `ignore_calendars = ["好友生日", "Tasks"]`
    /// 被忽略的日历不会出现在 `cal calendars` / `cal list` / `cal add` 中。
    #[serde(default)]
    pub ignore_calendars: Vec<String>,
}

// ---- RSS ----

/// RSS 模块配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RssConfig {
    /// 订阅源列表。
    #[serde(default)]
    pub feeds: Vec<RssFeed>,
}

/// 单个 RSS 源。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssFeed {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub category: Option<String>,
}

// ---- 加载 / 保存 ----

impl Config {
    /// 返回配置文件标准路径。
    pub fn config_path() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
        Ok(dir.join("everyday").join("config.toml"))
    }

    /// 从指定路径加载。
    pub fn load_from(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let cfg: Self = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// 从默认路径加载；文件不存在则返回默认配置（不报错）。
    pub fn load_or_default() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load_from(&path)
    }

    /// 保存到指定路径（自动创建父目录）。
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| AgentError::Config(format!("failed to serialize config: {e}")))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// 保存到默认路径。
    pub fn save(&self) -> Result<()> {
        Self::save_to(self, &Self::config_path()?)
    }

    // ---- 账户查找 ----

    /// 解析邮件账户：优先 `override_name`，其次默认，最后报错。
    pub fn mail_account(&self, override_name: Option<&str>) -> Result<&MailAccount> {
        let want = override_name.or(self.default_account.mail.as_deref());
        let name = want.ok_or_else(|| {
            AgentError::AccountNotFound(
                "no mail account specified and no default set in [default_account]".into(),
            )
        })?;
        self.mail
            .accounts
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| AgentError::AccountNotFound(format!("mail account '{name}'")))
    }

    /// 解析日历账户。
    pub fn calendar_account(&self, override_name: Option<&str>) -> Result<&CalendarAccount> {
        let want = override_name.or(self.default_account.calendar.as_deref());
        let name = want.ok_or_else(|| {
            AgentError::AccountNotFound(
                "no calendar account specified and no default set in [default_account]".into(),
            )
        })?;
        self.calendar
            .accounts
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| AgentError::AccountNotFound(format!("calendar account '{name}'")))
    }

    /// keyring 服务名约定：`everyday/<module>/<account>`。
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
        cfg.save_to(&tmp).unwrap();
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
}
