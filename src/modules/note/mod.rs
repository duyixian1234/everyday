//! Note module: notes / knowledge-base management, local SQLite only
//! (Notion provider removed in v0.13.0 — [R019](../../../docs/adr/R019-remove-notion-provider.md)).
//!
//! Design goal: hide storage details and expose two high-level capabilities to the Agent:
//! **plain-text / Markdown append** and **simplified property operations**.
//!
//! Supported `action`s:
//! - `search`  search notes by title keyword
//! - `create`  create a note (with title and simplified properties)
//! - `read`    read note body, aggregated into Markdown (`--json` returns a structured object)
//! - `append`  append a text block to the end of a note (supports `--text` or piped stdin)
//! - `update`  modify note properties (meta info)
//! - `list`    list notes (title + properties)

pub mod backend;
pub mod local;

use std::collections::HashMap;
use std::io::{IsTerminal, Read};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::{NoteAccount, NoteModuleConfig};
use crate::error::{AgentError, Result};
use crate::modules::Executor;
use crate::modules::note::backend::{
    NoteAppended, NoteBackend, NoteCreated, NoteListEntry, NoteRead, NoteSummary, NoteUpdated,
    for_account,
};
use crate::output::Output;

pub struct NoteModule {
    config: NoteModuleConfig,
}

impl NoteModule {
    pub fn new(config: NoteModuleConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for NoteModule {
    fn description(&self) -> &'static str {
        "Note & knowledge-base (local sqlite): search, list, create, read, append, update."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            cli_action!(
                "search",
                "搜索页面",
                "everyday note search --query Q [--limit N] [--account NAME]",
                &[flag!("query", "搜索关键词"), flag!("limit", "条数上限"),]
            ),
            cli_action!(
                "create",
                "新建页面",
                "everyday note create --title T [--prop K:V ...] [--account NAME]",
                &[
                    flag!("title", "页面标题"),
                    flag!("prop", "属性 K:V（可重复）", Multi),
                ]
            ),
            cli_action!(
                "read",
                "读取页面内容（默认账户默认页）",
                "everyday note read [<id>] [--account NAME]",
                &[],
                Positional::OptionalSingle
            ),
            cli_action!(
                "append",
                "追加内容到页面（默认账户默认页，或从 stdin 读取）",
                "everyday note append [<id>] --text TEXT [--account NAME]",
                &[flag!("text", "追加文本（缺省从 stdin 读取）")],
                Positional::OptionalSingle
            ),
            cli_action!(
                "update",
                "更新页面属性",
                "everyday note update <id> --prop K:V ... [--account NAME]",
                &[flag!("prop", "属性 K:V（至少一个，可重复）", Multi)],
                Positional::OptionalSingle
            ),
            cli_action!(
                "list",
                "列出数据库中的页面",
                "everyday note list [--limit N] [--account NAME]",
                &[flag!("limit", "条数上限"),]
            ),
        ];
        ModuleArgSpec {
            name: "note",
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
        let (flags, multi, positional) = parse_args(args);
        let account = self
            .config
            .resolve_account(flags.get("account").map(|s| s.as_str()))?;

        // DI seam: the module never branches on provider or touches the keyring —
        // all of that lives in `for_account`.
        let backend = for_account(account)?;
        dispatch(
            backend.as_ref(),
            account,
            action,
            &flags,
            &multi,
            &positional,
        )
        .await
    }

    /// P3 health: local-only since v0.13.0 — no credentials to check, so the
    /// module is healthy whenever an account is resolvable (or none is configured).
    async fn health_check(&self) -> Result<crate::modules::HealthStatus> {
        Ok(crate::modules::HealthStatus::healthy())
    }
}

/// Core action dispatch, parameterized over a `NoteBackend`. `execute` supplies a real
/// backend via `for_account`; tests supply a `MockNoteBackend`. This is the DI seam that
/// keeps the action path free of provider / keyring concerns
/// ([R016](../../../docs/adr/R016-action-backend-di.md)).
async fn dispatch(
    backend: &dyn NoteBackend,
    account: &NoteAccount,
    action: &str,
    flags: &HashMap<String, String>,
    multi: &[(String, String)],
    positional: &[String],
) -> Result<Output> {
    match action {
        "search" => {
            let query = flags.get("query").ok_or_else(|| {
                AgentError::InvalidArgument("search requires --query <keyword>".into())
            })?;
            let limit: usize = flags
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10)
                .min(100);
            let results = backend.search(query, limit).await?;
            Ok(render_search(results))
        }
        "list" => {
            let limit: usize = flags
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50)
                .min(100);
            let rows = backend.list(limit).await?;
            Ok(render_list(rows))
        }
        "create" => {
            let title = flags.get("title").ok_or_else(|| {
                AgentError::InvalidArgument("create requires --title <title>".into())
            })?;
            let props = split_props(multi)?;
            let created = backend.create(title, &props).await?;
            Ok(render_create(created))
        }
        "read" => {
            let page_id = resolve_page_id(account, positional)?;
            let detail = backend.read(&page_id).await?;
            Ok(render_read(detail))
        }
        "append" => {
            let page_id = resolve_page_id(account, positional)?;
            let text = resolve_append_text(flags)?;
            let appended = backend.append(&page_id, &text).await?;
            Ok(render_append(appended))
        }
        "update" => {
            let page_id = positional
                .first()
                .ok_or_else(|| AgentError::InvalidArgument("update requires <id>".into()))?
                .clone();
            let props = split_props(multi)?;
            let updated = backend.update(&page_id, &props).await?;
            Ok(render_update(updated))
        }
        other => Err(AgentError::UnknownAction(format!("note {other}"))),
    }
}

