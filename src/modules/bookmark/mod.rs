//! Bookmark module: save / browse web bookmarks, local SQLite only
//! (Notion provider removed in v0.13.0 — [R019](../../../docs/adr/R019-remove-notion-provider.md)).
//!
//! Action dispatch is dependency-inverted: `execute` resolves the account, builds a
//! `Box<dyn BookmarkBackend>` via [`for_account`], calls the corresponding trait method, and
//! renders the returned domain struct ([R016](../../../docs/adr/R016-action-backend-di.md)
//! / [R018](../../../docs/adr/R018-backend-domain-mocks.md)).
//!
//! Supported `action`s:
//! - `add`      collect a bookmark (`--url` required, `--title` required, `--tags` optional comma-separated)
//! - `list`     list bookmarks, `--tag <TAG>` filters by tag
//! - `search`   GLOB-search title / url / tags (`<query>` required, `--limit` optional)

pub mod backend;
pub mod local;

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::BookmarkModuleConfig;
use crate::error::{AgentError, Result};
use crate::modules::bookmark::backend::{
    BookmarkAdded, BookmarkBackend, BookmarkItem, for_account,
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
    config: BookmarkModuleConfig,
}

impl BookmarkModule {
    pub fn new(config: BookmarkModuleConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for BookmarkModule {
    fn description(&self) -> &'static str {
        "Bookmarks (local sqlite): add, list."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!(
                "add",
                "新增书签",
                "everyday bookmark add --url U --title T [--tags a,b] [--account NAME]",
                &[
                    flag!("url", "书签 URL"),
                    flag!("title", "标题"),
                    flag!("tags", "标签，逗号分隔（如 rust,cli）"),
                ]
            ),
            cli_action!(
                "list",
                "列出书签",
                "everyday bookmark list [--tag TAG] [--account NAME]",
                &[flag!("tag", "按标签精确过滤"),]
            ),
            cli_action!(
                "search",
                "搜索书签",
                "everyday bookmark search <query> [--limit N] [--account NAME]",
                &[flag!("limit", "最多返回条数（默认 50）"),],
                Positional::OneOrMore
            ),
        ];
        ModuleArgSpec {
            name: "bookmark",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(
        &self,
        action: &str,
        args: &[String],
        _ctx: &crate::shared::request_context::RequestContext,
    ) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let account = self
            .config
            .resolve_account(flags.get("account").map(|s| s.as_str()))?;

        // DI seam: the module never branches on provider or touches the keyring —
        // all of that lives in `for_account`.
        let backend = for_account(account)?;
        dispatch(&*backend, action, &flags, &positional).await
    }

    /// P3 health: local-only since v0.13.0 — no credentials to check.
    async fn health_check(&self) -> Result<crate::modules::HealthStatus> {
        Ok(crate::modules::HealthStatus::healthy())
    }
}

/// Provider-agnostic action dispatch. Shared by `execute` (real backend) and the
/// `MockBookmarkBackend` acceptance tests ([R018](../../../docs/adr/R018-backend-domain-mocks.md)).
pub(crate) async fn dispatch(
    backend: &dyn BookmarkBackend,
    action: &str,
    flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    match action {
        "add" => {
            let url = flags
                .get("url")
                .ok_or_else(|| AgentError::InvalidArgument("add requires --url <url>".into()))?;
            let title = flags.get("title").ok_or_else(|| {
                AgentError::InvalidArgument("add requires --title <title>".into())
            })?;
            let tags = parse_tags(flags.get("tags"));
            let r = backend.add(url, title, &tags).await?;
            Ok(render_add(r))
        }
        "list" => {
            let tag = flags.get("tag").map(|s| s.as_str());
            let items = backend.list(tag).await?;
            Ok(render_list(items))
        }
        "search" => {
            let query = positional
                .first()
                .map(|s| s.as_str())
                .ok_or_else(|| AgentError::InvalidArgument("search requires <query>".into()))?;
            let limit = match flags.get("limit") {
                Some(raw) => Some(raw.parse::<usize>().map_err(|_| {
                    AgentError::InvalidArgument(format!("--limit must be a number, got '{raw}'"))
                })?),
                None => None,
            };
            let items = backend.search(query, limit).await?;
            Ok(render_list(items))
        }
        other => Err(AgentError::UnknownAction(format!("bookmark {other}"))),
    }
}

// ============ Rendering (R018) ============

/// Render `add` result.
fn render_add(r: BookmarkAdded) -> Output {
    if mode_json() {
        return Output::Json(json!({
            "id": r.id,
            "url": r.url,
            "title": r.title,
            "tags": r.tags,
        }));
    }
    Output::text(format!(
        "added bookmark '{}' (id={})\n{}",
        r.title, r.id, r.url
    ))
}

/// Render `list` result: a Records table (text) or a JSON array.
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

