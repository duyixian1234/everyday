//! todo 模块：基于 Notion 的待办任务管理。
//!
//! 两层架构的上层业务：持有共享 [`NotionClient`]，将干净领域模型 [`TodoItem`]
//! 与 Notion 原始 `Properties` 做强类型双向映射，屏蔽 Notion 属性套娃。
//!
//! 命令（action）：
//! - `login`    交互式把 Notion Integration Token 存入密钥环
//! - `init-db`  在 Notion 创建任务数据库（需要 `parent_page_id`），并回填 `database_id` 到 config
//! - `list`     列出未完成任务（按 Due 升序），`--all` 列出全部
//! - `add`      新增任务（`--title` 必填，`--due` / `--priority` 可选）
//! - `start`    将任务标记为 In Progress
//! - `complete` 将任务标记为 Done
//!
//! 凭证安全：Token 仅存系统密钥环（service = `everyday/todo/<account>`），绝不落盘 config。
//! `database_id` / `parent_page_id` 等非机密元数据可存 config。

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::TodoAccount;
use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor, parse_simple_args};
use crate::notion_client::NotionClient;
use crate::output::Output;

/// 状态选项名（须与 `init-db` 创建的 schema 一致）。
const STATUS_TODO: &str = "Todo";
const STATUS_IN_PROGRESS: &str = "In Progress";
const STATUS_DONE: &str = "Done";

/// 密钥环中存放 token 的条目用户名（与 note 一致：同 service 下唯一）。
const KEYRING_USER: &str = "token";

/// 干净的领域模型（输出给 Agent / 终端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub due: Option<String>,
    pub priority: Option<String>,
}

// ============ Notion 原始数据结构（强类型映射） ============

/// 对应 Notion Page 原始返回（仅取我们关心的字段，未知字段由 serde 忽略）。
#[derive(Debug, Deserialize)]
struct NotionPage {
    id: String,
    properties: TodoProperties,
}

