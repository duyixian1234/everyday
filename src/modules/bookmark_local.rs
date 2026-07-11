//! bookmark 模块的本地 SQLite provider。
//!
//! 与 Notion provider 对等实现 `add` / `list` 语义，数据落在账户配置的本地 SQLite 文件中。
//! `login` 对本地 provider 无意义（无需凭证），`init-db` 仅建表并汇报路径。
//!
//! 数据模型：
//! - `bookmarks(id, url, title, created_at)`：一条书签 = URL + 标题。
//! - `bookmark_tags(bookmark_id, tag)`：书签的标签（多对多），用于按标签精确过滤。
//!
//! 输出形态（列名 / JSON key）刻意与 `bookmark.rs` 的 Notion 版本保持一致：
//! `id` / `title` / `url` / `tags`。

use std::collections::HashMap;

use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};

use crate::config::BookmarkAccount;
use crate::error::{AgentError, Result};
use crate::modules::bookmark::BookmarkItem;
use crate::modules::local::{connect, mode_json, resolve_db_path};
use crate::output::Output;

/// 建表语句：书签主表 + 标签关联表。
const CREATE_BOOKMARKS_SQL: &str = "CREATE TABLE IF NOT EXISTS bookmarks (\
    id TEXT PRIMARY KEY, \
    url TEXT NOT NULL, \
    title TEXT NOT NULL, \
    created_at TEXT NOT NULL)";

const CREATE_TAGS_SQL: &str = "CREATE TABLE IF NOT EXISTS bookmark_tags (\
    bookmark_id TEXT NOT NULL, \
    tag TEXT NOT NULL, \
    PRIMARY KEY (bookmark_id, tag))";

/// 打开连接并确保表存在。
async fn open(account: &BookmarkAccount) -> Result<SqlitePool> {
    let path = resolve_db_path("bookmark", &account.name, account.db_path.as_deref())?;
    let pool = connect(&path).await?;
    sqlx::query(CREATE_BOOKMARKS_SQL).execute(&pool).await?;
    sqlx::query(CREATE_TAGS_SQL).execute(&pool).await?;
    Ok(pool)
}

/// 生成短唯一 ID（bookmark 前缀 `b`；实现见 [`crate::util::id::gen_id`]）。
fn gen_id() -> String {
    crate::util::id::gen_id("b")
}

// ============ actions ============

/// `bookmark login`（本地）：本地 provider 无需凭证。
pub fn login(account: &BookmarkAccount) -> Result<Output> {
    Ok(Output::text(format!(
        "bookmark account '{}' uses the local sqlite provider; no login required",
        account.name
    )))
}

/// `bookmark init-db`（本地）：建表并汇报数据库路径。
pub async fn init_db(account: &BookmarkAccount) -> Result<Output> {
    let path = resolve_db_path("bookmark", &account.name, account.db_path.as_deref())?;
    let _ = open(account).await?;
    let path_str = path.to_string_lossy().to_string();
    if mode_json() {
        Ok(Output::Json(
            json!({ "account": account.name, "db_path": path_str, "provider": "local" }),
        ))
    } else {
        Ok(Output::text(format!(
            "initialized local bookmark database for account '{}'\n{}",
            account.name, path_str
        )))
    }
}

/// `bookmark add --url U --title T [--tags a,b]`（本地）：收藏书签。
pub async fn add(account: &BookmarkAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let url = flags
        .get("url")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --url <url>".into()))?;
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --title <title>".into()))?;
    let tags = crate::modules::local::parse_tags(flags.get("tags"));
    let pool = open(account).await?;
    let id = gen_id();
    let created_at = chrono::Utc::now().to_rfc3339();

    sqlx::query("INSERT INTO bookmarks (id, url, title, created_at) VALUES (?1, ?2, ?3, ?4)")
        .bind(&id)
        .bind(url)
        .bind(title)
        .bind(&created_at)
        .execute(&pool)
        .await?;

    for tag in &tags {
        sqlx::query("INSERT OR IGNORE INTO bookmark_tags (bookmark_id, tag) VALUES (?1, ?2)")
            .bind(&id)
            .bind(tag)
            .execute(&pool)
            .await?;
    }

    if mode_json() {
        Ok(Output::Json(
            json!({ "id": id, "url": url, "title": title, "tags": tags }),
        ))
    } else {
        Ok(Output::text(format!(
            "added bookmark '{}' (id={}, tags={})",
            title,
            id,
            tags.join(", ")
        )))
    }
}

