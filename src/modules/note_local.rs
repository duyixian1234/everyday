//! note 模块的本地 SQLite provider。
//!
//! 与 Notion provider 对等实现 `search` / `list` / `create` / `read` / `append`
//! / `update` 语义，数据落在账户配置的本地 SQLite 文件中。`login` 对本地
//! provider 无意义（无需凭证）。
//!
//! 数据模型：
//! - `notes(id, title, content, created_at, updated_at)`：一条笔记 = 标题 + 正文
//!   （Markdown 纯文本）。
//! - `note_props(note_id, key, value)`：简化的键值属性（对应 Notion 页面属性）。
//!
//! 输出形态（列名 / JSON key）刻意与 `note.rs` 的 Notion 版本保持一致。

use std::collections::HashMap;
use std::io::{IsTerminal, Read};

use serde_json::{Map, Value, json};
use sqlx::{Row, SqlitePool};

use crate::config::NoteAccount;
use crate::error::{AgentError, Result};
use crate::modules::local::{connect, mode_json, resolve_db_path};
use crate::output::Output;

const CREATE_NOTES_SQL: &str = "CREATE TABLE IF NOT EXISTS notes (\
    id TEXT PRIMARY KEY, \
    title TEXT NOT NULL, \
    content TEXT NOT NULL DEFAULT '', \
    created_at TEXT NOT NULL, \
    updated_at TEXT NOT NULL)";

const CREATE_PROPS_SQL: &str = "CREATE TABLE IF NOT EXISTS note_props (\
    note_id TEXT NOT NULL, \
    key TEXT NOT NULL, \
    value TEXT NOT NULL, \
    PRIMARY KEY (note_id, key))";

/// 打开连接并确保表存在。
async fn open(account: &NoteAccount) -> Result<SqlitePool> {
    let path = resolve_db_path("note", &account.name, account.db_path.as_deref())?;
    let pool = connect(&path).await?;
    sqlx::query(CREATE_NOTES_SQL).execute(&pool).await?;
    sqlx::query(CREATE_PROPS_SQL).execute(&pool).await?;
    Ok(pool)
}

/// 生成短唯一 ID（note 前缀 `n`；实现见 [`crate::util::id::gen_id`]）。
fn gen_id() -> String {
    crate::util::id::gen_id("n")
}

/// 解析 page_id：优先位置参数，否则账户 default_page_id。
fn resolve_page_id(account: &NoteAccount, positional: &[String]) -> Result<String> {
    if let Some(first) = positional.first() {
        return Ok(first.clone());
    }
    account.default_page_id.clone().ok_or_else(|| {
        AgentError::InvalidArgument(
            "no <page_id> given and no default_page_id set for this account".into(),
        )
    })
}

/// 读取某条笔记的属性为 `key -> value` map。
async fn load_props(pool: &SqlitePool, note_id: &str) -> Result<Map<String, Value>> {
    let rows = sqlx::query("SELECT key, value FROM note_props WHERE note_id = ?1 ORDER BY key")
        .bind(note_id)
        .fetch_all(pool)
        .await?;
    let mut m = Map::new();
    for r in &rows {
        m.insert(
            r.get::<String, _>("key"),
            Value::String(r.get::<String, _>("value")),
        );
    }
    Ok(m)
}

// ============ actions ============

/// `note login`（本地）：本地 provider 无需凭证。
pub fn login(account: &NoteAccount) -> Result<Output> {
    Ok(Output::text(format!(
        "note account '{}' uses the local sqlite provider; no login required",
        account.name
    )))
}

/// `note search --query Q [--limit N]`（本地）：按标题模糊搜索。
pub async fn search(account: &NoteAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let query = flags
        .get("query")
        .ok_or_else(|| AgentError::InvalidArgument("search requires --query <keyword>".into()))?;
    let limit: i64 = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
        .min(100);
    let pool = open(account).await?;

    let pattern = format!("%{query}%");
    let rows = sqlx::query(
        "SELECT id, title, updated_at FROM notes WHERE title LIKE ?1 \
         ORDER BY updated_at DESC LIMIT ?2",
    )
    .bind(&pattern)
    .bind(limit)
    .fetch_all(&pool)
    .await?;

    if mode_json() {
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.get::<String, _>("id"),
                    "type": "page",
                    "title": r.get::<String, _>("title"),
                    "last_edited": r.get::<String, _>("updated_at"),
                })
            })
            .collect();
        Ok(Output::Json(Value::Array(items)))
    } else {
        let table_rows = rows
            .iter()
            .map(|r| {
                vec![
                    r.get::<String, _>("id"),
                    "page".to_string(),
                    r.get::<String, _>("title"),
                    r.get::<String, _>("updated_at"),
                ]
            })
            .collect();
        Ok(Output::records(
            vec![
                "id".into(),
                "type".into(),
                "title".into(),
                "last_edited".into(),
            ],
            table_rows,
        ))
    }
}