/// 任务数据库的属性集合（字段名为 Notion 属性名，用 `rename` 对齐）。
#[derive(Debug, Deserialize)]
struct TodoProperties {
    #[serde(rename = "Task")]
    task: TitleProperty,
    #[serde(rename = "Status")]
    status: StatusProperty,
    #[serde(rename = "Due")]
    due: Option<DateProperty>,
    #[serde(rename = "Priority")]
    priority: Option<SelectProperty>,
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
struct StatusProperty {
    /// Notion 有两种状态属性类型：`select`（init-db 创建的类型）与 `status`
    /// （Notion 新版状态属性）。API 响应里分别以 `select` / `status` 为键返回，
    /// 这里两个都收，读哪个用哪个，保证与 init-db 创建的 `select` 数据库兼容，
    /// 也能兼容手动建成的 `status` 属性数据库。
    #[serde(default)]
    select: Option<SelectDetail>,
    #[serde(default)]
    status: Option<SelectDetail>,
}
#[derive(Debug, Deserialize)]
struct DateProperty {
    date: Option<DateDetail>,
}
#[derive(Debug, Deserialize)]
struct DateDetail {
    start: String,
}
#[derive(Debug, Deserialize)]
struct SelectProperty {
    select: Option<SelectDetail>,
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

impl From<NotionPage> for TodoItem {
    fn from(page: NotionPage) -> Self {
        Self {
            id: page.id,
            title: page
                .properties
                .task
                .title
                .first()
                .map(|t| t.plain_text.clone())
                .unwrap_or_default(),
            status: page
                .properties
                .status
                .select
                .or(page.properties.status.status)
                .map(|d| d.name)
                .unwrap_or_default(),
            due: page.properties.due.and_then(|d| d.date).map(|d| d.start),
            priority: page
                .properties
                .priority
                .and_then(|s| s.select)
                .map(|s| s.name),
        }
    }
}

// ============ 模块 ============

pub struct TodoModule {
    config: std::sync::Arc<crate::config::Config>,
}

impl TodoModule {
    pub fn new(config: std::sync::Arc<crate::config::Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for TodoModule {
    fn name(&self) -> &'static str {
        "todo"
    }

    fn description(&self) -> &'static str {
        "Todo tasks (Notion or local sqlite): login, init-db, list, add, start, complete."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new(
                "login",
                "Store Notion Integration Token in system keyring",
                "everyday todo login [--account NAME]",
            ),
            ActionDoc::new(
                "init-db",
                "Create the todo database in Notion (needs parent_page_id)",
                "everyday todo init-db [--account NAME] [--parent PAGE_ID]",
            ),
            ActionDoc::new(
                "list",
                "List incomplete todos (sorted by due)",
                "everyday todo list [--db ID] [--all]",
            ),
            ActionDoc::new(
                "add",
                "Add a todo",
                "everyday todo add --title T [--due DATE] [--priority P] [--db ID]",
            ),
            ActionDoc::new(
                "start",
                "Mark a todo as In Progress",
                "everyday todo start <page_id>",
            ),
            ActionDoc::new(
                "complete",
                "Mark a todo as Done",
                "everyday todo complete <page_id>",
            ),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let account = self
            .config
            .todo_account(flags.get("account").map(|s| s.as_str()))?;

        // 本地 SQLite provider：路由到本地实现；否则走 Notion。
        if crate::modules::local::is_local_provider(&account.provider) {
            use crate::modules::todo_local as local;
            return match action {
                "login" => local::login(account),
                "init-db" => local::init_db(account).await,
                "list" => local::list(account, &flags).await,
                "add" => local::add(account, &flags).await,
                "start" => {
                    local::set_status(account, positional.first(), local::STATUS_IN_PROGRESS).await
                }
                "complete" => {
                    local::set_status(account, positional.first(), local::STATUS_DONE).await
                }
                other => Err(AgentError::UnknownAction(format!("todo {other}"))),
            };
        }

        match action {
            "login" => todo_login(account).await,
            "init-db" => todo_init_db(account, &flags).await,
            "list" => todo_list(account, &flags).await,
            "add" => todo_add(account, &flags).await,
            "start" => todo_set_status(account, positional.first(), STATUS_IN_PROGRESS).await,
            "complete" => todo_set_status(account, positional.first(), STATUS_DONE).await,
            other => Err(AgentError::UnknownAction(format!("todo {other}"))),
        }
    }
}

// ============ 凭证（keyring） ============

/// 从密钥环读取 Notion Token。缺失时给出可执行提示。
fn get_token(account: &TodoAccount) -> Result<String> {
    let service = crate::config::Config::keyring_service("todo", &account.name);
    let entry = keyring::Entry::new(&service, KEYRING_USER)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry.get_password().map_err(|e| {
        AgentError::Auth(format!(
            "no Notion token in keyring for todo account '{}' ({}). \
             Run `everyday todo login --account {}` to store it.",
            account.name, e, account.name
        ))
    })
}

/// 交互式输入 Token 并存入密钥环。
async fn todo_login(account: &TodoAccount) -> Result<Output> {
    let service = crate::config::Config::keyring_service("todo", &account.name);
    let entry = keyring::Entry::new(&service, KEYRING_USER)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    let account_name = account.name.clone();
    let prompt = format!(
        "Paste Notion Integration Token (ntn_...) for todo account '{account_name}': "
    );
    // rpassword 为同步 API，放进 spawn_blocking 避免阻塞运行时。
    let password = tokio::task::spawn_blocking(move || rpassword::prompt_password(prompt))
        .await
        .map_err(|e| AgentError::Other(format!("join token prompt: {e}")))?
        .map_err(|e| AgentError::Other(format!("read token: {e}")))?;
    let token = password.trim().to_string();
    if token.is_empty() {
        return Err(AgentError::InvalidArgument(
            "token must not be empty".into(),
        ));
    }
    entry
        .set_password(&token)
        .map_err(|e| AgentError::Auth(format!("keyring set: {e}")))?;
    Ok(Output::text(format!(
        "Notion token stored for todo account '{account_name}'"
    )))
}

// ============ init-db ============

/// `todo init-db [--parent PAGE_ID]`：在 Notion 创建任务数据库并回填 database_id。
async fn todo_init_db(account: &TodoAccount, flags: &HashMap<String, String>) -> Result<Output> {
    // 父级页面：优先 --parent，否则取账户配置的 parent_page_id。
    let parent = flags
        .get("parent")
        .cloned()
        .or_else(|| account.parent_page_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(format!(
                "todo account '{}' has no parent_page_id. Set it in config under \
                 [[todo.accounts]] (parent_page_id = \"...\") or pass --parent PAGE_ID.",
                account.name
            ))
        })?;

