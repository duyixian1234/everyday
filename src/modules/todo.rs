//! `todo` module: todo task management. Defaults to the local SQLite provider
//! (`local`), switchable to Notion (`provider = "notion"`)
//! [T001](../../docs/adr/T001-notion-todo-module.md)
//! [F005](../../docs/adr/F005-default-provider-local.md).
//!
//! The Notion branch is the upper layer of a two-layer architecture: it owns a shared
//! [`NotionClient`] and strongly maps the clean domain model [`TodoItem`] back and forth
//! against Notion's raw `Properties`, hiding Notion's property nesting
//! [F004](../../docs/adr/F004-shared-notion-client.md).
//!
//! Commands (actions):
//! - `auth login` stores the Notion Integration Token in the system keyring (see `auth` module)
//! - `init-db`  creates the task database in Notion (needs `parent_page_id`) and writes
//!   `database_id` back into the config
//! - `list`     lists open tasks (by Due ascending); `--all` lists every task
//! - `add`      adds a task (`--title` required; `--due` / `--priority` optional)
//! - `start`    marks the task In Progress
//! - `complete` marks the task Done
//! - `delete`   archives the Notion page (soft delete)
//!
//! Credential safety: the token lives only in the system keyring
//! (service = `everyday/todo/<account>`) [F002](../../docs/adr/F002-multi-account-keyring.md)
//! and never lands on disk in the config. Non-secret metadata such as
//! `database_id` / `parent_page_id` may live in the config.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Config, TodoAccount};
use crate::error::{AgentError, Result};
use crate::modules::{Executor, parse_simple_args};
use crate::notion_client::NotionClient;
use crate::output::Output;

/// Status option names (must match the schema created by `init-db`).
const STATUS_TODO: &str = "Todo";
const STATUS_IN_PROGRESS: &str = "In Progress";
const STATUS_DONE: &str = "Done";

/// Clean domain model (output to the Agent / terminal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub due: Option<String>,
    pub priority: Option<String>,
}

// ============ Notion raw data structures (strongly typed mapping) ============

/// Mirrors the raw Notion Page response (only the fields we care about; unknown fields are ignored by serde).
#[derive(Debug, Deserialize)]
struct NotionPage {
    id: String,
    properties: TodoProperties,
}

/// Task-database property set (field names match Notion property names via `rename`).
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

/// Nested type leaf nodes.
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
    /// Notion has two status property kinds: `select` (the kind created by `init-db`)
    /// and `status` (Notion's newer status property). API responses carry them under
    /// `select` / `status` keys respectively; we accept both and read whichever is
    /// present, staying compatible with the `select` databases created by `init-db`
    /// while also supporting hand-built `status`-type databases.
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

/// Response of `POST /databases/{id}/query` (only `results`).
#[derive(Debug, Deserialize)]
struct QueryResponse {
    results: Vec<NotionPage>,
}

// ============ two-way converter ============

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

