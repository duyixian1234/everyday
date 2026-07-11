//! Ops-log AOP hook：在 dispatch 层记录 CLI 对 notion 账户的写操作。
//!
//! 设计要点（ADR 0007）：
//! - 仅 `todo`/`note`/`bookmark` 模块的写操作记录。
//! - 仅 `provider = "notion"` 的账户记录（local 账户的 timeline provider 直接拉 SQLite）。
//! - 从 Output 的 JSON 提取 `id`（→ ref_id）和 `title`（可能缺失，取空）。
//! - 写入失败不阻断用户命令。
//!
//! Ops-log 数据库：`~/.config/everyday/ops-log.db`。

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::config::Config;
use crate::error::Result;
use crate::output::Output;

const CREATE_OPS_LOG_SQL: &str = "CREATE TABLE IF NOT EXISTS ops_log (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, \
    module TEXT NOT NULL, \
    account TEXT NOT NULL, \
    action TEXT NOT NULL, \
    ref_id TEXT NOT NULL, \
    title TEXT NOT NULL, \
    metadata TEXT NOT NULL DEFAULT '{}', \
    occurred_at TEXT NOT NULL)";

const IX_OPS_SQL: &str = "CREATE INDEX IF NOT EXISTS ix_ops_module_account_time \
    ON ops_log(module, account, occurred_at)";

/// 需要记录 ops-log 的模块。
const LOGGED_MODULES: &[&str] = &["todo", "note", "bookmark"];

/// 需要记录 ops-log 的写操作（非查询类）。
const LOGGED_ACTIONS: &[&str] = &["add", "start", "complete", "delete", "create", "update"];

/// 解析 ops-log.db 路径。
pub(crate) fn ops_log_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().ok_or_else(|| {
        crate::error::AgentError::Config("cannot determine config directory".into())
    })?;
    Ok(dir.join("everyday").join("ops-log.db"))
}

/// 打开（必要时创建）ops-log.db 连接池。
pub(crate) async fn open() -> Result<SqlitePool> {
    let path = ops_log_path()?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::query(CREATE_OPS_LOG_SQL).execute(&pool).await?;
    sqlx::query(IX_OPS_SQL).execute(&pool).await?;
    Ok(pool)
}

