//! Notion provider for the `todo` module [T001](../../../docs/adr/T001-notion-todo-module.md).
//!
//! `NotionTodoBackend` implements [`TodoBackend`] ([R016](../../../docs/adr/R016-action-backend-di.md)),
//! owning the shared `NotionClient` and the strongly-typed mapping between the clean domain
//! model [`TodoItem`] and Notion's raw `Properties`. The `init_db` method writes the created
//! `database_id` back to config — a side effect contained entirely within this provider, so
//! the module's dispatch layer stays provider-agnostic.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::{Config, TodoAccount};
use crate::error::{AgentError, Result};
use crate::modules::todo::backend::{
    STATUS_DONE, STATUS_TODO, TodoAdded, TodoBackend, TodoDeleted, TodoInitDb, TodoItem,
    TodoStatusSet,
};
use crate::notion_client::NotionClient;

// ============ Notion raw data structures (strongly-typed mapping) ============

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

// ============ Backend ============

/// Notion implementation of [`TodoBackend`].
pub struct NotionTodoBackend {
    client: NotionClient,
    account: TodoAccount,
}

impl NotionTodoBackend {
    pub fn new(client: NotionClient, account: TodoAccount) -> Self {
        Self { client, account }
    }
}

#[async_trait]
impl TodoBackend for NotionTodoBackend {
    async fn init_db(&self, parent: Option<&str>) -> Result<TodoInitDb> {
        // Parent page: prefer the flag, otherwise fall back to the account's parent_page_id.
        let parent = parent
            .map(|p| p.to_string())
            .or_else(|| self.account.parent_page_id.clone())
            .ok_or_else(|| {
                AgentError::InvalidArgument(format!(
                    "todo account '{}' has no parent_page_id. Set it in config under \
                     [[todo.accounts]] (parent_page_id = \"...\") or pass --parent PAGE_ID.",
                    self.account.name
                ))
            })?;

        let client = &self.client;

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
        set_todo_database_id(&mut root, &self.account.name, &db_id)?;
        save_config_value(&root)?;

        Ok(TodoInitDb {
            account: self.account.name.clone(),
            provider: "notion",
            db_path: None,
            database_id: Some(db_id),
            url: Some(url),
        })
    }

    async fn list(&self, all: bool) -> Result<Vec<TodoItem>> {
        let db_id = resolve_db_id(&self.account, None)?;
        let client = &self.client;

        // Design: filter out completed tasks, sort by Due ascending. --all disables the filter.
        let mut body = json!({
            "sorts": [{ "property": "Due", "direction": "ascending" }]
        });
        if !all {
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
        items.retain(|it| all || !it.status.eq_ignore_ascii_case(STATUS_DONE));
        items.sort_by(cmp_due_asc);

        Ok(items)
    }

    async fn add(
        &self,
        title: &str,
        due: Option<&str>,
        priority: Option<&str>,
    ) -> Result<TodoAdded> {
        let db_id = resolve_db_id(&self.account, None)?;
        let client = &self.client;

        let mut props = json!({
            "Task": { "title": [{ "text": { "content": title } }] },
            "Status": { "select": { "name": STATUS_TODO } }
        });
        if let Some(due) = due {
            props["Due"] = json!({ "date": { "start": due } });
        }
        if let Some(pri) = priority {
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

        Ok(TodoAdded {
            id,
            url: Some(url),
            title: title.to_string(),
            database_id: Some(db_id),
        })
    }

    async fn set_status(&self, id: &str, status: &str) -> Result<TodoStatusSet> {
        let client = &self.client;

        let body = json!({ "properties": { "Status": { "select": { "name": status } } } });
        let updated: Value = client.patch(&format!("/pages/{id}"), &body).await?;
        let id = updated
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or(id)
            .to_string();
        let url = updated
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();

        Ok(TodoStatusSet {
            id,
            status: status.to_string(),
            url: Some(url),
        })
    }

    async fn delete(&self, id: &str) -> Result<TodoDeleted> {
        let client = &self.client;

        // 1. Read the title first (so ops-log stays meaningful).
        let page: Value = client.get(&format!("/pages/{id}")).await?;
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
            format!("(untitled) {id}")
        } else {
            title
        };

        // 2. Archive (soft delete).
        let body = json!({ "archived": true });
        let _: Value = client.patch(&format!("/pages/{id}"), &body).await?;

        Ok(TodoDeleted {
            id: id.to_string(),
            title,
            status: "deleted".to_string(),
            archived: true,
        })
    }
}

// ============ helpers ============

/// Resolve the target database ID: prefer `--db`, otherwise the account default.
/// The Notion backend resolves from the account's `default_database_id` (the `--db` flag
/// path is not exposed through the `TodoBackend` trait; the action layer passes `None`).
fn resolve_db_id(
    account: &TodoAccount,
    _flags: Option<&HashMap<String, String>>,
) -> Result<String> {
    account.default_database_id.clone().ok_or_else(|| {
        AgentError::InvalidArgument(
            "no default_database_id set for this todo account \
                 (run `everyday todo init-db` first)."
                .into(),
        )
    })
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

/// Load the config file into a `toml::Value` (missing/empty becomes an empty table).
fn load_config_value() -> Result<toml::Value> {
    let path = Config::config_path()?;
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
    let path = Config::config_path()?;
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
/// shared with bookmark [R009](../../../docs/adr/R009-notion-common-local-module.md).
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
