//! `todo` module: todo task management. Defaults to the local SQLite provider
//! (`local`), switchable to Notion (`provider = "notion"`)
//! [T001](../../../docs/adr/T001-notion-todo-module.md)
//! [F005](../../../docs/adr/F005-default-provider-local.md).
//!
//! Action dispatch is dependency-inverted: `execute` resolves the account, builds a
//! `Box<dyn TodoBackend>` via [`for_account`], calls the corresponding trait method, and
//! renders the returned domain struct ([R016](../../../docs/adr/R016-action-backend-di.md)
//! / [R018](../../../docs/adr/R018-backend-domain-mocks.md)). The module never names
//! `NotionClient`, never branches on provider, and never touches the keyring — all of that
//! lives in the `for_account` factory and the provider implementations.
//!
//! Commands (actions):
//! - `auth login` stores the Notion Integration Token in the system keyring (see `auth` module)
//! - `init-db`  creates the task database in Notion (needs `parent_page_id`) and writes
//!   `database_id` back into the config; for the local provider it just creates the table
//! - `list`     lists open tasks (by Due ascending); `--all` lists every task
//! - `add`      adds a task (`--title` required; `--due` / `--priority` optional)
//! - `start`    marks the task In Progress
//! - `complete` marks the task Done
//! - `delete`   archives the Notion page (soft delete) / physically deletes the local row

pub mod backend;
pub mod local;
pub mod notion;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::todo::backend::{
    STATUS_DONE, STATUS_IN_PROGRESS, TodoAdded, TodoBackend, TodoDeleted, TodoInitDb, TodoItem,
    TodoStatusSet, for_account,
};
use crate::modules::{Executor, parse_simple_args};
use crate::output::Output;

/// Detect the current render mode. The JSON global flag is decided uniformly by the
/// process-level `--json` flag [R001](../../../docs/adr/R001-thread-local-json-mode.md).
fn mode_json() -> bool {
    crate::util::json_mode::is_json()
}

// ============ module ============

pub struct TodoModule {
    config: Arc<Config>,
}

impl TodoModule {
    pub fn new(config: Arc<Config>) -> Self {
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
                description: "初始化待办数据库（local 建表 / Notion 建库）",
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

        // DI seam: the module never names `NotionClient`, never branches on provider,
        // never touches the keyring — all of that lives in `for_account`.
        let backend = for_account(&self.config, account)?;
        dispatch(&*backend, action, &flags, &positional).await
    }
}

/// Provider-agnostic action dispatch. Shared by `execute` (real backend) and the
/// `MockTodoBackend` acceptance tests ([R018](../../../docs/adr/R018-backend-domain-mocks.md)):
/// it maps an action + parsed args onto a `TodoBackend` trait call and renders the returned
/// domain struct. Because it takes `&dyn TodoBackend`, the test suite can inject an in-memory
/// mock and exercise the entire action path without a `NotionClient`, SQLite, or keyring.
pub(crate) async fn dispatch(
    backend: &dyn TodoBackend,
    action: &str,
    flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    match action {
        "init-db" => {
            let parent = flags.get("parent").map(|s| s.as_str());
            let r = backend.init_db(parent).await?;
            Ok(render_init_db(r))
        }
        "list" => {
            let all = flags.contains_key("all");
            let items = backend.list(all).await?;
            Ok(render_list(items))
        }
        "add" => {
            let title = flags.get("title").ok_or_else(|| {
                AgentError::InvalidArgument("add requires --title <title>".into())
            })?;
            let due = flags.get("due").map(|s| s.as_str());
            let priority = flags.get("priority").map(|s| s.as_str());
            let r = backend.add(title, due, priority).await?;
            Ok(render_add(r))
        }
        "start" => {
            let id = positional
                .first()
                .ok_or_else(|| AgentError::InvalidArgument("`start` requires <page_id>".into()))?;
            let r = backend.set_status(id, STATUS_IN_PROGRESS).await?;
            Ok(render_status(r))
        }
        "complete" => {
            let id = positional.first().ok_or_else(|| {
                AgentError::InvalidArgument("`complete` requires <page_id>".into())
            })?;
            let r = backend.set_status(id, STATUS_DONE).await?;
            Ok(render_status(r))
        }
        "delete" => {
            let id = positional
                .first()
                .ok_or_else(|| AgentError::InvalidArgument("`delete` requires <page_id>".into()))?;
            let r = backend.delete(id).await?;
            Ok(render_delete(r))
        }
        other => Err(AgentError::UnknownAction(format!("todo {other}"))),
    }
}

// ============ Rendering (R018) ============

