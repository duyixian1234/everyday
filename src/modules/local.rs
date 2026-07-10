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

/// 探测当前渲染模式是否为 JSON（与 note/todo 模块保持一致：以进程参数中的
/// `--json` 为准）。
pub fn mode_json() -> bool {
    crate::util::json_mode::is_json()
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
}