// ============ module ============

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
    fn description(&self) -> &'static str {
        "Todo tasks (Notion or local sqlite): init-db, list, add, start, complete, delete."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "init-db",
                description: "在 Notion 初始化待办数据库",
                usage: "everyday todo init-db [--parent PAGE_ID] [--account NAME]",
                args: &[ArgSpec {
                    name: "parent",
                    help: "父页面 ID（默认账户父页）",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "list",
                description: "列出待办",
                usage: "everyday todo list [--db ID] [--all] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "db",
                        help: "数据库 ID",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "all",
                        help: "列出全部（默认仅未完成）",
                        kind: ArgKind::Bool,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "add",
                description: "新增待办",
                usage: "everyday todo add --title T [--due DATE] [--priority P] [--db ID] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "title",
                        help: "标题",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "due",
                        help: "截止日期（如 2026-07-15）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "priority",
                        help: "优先级（如 P0/P1/P2）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "db",
                        help: "数据库 ID",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "start",
                description: "标记进行中",
                usage: "everyday todo start <page_id> [--account NAME]",
                args: &[],
                positional: Positional::Exactly(1),
            },
            ActionArgSpec {
                name: "complete",
                description: "标记完成",
                usage: "everyday todo complete <page_id> [--account NAME]",
                args: &[],
                positional: Positional::Exactly(1),
            },
            ActionArgSpec {
                name: "delete",
                description: "删除待办",
                usage: "everyday todo delete <page_id> [--account NAME]",
                args: &[],
                positional: Positional::Exactly(1),
            },
        ];
        ModuleArgSpec {
            name: "todo",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let account = self
            .config
            .todo_account(flags.get("account").map(|s| s.as_str()))?;

        // Local SQLite provider: routed to the local impl; otherwise go through Notion.
        if crate::modules::local::is_local_provider(&account.provider) {
            use crate::modules::todo_local as local;
            return match action {
                "init-db" => local::init_db(account).await,
                "list" => local::list(account, &flags).await,
                "add" => local::add(account, &flags).await,
                "start" => {
                    local::set_status(account, positional.first(), local::STATUS_IN_PROGRESS).await
                }
                "complete" => {
                    local::set_status(account, positional.first(), local::STATUS_DONE).await
                }
                "delete" => local::delete(account, positional.first()).await,
                other => Err(AgentError::UnknownAction(format!("todo {other}"))),
            };
        }

        match action {
            "init-db" => todo_init_db(&self.config, account, &flags).await,
            "list" => todo_list(&self.config, account, &flags).await,
            "add" => todo_add(&self.config, account, &flags).await,
            "start" => {
                todo_set_status(
                    &self.config,
                    account,
                    positional.first(),
                    STATUS_IN_PROGRESS,
                )
                .await
            }
            "complete" => {
                todo_set_status(&self.config, account, positional.first(), STATUS_DONE).await
            }
            "delete" => todo_delete(&self.config, account, positional.first()).await,
            other => Err(AgentError::UnknownAction(format!("todo {other}"))),
        }
    }
}

// ============ credentials (keyring) ============

/// Read the Notion token from the OS keyring via the consolidated `auth` module
/// ([R013](../../docs/adr/R013-auth-module-consolidation.md)).
fn get_token(config: &Config, account: &TodoAccount) -> Result<String> {
    crate::modules::auth::get_credential(config, "todo", &account.name)
}

// ============ init-db ============