    let token = get_token(account)?;
    let client = NotionClient::new(token)?;

    // 创建数据库：Task(title) / Status(select) / Due(date) / Priority(select)。
    let body = json!({
        "parent": { "page_id": parent },
        "title": [{ "type": "text", "text": { "content": "Everyday Todos" } }],
        "properties": {
            "Task": { "title": {} },
            "Status": { "select": { "options": [
                { "name": "Todo", "color": "default" },
                { "name": "In Progress", "color": "blue" },
                { "name": "Done", "color": "green" }
            ] } },
            "Due": { "date": {} },
            "Priority": { "select": { "options": [
                { "name": "P0", "color": "red" },
                { "name": "P1", "color": "yellow" },
                { "name": "P2", "color": "default" }
            ] } }
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

    // 写回 config：仅更新 todo.accounts 中对应账户的 default_database_id，
    // 不动其他段落（toml::Value 局部编辑，保留用户格式与注释）。
    let mut root = load_config_value()?;
    set_todo_database_id(&mut root, &account.name, &db_id)?;
    save_config_value(&root)?;

    let json_out = json!({ "id": db_id, "url": url, "account": account.name });
    if mode_json() {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "created todo database '{}' for account '{}'\n{}",
            db_id, account.name, url
        )))
    }
}

// ============ list ============

/// `todo list [--db ID] [--all]`：列出任务。
async fn todo_list(account: &TodoAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let db_id = resolve_db_id(account, flags)?;
    let token = get_token(account)?;
    let client = NotionClient::new(token)?;

    // 设计文档：过滤未完成，按 Due 升序。--all 时不过滤。
    let show_all = flags.contains_key("all");
    let mut body = json!({
        "sorts": [{ "property": "Due", "direction": "ascending" }]
    });
    if !show_all {
        body["filter"] = json!({
            "property": "Status",
            "select": { "does_not_equal": "Done" }
        });
    }

    let resp: QueryResponse = client
        .post(&format!("/databases/{db_id}/query"), &body)
        .await?;

    let mut items: Vec<TodoItem> = resp.results.into_iter().map(TodoItem::from).collect();

    // 客户端兜底：确保只列未完成（API 过滤失败时）并按 due 升序、null 排最后。
    items.retain(|it| show_all || !it.status.eq_ignore_ascii_case(STATUS_DONE));
    items.sort_by(cmp_due_asc);

    if mode_json() {
        let arr: Vec<Value> = items
            .iter()
            .map(|it| serde_json::to_value(it).unwrap_or(Value::Null))
            .collect();
        Ok(Output::Json(Value::Array(arr)))
    } else {
        let rows = items
            .iter()
            .map(|it| {
                vec![
                    it.id.clone(),
                    it.title.clone(),
                    it.status.clone(),
                    it.due.clone().unwrap_or_default(),
                    it.priority.clone().unwrap_or_default(),
                ]
            })
            .collect();
        Ok(Output::records(
            vec![
                "id".into(),
                "title".into(),
                "status".into(),
                "due".into(),
                "priority".into(),
            ],
            rows,
        ))
    }
}

/// 按 due 升序：有日期 < 无日期；同有日期则字符串比较（ISO 8601 可直接排序）。
fn cmp_due_asc(a: &TodoItem, b: &TodoItem) -> std::cmp::Ordering {
    match (&a.due, &b.due) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

// ============ add ============

/// `todo add --title T [--due DATE] [--priority P] [--db ID]`：新增任务。
async fn todo_add(account: &TodoAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --title <title>".into()))?;
    let db_id = resolve_db_id(account, flags)?;
    let token = get_token(account)?;
    let client = NotionClient::new(token)?;

    let mut props = json!({
        "Task": { "title": [{ "text": { "content": title } }] },
        "Status": { "select": { "name": STATUS_TODO } }
    });
    if let Some(due) = flags.get("due") {
        props["Due"] = json!({ "date": { "start": due } });
    }
    if let Some(pri) = flags.get("priority") {
        props["Priority"] = json!({ "select": { "name": pri } });
    }

    let body = json!({ "parent": { "database_id": db_id }, "properties": props });
    let created: Value = client.post("/pages", &body).await?;
    let id = created
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    let url = created
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();

    let json_out = json!({ "id": id, "url": url, "title": title, "database_id": db_id });
    if mode_json() {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "added todo '{}' (id={})\n{}",
            title, id, url
        )))
    }
}