// ============ Argument parsing ============
//
// Unlike `parse_simple_args`, note's `--prop` may repeat and its value contains a colon,
// so a dedicated parser is implemented: single-value flags take the last occurrence, while
// repeated flags (e.g. prop) are collected separately into an ordered list.

/// Parse result: `(single-value flags, repeated-flag list, positional args)`.
type ParsedArgs = (HashMap<String, String>, Vec<(String, String)>, Vec<String>);

fn parse_args(args: &[String]) -> ParsedArgs {
    let mut flags: HashMap<String, String> = HashMap::new();
    let mut multi: Vec<(String, String)> = Vec::new();
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(stripped) = a.strip_prefix("--") {
            if let Some((k, v)) = stripped.split_once('=') {
                push_flag(&mut flags, &mut multi, k, v.to_string());
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                push_flag(&mut flags, &mut multi, stripped, args[i + 1].clone());
                i += 1;
            } else {
                push_flag(&mut flags, &mut multi, stripped, "true".to_string());
            }
        } else {
            positional.push(a.clone());
        }
        i += 1;
    }
    (flags, multi, positional)
}

/// Insert a flag into the single-value map; repeated flags like `prop` also go into the `multi` list.
fn push_flag(
    flags: &mut HashMap<String, String>,
    multi: &mut Vec<(String, String)>,
    key: &str,
    value: String,
) {
    flags.insert(key.to_string(), value.clone());
    if key == "prop" {
        multi.push((key.to_string(), value));
    }
}

/// Split a `("prop", "K:V")` list into `(K, V)` pairs (validates the `K:V` shape).
fn split_props(multi: &[(String, String)]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (_, kv) in multi {
        let (k, v) = kv
            .split_once(':')
            .ok_or_else(|| AgentError::InvalidArgument(format!("prop must be K:V, got '{kv}'")))?;
        out.push((k.to_string(), v.to_string()));
    }
    Ok(out)
}

/// Resolve page_id from positional args or the account default config.
fn resolve_page_id(account: &NoteAccount, positional: &[String]) -> Result<String> {
    if let Some(first) = positional.first() {
        return Ok(first.clone());
    }
    account.default_page_id.clone().ok_or_else(|| {
        AgentError::InvalidArgument(
            "no <id> given and no default_page_id set for this account".into(),
        )
    })
}