    use crate::modules::bookmark::backend::testkit::MockBookmarkBackend;
    use crate::util::json_mode;

    #[test]
    fn parse_tags_splits_and_trims() {
        // The shared helper's regression tests live in local.rs; here we only verify the alias path also calls it.
        assert_eq!(parse_tags(None), Vec::<String>::new());
        assert_eq!(
            parse_tags(Some(&"rust, cli ,  web ".to_string())),
            vec!["rust", "cli", "web"]
        );
    }

    fn sample_item() -> BookmarkItem {
        BookmarkItem {
            id: "b1".into(),
            url: "https://example.com".into(),
            title: "示例书签".into(),
            tags: vec!["rust".into(), "cli".into()],
        }
    }

    /// (a) The full action path — parse → backend → render — runs end-to-end against a
    /// `MockBookmarkBackend` with no SQLite. This proves the DI seam keeps provider /
    /// keyring concerns out of the action layer.
    #[tokio::test]
    async fn dispatch_with_mock_runs_action_path() {
        let backend = MockBookmarkBackend {
            items: vec![sample_item()],
            ..Default::default()
        };
        let flags = HashMap::new();

        let out = dispatch(&backend, "list", &flags, &[]).await.unwrap();
        match out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["id", "title", "url", "tags"]);
                assert_eq!(
                    rows,
                    vec![vec!["b1", "示例书签", "https://example.com", "rust, cli"]]
                );
            }
            other => panic!("expected Records, got {other:?}"),
        }
    }

    /// The render layer is provider-agnostic: the same domain data renders identically
    /// regardless of the source backend. We render directly with
    /// MockBookmarkBackend-supplied data and assert both text (Records) and JSON shapes.
    #[test]
    fn render_is_provider_agnostic_for_same_domain_data() {
        let item = sample_item();

        // Text mode → table with stable columns (backend-independent).
        let text_out = render_list(vec![item.clone()]);
        match text_out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["id", "title", "url", "tags"]);
                assert_eq!(rows[0][0], "b1");
                assert_eq!(rows[0][1], "示例书签");
            }
            other => panic!("expected Records, got {other:?}"),
        }

        // JSON mode → object with the same keys, regardless of source backend.
        json_mode::set_json_mode(true);
        let json_out = render_list(vec![item]);
        json_mode::set_json_mode(false);
        if let Output::Json(Value::Array(arr)) = json_out {
            assert_eq!(arr.len(), 1);
            assert_eq!(arr[0]["id"], json!("b1"));
            assert_eq!(arr[0]["url"], json!("https://example.com"));
            assert_eq!(arr[0]["title"], json!("示例书签"));
            assert_eq!(arr[0]["tags"], json!(["rust", "cli"]));
        } else {
            panic!("expected JSON array, got {json_out:?}");
        }
    }

    /// `search` reaches the backend with the positional query and renders like
    /// `list`.
    #[tokio::test]
    async fn dispatch_search_passes_query_and_renders() {
        let backend = MockBookmarkBackend {
            items: vec![sample_item()],
            ..Default::default()
        };

        let out = dispatch(&backend, "search", &HashMap::new(), &["示例".to_string()])
            .await
            .unwrap();
        match out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["id", "title", "url", "tags"]);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0][0], "b1");
            }
            other => panic!("expected Records, got {other:?}"),
        }
    }

    /// Matching a tag also finds the bookmark, and a miss returns an empty set
    /// rather than an error.
    #[tokio::test]
    async fn dispatch_search_matches_tag_and_empty_on_miss() {
        let backend = MockBookmarkBackend {
            items: vec![sample_item()],
            ..Default::default()
        };

        let hit = dispatch(&backend, "search", &HashMap::new(), &["rust".to_string()])
            .await
            .unwrap();
        match hit {
            Output::Records { rows, .. } => assert_eq!(rows.len(), 1),
            other => panic!("expected Records, got {other:?}"),
        }

        let miss = dispatch(&backend, "search", &HashMap::new(), &["nope".to_string()])
            .await
            .unwrap();
        match miss {
            Output::Records { rows, .. } => assert!(rows.is_empty()),
            other => panic!("expected Records, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_search_without_query_errors() {
        let backend = MockBookmarkBackend::default();
        let err = dispatch(&backend, "search", &HashMap::new(), &[])
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }

    #[tokio::test]
    async fn dispatch_search_rejects_non_numeric_limit() {
        let backend = MockBookmarkBackend::default();
        let mut flags = HashMap::new();
        flags.insert("limit".into(), "abc".into());
        let err = dispatch(&backend, "search", &flags, &["x".to_string()])
            .await
            .unwrap_err();
        assert_eq!(err.type_name(), "InvalidArgument");
    }
}