// ============ start / complete ============

/// `todo start/complete <page_id>`：更新任务状态。
async fn todo_set_status(
    account: &TodoAccount,
    page_id: Option<&String>,
    status: &str,
) -> Result<Output> {
    let page_id = page_id.ok_or_else(|| {
        AgentError::InvalidArgument(format!("`{status}` requires <page_id>"))
    })?;
    let token = get_token(account)?;
    let client = NotionClient::new(token)?;

    let body = json!({ "properties": { "Status": { "select": { "name": status } } } });
    let updated: Value = client.patch(&format!("/pages/{page_id}"), &body).await?;
    let id = updated
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or(page_id)
        .to_string();
    let url = updated
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();

    let json_out = json!({ "id": id, "status": status, "url": url });
    if mode_json() {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "set todo {} -> status '{}'\n{}",
            id, status, url
        )))
    }
}

// ============ 小工具 ============

/// 解析目标数据库 ID：优先 `--db`，否则账户默认。
fn resolve_db_id(account: &TodoAccount, flags: &HashMap<String, String>) -> Result<String> {
    flags
        .get("db")
        .cloned()
        .or_else(|| account.default_database_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(
                "no --db given and no default_database_id set for this todo account \
                 (run `everyday todo init-db` first)."
                    .into(),
            )
        })
}