/// 在模块动作成功执行后调用。若是 notion 账户的写操作，记录到 ops-log。
///
/// 失败不阻断：`main.rs` 用 `let _ =` 吞掉错误。
pub async fn maybe_log_op(
    module: &str,
    action: &str,
    account_override: Option<&str>,
    config: &Config,
    output: &Output,
) -> Result<()> {
    // 1. 只记录 todo/note/bookmark 模块。
    if !LOGGED_MODULES.contains(&module) {
        return Ok(());
    }

    // 2. 只记录写操作。
    if !LOGGED_ACTIONS.contains(&action) {
        return Ok(());
    }

    // 3. 解析账户名。
    let account_name = resolve_account_name(module, account_override, config)?;
    let Some(account_name) = account_name else {
        return Ok(()); // 无账户配置，跳过
    };

    // 4. 检查 provider 是否为 notion。
    let is_notion = check_notion_provider(module, &account_name, config);
    if !is_notion {
        return Ok(()); // local provider 走 SQLite 直拉，不需要 ops-log
    }

    // 5. 从 Output 提取 ref_id 和 title。
    let (ref_id, title, metadata) = extract_from_output(module, action, output)?;

    if ref_id.is_empty() {
        // 无 ref_id 无法记录，跳过。
        return Ok(());
    }

    // 6. 写入 ops-log。
    let pool = open().await?;
    let now = chrono::Utc::now().to_rfc3339();
    let metadata_str = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".into());

    sqlx::query(
        "INSERT INTO ops_log (module, account, action, ref_id, title, metadata, occurred_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(module)
    .bind(&account_name)
    .bind(action)
    .bind(&ref_id)
    .bind(&title)
    .bind(&metadata_str)
    .bind(&now)
    .execute(&pool)
    .await?;

    Ok(())
}

/// 解析账户名：优先 override，其次 config 默认。
fn resolve_account_name(
    module: &str,
    override_name: Option<&str>,
    config: &Config,
) -> Result<Option<String>> {
    let name = match module {
        "todo" => override_name
            .map(|s| s.to_string())
            .or_else(|| config.default_account.todo.clone()),
        "note" => override_name
            .map(|s| s.to_string())
            .or_else(|| config.default_account.note.clone()),
        "bookmark" => override_name
            .map(|s| s.to_string())
            .or_else(|| config.default_account.bookmark.clone()),
        _ => return Ok(None),
    };
    Ok(name)
}

/// 检查某 (module, account) 的 provider 是否为 notion。
fn check_notion_provider(module: &str, account_name: &str, config: &Config) -> bool {
    match module {
        "todo" => config
            .todo
            .accounts
            .iter()
            .find(|a| a.name == account_name)
            .map(|a| !crate::modules::local::is_local_provider(&a.provider))
            .unwrap_or(false),
        "note" => config
            .note
            .accounts
            .iter()
            .find(|a| a.name == account_name)
            .map(|a| !crate::modules::local::is_local_provider(&a.provider))
            .unwrap_or(false),
        "bookmark" => config
            .bookmark
            .accounts
            .iter()
            .find(|a| a.name == account_name)
            .map(|a| !crate::modules::local::is_local_provider(&a.provider))
            .unwrap_or(false),
        _ => false,
    }
}

/// 从 Output 提取 ref_id（id 字段）和 title。
///
/// 非 JSON 输出转 JSON 后提取。缺失字段取空串。
fn extract_from_output(
    module: &str,
    action: &str,
    output: &Output,
) -> Result<(String, String, Value)> {
    let json_val = match output {
        Output::Json(v) => v.clone(),
        Output::Text(_s) => {
            // 文本模式无法可靠提取结构化数据，跳过。
            return Ok((String::new(), String::new(), Value::Null));
        }
        Output::Records { headers, rows } => {
            // 表格输出转 JSON 对象数组，取第一行。
            if rows.is_empty() {
                return Ok((String::new(), String::new(), Value::Null));
            }
            let mut obj = serde_json::Map::new();
            for (h, v) in headers.iter().zip(rows[0].iter()) {
                obj.insert(h.clone(), Value::String(v.clone()));
            }
            Value::Object(obj)
        }
    };

    let ref_id = json_val
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let title = json_val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 构造 metadata（记录模块特有字段）。
    let metadata = match module {
        "todo" => {
            let status = json_val.get("status").cloned().unwrap_or(Value::Null);
            serde_json::json!({"status": status, "action": action})
        }
        "note" => serde_json::json!({"action": action}),
        "bookmark" => {
            let url = json_val.get("url").cloned().unwrap_or(Value::Null);
            let tags = json_val.get("tags").cloned().unwrap_or(Value::Null);
            serde_json::json!({"url": url, "tags": tags, "action": action})
        }
        _ => Value::Null,
    };

    Ok((ref_id, title, metadata))
}

// ============ 读取（用于 timeline 投影） ============

/// ops-log 行投影供 timeline 使用的形态。
#[derive(Debug, Clone)]
#[allow(dead_code)] // module 已通过 source 暴露给 timeline,保留供未来过滤
pub struct OpsLogEntry {
    /// 模块名（`todo` / `note` / `bookmark`），同时作为 timeline source。
    pub module: String,
    pub account: String,
    /// 操作类型（`add` / `create` / `update` / `complete` / `start` / `delete`），
    /// 作为 timeline `event_type`。
    pub action: String,
    pub ref_id: String,
    pub title: String,
    pub metadata: Value,
    /// RFC3339 字符串，timeline 端 parse 为 UTC DateTime。
    pub occurred_at: String,
}

/// 读取 ops-log.db 中指定 module 在 `[from, to]` 窗口内的所有条目。
///
/// - `module`：`todo` / `note` / `bookmark` 之一。
/// - `from`：UTC 时间，inclusive；entries with `occurred_at >= from`。
/// - `to`：UTC 时间，inclusive（None = 不设上限）。
///
/// 用于 [`OpsLogProvider`] 在每次 sync 时把 ops-log 行投影到 timeline events 表。
/// DB 不存在时返回 `Ok(vec![])`（不报错），让 `--sync` 在新用户环境也能用。
pub async fn fetch_ops_log_for_timeline(
    module: &str,
    from: DateTime<Utc>,
    to: Option<DateTime<Utc>>,
) -> Result<Vec<OpsLogEntry>> {
    use chrono::{DateTime, Utc};

    let path = ops_log_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let pool = open().await?;
    let from_str = from.to_rfc3339();
    let to_str = to.map(|t: DateTime<Utc>| t.to_rfc3339());

    // 动态 SQL：`to` 可能是 None。
    let rows = if let Some(ref to_s) = to_str {
        sqlx::query_as::<_, OpsRow>(
            "SELECT module, account, action, ref_id, title, metadata, occurred_at \
             FROM ops_log \
             WHERE module = ?1 AND occurred_at >= ?2 AND occurred_at <= ?3 \
             ORDER BY occurred_at ASC",
        )
        .bind(module)
        .bind(&from_str)
        .bind(to_s)
        .fetch_all(&pool)
        .await?
    } else {
        sqlx::query_as::<_, OpsRow>(
            "SELECT module, account, action, ref_id, title, metadata, occurred_at \
             FROM ops_log \
             WHERE module = ?1 AND occurred_at >= ?2 \
             ORDER BY occurred_at ASC",
        )
        .bind(module)
        .bind(&from_str)
        .fetch_all(&pool)
        .await?
    };

    Ok(rows.into_iter().map(Into::into).collect())
}

/// sqlx 行中间结构（手写 FromRow 避免依赖 sqlx macros feature）。
struct OpsRow {
    module: String,
    account: String,
    action: String,
    ref_id: String,
    title: String,
    metadata: String,
    occurred_at: String,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for OpsRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            module: row.try_get("module")?,
            account: row.try_get("account")?,
            action: row.try_get("action")?,
            ref_id: row.try_get("ref_id")?,
            title: row.try_get("title")?,
            metadata: row.try_get("metadata")?,
            occurred_at: row.try_get("occurred_at")?,
        })
    }
}

