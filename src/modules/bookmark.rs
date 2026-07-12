//! Bookmark module: save / browse web bookmarks. Defaults to the local SQLite provider (`local`),
//! but can switch to Notion (`provider = "notion"`) [B001](../../docs/adr/B001-bookmark-dual-provider.md).
//!
//! Design goal: a lightweight "read-later / favorites" store where each bookmark has a URL, a title,
//! and a set of tags. Exposes three high-level actions to the Agent: `init-db` (create DB),
//! `add` (collect), `list` (browse filtered by tag).
//!
//! Supported `action`s:
//! - `auth login` stores the Notion Integration Token in the keyring (notion provider only)
//! - `init-db`  create the local table / create the bookmark database in Notion (needs `parent_page_id`)
//! - `add`      collect a bookmark (`--url` required, `--title` required, `--tags` optional comma-separated)
//! - `list`     list bookmarks, `--tag <TAG>` filters by tag (`--db` selects the Notion database)
//!
//! Credential safety: the token is stored only in the system keyring (service = `everyday/bookmark/<account>`),
//! never persisted to config [F002](../../docs/adr/F002-multi-account-keyring.md). Non-secret metadata such
//! as `database_id` / `parent_page_id` may be stored in config.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{BookmarkAccount, Config};
use crate::error::{AgentError, Result};
use crate::modules::{Executor, parse_simple_args};
use crate::notion_client::NotionClient;
use crate::output::Output;

/// Clean domain model (output to Agent / terminal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkItem {
    pub id: String,
    pub url: String,
    pub title: String,
    pub tags: Vec<String>,
}

// ============ Notion raw data structures (strongly-typed mapping) ============

/// Mirrors the raw Notion Page response (only the fields we care about; unknown fields ignored by serde).
#[derive(Debug, Deserialize)]
struct NotionPage {
    id: String,
    properties: BookmarkProperties,
}

/// Property set of the bookmark database (field names are Notion property names, aligned via `rename`).
#[derive(Debug, Deserialize)]
struct BookmarkProperties {
    #[serde(rename = "Title")]
    title: TitleProperty,
    #[serde(rename = "URL")]
    url: Option<UrlProperty>,
    #[serde(rename = "Tags")]
    tags: Option<TagsProperty>,
}

/// Leaf node of a nested type.
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

/// Response of `POST /databases/{id}/query` (only `results` is taken).
#[derive(Debug, Deserialize)]
struct QueryResponse {
    results: Vec<NotionPage>,
}

// ============ Bidirectional converters ============

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

// ============ Module ============

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
    fn description(&self) -> &'static str {
        "Bookmarks (Notion or local sqlite): init-db, add, list."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "init-db",
                description: "在 Notion 初始化书签数据库",
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

        // Local SQLite provider: route to the local implementation; otherwise go through Notion.
        if crate::modules::local::is_local_provider(&account.provider) {
            use crate::modules::bookmark_local as local;
            return match action {
                "init-db" => local::init_db(account).await,
                "add" => local::add(account, &flags).await,
                "list" => local::list(account, &flags).await,
                other => Err(AgentError::UnknownAction(format!("bookmark {other}"))),
            };
        }

        match action {
            "init-db" => bookmark_init_db(&self.config, account, &flags).await,
            "add" => bookmark_add(&self.config, account, &flags).await,
            "list" => bookmark_list(&self.config, account, &flags).await,
            other => Err(AgentError::UnknownAction(format!("bookmark {other}"))),
        }
    }
}

// ============ Tag parsing ============

// See `crate::modules::local::parse_tags` — both bookmark providers share this implementation [R009](../../docs/adr/R009-notion-common-local-module.md).

// ============ Credentials (keyring) ============

/// Read the Notion token from the OS keyring via the consolidated `auth` module
/// ([R013](../../docs/adr/R013-auth-module-consolidation.md)).
fn get_token(config: &Config, account: &BookmarkAccount) -> Result<String> {
    crate::modules::auth::get_credential(config, "bookmark", &account.name)
}

// ============ init-db (notion) ============

/// `bookmark init-db [--parent PAGE_ID]`: create the bookmark database in Notion and backfill database_id.
async fn bookmark_init_db(
    config: &Config,
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

    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    // Create the database: Title (title) / URL (url) / Tags (multi_select).
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

    // Write back to config: only update the matching account's default_database_id under bookmark.accounts.
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

/// `bookmark add --url U --title T [--tags a,b] [--db ID]`: collect a bookmark.
async fn bookmark_add(
    config: &Config,
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
    let token = get_token(config, account)?;
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

/// `bookmark list [--tag TAG] [--db ID]`: list bookmarks, optionally filtered by tag.
async fn bookmark_list(
    config: &Config,
    account: &BookmarkAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let db_id = resolve_db_id(account, flags)?;
    let token = get_token(config, account)?;
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
    // Sort by creation time descending (Notion defaults to ascending); fall back to id when created_time is absent.
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

/// Text-mode table rendering (shared by `list`).
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

// ============ Helpers ============

/// Resolve the target database ID: prefer `--db`, otherwise the account default.
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

/// Detect the current render mode. The JSON global flag is injected into args and captured by
/// parse_simple_args into flags, but mode detection uniformly uses `--json` from the process args [R001](../../docs/adr/R001-thread-local-json-mode.md).
fn mode_json() -> bool {
    crate::util::json_mode::is_json()
}

/// Read the config file into a toml::Value (empty table if absent/empty).
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

/// Write a toml::Value back to the config file (creating the parent dir).
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

/// Find the account with a matching name under config's `bookmark.accounts` and write `default_database_id`.
/// See `crate::modules::local::set_module_database_id` — shared with todo [R009](../../docs/adr/R009-notion-common-local-module.md).
fn set_bookmark_database_id(root: &mut toml::Value, account_name: &str, db_id: &str) -> Result<()> {
    crate::modules::local::set_module_database_id(root, "bookmark", account_name, db_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_splits_and_trims() {
        // The shared helper's regression tests live in local.rs; here we only verify the alias path also calls it.
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

    // The full test for set_bookmark_database_id is in local.rs (authoritative regression for the shared helper).
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