/// 探测当前渲染模式（JSON 全局 flag 已注入 args 并被 parse_simple_args 捕获到 flags，
/// 但模式判定统一以进程参数中的 `--json` 为准，与 note 模块一致）。
fn mode_json() -> bool {
    std::env::args().any(|a| a == "--json")
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

/// 在 config 的 `todo.accounts` 中找到 name 匹配的账户，写入 `default_database_id`。
fn set_todo_database_id(root: &mut toml::Value, account_name: &str, db_id: &str) -> Result<()> {
    let table = root
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("config root is not a table".into()))?;
    let todo = table
        .get_mut("todo")
        .ok_or_else(|| AgentError::Config("no [todo] section in config".into()))?;
    let todo_table = todo
        .as_table_mut()
        .ok_or_else(|| AgentError::Config("todo is not a table".into()))?;
    let accounts = todo_table
        .get_mut("accounts")
        .ok_or_else(|| AgentError::Config("todo.accounts missing".into()))?;
    let arr = accounts
        .as_array_mut()
        .ok_or_else(|| AgentError::Config("todo.accounts is not an array".into()))?;

    let mut found = false;
    for acc in arr.iter_mut() {
        if acc.get("name").and_then(|n| n.as_str()) == Some(account_name) {
            acc.as_table_mut()
                .ok_or_else(|| AgentError::Config("todo account is not a table".into()))?
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
            "todo account '{account_name}' not found in config"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个 NotionPage 原始 JSON，验证 From 转换。
    /// init-db 创建的 Status 为 `select` 类型，这里用 select 键。
    #[test]
    fn notion_page_to_todo_item() {
        let page = json!({
            "id": "page_123",
            "properties": {
                "Task": { "title": [{ "plain_text": "写文档" }] },
                "Status": { "select": { "name": "In Progress" } },
                "Due": { "date": { "start": "2026-07-15" } },
                "Priority": { "select": { "name": "P0" } }
            }
        });
        let np: NotionPage = serde_json::from_value(page).unwrap();
        let item = TodoItem::from(np);
        assert_eq!(item.id, "page_123");
        assert_eq!(item.title, "写文档");
        assert_eq!(item.status, "In Progress");
        assert_eq!(item.due.as_deref(), Some("2026-07-15"));
        assert_eq!(item.priority.as_deref(), Some("P0"));
    }

    /// 兼容手动建成的 `status` 类型属性（新版 Notion 状态属性）。
    #[test]
    fn notion_page_status_property_fallback() {
        let page = json!({
            "id": "p_status",
            "properties": {
                "Task": { "title": [{ "plain_text": "状态属性任务" }] },
                "Status": { "status": { "name": "Done" } }
            }
        });
        let np: NotionPage = serde_json::from_value(page).unwrap();
        let item = TodoItem::from(np);
        assert_eq!(item.status, "Done");
    }

    /// 缺失 Due / Priority 时应为 None。
    #[test]
    fn notion_page_without_optional_fields() {
        let page = json!({
            "id": "p2",
            "properties": {
                "Task": { "title": [{ "plain_text": "裸任务" }] },
                "Status": { "select": { "name": "Todo" } }
            }
        });
        let np: NotionPage = serde_json::from_value(page).unwrap();
        let item = TodoItem::from(np);
        assert_eq!(item.title, "裸任务");
        assert!(item.due.is_none());
        assert!(item.priority.is_none());
    }

    #[test]
    fn cmp_due_asc_orders_correctly() {
        let a = TodoItem {
            id: "a".into(),
            title: "a".into(),
            status: "Todo".into(),
            due: Some("2026-07-10".into()),
            priority: None,
        };
        let b = TodoItem {
            id: "b".into(),
            title: "b".into(),
            status: "Todo".into(),
            due: Some("2026-07-12".into()),
            priority: None,
        };
        let none = TodoItem {
            id: "c".into(),
            title: "c".into(),
            status: "Todo".into(),
            due: None,
            priority: None,
        };
        // 早的排前
        assert_eq!(cmp_due_asc(&a, &b), std::cmp::Ordering::Less);
        // 有日期排在无日期之前
        assert_eq!(cmp_due_asc(&a, &none), std::cmp::Ordering::Less);
        assert_eq!(cmp_due_asc(&none, &b), std::cmp::Ordering::Greater);
        assert_eq!(cmp_due_asc(&none, &none), std::cmp::Ordering::Equal);
    }

    /// 空标题应回退为空串（不 panic）。
    #[test]
    fn empty_title_falls_back() {
        let page = json!({
            "id": "p",
            "properties": {
                "Task": { "title": [] },
                "Status": { "status": { "name": "Todo" } }
            }
        });
        let np: NotionPage = serde_json::from_value(page).unwrap();
        let item = TodoItem::from(np);
        assert_eq!(item.title, "");
    }

    /// set_todo_database_id 局部编辑，不破坏其它账户/段落。
    #[test]
    fn set_todo_database_id_edits_only_target() {
        let mut root: toml::Value = toml::from_str(
            r#"
[default_account]
todo = "work"

[[mail.accounts]]
name = "work"
imap_host = "imap.x.com"

[[todo.accounts]]
name = "personal"
parent_page_id = "page_p"

[[todo.accounts]]
name = "work"
parent_page_id = "page_w"
"#,
        )
        .unwrap();
        set_todo_database_id(&mut root, "work", "db_new").unwrap();

        // work 账户被写入 database_id。
        let accounts = root
            .get("todo")
            .unwrap()
            .get("accounts")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(accounts[0].get("default_database_id"), None); // personal 未动
        assert_eq!(
            accounts[1].get("default_database_id").unwrap().as_str(),
            Some("db_new")
        );
        // mail 段落完好。
        assert_eq!(
            root.get("mail")
                .unwrap()
                .get("accounts")
                .unwrap()
                .get(0)
                .unwrap()
                .get("imap_host")
                .unwrap()
                .as_str(),
            Some("imap.x.com")
        );
    }

    #[test]
    fn set_todo_database_id_missing_account_errors() {
        let mut root: toml::Value = toml::from_str("[[todo.accounts]]\nname = \"x\"\n").unwrap();
        assert!(set_todo_database_id(&mut root, "ghost", "db").is_err());
    }
}
