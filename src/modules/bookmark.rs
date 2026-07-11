//! 书签模块：保存 / 浏览 Web 书签。默认使用本地 SQLite provider（`local`），
//! 也可切换为 Notion（`provider = "notion"`）。
//!
//! 设计目标：一个轻量的「稍后读 / 收藏夹」存储，每个书签含 URL、标题与一组标签。
//! 向 Agent 暴露三个高层动作：`init-db`（建库）、`add`（收藏）、`list`（按标签过滤浏览）。
//!
//! 支持的 `action`：
//! - `login`    交互式把 Notion Integration Token 存入密钥环（仅 notion provider）
//! - `init-db`  本地建表 / 在 Notion 创建书签数据库（需要 `parent_page_id`）
//! - `add`      收藏一个书签（`--url` 必填，`--title` 必填，`--tags` 可选逗号分隔）
//! - `list`     列出书签，`--tag <TAG>` 按标签过滤（`--db` 指定 Notion 数据库）
//!
//! 凭证安全：Token 仅存系统密钥环（service = `everyday/bookmark/<account>`），绝不落盘 config。
//! `database_id` / `parent_page_id` 等非机密元数据可存 config。

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::BookmarkAccount;
use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor, parse_simple_args};
use crate::notion_client::NotionClient;
use crate::output::Output;

/// 密钥环中存放 token 的条目用户名（同 service 下唯一）。
/// 见 `crate::keyring_user` —— 三个 notion 模块共享同一常量。
pub(crate) use crate::keyring_user::KEYRING_USER;

/// 干净的领域模型（输出给 Agent / 终端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkItem {
    pub id: String,
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
}

// ============ Notion 原始数据结构（强类型映射） ============

/// 对应 Notion Page 原始返回（仅取我们关心的字段，未知字段由 serde 忽略）。
#[derive(Debug, Deserialize)]
struct NotionPage {
    id: String,
    properties: BookmarkProperties,
}

/// 书签数据库的属性集合（字段名为 Notion 属性名，用 `rename` 对齐）。
#[derive(Debug, Deserialize)]
struct BookmarkProperties {
    #[serde(rename = "Title")]
    title: TitleProperty,
    #[serde(rename = "URL")]
    url: Option<UrlProperty>,
    #[serde(rename = "Tags")]
    tags: Option<TagsProperty>,
}

/// 嵌套类型叶子节点。
#[derive(Debug, Deserialize)]
struct TitleProperty {
    title: Vec<TextWrapper>,
}
#[derive(Debug, Deserialize)]
struct TextWrapper {
    plain_text: String,
}
#[derive(Debug, Deserialize)]
struct UrlProperty {
    url: Option<String>,
}
#[derive(Debug, Deserialize)]
struct TagsProperty {
    multi_select: Option<Vec<SelectDetail>>,
}
#[derive(Debug, Deserialize)]
struct SelectDetail {
    name: String,
}

/// `POST /databases/{id}/query` 的响应（仅取 results）。
#[derive(Debug, Deserialize)]
struct QueryResponse {
    results: Vec<NotionPage>,
}

// ============ 双向转换器 ============

impl From<NotionPage> for BookmarkItem {
    fn from(page: NotionPage) -> Self {
        Self {
            id: page.id,
            title: page
                .properties
                .title
                .title
                .first()
                .map(|t| t.plain_text.clone())
                .unwrap_or_default(),
            url: page.properties.url.and_then(|u| u.url).unwrap_or_default(),
            tags: page
                .properties
                .tags
                .and_then(|t| t.multi_select)
                .map(|v| v.into_iter().map(|d| d.name).collect())
                .unwrap_or_default(),
        }
    }
}

// ============ 模块 ============

pub struct BookmarkModule {
    config: std::sync::Arc<crate::config::Config>,
}

impl BookmarkModule {
    pub fn new(config: std::sync::Arc<crate::config::Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for BookmarkModule {
    fn name(&self) -> &'static str {
        "bookmark"
    }

    fn description(&self) -> &'static str {
        "Bookmarks (Notion or local sqlite): init-db, add, list, login."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new(
                "login",
                "Store Notion Integration Token in system keyring (notion provider)",
                "everyday bookmark login [--account NAME]",
            ),
            ActionDoc::new(
                "init-db",
                "Create the bookmark database (Notion needs parent_page_id; local just creates the table)",
                "everyday bookmark init-db [--account NAME] [--parent PAGE_ID]",
            ),
            ActionDoc::new(
                "add",
                "Add a bookmark",
                "everyday bookmark add --url U --title T [--tags a,b] [--db ID]",
            ),
            ActionDoc::new(
                "list",
                "List bookmarks (optionally filtered by --tag)",
                "everyday bookmark list [--tag TAG] [--db ID]",
            ),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _positional) = parse_simple_args(args);
        let account = self
            .config
            .bookmark_account(flags.get("account").map(|s| s.as_str()))?;