/// Resolve the `append` text source: `--text`, else piped stdin (error if stdin is a TTY).
fn resolve_append_text(flags: &HashMap<String, String>) -> Result<String> {
    match flags.get("text") {
        Some(t) => Ok(t.clone()),
        None => {
            if std::io::stdin().is_terminal() {
                Err(AgentError::InvalidArgument(
                    "append requires --text TEXT or piped stdin".into(),
                ))
            } else {
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| AgentError::Io(e.to_string()))?;
                Ok(buf)
            }
        }
    }
}

// ============ Rendering (module owns Output; backends return domain structs) ============

/// Render `search` results (rows: id / title / last_edited; JSON: id / title / last_edited / url).
/// The Notion object-type `type` column was removed with the provider (v0.13.0);
/// `url` is kept (always empty for local notes) for `--json` consumers.
fn render_search(results: Vec<NoteSummary>) -> Output {
    if crate::util::json_mode::is_json() {
        let items: Vec<Value> = results
            .into_iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "title": s.title,
                    "last_edited": s.updated,
                    "url": "",
                })
            })
            .collect();
        Output::Json(Value::Array(items))
    } else {
        let rows: Vec<Vec<String>> = results
            .into_iter()
            .map(|s| vec![s.id, s.title, s.updated])
            .collect();
        Output::records(
            vec!["id".into(), "title".into(), "last_edited".into()],
            rows,
        )
    }
}

/// Render `list` results (rows: id / title / last_edited; JSON includes url + properties).
fn render_list(rows: Vec<NoteListEntry>) -> Output {
    if crate::util::json_mode::is_json() {
        let items: Vec<Value> = rows
            .into_iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "title": s.title,
                    "url": "",
                    "last_edited": s.updated,
                    "properties": Value::Object(s.properties),
                })
            })
            .collect();
        Output::Json(Value::Array(items))
    } else {
        let rows_t: Vec<Vec<String>> = rows
            .into_iter()
            .map(|s| vec![s.id, s.title, s.updated])
            .collect();
        Output::records(
            vec!["id".into(), "title".into(), "last_edited".into()],
            rows_t,
        )
    }
}

/// Render `create` result (local note record).
fn render_create(d: NoteCreated) -> Output {
    let json_out = json!({ "id": d.id, "title": d.title, "properties": d.prop_count });
    if crate::util::json_mode::is_json() {
        Output::Json(json_out)
    } else {
        Output::text(format!(
            "created note '{}' (id={}, props={})",
            d.title, d.id, d.prop_count
        ))
    }
}

/// Render `read` result: aggregated Markdown body + properties.
fn render_read(d: NoteRead) -> Output {
    let json_out = json!({
        "id": d.id,
        "title": d.title,
        "url": "",
        "properties": Value::Object(d.properties),
        "content": d.content,
    });
    if crate::util::json_mode::is_json() {
        Output::Json(json_out)
    } else {
        let mut text = String::new();
        if !d.title.is_empty() {
            text.push_str(&format!("# {}\n\n", d.title));
        }
        text.push_str(&d.content);
        Output::text(text)
    }
}

/// Render `append` result.
fn render_append(d: NoteAppended) -> Output {
    let json_out = json!({ "id": d.id, "url": "", "appended": d.appended });
    if crate::util::json_mode::is_json() {
        Output::Json(json_out)
    } else {
        Output::text(format!("appended {} line(s) to note {}", d.appended, d.id))
    }
}