/// `note list [--limit N]`（本地）：列出全部笔记。
pub async fn list(account: &NoteAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let limit: i64 = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(100);
    let pool = open(account).await?;

    let rows =
        sqlx::query("SELECT id, title, updated_at FROM notes ORDER BY updated_at DESC LIMIT ?1")
            .bind(limit)
            .fetch_all(&pool)
            .await?;

    if mode_json() {
        let mut items: Vec<Value> = Vec::new();
        for r in &rows {
            let id: String = r.get("id");
            let props = load_props(&pool, &id).await?;
            items.push(json!({
                "id": id,
                "title": r.get::<String, _>("title"),
                "url": "",
                "last_edited": r.get::<String, _>("updated_at"),
                "properties": Value::Object(props),
            }));
        }
        Ok(Output::Json(Value::Array(items)))
    } else {
        let table_rows = rows
            .iter()
            .map(|r| {
                vec![
                    r.get::<String, _>("id"),
                    r.get::<String, _>("title"),
                    r.get::<String, _>("updated_at"),
                ]
            })
            .collect();
        Ok(Output::records(
            vec!["id".into(), "title".into(), "last_edited".into()],
            table_rows,
        ))
    }
}

/// `note create --title T [--prop K:V ...]`（本地）：新建一条笔记。
pub async fn create(
    account: &NoteAccount,
    flags: &HashMap<String, String>,
    multi: &[(String, String)],
) -> Result<Output> {
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("create requires --title <title>".into()))?;
    let pool = open(account).await?;
    let id = gen_id();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO notes (id, title, content, created_at, updated_at) VALUES (?1, ?2, '', ?3, ?3)",
    )
    .bind(&id)
    .bind(title)
    .bind(&now)
    .execute(&pool)
    .await?;

    let mut count = 0usize;
    for (k, v) in split_props(multi)? {
        upsert_prop(&pool, &id, &k, &v).await?;
        count += 1;
    }

    if mode_json() {
        Ok(Output::Json(
            json!({ "id": id, "title": title, "properties": count }),
        ))
    } else {
        Ok(Output::text(format!(
            "created note '{title}' (id={id}, props={count})"
        )))
    }
}

/// `note read [page_id]`（本地）：读取标题 + 属性 + 正文。
pub async fn read(account: &NoteAccount, positional: &[String]) -> Result<Output> {
    let id = resolve_page_id(account, positional)?;
    let pool = open(account).await?;

    let row = sqlx::query("SELECT title, content FROM notes WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| {
            AgentError::InvalidArgument(format!("no note with id '{id}' in local database"))
        })?;
    let title: String = row.get("title");
    let content: String = row.get("content");
    let props = load_props(&pool, &id).await?;

    if mode_json() {
        Ok(Output::Json(json!({
            "id": id,
            "title": title,
            "url": "",
            "properties": Value::Object(props),
            "content": content,
        })))
    } else {
        let mut text = String::new();
        if !title.is_empty() {
            text.push_str(&format!("# {title}\n\n"));
        }
        text.push_str(&content);
        Ok(Output::text(text))
    }
}