        // 本地 SQLite provider：路由到本地实现；否则走 Notion。
        if crate::modules::local::is_local_provider(&account.provider) {
            use crate::modules::bookmark_local as local;
            return match action {
                "login" => local::login(account),
                "init-db" => local::init_db(account).await,
                "add" => local::add(account, &flags).await,
                "list" => local::list(account, &flags).await,
                other => Err(AgentError::UnknownAction(format!("bookmark {other}"))),
            };
        }

        match action {
            "login" => bookmark_login(account).await,
            "init-db" => bookmark_init_db(account, &flags).await,
            "add" => bookmark_add(account, &flags).await,
            "list" => bookmark_list(account, &flags).await,
            other => Err(AgentError::UnknownAction(format!("bookmark {other}"))),
        }
    }
}

// ============ 标签解析 ============

// 见 `crate::modules::local::parse_tags` —— 两个 bookmark provider 共享同一实现。

// ============ 凭证（keyring） ============

/// 从密钥环读取 Notion Token。缺失时给出可执行提示。
fn get_token(account: &BookmarkAccount) -> Result<String> {
    let service = crate::config::Config::keyring_service("bookmark", &account.name);
    let entry = keyring::Entry::new(&service, KEYRING_USER)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry.get_password().map_err(|e| {
        AgentError::Auth(format!(
            "no Notion token in keyring for bookmark account '{}' ({}). \
             Run `everyday bookmark login --account {}` to store it.",
            account.name, e, account.name
        ))
    })
}

/// 交互式输入 Token 并存入密钥环。
/// 见 `crate::modules::local::login_notion` —— 与 note/todo 共享实现。
async fn bookmark_login(account: &BookmarkAccount) -> Result<Output> {
    let account_name = account.name.clone();
    crate::modules::local::login_notion("bookmark", &account_name).await?;
    Ok(Output::text(format!(
        "Notion token stored for bookmark account '{account_name}'"
    )))
}

// ============ init-db (notion) ============

/// `bookmark init-db [--parent PAGE_ID]`：在 Notion 创建书签数据库并回填 database_id。
async fn bookmark_init_db(
    account: &BookmarkAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let parent = flags
        .get("parent")
        .cloned()
        .or_else(|| account.parent_page_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(format!(
                "bookmark account '{}' has no parent_page_id. Set it in config under \
                 [[bookmark.accounts]] (parent_page_id = \"...\") or pass --parent PAGE_ID.",
                account.name
            ))
        })?;

    let token = get_token(account)?;
    let client = NotionClient::new(token)?;

    // 创建数据库：Title(title) / URL(url) / Tags(multi_select)。
    let body = json!({
        "parent": { "page_id": parent },
        "title": [{ "type": "text", "text": { "content": "Everyday Bookmarks" } }],
        "properties": {
            "Title": { "title": {} },
            "URL": { "url": {} },
            "Tags": { "multi_select": { "options": [] } }
        }
    });

    let created: Value = client.post("/databases", &body).await?;
    let db_id = created
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    let url = created
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();

    // 写回 config：仅更新 bookmark.accounts 中对应账户的 default_database_id。
    let mut root = load_config_value()?;
    set_bookmark_database_id(&mut root, &account.name, &db_id)?;
    save_config_value(&root)?;

    let json_out = json!({ "id": db_id, "url": url, "account": account.name });
    if mode_json() {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "created bookmark database '{}' for account '{}'\n{}",
            db_id, account.name, url
        )))
    }
}

// ============ add (notion) ============

/// `bookmark add --url U --title T [--tags a,b] [--db ID]`：收藏书签。
async fn bookmark_add(
    account: &BookmarkAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let url = flags
        .get("url")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --url <url>".into()))?;
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --title <title>".into()))?;
    let tags = crate::modules::local::parse_tags(flags.get("tags"));
    let db_id = resolve_db_id(account, flags)?;
    let token = get_token(account)?;
    let client = NotionClient::new(token)?;

    let multi_select: Vec<Value> = tags.iter().map(|t| json!({ "name": t })).collect();

    let props = json!({
        "Title": { "title": [{ "text": { "content": title } }] },
        "URL": { "url": url },
        "Tags": { "multi_select": multi_select }
    });

    let body = json!({ "parent": { "database_id": db_id }, "properties": props });
    let created: Value = client.post("/pages", &body).await?;
    let id = created
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    let page_url = created
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();

    let json_out = json!({
        "id": id,
        "url": url,
        "title": title,
        "tags": tags,
        "database_id": db_id
    });
    if mode_json() {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "added bookmark '{}' (id={})\n{}",
            title, id, page_url
        )))
    }
}

// ============ list (notion) ============