/// Render `update` result.
fn render_update(d: NoteUpdated) -> Output {
    let json_out = json!({ "id": d.id, "url": "", "updated": d.updated_count });
    if crate::util::json_mode::is_json() {
        Output::Json(json_out)
    } else {
        Output::text(format!(
            "updated {} propert(ies) on note {}",
            d.updated_count, d.id
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::note::backend::testkit::MockNoteBackend;

    #[test]
    fn parse_args_handles_repeated_prop() {
        let args: Vec<String> = [
            "--title",
            "Rust 异步",
            "--prop",
            "类型:文章",
            "--prop=状态:未读",
            "--prop",
            "URL:https://x",
            "page_id_here",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (flags, multi, positional) = parse_args(&args);
        assert_eq!(flags.get("title"), Some(&"Rust 异步".to_string()));
        // Single-value flag keeps the last prop value (fallback only; real logic uses multi).
        assert_eq!(positional, vec!["page_id_here"]);
        assert_eq!(multi.len(), 3);
        assert_eq!(multi[0], ("prop".to_string(), "类型:文章".to_string()));
        assert_eq!(multi[1], ("prop".to_string(), "状态:未读".to_string()));
        assert_eq!(multi[2], ("prop".to_string(), "URL:https://x".to_string()));
    }

    #[test]
    fn split_props_parses_kv() {
        let multi = vec![
            ("prop".to_string(), "类型:文章".to_string()),
            ("prop".to_string(), "状态:未读".to_string()),
        ];
        let out = split_props(&multi).unwrap();
        assert_eq!(out[0], ("类型".to_string(), "文章".to_string()));
        assert_eq!(out[1], ("状态".to_string(), "未读".to_string()));
    }

    #[test]
    fn split_props_rejects_missing_colon() {
        let multi = vec![("prop".to_string(), "invalid".to_string())];
        assert!(split_props(&multi).is_err());
    }

    // ---- T13.3: DI acceptance guard via MockNoteBackend ----

    /// Build a minimal local `NoteAccount` with no defaults; enough to drive `dispatch`
    /// without touching the keyring or any provider.
    fn dummy_account() -> NoteAccount {
        NoteAccount {
            name: "test".into(),
            provider: "local".into(),
            default_page_id: None,
            db_path: None,
        }
    }

    /// (a) The full action path — parse → backend → render — runs end-to-end against a
    /// `MockNoteBackend` with no SQLite. This proves the DI seam keeps provider /
    /// keyring concerns out of the action layer.
    #[tokio::test]
    async fn dispatch_with_mock_runs_action_path() {
        let backend = MockNoteBackend {
            summaries: vec![NoteSummary {
                id: "p1".into(),
                title: "Rust 笔记".into(),
                updated: "2026-07-12".into(),
            }],
            ..Default::default()
        };
        let account = dummy_account();
        let mut flags = HashMap::new();
        flags.insert("query".into(), "rust".into());

        let out = dispatch(&backend, &account, "search", &flags, &[], &[])
            .await
            .unwrap();
        match out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["id", "title", "last_edited"]);
                assert_eq!(rows, vec![vec!["p1", "Rust 笔记", "2026-07-12"]]);
            }
            other => panic!("expected Records, got {other:?}"),
        }
    }

    /// (b) The render layer emits stable shapes for the local-only domain data.
    #[test]
    fn render_is_stable_for_domain_data() {
        let summary = NoteSummary {
            id: "p1".into(),
            title: "Rust 笔记".into(),
            updated: "2026-07-12".into(),
        };

        // Text mode → table with stable columns.
        let text_out = render_search(vec![summary.clone()]);
        match text_out {
            Output::Records { headers, rows } => {
                assert_eq!(headers, vec!["id", "title", "last_edited"]);
                assert_eq!(rows[0][0], "p1");
                assert_eq!(rows[0][1], "Rust 笔记");
            }
            other => panic!("expected Records, got {other:?}"),
        }

        // JSON mode → object with the stable keys.
        crate::util::json_mode::set_json_mode(true);
        let json_out = render_search(vec![summary]);
        crate::util::json_mode::set_json_mode(false);
        if let Output::Json(Value::Array(arr)) = json_out {
            assert_eq!(arr.len(), 1);
            assert_eq!(arr[0]["id"], json!("p1"));
            assert_eq!(arr[0]["title"], json!("Rust 笔记"));
            assert_eq!(arr[0]["last_edited"], json!("2026-07-12"));
        } else {
            panic!("expected Json array, got {json_out:?}");
        }
    }
}
