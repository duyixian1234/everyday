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
        use crate::modules::{ActionArgSpec, ModuleArgSpec};
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
        let (flags, _positional) = parse_simple_args(args);
        let account = self
            .config
            .resolve_account(flags.get("account").map(|s| s.as_str()))?;

        // DI seam: the module never branches on provider or touches the keyring —
        // all of that lives in `for_account`.
        let backend = for_account(account)?;
        dispatch(&*backend, action, &flags).await
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

        let out = dispatch(&backend, "list", &flags).await.unwrap();
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

    /// (b) The render layer is provider-agnostic: the same domain data renders identically
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
}
