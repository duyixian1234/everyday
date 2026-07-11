//! 本地 SQLite provider 的共享基础设施。
//!
//! `note` / `todo` 模块的 `local`（别名 `sqlite`）provider 复用此处的连接建立、
//! 数据库路径解析与 provider 判别逻辑，避免两个模块各写一份。
//!
//! 设计要点：
//! - 用 [`sqlx`] 的 `SqliteConnectOptions`（而非 URL 字符串）建连，规避 Windows
//!   反斜杠路径在 `sqlite://` URL 中的转义问题。
//! - `create_if_missing(true)`：文件不存在时自动创建，配合各模块的建表语句实现
//!   「首次使用即可用」。
//! - 连接池限制为单连接：CLI 每次调用是短生命周期进程，单连接足够且避免
//!   SQLite 写并发锁问题。

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::error::{AgentError, Result};

/// 判断某个 provider 字符串是否为本地 SQLite provider。
///
/// 接受 `local` 与 `sqlite` 两个别名（大小写不敏感）。
pub fn is_local_provider(provider: &str) -> bool {
    matches!(provider.to_ascii_lowercase().as_str(), "local" | "sqlite")
}

/// 解析本地 SQLite 数据库文件路径。
///
/// - `override_path`：账户配置里的 `db_path`，若存在直接使用。
/// - 否则回退到 `~/.config/everyday/<module>-<account>.db`。
pub fn resolve_db_path(
    module: &str,
    account_name: &str,
    override_path: Option<&str>,
) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(PathBuf::from(p));
    }
    let dir = dirs::config_dir()
        .ok_or_else(|| AgentError::Config("cannot determine config directory".into()))?;
    Ok(dir
        .join("everyday")
        .join(format!("{module}-{account_name}.db")))
}

/// 打开（必要时创建）SQLite 连接池。
///
/// 自动创建父目录；文件不存在时创建数据库文件。
pub async fn connect(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    Ok(pool)
}

/// 探测当前渲染模式是否为 JSON（与 note/todo 模块保持一致：以线程局部
/// `is_json()` 为准，由 main.rs 在启动时设置）。
pub fn mode_json() -> bool {
    crate::util::json_mode::is_json()
}

/// 把逗号分隔的标签字符串解析为清洗后的 `Vec<String>`。
///
/// - 去每项首尾空白；
/// - 过滤空项（避免 `"rust, ,cli"` 产生空 tag）；
/// - `None` 输入返空 Vec。
///
/// 之前 bookmark.rs 与 bookmark_local.rs 各有一份相同的实现（`parse_tags`
/// 与 `parse_tags_local_splits`），集中到此处。
pub fn parse_tags(raw: Option<&String>) -> Vec<String> {
    match raw {
        None => Vec::new(),
        Some(s) => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    }
}

/// 在 config 的 `<module>.accounts[]` 中找到 `name` 匹配的账户，
/// 写入 `default_database_id = db_id`。
///
/// 局部编辑 config TOML，不动其它账户或其它段落。
/// 之前 todo.rs 与 bookmark.rs 各有一份约 35 行逐字复制
/// （`set_todo_database_id` / `set_bookmark_database_id`），集中到此处。
pub fn set_module_database_id(
    root: &mut toml::Value,
    module: &str,
    account_name: &str,
    db_id: &str,
) -> Result<()> {
    let table = root
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("config root is not a table".into()))?;
    let section = table
        .get_mut(module)
        .ok_or_else(|| AgentError::Config(format!("no [{module}] section in config")))?;
    let section_table = section
        .as_table_mut()
        .ok_or_else(|| AgentError::Config(format!("{module} is not a table")))?;
    let accounts = section_table
        .get_mut("accounts")
        .ok_or_else(|| AgentError::Config(format!("{module}.accounts missing")))?;
    let arr = accounts
        .as_array_mut()
        .ok_or_else(|| AgentError::Config(format!("{module}.accounts is not an array")))?;

    let mut found = false;
    for acc in arr.iter_mut() {
        if acc.get("name").and_then(|n| n.as_str()) == Some(account_name) {
            acc.as_table_mut()
                .ok_or_else(|| {
                    AgentError::Config(format!("{module} account is not a table"))
                })?
                .insert(
                    "default_database_id".into(),
                    toml::Value::String(db_id.to_string()),
                );
            found = true;
            break;
        }
    }
    if !found {
        return Err(AgentError::Config(format!(
            "{module} account '{account_name}' not found in config"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_provider_accepts_aliases() {
        assert!(is_local_provider("local"));
        assert!(is_local_provider("sqlite"));
        assert!(is_local_provider("SQLite"));
        assert!(!is_local_provider("notion"));
    }

    #[test]
    fn resolve_db_path_prefers_override() {
        let p = resolve_db_path("note", "x", Some("/tmp/custom.db")).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/custom.db"));
    }

    #[test]
    fn resolve_db_path_default_contains_module_and_account() {
        let p = resolve_db_path("todo", "work", None).unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("todo-work.db"));
        assert!(s.contains("everyday"));
    }

    #[test]
    fn parse_tags_none_is_empty() {
        assert!(parse_tags(None).is_empty());
    }

    #[test]
    fn parse_tags_splits_trims_drops_empty() {
        let raw = "  rust , cli , ,  timeline  ".to_string();
        assert_eq!(parse_tags(Some(&raw)), vec!["rust", "cli", "timeline"]);
    }

    #[test]
    fn parse_tags_single_token() {
        let raw = "rust".to_string();
        assert_eq!(parse_tags(Some(&raw)), vec!["rust"]);
    }

    #[test]
    fn set_module_database_id_edits_only_target() {
        let mut root: toml::Value = toml::from_str(
            r#"
[default_account]
todo = "t1"
bookmark = "b1"

[[todo.accounts]]
name = "t1"
default_database_id = "old_t"

[[todo.accounts]]
name = "t2"

[[bookmark.accounts]]
name = "b1"
default_database_id = "old_b"
"#,
        )
        .unwrap();
        set_module_database_id(&mut root, "todo", "t1", "new_t").unwrap();

        // t1 应被更新；t2 / b1 / default_account 不动。
        let todo_accounts = root.get("todo").unwrap().get("accounts").unwrap();
        let t1 = todo_accounts.as_array().unwrap().iter().find(|a| {
            a.get("name").and_then(|n| n.as_str()) == Some("t1")
        });
        assert_eq!(
            t1.unwrap().get("default_database_id").unwrap().as_str(),
            Some("new_t")
        );

        // b1 应未变。
        let b1 = root
            .get("bookmark")
            .unwrap()
            .get("accounts")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some("b1"))
            .unwrap();
        assert_eq!(
            b1.get("default_database_id").unwrap().as_str(),
            Some("old_b")
        );
    }

    #[test]
    fn set_module_database_id_missing_account_errors() {
        let mut root: toml::Value = toml::from_str(
            r#"
[[todo.accounts]]
name = "x"
"#,
        )
        .unwrap();
        let err = set_module_database_id(&mut root, "todo", "ghost", "db").unwrap_err();
        assert!(err.message().contains("ghost"));
    }

    #[test]
    fn set_module_database_id_missing_module_errors() {
        let mut root: toml::Value = toml::Value::Table(toml::value::Table::new());
        let err = set_module_database_id(&mut root, "todo", "x", "db").unwrap_err();
        assert!(err.message().contains("todo"));
    }
}