/// `bookmark list [--tag TAG]`（本地）：列出书签，可按标签过滤。
pub async fn list(account: &BookmarkAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let pool = open(account).await?;

    // 基础查询：按标签过滤时 JOIN bookmark_tags，否则取全部。
    let rows = if let Some(tag) = flags.get("tag") {
        let sql = "SELECT b.id, b.url, b.title, b.created_at FROM bookmarks b \
            JOIN bookmark_tags t ON t.bookmark_id = b.id \
            WHERE t.tag = ?1 ORDER BY b.created_at DESC, b.id DESC";
        sqlx::query(sql).bind(tag).fetch_all(&pool).await?
    } else {
        let sql = "SELECT id, url, title, created_at FROM bookmarks \
            ORDER BY created_at DESC, id DESC";
        sqlx::query(sql).fetch_all(&pool).await?
    };

    // 逐条装载标签，组装 BookmarkItem。
    let mut items: Vec<BookmarkItem> = Vec::with_capacity(rows.len());
    for r in &rows {
        let id: String = r.get("id");
        let tag_rows =
            sqlx::query("SELECT tag FROM bookmark_tags WHERE bookmark_id = ?1 ORDER BY tag")
                .bind(&id)
                .fetch_all(&pool)
                .await?;
        let tags: Vec<String> = tag_rows
            .iter()
            .map(|tr| tr.get::<String, _>("tag"))
            .collect();
        items.push(BookmarkItem {
            id,
            url: r.get::<String, _>("url"),
            title: r.get::<String, _>("title"),
            tags,
        });
    }

    if mode_json() {
        let arr: Vec<Value> = items
            .iter()
            .map(|it| serde_json::to_value(it).unwrap_or(Value::Null))
            .collect();
        Ok(Output::Json(Value::Array(arr)))
    } else {
        let table_rows = items
            .iter()
            .map(|it| {
                vec![
                    it.id.clone(),
                    it.title.clone(),
                    it.url.clone(),
                    it.tags.join(", "),
                ]
            })
            .collect();
        Ok(Output::records(
            vec!["id".into(), "title".into(), "url".into(), "tags".into()],
            table_rows,
        ))
    }
}

// ============ Timeline 数据拉取 ============

/// Timeline 拉取用：bookmark 条目原始数据。
pub struct BookmarkTimelineEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub tags: Vec<String>,
    pub created_at: String,
}

