//! Note module: notes / knowledge-base management. Defaults to the local SQLite provider (`local`),
//! but can switch to the Notion API (`provider = "notion"`) [N001](../../../docs/adr/N001-notion-note-module.md)
//! [F005](../../../docs/adr/F005-default-provider-local.md).
//!
//! Design goal: hide Notion's verbose Block nesting and expose two high-level capabilities to the Agent:
//! **plain-text / Markdown append** and **simplified property operations**.
//!
//! Supported `action`s:
//! - `auth login` stores the Notion Integration Token in the keyring (see the `auth` module)
//! - `search`  search pages / databases by title keyword
//! - `create`  create a record in a database (with title and simplified properties)
//! - `read`    read page body, aggregated into Markdown (`--json` returns a structured object)
//! - `append`  append a text block to the end of a page (supports `--text` or piped stdin)
//! - `update`  modify page properties (meta info)
//! - `list`    list all pages under a database (title + properties)
//!
//! Credential safety: the token is stored only in the system keyring (service = `everyday/note/<account>`),
//! never persisted to config [F002](../../../docs/adr/F002-multi-account-keyring.md).
//!
//! Provider selection + token fetch happen entirely inside
//! [`NoteBackend::for_account`](../../../docs/adr/R016-action-backend-di.md); this module only
//! dispatches actions to a `Box<dyn NoteBackend>` and renders the returned domain structs.

pub mod backend;
pub mod local;
pub mod notion;

use std::collections::HashMap;
use std::io::{IsTerminal, Read};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::config::NoteAccount;
use crate::error::{AgentError, Result};
use crate::modules::Executor;
use crate::modules::note::backend::{
    NoteAppended, NoteCreated, NoteListEntry, NoteRead, NoteSummary, NoteUpdated, for_account,
};
use crate::output::Output;

pub struct NoteModule {
    config: std::sync::Arc<crate::config::Config>,
}

impl NoteModule {
    pub fn new(config: std::sync::Arc<crate::config::Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for NoteModule {
    fn description(&self) -> &'static str {
        "Note & knowledge-base (Notion or local sqlite): search, list, create, read, append, update."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "search",
                description: "搜索页面",
                usage: "everyday note search --query Q [--limit N] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "query",
                        help: "搜索关键词",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "limit",
                        help: "条数上限",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "create",
                description: "新建页面",
                usage: "everyday note create --title T [--db ID] [--prop K:V ...] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "title",
                        help: "页面标题",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "db",
                        help: "数据库 ID（默认账户默认库）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "prop",
                        help: "属性 K:V（可重复）",
                        kind: ArgKind::Multi,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "read",
                description: "读取页面内容（默认账户默认页）",
                usage: "everyday note read [<page_id>] [--account NAME]",
                args: &[],
                positional: Positional::OptionalSingle,
            },
            ActionArgSpec {
                name: "append",
                description: "追加内容到页面（默认账户默认页，或从 stdin 读取）",
                usage: "everyday note append [<page_id>] --text TEXT [--account NAME]",
                args: &[ArgSpec {
                    name: "text",
                    help: "追加文本（缺省从 stdin 读取）",
                    kind: ArgKind::Value,
                }],
                positional: Positional::OptionalSingle,
            },
            ActionArgSpec {
                name: "update",
                description: "更新页面属性",
                usage: "everyday note update <page_id> --prop K:V ... [--account NAME]",
                args: &[ArgSpec {
                    name: "prop",
                    help: "属性 K:V（至少一个，可重复）",
                    kind: ArgKind::Multi,
                }],
                positional: Positional::OptionalSingle,
            },
            ActionArgSpec {
                name: "list",
                description: "列出数据库中的页面",
                usage: "everyday note list [--db ID] [--limit N] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "db",
                        help: "数据库 ID",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "limit",
                        help: "条数上限",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "note",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, multi, positional) = parse_args(args);
        let account = self
            .config
            .note_account(flags.get("account").map(|s| s.as_str()))?;

        // DI seam: the module never names `NotionClient`, never branches on provider,
        // never touches the keyring — all of that lives in `for_account`.
        let backend = for_account(&self.config, account)?;

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
                let db_id = flags
                    .get("db")
                    .map(|s| s.as_str())
                    .or(account.default_database_id.as_deref());
                let limit: usize = flags
                    .get("limit")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(50)
                    .min(100);
                let rows = backend.list(db_id, limit).await?;
                Ok(render_list(rows))
            }
            "create" => {
                let title = flags.get("title").ok_or_else(|| {
                    AgentError::InvalidArgument("create requires --title <title>".into())
                })?;
                let db_id = flags
                    .get("db")
                    .map(|s| s.as_str())
                    .or(account.default_database_id.as_deref());
                let props = split_props(&multi)?;
                let created = backend.create(title, db_id, &props).await?;
                Ok(render_create(created))
            }
            "read" => {
                let page_id = resolve_page_id(account, &positional)?;
                let detail = backend.read(&page_id).await?;
                Ok(render_read(detail))
            }
            "append" => {
                let page_id = resolve_page_id(account, &positional)?;
                let text = resolve_append_text(&flags)?;
                let appended = backend.append(&page_id, &text).await?;
                Ok(render_append(appended))
            }
            "update" => {
                let page_id = positional
                    .first()
                    .ok_or_else(|| AgentError::InvalidArgument("update requires <page_id>".into()))?
                    .clone();
                let props = split_props(&multi)?;
                let updated = backend.update(&page_id, &props).await?;
                Ok(render_update(updated))
            }
            other => Err(AgentError::UnknownAction(format!("note {other}"))),
        }
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
            "no <page_id> given and no default_page_id set for this account".into(),
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