/// `todo init-db [--parent PAGE_ID]`: create the task database in Notion and write database_id back.
async fn todo_init_db(
    config: &Config,
    account: &TodoAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    // Parent page: prefer --parent, otherwise fall back to the account's parent_page_id.
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

    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    // Create the database: Task(title) / Status(select) / Due(date) / Priority(select).
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

    // Write back to config: only update the matching account's default_database_id under
    // todo.accounts, leave the rest untouched (local edit on toml::Value to preserve user
    // formatting and comments).
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

/// `todo list [--db ID] [--all]`: list tasks.
async fn todo_list(
    config: &Config,
    account: &TodoAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let db_id = resolve_db_id(account, flags)?;
    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    // Design: filter out completed tasks, sort by Due ascending. --all disables the filter.
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

    // Client-side fallback: ensure only open tasks are listed (in case the API filter failed)
    // and sort by due ascending with nulls last.
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

/// Sort by due ascending: dated < undated; same-dated entries are compared as strings
/// (ISO 8601 sorts lexicographically).
fn cmp_due_asc(a: &TodoItem, b: &TodoItem) -> std::cmp::Ordering {
    match (&a.due, &b.due) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

// ============ add ============

/// `todo add --title T [--due DATE] [--priority P] [--db ID]`: add a task.
async fn todo_add(
    config: &Config,
    account: &TodoAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("add requires --title <title>".into()))?;
    let db_id = resolve_db_id(account, flags)?;
    let token = get_token(config, account)?;
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

/// `todo start/complete <page_id>`: update the task status.
async fn todo_set_status(
    config: &Config,
    account: &TodoAccount,
    page_id: Option<&String>,
    status: &str,
) -> Result<Output> {
    let page_id = page_id
        .ok_or_else(|| AgentError::InvalidArgument(format!("`{status}` requires <page_id>")))?;
    let token = get_token(config, account)?;
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

// ============ delete ============

/// `todo delete <page_id>` (Notion): archive the page.
///
/// Notion has no real delete API; setting `archived: true` hides it from the UI and
/// soft-deletes it [T002](../../docs/adr/T002-todo-delete-action.md). Same error path
/// as `start`/`complete` (`InvalidArgument` for missing id).
///
/// GET the title before archiving so the ops-log + timeline delete events carry the
/// title instead of being blank. One extra API call in exchange for readable audit
/// trails; the delete path is not a performance hot spot.
async fn todo_delete(
    config: &Config,
    account: &TodoAccount,
    page_id: Option<&String>,
) -> Result<Output> {
    let page_id =
        page_id.ok_or_else(|| AgentError::InvalidArgument("`delete` requires <page_id>".into()))?;
    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    // 1. Read the title first (so ops-log stays meaningful).
    let page: Value = client.get(&format!("/pages/{page_id}")).await?;
    let title = page
        .get("properties")
        .and_then(|props| props.get("Task"))
        .and_then(|t| t.get("title"))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.first())
        .and_then(|t| t.get("plain_text"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    // Empty title is not an error (it may have been cleared externally); emit a
    // placeholder so timeline rendering never produces blank rows.
    let title = if title.is_empty() {
        format!("(untitled) {page_id}")
    } else {
        title
    };

    // 2. Archive (soft delete).
    let body = json!({ "archived": true });
    let _: Value = client.patch(&format!("/pages/{page_id}"), &body).await?;

    let json_out = json!({ "id": page_id, "title": title, "status": "deleted", "archived": true });
    if mode_json() {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "deleted todo '{title}' (id={page_id})"
        )))
    }
}

// ============ helpers ============

/// Resolve the target database ID: prefer `--db`, otherwise the account default.
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

/// Detect the current render mode. The JSON global flag is already injected into args
/// and captured by `parse_simple_args` into `flags`, but the mode is decided uniformly
/// by the process-level `--json` flag, matching the `note` module
/// [R001](../../docs/adr/R001-thread-local-json-mode.md).
fn mode_json() -> bool {
    crate::util::json_mode::is_json()
}

/// Load the config file into a `toml::Value` (missing/empty becomes an empty table).
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

/// Write a `toml::Value` back to the config file (creating the parent dir as needed).
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

/// Find the account matching `name` under `todo.accounts` in config and write
/// `default_database_id`. See `crate::modules::local::set_module_database_id` -
/// shared with bookmark [R009](../../docs/adr/R009-notion-common-local-module.md).
fn set_todo_database_id(root: &mut toml::Value, account_name: &str, db_id: &str) -> Result<()> {
    crate::modules::local::set_module_database_id(root, "todo", account_name, db_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a raw NotionPage JSON to verify the `From` conversion.
    /// `init-db` creates the Status property as `select`, hence the `select` key here.
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

    /// Stay compatible with hand-built `status` properties (Notion's newer status property).
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

    /// Missing Due / Priority should yield None.
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
        // Earlier first
        assert_eq!(cmp_due_asc(&a, &b), std::cmp::Ordering::Less);
        // Dated before undated
        assert_eq!(cmp_due_asc(&a, &none), std::cmp::Ordering::Less);
        assert_eq!(cmp_due_asc(&none, &b), std::cmp::Ordering::Greater);
        assert_eq!(cmp_due_asc(&none, &none), std::cmp::Ordering::Equal);
    }

    /// An empty title should fall back to an empty string (no panic).
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

    // The full test for `set_todo_database_id` lives in local.rs (the authoritative regression for the shared helper).
    #[test]
    fn set_todo_database_id_is_shared_helper() {
        let mut root: toml::Value = toml::from_str(
            r#"
[[todo.accounts]]
name = "x"
"#,
        )
        .unwrap();
        assert!(set_todo_database_id(&mut root, "ghost", "db").is_err());
    }
}