/// Timeline 增量拉取：返回 `created_at` 落在窗口内的 bookmark。
pub async fn fetch_for_timeline(
    account: &BookmarkAccount,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<BookmarkTimelineEntry>> {
    let pool = open(account).await?;
    let from_str = from.to_rfc3339();
    let to_str = to.to_rfc3339();
    let rows = sqlx::query(
        "SELECT id, url, title, created_at FROM bookmarks \
         WHERE created_at >= ?1 AND created_at <= ?2 \
         ORDER BY created_at ASC",
    )
    .bind(&from_str)
    .bind(&to_str)
    .fetch_all(&pool)
    .await?;

    let mut entries = Vec::with_capacity(rows.len());
    for r in &rows {
        let id: String = r.get("id");
        let tag_rows =
            sqlx::query("SELECT tag FROM bookmark_tags WHERE bookmark_id = ?1 ORDER BY tag")
                .bind(&id)
                .fetch_all(&pool)
                .await?;
        let tags: Vec<String> = tag_rows
            .iter()
            .map(|tr| tr.get::<String, _>("tag"))
            .collect();
        entries.push(BookmarkTimelineEntry {
            id,
            url: r.get("url"),
            title: r.get("title"),
            tags,
            created_at: r.get("created_at"),
        });
    }
    Ok(entries)
}

// ============ 小工具 ============

// parse_tags 见 `crate::modules::local::parse_tags` —— 两处 bookmark provider 共享。

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_account() -> BookmarkAccount {
        let file = std::env::temp_dir().join(format!("everyday-bookmark-test-{}.db", gen_id()));
        BookmarkAccount {
            name: "test".into(),
            provider: "local".into(),
            parent_page_id: None,
            default_database_id: None,
            db_path: Some(file.to_string_lossy().to_string()),
        }
    }

    /// 统计某 tag 下的书签数量（JOIN bookmark_tags 精确匹配）。
    async fn count_tag(pool: &SqlitePool, tag: &str) -> i64 {
        sqlx::query(
            "SELECT COUNT(*) as c FROM bookmarks b \
             JOIN bookmark_tags t ON t.bookmark_id = b.id WHERE t.tag = ?1",
        )
        .bind(tag)
        .fetch_one(pool)
        .await
        .unwrap()
        .get::<i64, _>("c")
    }

    #[tokio::test]
    async fn add_and_list_roundtrip() {
        let acc = tmp_account();

        let mut f1 = HashMap::new();
        f1.insert("url".into(), "https://www.rust-lang.org".into());
        f1.insert("title".into(), "Rust 官网".into());
        f1.insert("tags".into(), "rust,lang".into());
        add(&acc, &f1).await.unwrap();

        let mut f2 = HashMap::new();
        f2.insert("url".into(), "https://doc.rust-lang.org".into());
        f2.insert("title".into(), "Rust 文档".into());
        f2.insert("tags".into(), "rust,doc".into());
        add(&acc, &f2).await.unwrap();

        let pool = open(&acc).await.unwrap();

        // 全部 2 条。
        let all: i64 = sqlx::query("SELECT COUNT(*) as c FROM bookmarks")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("c");
        assert_eq!(all, 2);

        // 按 tag 过滤（JOIN bookmark_tags 精确匹配）：rust -> 2，doc -> 1，lang -> 1。
        assert_eq!(count_tag(&pool, "rust").await, 2);
        assert_eq!(count_tag(&pool, "doc").await, 1);
        assert_eq!(count_tag(&pool, "lang").await, 1);

        // list 输出（默认文本模式返回 Records，JSON 模式返回数组）形态正确。
        let mut fr = HashMap::new();
        fr.insert("tag".into(), "doc".into());
        let out = list(&acc, &fr).await.unwrap();
        let rows = match out {
            Output::Records { rows, .. } => rows,
            Output::Json(v) => v
                .as_array()
                .unwrap()
                .iter()
                .map(|it| {
                    vec![
                        it["id"].as_str().unwrap_or("").to_string(),
                        it["title"].as_str().unwrap_or("").to_string(),
                        it["url"].as_str().unwrap_or("").to_string(),
                        it["tags"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default(),
                    ]
                })
                .collect(),
            other => panic!("unexpected output: {other:?}"),
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][1], "Rust 文档");

        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[tokio::test]
    async fn add_missing_url_errors() {
        let acc = tmp_account();
        let mut f = HashMap::new();
        f.insert("title".into(), "no url".into());
        let err = add(&acc, &f).await.unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
        let _ = std::fs::remove_file(acc.db_path.unwrap());
    }

    #[test]
    fn parse_tags_local_splits() {
        // 共享 helper 的完整测试在 local.rs；这里只验证 alias 调用。
        assert_eq!(
            crate::modules::local::parse_tags(Some(&"a, b ,c".to_string())),
            vec!["a", "b", "c"]
        );
    }
}
