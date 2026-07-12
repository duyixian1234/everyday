//! Bookmark module: save / browse web bookmarks. Defaults to the local SQLite provider (`local`),
//! but can switch to Notion (`provider = "notion"`) [B001](../../../docs/adr/B001-bookmark-dual-provider.md).
//!
//! Action dispatch is dependency-inverted: `execute` resolves the account, builds a
//! `Box<dyn BookmarkBackend>` via [`for_account`], calls the corresponding trait method, and
//! renders the returned domain struct ([R016](../../../docs/adr/R016-action-backend-di.md)
//! / [R018](../../../docs/adr/R018-backend-domain-mocks.md)). The module never names
//! `NotionClient`, never branches on provider, and never touches the keyring — all of that
//! lives in the `for_account` factory and the provider implementations.
//!
//! Supported `action`s:
//! - `auth login` stores the Notion Integration Token in the keyring (notion provider only)
//! - `init-db`  create the local table / create the bookmark database in Notion (needs `parent_page_id`)
//! - `add`      collect a bookmark (`--url` required, `--title` required, `--tags` optional comma-separated)
//! - `list`     list bookmarks, `--tag <TAG>` filters by tag (`--db` selects the Notion database)

pub mod backend;
pub mod local;
pub mod notion;

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{AgentError, Result};
use crate::modules::bookmark::backend::{
    BookmarkAdded, BookmarkBackend, BookmarkInitDb, BookmarkItem, for_account,
};
use crate::modules::local::parse_tags;
use crate::modules::{Executor, parse_simple_args};
use crate::output::Output;

/// Detect the current render mode. The JSON global flag is decided uniformly by the
/// process-level `--json` flag [R001](../../../docs/adr/R001-thread-local-json-mode.md).
fn mode_json() -> bool {
    crate::util::json_mode::is_json()
}

// ============ module ============

pub struct BookmarkModule {
    config: std::sync::Arc<Config>,
}

impl BookmarkModule {
    pub fn new(config: std::sync::Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for BookmarkModule {
    fn description(&self) -> &'static str {
        "Bookmarks (Notion or local sqlite): init-db, add, list."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "init-db",
                description: "初始化书签数据库（local 建表 / Notion 建库）",
                usage: "everyday bookmark init-db [--parent PAGE_ID] [--account NAME]",
                args: &[ArgSpec {
                    name: "parent",
                    help: "父页面 ID（默认账户父页）",
                    kind: ArgKind::Value,
                }],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "add",
                description: "新增书签",
                usage: "everyday bookmark add --url U --title T [--tags a,b] [--db ID] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "url",
                        help: "书签 URL",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "title",
                        help: "标题",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "tags",
                        help: "标签，逗号分隔（如 rust,cli）",
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
                name: "list",
                description: "列出书签",
                usage: "everyday bookmark list [--tag TAG] [--db ID] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "tag",
                        help: "按标签精确过滤",
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
        ];
        ModuleArgSpec {
            name: "bookmark",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, _positional) = parse_simple_args(args);
        let account = self
            .config
            .bookmark_account(flags.get("account").map(|s| s.as_str()))?;

        // DI seam: the module never names `NotionClient`, never branches on provider,
        // never touches the keyring — all of that lives in `for_account`.
        let backend = for_account(&self.config, account)?;
        dispatch(&*backend, action, &flags).await
    }
}

/// Provider-agnostic action dispatch. Shared by `execute` (real backend) and the
/// `MockBookmarkBackend` acceptance tests ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).
pub(crate) async fn dispatch(
    backend: &dyn BookmarkBackend,
    action: &str,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    match action {
        "init-db" => {
            let parent = flags.get("parent").map(|s| s.as_str());
            let r = backend.init_db(parent).await?;
            Ok(render_init_db(r))
        }
        "add" => {
            let url = flags
                .get("url")
                .ok_or_else(|| AgentError::InvalidArgument("add requires --url <url>".into()))?;
            let title = flags.get("title").ok_or_else(|| {
                AgentError::InvalidArgument("add requires --title <title>".into())
            })?;
            let tags = parse_tags(flags.get("tags"));
            let db_id = flags.get("db").map(|s| s.as_str());
            let r = backend.add(url, title, &tags, db_id).await?;
            Ok(render_add(r))
        }
        "list" => {
            let tag = flags.get("tag").map(|s| s.as_str());
            let db_id = flags.get("db").map(|s| s.as_str());
            let items = backend.list(tag, db_id).await?;
            Ok(render_list(items))
        }
        other => Err(AgentError::UnknownAction(format!("bookmark {other}"))),
    }
}

// ============ Rendering (R018) ============

/// Render `init-db` result. Text mode names the provider and prints the location
/// (local db path or Notion url); JSON mode emits all populated fields.
fn render_init_db(r: BookmarkInitDb) -> Output {
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
        "{verb} bookmark database for account '{}'\n{}",
        r.account, location
    ))
}

/// Render `add` result.
fn render_add(r: BookmarkAdded) -> Output {
    if mode_json() {
        return Output::Json(json!({
            "id": r.id,
            "url": r.url,
            "title": r.title,
            "tags": r.tags,
            "database_id": r.database_id,
        }));
    }
    Output::text(format!(
        "added bookmark '{}' (id={})\n{}",
        r.title, r.id, r.url
    ))
}

/// Render `list` result: a Records table (text) or a JSON array (both provider shapes identical).
fn render_list(items: Vec<BookmarkItem>) -> Output {
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
                    it.url.clone(),
                    it.tags.join(", "),
                ]
            })
            .collect();
        Output::records(
            vec!["id".into(), "title".into(), "url".into(), "tags".into()],
            rows,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_splits_and_trims() {
        // The shared helper's regression tests live in local.rs; here we only verify the alias path also calls it.
        assert_eq!(parse_tags(None), Vec::<String>::new());
        assert_eq!(
            parse_tags(Some(&"rust, cli ,  web ".to_string())),
            vec!["rust", "cli", "web"]
        );
    }
}