/// `bookmark list [--tag TAG] [--db ID]`：列出书签，可按标签过滤。
async fn bookmark_list(
    account: &BookmarkAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let db_id = resolve_db_id(account, flags)?;
    let token = get_token(account)?;
    let client = NotionClient::new(token)?;

    let mut body = json!({});
    if let Some(tag) = flags.get("tag") {
        body["filter"] = json!({
            "property": "Tags",
            "multi_select": { "contains": tag }
        });
    }

    let resp: QueryResponse = client
        .post(&format!("/databases/{db_id}/query"), &body)
        .await?;

    let mut items: Vec<BookmarkItem> = resp.results.into_iter().map(BookmarkItem::from).collect();
    // 按创建时间降序（Notion 默认升序）；无 created_time 时用 id 兜底。
    items.sort_by(|a, b| b.id.cmp(&a.id));

    if mode_json() {
        let arr: Vec<Value> = items
            .iter()
            .map(|it| serde_json::to_value(it).unwrap_or(Value::Null))
            .collect();
        Ok(Output::Json(Value::Array(arr)))
    } else {
        render_list_text(&items)
    }
}

/// 文本模式表格渲染（list 共用）。
fn render_list_text(items: &[BookmarkItem]) -> Result<Output> {
    let rows = items
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
        rows,
    ))
}

// ============ 小工具 ============

/// 解析目标数据库 ID：优先 `--db`，否则账户默认。
fn resolve_db_id(account: &BookmarkAccount, flags: &HashMap<String, String>) -> Result<String> {
    flags
        .get("db")
        .cloned()
        .or_else(|| account.default_database_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(
                "no --db given and no default_database_id set for this bookmark account \
                 (run `everyday bookmark init-db` first)."
                    .into(),
            )
        })
}

/// 探测当前渲染模式（JSON 全局 flag 已注入 args 并被 parse_simple_args 捕获到 flags，
/// 但模式判定统一以进程参数中的 `--json` 为准）。
fn mode_json() -> bool {
    crate::util::json_mode::is_json()
}

/// 读取配置文件为 toml::Value（不存在/空则空表）。
fn load_config_value() -> Result<toml::Value> {
    let path = crate::config::Config::config_path()?;
    if !path.exists() {
        return Ok(toml::Value::Table(toml::value::Table::new()));
    }
    let text = std::fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        return Ok(toml::Value::Table(toml::value::Table::new()));
    }
    Ok(toml::from_str(&text)?)
}

/// 把 toml::Value 写回配置文件（自动建父目录）。
fn save_config_value(root: &toml::Value) -> Result<()> {
    let path = crate::config::Config::config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(root)
        .map_err(|e| AgentError::Config(format!("serialize config: {e}")))?;
    std::fs::write(&path, text)?;
    Ok(())
}

/// 在 config 的 `bookmark.accounts` 中找到 name 匹配的账户，写入 `default_database_id`。
/// 见 `crate::modules::local::set_module_database_id` —— 与 todo 共享实现。
fn set_bookmark_database_id(root: &mut toml::Value, account_name: &str, db_id: &str) -> Result<()> {
    crate::modules::local::set_module_database_id(root, "bookmark", account_name, db_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_splits_and_trims() {
        // 共享 helper 的回归测试在 local.rs 那边；这里只验证 alias 路径也调到了。
        assert_eq!(
            crate::modules::local::parse_tags(None),
            Vec::<String>::new()
        );
        assert_eq!(
            crate::modules::local::parse_tags(Some(&"rust, cli ,  web ".to_string())),
            vec!["rust", "cli", "web"]
        );
    }

    #[test]
    fn notion_page_to_bookmark_item() {
        let page = json!({
            "id": "page_123",
            "properties": {
                "Title": { "title": [{ "plain_text": "Rust 官网" }] },
                "URL": { "url": "https://www.rust-lang.org" },
                "Tags": { "multi_select": [{ "name": "rust" }, { "name": "lang" }] }
            }
        });
        let np: NotionPage = serde_json::from_value(page).unwrap();
        let item = BookmarkItem::from(np);
        assert_eq!(item.id, "page_123");
        assert_eq!(item.title, "Rust 官网");
        assert_eq!(item.url, "https://www.rust-lang.org");
        assert_eq!(item.tags, vec!["rust", "lang"]);
    }

    #[test]
    fn notion_page_without_optional_fields() {
        let page = json!({
            "id": "p",
            "properties": {
                "Title": { "title": [{ "plain_text": "无标签" }] }
            }
        });
        let np: NotionPage = serde_json::from_value(page).unwrap();
        let item = BookmarkItem::from(np);
        assert_eq!(item.url, "");
        assert!(item.tags.is_empty());
    }

    // set_bookmark_database_id 的完整测试在 local.rs（共享 helper 的权威回归）。
    #[test]
    fn set_bookmark_database_id_is_shared_helper() {
        let mut root: toml::Value = toml::from_str(
            r#"
[[bookmark.accounts]]
name = "x"
"#,
        )
        .unwrap();
        assert!(set_bookmark_database_id(&mut root, "ghost", "db").is_err());
    }
}
