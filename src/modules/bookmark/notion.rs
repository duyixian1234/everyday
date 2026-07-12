//! Notion provider for the `bookmark` module [B001](../../../docs/adr/B001-bookmark-dual-provider.md).
//!
//! `NotionBookmarkBackend` implements [`BookmarkBackend`] ([R016](../../../docs/adr/R016-action-backend-di.md)),
//! owning the shared `NotionClient` and the strongly-typed mapping between the clean domain
//! model [`BookmarkItem`] and Notion's raw `Properties`. The `init_db` method writes the
//! created `database_id` back to config — a side effect contained entirely within this
//! provider, so the module's dispatch layer stays provider-agnostic.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::{BookmarkAccount, Config};
use crate::error::{AgentError, Result};
use crate::modules::bookmark::backend::{
    BookmarkAdded, BookmarkBackend, BookmarkInitDb, BookmarkItem,
};
use crate::notion_client::NotionClient;

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

// ============ Backend ============

/// Notion implementation of [`BookmarkBackend`].
pub struct NotionBookmarkBackend {
    client: NotionClient,
    account: BookmarkAccount,
}

impl NotionBookmarkBackend {
    pub fn new(client: NotionClient, account: BookmarkAccount) -> Self {
        Self { client, account }
    }
}

#[async_trait]
impl BookmarkBackend for NotionBookmarkBackend {
    /// `init-db [--parent PAGE_ID]`: create the bookmark database in Notion and backfill database_id.
    async fn init_db(&self, parent: Option<&str>) -> Result<BookmarkInitDb> {
        let parent = parent
            .map(|p| p.to_string())
            .or_else(|| self.account.parent_page_id.clone())
            .ok_or_else(|| {
                AgentError::InvalidArgument(format!(
                    "bookmark account '{}' has no parent_page_id. Set it in config under \
                     [[bookmark.accounts]] (parent_page_id = \"...\") or pass --parent PAGE_ID.",
                    self.account.name
                ))
            })?;

        let client = &self.client;

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
        set_bookmark_database_id(&mut root, &self.account.name, &db_id)?;
        save_config_value(&root)?;

        Ok(BookmarkInitDb {
            account: self.account.name.clone(),
            provider: "notion",
            db_path: None,
            database_id: Some(db_id),
            url: Some(url),
        })
    }

    /// `add --url U --title T [--tags a,b] [--db ID]`: collect a bookmark.
    async fn add(
        &self,
        url: &str,
        title: &str,
        tags: &[String],
        db_id: Option<&str>,
    ) -> Result<BookmarkAdded> {
        let db_id = resolve_db_id(&self.account, db_id)?;
        let client = &self.client;

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

        // The domain `BookmarkAdded` carries the bookmark's own `url`; the Notion page URL
        // is intentionally not surfaced (kept parity with the local provider's shape).
        Ok(BookmarkAdded {
            id,
            url: url.to_string(),
            title: title.to_string(),
            tags: tags.to_vec(),
            database_id: Some(db_id),
        })
    }

    /// `list [--tag TAG] [--db ID]`: list bookmarks, optionally filtered by tag.
    async fn list(&self, tag: Option<&str>, db_id: Option<&str>) -> Result<Vec<BookmarkItem>> {
        let db_id = resolve_db_id(&self.account, db_id)?;
        let client = &self.client;

        let mut body = json!({});
        if let Some(tag) = tag {
            body["filter"] = json!({
                "property": "Tags",
                "multi_select": { "contains": tag }
            });
        }

        let resp: QueryResponse = client
            .post(&format!("/databases/{db_id}/query"), &body)
            .await?;

        let mut items: Vec<BookmarkItem> =
            resp.results.into_iter().map(BookmarkItem::from).collect();
        // Sort by creation time descending (Notion defaults to ascending); fall back to id when created_time is absent.
        items.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(items)
    }
}

// ============ Helpers ============

/// Resolve the target database ID: prefer `--db`, otherwise the account default.
fn resolve_db_id(account: &BookmarkAccount, db_id: Option<&str>) -> Result<String> {
    db_id
        .map(|s| s.to_string())
        .or_else(|| account.default_database_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(
                "no --db given and no default_database_id set for this bookmark account \
                 (run `everyday bookmark init-db` first)."
                    .into(),
            )
        })
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

/// Find the account with a matching name under config's `bookmark.accounts` and write
/// `default_database_id`. See `crate::modules::local::set_module_database_id` - shared with todo
/// [R009](../../../docs/adr/R009-notion-common-local-module.md).
fn set_bookmark_database_id(root: &mut toml::Value, account_name: &str, db_id: &str) -> Result<()> {
    crate::modules::local::set_module_database_id(root, "bookmark", account_name, db_id)
}

#[cfg(test)]
mod tests {
    use super::*;

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