/// Render `search` results (rows: id / type / title / last_edited; JSON: id / type / title / last_edited / url?).
fn render_search(results: Vec<NoteSummary>) -> Output {
    if crate::util::json_mode::is_json() {
        let items: Vec<Value> = results
            .into_iter()
            .map(|s| {
                let mut m = Map::new();
                m.insert("id".into(), Value::String(s.id));
                m.insert("type".into(), Value::String(s.kind));
                m.insert("title".into(), Value::String(s.title));
                m.insert("last_edited".into(), Value::String(s.updated));
                if let Some(u) = s.url {
                    m.insert("url".into(), Value::String(u));
                }
                Value::Object(m)
            })
            .collect();
        Output::Json(Value::Array(items))
    } else {
        let rows: Vec<Vec<String>> = results
            .into_iter()
            .map(|s| vec![s.id, s.kind, s.title, s.updated])
            .collect();
        Output::records(
            vec![
                "id".into(),
                "type".into(),
                "title".into(),
                "last_edited".into(),
            ],
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
                let mut m = Map::new();
                m.insert("id".into(), Value::String(s.id));
                m.insert("title".into(), Value::String(s.title));
                m.insert("url".into(), Value::String(s.url.unwrap_or_default()));
                m.insert("last_edited".into(), Value::String(s.updated));
                m.insert("properties".into(), Value::Object(s.properties));
                Value::Object(m)
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

/// Render `create` result. Notion (`database_id` set) emits a page record; local emits a note record.
fn render_create(d: NoteCreated) -> Output {
    let json_out = if d.database_id.is_some() {
        json!({
            "id": d.id,
            "url": d.url.clone().unwrap_or_default(),
            "title": d.title,
            "database_id": d.database_id.clone().unwrap_or_default(),
        })
    } else {
        json!({ "id": d.id, "title": d.title, "properties": d.prop_count })
    };
    if crate::util::json_mode::is_json() {
        Output::Json(json_out)
    } else if d.resource == "page" {
        let url = d.url.clone().unwrap_or_default();
        let db = d.database_id.clone().unwrap_or_default();
        Output::text(format!(
            "created page '{}' (id={}, database={})\n{}",
            d.title, d.id, db, url
        ))
    } else {
        Output::text(format!(
            "created note '{}' (id={}, props={})",
            d.title, d.id, d.prop_count
        ))
    }
}

/// Render `read` result: aggregated Markdown body + properties.
fn render_read(d: NoteRead) -> Output {
    let url = d.url.clone().unwrap_or_default();
    let json_out = json!({
        "id": d.id,
        "title": d.title,
        "url": url,
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
        if !url.is_empty() {
            text.push_str(&format!("({url})\n\n"));
        }
        text.push_str(&d.content);
        Output::text(text)
    }
}

/// Render `append` result.
fn render_append(d: NoteAppended) -> Output {
    let url = d.url.clone().unwrap_or_default();
    let json_out = json!({ "id": d.id, "url": url, "appended": d.appended });
    if crate::util::json_mode::is_json() {
        Output::Json(json_out)
    } else {
        let suffix = if url.is_empty() {
            String::new()
        } else {
            format!("\n{url}")
        };
        Output::text(format!(
            "appended {} {}(s) to {} {}{}",
            d.appended, d.unit, d.resource, d.id, suffix
        ))
    }
}

/// Render `update` result.
fn render_update(d: NoteUpdated) -> Output {
    let url = d.url.clone().unwrap_or_default();
    let json_out = json!({ "id": d.id, "url": url, "updated": d.updated_count });
    if crate::util::json_mode::is_json() {
        Output::Json(json_out)
    } else {
        let suffix = if url.is_empty() {
            String::new()
        } else {
            format!("\n{url}")
        };
        Output::text(format!(
            "updated {} propert(ies) on {} {}{}",
            d.updated_count, d.resource, d.id, suffix
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