impl From<OpsRow> for OpsLogEntry {
    fn from(r: OpsRow) -> Self {
        let metadata = serde_json::from_str(&r.metadata).unwrap_or(Value::Null);
        Self {
            module: r.module,
            account: r.account,
            action: r.action,
            ref_id: r.ref_id,
            title: r.title,
            metadata,
            occurred_at: r.occurred_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_modules_excludes_mail_cal_rss() {
        assert!(LOGGED_MODULES.contains(&"todo"));
        assert!(LOGGED_MODULES.contains(&"note"));
        assert!(LOGGED_MODULES.contains(&"bookmark"));
        assert!(!LOGGED_MODULES.contains(&"mail"));
        assert!(!LOGGED_MODULES.contains(&"cal"));
        assert!(!LOGGED_MODULES.contains(&"rss"));
    }

    #[test]
    fn logged_actions_excludes_queries() {
        assert!(LOGGED_ACTIONS.contains(&"add"));
        assert!(LOGGED_ACTIONS.contains(&"create"));
        assert!(LOGGED_ACTIONS.contains(&"complete"));
        assert!(!LOGGED_ACTIONS.contains(&"list"));
        assert!(!LOGGED_ACTIONS.contains(&"search"));
        assert!(!LOGGED_ACTIONS.contains(&"login"));
    }

    #[test]
    fn extract_from_json_output() {
        let output = Output::Json(serde_json::json!({
            "id": "t123",
            "title": "写周报",
            "status": "Todo"
        }));
        let (ref_id, title, metadata) = extract_from_output("todo", "add", &output).unwrap();
        assert_eq!(ref_id, "t123");
        assert_eq!(title, "写周报");
        assert_eq!(metadata["status"], "Todo");
    }

    #[test]
    fn extract_from_records_output() {
        let output = Output::Records {
            headers: vec!["id".into(), "title".into()],
            rows: vec![vec!["b1".into(), "Rust 官网".into()]],
        };
        let (ref_id, title, _) = extract_from_output("bookmark", "add", &output).unwrap();
        assert_eq!(ref_id, "b1");
        assert_eq!(title, "Rust 官网");
    }

    #[test]
    fn extract_from_text_output_empty() {
        let output = Output::text("some text");
        let (ref_id, title, _) = extract_from_output("todo", "add", &output).unwrap();
        assert!(ref_id.is_empty());
        assert!(title.is_empty());
    }

    #[test]
    fn check_notion_provider_local_returns_false() {
        let config = Config {
            todo: crate::config::TodoConfig {
                accounts: vec![crate::config::TodoAccount {
                    name: "personal".into(),
                    provider: "local".into(),
                    parent_page_id: None,
                    default_database_id: None,
                    db_path: None,
                }],
            },
            ..Default::default()
        };
        assert!(!check_notion_provider("todo", "personal", &config));
    }

    #[test]
    fn check_notion_provider_notion_returns_true() {
        let config = Config {
            todo: crate::config::TodoConfig {
                accounts: vec![crate::config::TodoAccount {
                    name: "work".into(),
                    provider: "notion".into(),
                    parent_page_id: None,
                    default_database_id: None,
                    db_path: None,
                }],
            },
            ..Default::default()
        };
        assert!(check_notion_provider("todo", "work", &config));
    }
}