/// `note append [page_id] --text TEXT`（本地）：向正文末尾追加文本。
pub async fn append(
    account: &NoteAccount,
    flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    let id = resolve_page_id(account, positional)?;

    let text = match flags.get("text") {
        Some(t) => t.clone(),
        None => {
            if std::io::stdin().is_terminal() {
                return Err(AgentError::InvalidArgument(
                    "append requires --text TEXT or piped stdin".into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| AgentError::Io(e.to_string()))?;
            buf
        }
    };
    if text.trim().is_empty() {
        return Err(AgentError::InvalidArgument(
            "nothing to append (empty text)".into(),
        ));
    }

    let pool = open(account).await?;
    let row = sqlx::query("SELECT content FROM notes WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| {
            AgentError::InvalidArgument(format!("no note with id '{id}' in local database"))
        })?;
    let existing: String = row.get("content");
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let new_content = format!("{existing}{separator}{text}");
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE notes SET content = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(&new_content)
        .bind(&now)
        .bind(&id)
        .execute(&pool)
        .await?;

    let appended = text.lines().filter(|l| !l.trim().is_empty()).count().max(1);
    if mode_json() {
        Ok(Output::Json(json!({ "id": id, "appended": appended })))
    } else {
        Ok(Output::text(format!(
            "appended {appended} line(s) to note {id}"
        )))
    }
}

/// `note update <page_id> --prop K:V ...`（本地）：更新（upsert）属性。
pub async fn update(
    account: &NoteAccount,
    positional: &[String],
    multi: &[(String, String)],
) -> Result<Output> {
    let id = positional
        .first()
        .ok_or_else(|| AgentError::InvalidArgument("update requires <page_id>".into()))?
        .clone();
    if multi.is_empty() {
        return Err(AgentError::InvalidArgument(
            "update requires at least one --prop K:V".into(),
        ));
    }
    let pool = open(account).await?;
    // 校验笔记存在。
    let exists = sqlx::query("SELECT 1 FROM notes WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&pool)
        .await?
        .is_some();
    if !exists {
        return Err(AgentError::InvalidArgument(format!(
            "no note with id '{id}' in local database"
        )));
    }

    let mut count = 0usize;
    for (k, v) in split_props(multi)? {
        upsert_prop(&pool, &id, &k, &v).await?;
        count += 1;
    }
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE notes SET updated_at = ?1 WHERE id = ?2")
        .bind(&now)
        .bind(&id)
        .execute(&pool)
        .await?;

    if mode_json() {
        Ok(Output::Json(json!({ "id": id, "updated": count })))
    } else {
        Ok(Output::text(format!(
            "updated {count} propert(ies) on note {id}"
        )))
    }
}

// ============ helpers ============

/// 把 `("prop", "K:V")` 列表拆成 `(K, V)`。
fn split_props(multi: &[(String, String)]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (_, kv) in multi {
        let (k, v) = kv
            .split_once(':')
            .ok_or_else(|| AgentError::InvalidArgument(format!("prop must be K:V, got '{kv}'")))?;
        out.push((k.to_string(), v.to_string()));
    }
    Ok(out)
}

/// 插入或更新单条属性。
async fn upsert_prop(pool: &SqlitePool, note_id: &str, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO note_props (note_id, key, value) VALUES (?1, ?2, ?3) \
         ON CONFLICT(note_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind(note_id)
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_props_parses_kv() {
        let multi = vec![
            ("prop".to_string(), "类型:文章".to_string()),
            ("prop".to_string(), "状态:未读".to_string()),
        ];
        let out = split_props(&multi).unwrap();
        assert_eq!(out[0], ("类型".to_string(), "文章".to_string()));
        assert_eq!(out[1], ("状态".to_string(), "未读".to_string()));
    }

    #[test]
    fn split_props_rejects_missing_colon() {
        let multi = vec![("prop".to_string(), "invalid".to_string())];
        assert!(split_props(&multi).is_err());
    }

    #[test]
    fn gen_id_has_prefix() {
        assert!(gen_id().starts_with('n'));
    }

    fn tmp_account() -> NoteAccount {
        let file = std::env::temp_dir().join(format!("everyday-note-test-{}.db", gen_id()));
        NoteAccount {
            name: "test".into(),
            provider: "local".into(),
            default_database_id: None,
            default_page_id: None,
            db_path: Some(file.to_string_lossy().to_string()),
        }
    }

    #[tokio::test]
    async fn create_append_update_read_roundtrip() {
        let acc = tmp_account();
        let mut flags = HashMap::new();
        flags.insert("title".into(), "Rust 笔记".into());
        let multi = vec![("prop".to_string(), "类型:文章".to_string())];
        create(&acc, &flags, &multi).await.unwrap();

        // 取出 id。
        let pool = open(&acc).await.unwrap();
        let id: String = sqlx::query("SELECT id FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("id");

        // append 正文。
        let mut af = HashMap::new();
        af.insert("text".into(), "第一行\n第二行".into());
        append(&acc, &af, std::slice::from_ref(&id)).await.unwrap();

        // update 属性。
        let umulti = vec![("prop".to_string(), "状态:已读".to_string())];
        update(&acc, std::slice::from_ref(&id), &umulti)
            .await
            .unwrap();

        // 校验内容与属性。
        let content: String = sqlx::query("SELECT content FROM notes WHERE id = ?1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("content");
        assert!(content.contains("第一行"));
        let props = load_props(&pool, &id).await.unwrap();
        assert_eq!(props.get("类型").unwrap(), "文章");
        assert_eq!(props.get("状态").unwrap(), "已读");

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn search_matches_title() {
        let acc = tmp_account();
        let mut flags = HashMap::new();
        flags.insert("title".into(), "SQLite 存储".into());
        create(&acc, &flags, &[]).await.unwrap();

        let pool = open(&acc).await.unwrap();
        let rows = sqlx::query("SELECT id FROM notes WHERE title LIKE '%SQLite%'")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn read_missing_note_errors() {
        let acc = tmp_account();
        let err = read(&acc, &["ghost".to_string()]).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }
}