/// Render `init-db` result. Text mode names the provider and prints the location
/// (local db path or Notion url); JSON mode emits all populated fields.
fn render_init_db(r: TodoInitDb) -> Output {
    if mode_json() {
        return Output::Json(json!({
            "account": r.account,
            "provider": r.provider,
            "db_path": r.db_path,
            "database_id": r.database_id,
            "url": r.url,
        }));
    }
    let location = r.db_path.or(r.url).unwrap_or_default();
    let verb = if r.provider == "local" {
        "initialized local"
    } else {
        "created"
    };
    Output::text(format!(
        "{verb} todo database for account '{}'\n{}",
        r.account, location
    ))
}

/// Render `list` result: a Records table (text) or a JSON array (both provider shapes identical).
fn render_list(items: Vec<TodoItem>) -> Output {
    if mode_json() {
        let arr: Vec<Value> = items
            .iter()
            .map(|it| serde_json::to_value(it).unwrap_or(Value::Null))
            .collect();
        Output::Json(Value::Array(arr))
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
        Output::records(
            vec![
                "id".into(),
                "title".into(),
                "status".into(),
                "due".into(),
                "priority".into(),
            ],
            rows,
        )
    }
}

/// Render `add` result.
fn render_add(r: TodoAdded) -> Output {
    if mode_json() {
        return Output::Json(json!({
            "id": r.id,
            "url": r.url,
            "title": r.title,
            "database_id": r.database_id,
        }));
    }
    match r.url {
        Some(url) => Output::text(format!("added todo '{}' (id={})\n{}", r.title, r.id, url)),
        None => Output::text(format!("added todo '{}' (id={})", r.title, r.id)),
    }
}

/// Render `start` / `complete` result.
fn render_status(r: TodoStatusSet) -> Output {
    if mode_json() {
        return Output::Json(json!({
            "id": r.id,
            "status": r.status,
            "url": r.url,
        }));
    }
    match r.url {
        Some(url) => Output::text(format!(
            "set todo {} -> status '{}'\n{}",
            r.id, r.status, url
        )),
        None => Output::text(format!("set todo {} -> status '{}'", r.id, r.status)),
    }
}

/// Render `delete` result.
fn render_delete(r: TodoDeleted) -> Output {
    if mode_json() {
        return Output::Json(json!({
            "id": r.id,
            "title": r.title,
            "status": r.status,
            "archived": r.archived,
        }));
    }
    Output::text(format!("deleted todo '{}' (id={})", r.title, r.id))
}

// ============ Tests ============

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::todo::backend::testkit::MockTodoBackend;
    use crate::util::json_mode;

    fn sample_item() -> TodoItem {
        TodoItem {
            id: "t1".into(),
            title: "写文档".into(),
            status: "Todo".into(),
            due: Some("2026-07-15".into()),
            priority: Some("P0".into()),
        }
    }

    /// (a) The full action path — parse → backend → render — runs end-to-end against a
    /// `MockTodoBackend` that holds no `NotionClient` and no SQLite. This proves the DI seam
    /// removes `NotionClient` / provider branches / keyring reads from the action layer.
    #[tokio::test]
    async fn dispatch_with_mock_runs_action_path_without_notion_client() {
        let backend = MockTodoBackend {
            items: vec![sample_item()],
            ..Default::default()
        };
        let mut flags = HashMap::new();
        flags.insert("all".into(), String::new());

        let out = dispatch(&backend, "list", &flags, &[]).await.unwrap();
        match out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["id", "title", "status", "due", "priority"]);
                assert_eq!(rows, vec![vec!["t1", "写文档", "Todo", "2026-07-15", "P0"]]);
            }
            other => panic!("expected Records, got {other:?}"),
        }
    }

    /// (b) The render layer is provider-agnostic: the same domain data renders identically
    /// whether it originated from Notion or the local backend. We render directly with
    /// MockTodoBackend-supplied data and assert both text (Records) and JSON shapes.
    #[test]
    fn render_is_provider_agnostic_for_same_domain_data() {
        let item = sample_item();

        // Text mode → table with stable columns (backend-independent).
        let text_out = render_list(vec![item.clone()]);
        match text_out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["id", "title", "status", "due", "priority"]);
                assert_eq!(rows[0][0], "t1");
                assert_eq!(rows[0][2], "Todo");
            }
            other => panic!("expected Records, got {other:?}"),
        }

        // JSON mode → object with the same keys, regardless of source backend.
        json_mode::set_json_mode(true);
        let json_out = render_list(vec![item]);
        json_mode::set_json_mode(false);
        if let Output::Json(Value::Array(arr)) = json_out {
            assert_eq!(arr.len(), 1);
            assert_eq!(arr[0]["id"], json!("t1"));
            assert_eq!(arr[0]["title"], json!("写文档"));
            assert_eq!(arr[0]["status"], json!("Todo"));
            assert_eq!(arr[0]["due"], json!("2026-07-15"));
        } else {
            panic!("expected JSON array, got {json_out:?}");
        }
    }
}
