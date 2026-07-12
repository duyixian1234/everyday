//! Note module: notes / knowledge-base management. Defaults to the local SQLite provider (`local`),
//! but can switch to the Notion API (`provider = "notion"`) [N001](../../docs/adr/N001-notion-note-module.md)
//! [F005](../../docs/adr/F005-default-provider-local.md).
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
//! never persisted to config [F002](../../docs/adr/F002-multi-account-keyring.md).

use std::collections::HashMap;
use std::io::{IsTerminal, Read};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::config::{Config, NoteAccount};
use crate::error::{AgentError, Result};
use crate::modules::Executor;
use crate::notion_client::NotionClient;
use crate::output::Output;

/// Maximum recursion depth when rendering blocks, to prevent runaway expansion on malformed data.
const MAX_BLOCK_DEPTH: usize = 12;

/// Return-type alias for `parse_args` (avoids inlining a complex tuple type).
type ParsedArgs = (HashMap<String, String>, Vec<(String, String)>, Vec<String>);

pub struct NoteModule {
    config: Arc<crate::config::Config>,
}

impl NoteModule {
    pub fn new(config: Arc<crate::config::Config>) -> Self {
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

        // Local SQLite provider: route to the local implementation; otherwise go through Notion.
        if crate::modules::local::is_local_provider(&account.provider) {
            use crate::modules::note_local as local;
            return match action {
                "search" => local::search(account, &flags).await,
                "create" => local::create(account, &flags, &multi).await,
                "read" => local::read(account, &positional).await,
                "append" => local::append(account, &flags, &positional).await,
                "update" => local::update(account, &positional, &multi).await,
                "list" => local::list(account, &flags).await,
                other => Err(AgentError::UnknownAction(format!("note {other}"))),
            };
        }

        match action {
            "search" => note_search(&self.config, account, &flags).await,
            "create" => note_create(&self.config, account, &flags, &multi).await,
            "read" => note_read(&self.config, account, &flags, &positional).await,
            "append" => note_append(&self.config, account, &flags, &positional).await,
            "update" => note_update(&self.config, account, &flags, &positional, &multi).await,
            "list" => note_list(&self.config, account, &flags).await,
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

// ============ Credentials (keyring) ============

/// Read the Notion token from the OS keyring via the consolidated `auth` module
/// ([R013](../../docs/adr/R013-auth-module-consolidation.md)).
fn get_token(config: &Config, account: &NoteAccount) -> Result<String> {
    crate::modules::auth::get_credential(config, "note", &account.name)
}

// ============ HTTP wrapper ============
//
// All Notion HTTP requests go through the shared [`NotionClient`] (see `notion_client.rs`)
// [F004](../../docs/adr/F004-shared-notion-client.md): it injects auth headers, handles 429
// backoff retries, and maps error types. This module no longer carries its own HTTP layer.

/// Paginate and fetch all child blocks of a block (used for `read` content aggregation).
async fn fetch_all_blocks(client: &NotionClient, block_id: &str) -> Result<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut path = format!("/blocks/{block_id}/children?page_size=100");
        if let Some(c) = &cursor {
            path.push_str(&format!("&start_cursor={c}"));
        }
        let v: Value = client.get(&path).await?;
        if let Some(results) = v.get("results").and_then(|r| r.as_array()) {
            out.extend(results.iter().cloned());
        }
        if v.get("has_more").and_then(|h| h.as_bool()) == Some(true) {
            cursor = v
                .get("next_cursor")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
        } else {
            break;
        }
    }
    Ok(out)
}

// ============ Rich text / title extraction ============

/// Render a Notion rich_text array into plain text (no formatting).
fn rich_text_plain(rt: &[Value]) -> String {
    rt.iter()
        .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

/// Render a Notion rich_text array into Markdown with inline formatting (bold/italic/code/strike + links).
fn rich_text_md(rt: &[Value]) -> String {
    let mut out = String::new();
    for t in rt {
        let text = t
            .get("plain_text")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        let href = t
            .get("href")
            .and_then(|h| h.as_str())
            .map(|s| s.to_string());
        let ann = t.get("annotations");
        let code = ann.and_then(|a| a.get("code")).and_then(|b| b.as_bool()) == Some(true);
        let bold = ann.and_then(|a| a.get("bold")).and_then(|b| b.as_bool()) == Some(true);
        let italic = ann.and_then(|a| a.get("italic")).and_then(|b| b.as_bool()) == Some(true);
        let strike = ann
            .and_then(|a| a.get("strikethrough"))
            .and_then(|b| b.as_bool())
            == Some(true);

        let mut inner = text.clone();
        if code {
            inner = format!("`{inner}`");
        }
        if bold {
            inner = format!("**{inner}**");
        }
        if italic {
            inner = format!("*{inner}*");
        }
        if strike {
            inner = format!("~~{inner}~~");
        }
        match href {
            Some(h) => out.push_str(&format!("[{inner}]({h})")),
            None => out.push_str(&inner),
        }
    }
    out
}

/// Extract the title from a search/page object:
/// - database: top-level `title` array
/// - page: find the property with type == "title" in `properties`
fn extract_title(obj: &Value) -> String {
    if obj.get("object").and_then(|o| o.as_str()) == Some("database") {
        if let Some(title) = obj.get("title").and_then(|t| t.as_array()) {
            return rich_text_plain(title);
        }
        return String::new();
    }
    // page
    if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
        for p in props.values() {
            if p.get("type").and_then(|t| t.as_str()) == Some("title")
                && let Some(rt) = p.get("title").and_then(|t| t.as_array())
            {
                return rich_text_plain(rt);
            }
        }
    }
    String::new()
}

// ============ Property encoding (shared by create / update) ============

/// Encode a string value into a Notion property value based on its property type.
fn encode_property(ptype: &str, value: &str) -> Result<Value> {
    match ptype {
        "title" => Ok(json!({ "title": [{ "text": { "content": value } }] })),
        "rich_text" => Ok(json!({ "rich_text": [{ "text": { "content": value } }] })),
        "number" => {
            let n: f64 = value
                .parse()
                .map_err(|_| AgentError::InvalidArgument(format!("not a number: {value}")))?;
            Ok(json!({ "number": n }))
        }
        "checkbox" => {
            let b = parse_bool(value)?;
            Ok(json!({ "checkbox": b }))
        }
        "select" => Ok(json!({ "select": { "name": value } })),
        "multi_select" => Ok(json!({ "multi_select": [{ "name": value }] })),
        "url" => Ok(json!({ "url": value })),
        "email" => Ok(json!({ "email": value })),
        "phone_number" => Ok(json!({ "phone_number": value })),
        // Unknown type (formula/relation/file, etc.): fall back to rich_text to avoid hard errors.
        _ => Ok(json!({ "rich_text": [{ "text": { "content": value } }] })),
    }
}

/// Parse a string into a boolean (accepts true/false/yes/no/1/0, case-insensitive).
fn parse_bool(s: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(AgentError::InvalidArgument(format!(
            "not a boolean (expected true/false): {s}"
        ))),
    }
}

/// Look up a property's type in the database schema; error with available properties if missing.
fn db_property_type(schema: &Value, name: &str) -> Result<String> {
    let props = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .ok_or_else(|| AgentError::Network("database schema missing properties".into()))?;
    match props.get(name) {
        Some(p) => p
            .get("type")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| AgentError::Other(format!("property '{name}' has no type"))),
        None => {
            let available: Vec<String> = props.keys().cloned().collect();
            Err(AgentError::InvalidArgument(format!(
                "property '{name}' not found in database schema. available: {}",
                available.join(", ")
            )))
        }
    }
}

/// Find the name of the title-typed property in the database schema (used to hold `--title` on create).
fn db_title_property_name(schema: &Value) -> Result<String> {
    let props = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .ok_or_else(|| AgentError::Network("database schema missing properties".into()))?;
    for (name, p) in props {
        if p.get("type").and_then(|t| t.as_str()) == Some("title") {
            return Ok(name.clone());
        }
    }
    Err(AgentError::InvalidArgument(
        "database has no title property".into(),
    ))
}

/// Heuristic encoding when no schema is available (standalone pages without a database parent).
fn encode_property_heuristic(value: &str) -> Value {
    if let Ok(b) = parse_bool(value) {
        return json!({ "checkbox": b });
    }
    if let Ok(n) = value.parse::<f64>() {
        return json!({ "number": n });
    }
    json!({ "rich_text": [{ "text": { "content": value } }] })
}

/// Simplify page properties into a `name -> string value` map (used for `read`'s JSON output).
fn page_props_to_strings(props: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (name, p) in props {
        let ptype = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let value: Option<String> = match ptype {
            "title" | "rich_text" => p
                .get(ptype)
                .and_then(|t| t.as_array())
                .map(|r| rich_text_plain(r))
                .filter(|s| !s.is_empty()),
            "number" => p
                .get("number")
                .and_then(|n| n.as_f64())
                .map(|n| n.to_string()),
            "checkbox" => p
                .get("checkbox")
                .and_then(|b| b.as_bool())
                .map(|b| b.to_string()),
            "select" => p
                .get("select")
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string()),
            "status" => p
                .get("status")
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string()),
            "multi_select" => p
                .get("multi_select")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|o| o.get("name").and_then(|n| n.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|s| !s.is_empty()),
            "date" => p
                .get("date")
                .and_then(|d| d.get("start"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            "url" => p.get("url").and_then(|u| u.as_str()).map(|s| s.to_string()),
            "email" => p
                .get("email")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string()),
            "phone_number" => p
                .get("phone_number")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string()),
            _ => None,
        };
        if let Some(v) = value {
            out.insert(name.clone(), Value::String(v));
        }
    }
    out
}

// ============ Markdown -> Block ============

/// Split plain text / Markdown text into a Notion block array (Markdown-lite parser).
///
/// Supported syntax: ```code block```, `#/##/###` headings, `- /*` unordered lists, `1.` ordered lists,
/// `> ` quotes, `---` dividers, blank-line-separated paragraphs. Everything else is a normal paragraph.
fn text_to_blocks(text: &str) -> Vec<Value> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks: Vec<Value> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_buf: Vec<String> = Vec::new();

    let flush_para = |para: &mut Vec<String>, blocks: &mut Vec<Value>| {
        if !para.is_empty() {
            let content = para.join("\n");
            blocks.push(json!({
                "object": "block",
                "type": "paragraph",
                "paragraph": { "rich_text": rt_from_str(&content) }
            }));
            para.clear();
        }
    };

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Code fence.
        if trimmed.starts_with("```") {
            if !in_code {
                in_code = true;
                code_lang = trimmed.trim_start_matches('`').trim().to_string();
            } else {
                // Closing fence: emit a code block.
                let content = code_buf.join("\n");
                blocks.push(json!({
                    "object": "block",
                    "type": "code",
                    "code": {
                        "language": if code_lang.is_empty() { "plain text" } else { &code_lang },
                        "rich_text": rt_from_str(&content)
                    }
                }));
                code_buf.clear();
                in_code = false;
                code_lang.clear();
            }
            i += 1;
            continue;
        }
        if in_code {
            code_buf.push(line.to_string());
            i += 1;
            continue;
        }

        // Divider.
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            flush_para(&mut para, &mut blocks);
            blocks.push(json!({ "object": "block", "type": "divider", "divider": {} }));
            i += 1;
            continue;
        }

        // Heading.
        if let Some(rest) = trimmed.strip_prefix("### ") {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("heading_3", rest));
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("heading_2", rest));
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("heading_1", rest));
            i += 1;
            continue;
        }

        // Quote.
        if let Some(rest) = trimmed.strip_prefix("> ") {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("quote", rest));
            i += 1;
            continue;
        }

        // Unordered list.
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("bulleted_list_item", rest));
            i += 1;
            continue;
        }

        // Ordered list.
        if let Some(rest) = trimmed
            .split_once(". ")
            .filter(|(n, _)| n.parse::<usize>().is_ok())
        {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("numbered_list_item", rest.1));
            i += 1;
            continue;
        }

        // Blank line: paragraph separator.
        if trimmed.is_empty() {
            flush_para(&mut para, &mut blocks);
            i += 1;
            continue;
        }

        // Plain text line: accumulate into the paragraph.
        para.push(line.to_string());
        i += 1;
    }
    // Wrap up.
    if in_code {
        // Unclosed code block: fall back to plain text.
        let content = code_buf.join("\n");
        blocks.push(json!({
            "object": "block",
            "type": "code",
            "code": { "language": if code_lang.is_empty() { "plain text" } else { &code_lang }, "rich_text": rt_from_str(&content) }
        }));
    }
    flush_para(&mut para, &mut blocks);
    blocks
}

/// Build a block carrying a single rich_text (used for headings/lists/quotes/paragraphs).
fn block_text(block_type: &str, text: &str) -> Value {
    json!({
        "object": "block",
        "type": block_type,
        block_type: { "rich_text": rt_from_str(text) }
    })
}

/// Build a Notion rich_text array from a plain string.
fn rt_from_str(s: &str) -> Vec<Value> {
    if s.is_empty() {
        return vec![];
    }
    vec![json!({ "type": "text", "text": { "content": s }, "plain_text": s })]
}

// ============ Block -> Markdown (recursive aggregation) ============

/// Recursively render all page blocks into Markdown, as the aggregated body for `read`.
async fn blocks_to_markdown(
    client: &NotionClient,
    blocks: &[Value],
    depth: usize,
) -> Result<String> {
    let mut out = String::new();
    for b in blocks {
        let block_type = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let body = render_block_body(block_type, b);
        out.push_str(&body);

        // Recurse into child blocks (toggle / nested lists / list columns, etc.).
        if b.get("has_children").and_then(|h| h.as_bool()) == Some(true)
            && depth < MAX_BLOCK_DEPTH
            && let Some(id) = b.get("id").and_then(|i| i.as_str())
        {
            let children = fetch_all_blocks(client, id).await?;
            let child_md = Box::pin(blocks_to_markdown(client, &children, depth + 1)).await?;
            // Indent child list by 2 spaces to preserve nesting.
            let indented = child_md
                .lines()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n");
            if !indented.trim().is_empty() {
                out.push_str(&indented);
                if !indented.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    Ok(out)
}

/// Render a single block's body (excluding child blocks); returns a string with a trailing newline.
fn render_block_body(block_type: &str, b: &Value) -> String {
    match block_type {
        "paragraph" => {
            let rt = b
                .get("paragraph")
                .and_then(|p| p.get("rich_text"))
                .and_then(|r| r.as_array());
            format!("{}\n\n", rich_text_md(rt.unwrap_or(&vec![])))
        }
        "heading_1" => {
            let rt = rt_of(b, "heading_1");
            format!("# {}\n\n", rich_text_md(&rt))
        }
        "heading_2" => {
            let rt = rt_of(b, "heading_2");
            format!("## {}\n\n", rich_text_md(&rt))
        }
        "heading_3" => {
            let rt = rt_of(b, "heading_3");
            format!("### {}\n\n", rich_text_md(&rt))
        }
        "bulleted_list_item" => {
            let rt = rt_of(b, "bulleted_list_item");
            format!("- {}\n", rich_text_md(&rt))
        }
        "numbered_list_item" => {
            let rt = rt_of(b, "numbered_list_item");
            format!("1. {}\n", rich_text_md(&rt))
        }
        "to_do" => {
            let rt = rt_of(b, "to_do");
            let checked = b
                .get("to_do")
                .and_then(|t| t.get("checked"))
                .and_then(|c| c.as_bool())
                == Some(true);
            let mark = if checked { "x" } else { " " };
            format!("- [{}] {}\n", mark, rich_text_md(&rt))
        }
        "quote" => {
            let rt = rt_of(b, "quote");
            format!("> {}\n\n", rich_text_md(&rt))
        }
        "callout" => {
            let rt = rt_of(b, "callout");
            format!("> {}\n\n", rich_text_md(&rt))
        }
        "code" => {
            let lang = b
                .get("code")
                .and_then(|c| c.get("language"))
                .and_then(|l| l.as_str())
                .unwrap_or("plain text");
            let rt = b
                .get("code")
                .and_then(|c| c.get("rich_text"))
                .and_then(|r| r.as_array());
            let code = rich_text_plain(rt.unwrap_or(&vec![]));
            format!("```{lang}\n{code}\n```\n\n")
        }
        "divider" => "---\n\n".to_string(),
        "image" => {
            let (url, caption) = media_url_and_caption(b, "image");
            format!("![{}]({})\n\n", caption, url)
        }
        "bookmark" | "file" | "embed" | "video" | "pdf" | "audio" => {
            let (url, caption) = media_url_and_caption(b, block_type);
            if url.is_empty() {
                String::new()
            } else if caption.is_empty() {
                format!("{url}\n\n")
            } else {
                format!("[{}]({})\n\n", caption, url)
            }
        }
        "child_page" => {
            let title = b
                .get("child_page")
                .and_then(|c| c.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let url = b.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if url.is_empty() {
                format!("[{}]\n\n", title)
            } else {
                format!("[{}]({})\n\n", title, url)
            }
        }
        "child_database" => {
            let title = b
                .get("child_database")
                .and_then(|c| c.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let url = b.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if url.is_empty() {
                format!("[{}]\n\n", title)
            } else {
                format!("[{}]({})\n\n", title, url)
            }
        }
        // Other types: best-effort render their rich_text to avoid losing content.
        _ => {
            // Most block types place text in a child object named after the type.
            if let Some(obj) = b.get(block_type)
                && let Some(rt) = obj.get("rich_text").and_then(|r| r.as_array())
            {
                return format!("{}\n\n", rich_text_md(rt));
            }
            String::new()
        }
    }
}

/// Extract the rich_text array from a block's child object (empty if absent).
fn rt_of(b: &Value, block_type: &str) -> Vec<Value> {
    b.get(block_type)
        .and_then(|o| o.get("rich_text"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Extract a media block's URL and caption (image/bookmark/file/embed/video/pdf/audio).
fn media_url_and_caption(b: &Value, block_type: &str) -> (String, String) {
    let sub = match b.get(block_type) {
        Some(s) => s,
        None => return (String::new(), String::new()),
    };
    // Media content lives in the child field named by sub["type"] (external / file), or directly under url.
    let url = if let Some(u) = sub.get("url").and_then(|u| u.as_str()) {
        u.to_string()
    } else if let Some(kind) = sub.get("type").and_then(|t| t.as_str()) {
        sub.get(kind)
            .and_then(|k| k.get("url"))
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    let caption = sub
        .get("caption")
        .and_then(|c| c.as_array())
        .map(|r| rich_text_plain(r))
        .unwrap_or_default();
    (url, caption)
}

// ============ Action implementations ============

/// `note search --query Q [--limit N]`: search pages/databases by title.
async fn note_search(
    config: &Config,
    account: &NoteAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let query = flags
        .get("query")
        .ok_or_else(|| AgentError::InvalidArgument("search requires --query <keyword>".into()))?;
    let limit: usize = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
        .min(100);
    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    let body = json!({ "query": query, "page_size": limit });
    let resp: Value = client.post("/search", &body).await?;
    let results = resp
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut items: Vec<Value> = Vec::new();
    for r in &results {
        let obj = r
            .get("object")
            .and_then(|o| o.as_str())
            .unwrap_or("?")
            .to_string();
        let id = r
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string();
        let title = extract_title(r);
        let edited = r
            .get("last_edited_time")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        rows.push(vec![id.clone(), obj.clone(), title.clone(), edited.clone()]);
        let mut item = Map::new();
        item.insert("id".into(), Value::String(id));
        item.insert("type".into(), Value::String(obj));
        item.insert("title".into(), Value::String(title));
        item.insert("last_edited".into(), Value::String(edited));
        if let Some(u) = r.get("url").and_then(|u| u.as_str()) {
            item.insert("url".into(), Value::String(u.to_string()));
        }
        items.push(Value::Object(item));
    }

    // JSON mode returns an array of objects; text mode returns a table.
    if matches!(mode_from_json(), crate::output::RenderMode::Json) {
        Ok(Output::Json(Value::Array(items)))
    } else {
        Ok(Output::records(
            vec![
                "id".into(),
                "type".into(),
                "title".into(),
                "last_edited".into(),
            ],
            rows,
        ))
    }
}

/// `note list [--db ID] [--limit N]`: list pages under the given database.
///
/// Paginate via `POST /databases/{id}/query`, truncating to `--limit` (default 50, cap 100).
/// Target database prefers `--db`, falling back to the account's `default_database_id`.
async fn note_list(
    config: &Config,
    account: &NoteAccount,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let db_id = flags
        .get("db")
        .cloned()
        .or_else(|| account.default_database_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(
                "no --db given and no default_database_id set for this account".into(),
            )
        })?;
    let limit: usize = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(100);
    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    // Paginated query of pages under the database.
    let url = format!("/databases/{db_id}/query");
    let mut out: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut body = json!({ "page_size": 100u32 });
        if let Some(c) = &cursor {
            body["start_cursor"] = json!(c);
        }
        let v: Value = client.post(&url, &body).await?;
        if let Some(results) = v.get("results").and_then(|r| r.as_array()) {
            out.extend(results.iter().cloned());
        }
        // Keep paging until no more results, or until limit is reached (limit==0 means unlimited).
        let has_more = v.get("has_more").and_then(|h| h.as_bool()) == Some(true);
        if has_more && (limit == 0 || out.len() < limit) {
            match v.get("next_cursor").and_then(|c| c.as_str()) {
                Some(c) => cursor = Some(c.to_string()),
                None => break,
            }
        } else {
            break;
        }
    }
    if limit != 0 && out.len() > limit {
        out.truncate(limit);
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut items: Vec<Value> = Vec::new();
    for p in &out {
        let id = p
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string();
        let title = extract_title(p);
        let url = p
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        let edited = p
            .get("last_edited_time")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let props = p
            .get("properties")
            .and_then(|pr| pr.as_object())
            .cloned()
            .unwrap_or_default();
        let props_str = page_props_to_strings(&props);
        rows.push(vec![id.clone(), title.clone(), edited.clone()]);
        let mut item = Map::new();
        item.insert("id".into(), Value::String(id));
        item.insert("title".into(), Value::String(title));
        item.insert("url".into(), Value::String(url));
        item.insert("last_edited".into(), Value::String(edited));
        item.insert("properties".into(), Value::Object(props_str));
        items.push(Value::Object(item));
    }

    if matches!(mode_from_json(), crate::output::RenderMode::Json) {
        Ok(Output::Json(Value::Array(items)))
    } else {
        Ok(Output::records(
            vec!["id".into(), "title".into(), "last_edited".into()],
            rows,
        ))
    }
}

/// `note create --title T [--db ID] [--prop K:V ...]`: create a record in the database.
async fn note_create(
    config: &Config,
    account: &NoteAccount,
    flags: &HashMap<String, String>,
    multi: &[(String, String)],
) -> Result<Output> {
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("create requires --title <title>".into()))?;
    // Target database: prefer --db, otherwise the configured default.
    let db_id = flags
        .get("db")
        .cloned()
        .or_else(|| account.default_database_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(
                "no --db given and no default_database_id set for this account".into(),
            )
        })?;
    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    // Fetch the database schema to locate the title property name and validate --prop types.
    let schema: Value = client.get(&format!("/databases/{}", db_id)).await?;
    let title_prop = db_title_property_name(&schema)?;

    let mut props = Map::new();
    props.insert(
        title_prop,
        json!({ "title": [{ "text": { "content": title } }] }),
    );

    for (_, kv) in multi {
        let (k, v) = kv
            .split_once(':')
            .ok_or_else(|| AgentError::InvalidArgument(format!("prop must be K:V, got '{kv}'")))?;
        let ptype = db_property_type(&schema, k)?;
        props.insert(k.to_string(), encode_property(&ptype, v)?);
    }

    let body = json!({ "parent": { "database_id": db_id }, "properties": Value::Object(props) });
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

    let json_out = json!({
        "id": id,
        "url": url,
        "title": title,
        "database_id": db_id,
    });
    if matches!(mode_from_json(), crate::output::RenderMode::Json) {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "created page '{}' (id={}, database={})\n{}",
            title, id, db_id, url
        )))
    }
}

/// `note read [page_id]`: read page properties + body, aggregated into Markdown.
async fn note_read(
    config: &Config,
    account: &NoteAccount,
    _flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    let page_id = resolve_page_id(account, positional)?;
    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    let page: Value = client.get(&format!("/pages/{}", page_id)).await?;
    let title = extract_title(&page);
    let url = page
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let props = page
        .get("properties")
        .and_then(|p| p.as_object())
        .cloned()
        .unwrap_or_default();
    let props_str = page_props_to_strings(&props);

    // Body: recursively fetch all blocks and aggregate into Markdown.
    let blocks = fetch_all_blocks(&client, &page_id).await?;
    let content = blocks_to_markdown(&client, &blocks, 0).await?;

    let json_out = json!({
        "id": page_id,
        "title": title,
        "url": url,
        "properties": Value::Object(props_str),
        "content": content,
    });

    // JSON mode returns a structured object; text mode prints the aggregated Markdown.
    if matches!(mode_from_json(), crate::output::RenderMode::Json) {
        Ok(Output::Json(json_out))
    } else {
        let mut text = String::new();
        if !title.is_empty() {
            text.push_str(&format!("# {title}\n\n"));
        }
        if !url.is_empty() {
            text.push_str(&format!("({url})\n\n"));
        }
        text.push_str(&content);
        Ok(Output::text(text))
    }
}

/// `note append [page_id] --text TEXT`: append text blocks to the end of a page.
async fn note_append(
    config: &Config,
    account: &NoteAccount,
    flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    let page_id = resolve_page_id(account, positional)?;

    // Text source: prefer --text; otherwise read from stdin (only when stdin is not a TTY, to avoid blocking).
    let text = match flags.get("text") {
        Some(t) => t.clone(),
        None => {
            if std::io::stdin().is_terminal() {
                return Err(AgentError::InvalidArgument(
                    "append requires --text TEXT or piped stdin".into(),
                ));
            }
            read_stdin()?
        }
    };
    if text.trim().is_empty() {
        return Err(AgentError::InvalidArgument(
            "nothing to append (empty text)".into(),
        ));
    }

    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;
    let blocks = text_to_blocks(&text);
    if blocks.is_empty() {
        return Err(AgentError::InvalidArgument(
            "nothing to append (no blocks produced)".into(),
        ));
    }

    // Notion accepts at most 100 blocks per request; append in batches beyond that.
    let mut appended = 0usize;
    for chunk in blocks.chunks(100) {
        let body = json!({ "children": chunk });
        let _: Value = client
            .patch(&format!("/blocks/{}/children", page_id), &body)
            .await?;
        appended += chunk.len();
    }

    let url = format!("https://www.notion.so/{}", page_id.replace('-', ""));
    let json_out = json!({
        "id": page_id,
        "url": url,
        "appended": appended,
    });
    if matches!(mode_from_json(), crate::output::RenderMode::Json) {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "appended {} block(s) to page {}\n{}",
            appended, page_id, url
        )))
    }
}

/// `note update <page_id> --prop K:V ...`: modify page properties.
async fn note_update(
    config: &Config,
    account: &NoteAccount,
    _flags: &HashMap<String, String>,
    positional: &[String],
    multi: &[(String, String)],
) -> Result<Output> {
    let page_id = positional
        .first()
        .ok_or_else(|| AgentError::InvalidArgument("update requires <page_id>".into()))?
        .clone();
    if multi.is_empty() {
        return Err(AgentError::InvalidArgument(
            "update requires at least one --prop K:V".into(),
        ));
    }
    let token = get_token(config, account)?;
    let client = NotionClient::new(token)?;

    // Try to read the database schema from the page parent for precise property-type encoding;
    // fall back to heuristic encoding when there is no database parent (standalone page).
    let page: Value = client.get(&format!("/pages/{}", page_id)).await?;
    let schema_opt: Option<Value> = match page
        .get("parent")
        .and_then(|p| p.get("database_id"))
        .and_then(|d| d.as_str())
    {
        Some(db_id) => client
            .get::<Value>(&format!("/databases/{db_id}"))
            .await
            .ok(),
        None => None,
    };

    let mut props = Map::new();
    for (_, kv) in multi {
        let (k, v) = kv
            .split_once(':')
            .ok_or_else(|| AgentError::InvalidArgument(format!("prop must be K:V, got '{kv}'")))?;
        let encoded = match &schema_opt {
            Some(schema) => {
                let ptype = db_property_type(schema, k)?;
                encode_property(&ptype, v)?
            }
            None => encode_property_heuristic(v),
        };
        props.insert(k.to_string(), encoded);
    }

    let body = json!({ "properties": Value::Object(props) });
    let updated: Value = client.patch(&format!("/pages/{}", page_id), &body).await?;
    let id = updated
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string();
    let url = updated
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();

    let json_out = json!({
        "id": id,
        "url": url,
        "updated": multi.len(),
    });
    if matches!(mode_from_json(), crate::output::RenderMode::Json) {
        Ok(Output::Json(json_out))
    } else {
        Ok(Output::text(format!(
            "updated {} propert(ies) on page {}\n{}",
            multi.len(),
            page_id,
            url
        )))
    }
}

// ============ Small helpers ============

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

/// Read all of stdin.
fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| AgentError::Io(e.to_string()))?;
    Ok(buf)
}

/// Detect the current render mode (the global JSON flag injects `--json` into args; reuse the module-level check). See [R001](../../docs/adr/R001-thread-local-json-mode.md).
fn mode_from_json() -> crate::output::RenderMode {
    // note's actions decide their output shape internally, but `search`/`read` must branch by mode.
    // main injects `--json` into args and parse_args captures it as a flag, which we read here.
    // To avoid passing extra params, this function simply checks whether `--json` is present in the process args.
    if crate::util::json_mode::is_json() {
        crate::output::RenderMode::Json
    } else {
        crate::output::RenderMode::Text
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
    fn encode_property_types() {
        let t = encode_property("title", "hello").unwrap();
        assert_eq!(t["title"][0]["text"]["content"], "hello");
        let n = encode_property("number", "3.14").unwrap();
        assert_eq!(n["number"], "3.14".parse::<f64>().unwrap());
        let c = encode_property("checkbox", "true").unwrap();
        assert!(c["checkbox"].as_bool().unwrap());
        let s = encode_property("select", "未读").unwrap();
        assert_eq!(s["select"]["name"], "未读");
        assert!(encode_property("number", "abc").is_err());
    }

    #[test]
    fn parse_bool_variants() {
        for s in ["true", "yes", "1", "on"] {
            assert!(parse_bool(s).unwrap());
        }
        for s in ["false", "no", "0", "off"] {
            assert!(!parse_bool(s).unwrap());
        }
        assert!(parse_bool("maybe").is_err());
    }

    #[test]
    fn text_to_blocks_basic_markdown() {
        let text =
            "# 标题\n\n这是一段正文。\n\n- 项目一\n- 项目二\n\n```rust\nlet x = 1;\n```\n\n> 引用";
        let blocks = text_to_blocks(text);
        let types: Vec<&str> = blocks
            .iter()
            .filter_map(|b| b.get("type").and_then(|t| t.as_str()))
            .collect();
        assert!(types.contains(&"heading_1"));
        assert!(types.contains(&"paragraph"));
        assert!(types.contains(&"bulleted_list_item"));
        assert!(types.contains(&"code"));
        assert!(types.contains(&"quote"));
    }

    #[test]
    fn text_to_blocks_numbered_and_divider() {
        let text = "1. 第一\n2. 第二\n\n---\n\n## 小标题";
        let blocks = text_to_blocks(text);
        let types: Vec<&str> = blocks
            .iter()
            .filter_map(|b| b.get("type").and_then(|t| t.as_str()))
            .collect();
        assert!(types.contains(&"numbered_list_item"));
        assert!(types.contains(&"divider"));
        assert!(types.contains(&"heading_2"));
    }

    #[test]
    fn rich_text_plain_joins() {
        let rt = vec![
            json!({ "plain_text": "Hello " }),
            json!({ "plain_text": "World" }),
        ];
        assert_eq!(rich_text_plain(&rt), "Hello World");
    }

    #[test]
    fn rich_text_md_applies_formatting() {
        let rt = vec![json!({
            "plain_text": "bold",
            "href": null,
            "annotations": { "bold": true, "italic": false, "code": false, "strikethrough": false }
        })];
        assert_eq!(rich_text_md(&rt), "**bold**");

        let rt2 = vec![json!({
            "plain_text": "link",
            "href": "https://x.com",
            "annotations": { "bold": false, "italic": false, "code": false, "strikethrough": false }
        })];
        assert_eq!(rich_text_md(&rt2), "[link](https://x.com)");
    }

    #[test]
    fn extract_title_from_database() {
        let db = json!({
            "object": "database",
            "title": [{ "plain_text": "阅读清单" }]
        });
        assert_eq!(extract_title(&db), "阅读清单");
    }

    #[test]
    fn extract_title_from_page() {
        let page = json!({
            "object": "page",
            "properties": {
                "Name": { "type": "title", "title": [{ "plain_text": "我的页面" }] },
                "Status": { "type": "select", "select": { "name": "TODO" } }
            }
        });
        assert_eq!(extract_title(&page), "我的页面");
    }

    #[test]
    fn page_props_to_strings_maps_values() {
        let mut props = Map::new();
        props.insert(
            "Name".into(),
            json!({ "type": "title", "title": [{ "plain_text": "标题" }] }),
        );
        props.insert("Age".into(), json!({ "type": "number", "number": 30 }));
        props.insert(
            "Done".into(),
            json!({ "type": "checkbox", "checkbox": true }),
        );
        props.insert(
            "Tag".into(),
            json!({ "type": "select", "select": { "name": "工作" } }),
        );
        let m = page_props_to_strings(&props);
        assert_eq!(m["Name"], "标题");
        assert_eq!(m["Age"], "30");
        assert_eq!(m["Done"], "true");
        assert_eq!(m["Tag"], "工作");
    }

    #[test]
    fn db_property_type_lookup_and_missing() {
        let schema = json!({
            "properties": {
                "Name": { "type": "title" },
                "Status": { "type": "select" }
            }
        });
        assert_eq!(db_property_type(&schema, "Status").unwrap(), "select");
        assert!(db_property_type(&schema, "Missing").is_err());
        assert_eq!(db_title_property_name(&schema).unwrap(), "Name");
    }

    #[test]
    fn render_block_body_heading_and_code() {
        let h =
            json!({ "type": "heading_2", "heading_2": { "rich_text": [{ "plain_text": "Hi" }] } });
        assert_eq!(render_block_body("heading_2", &h), "## Hi\n\n");

        let code = json!({ "type": "code", "code": { "language": "rust", "rich_text": [{ "plain_text": "let x=1;" }] } });
        let out = render_block_body("code", &code);
        assert!(out.starts_with("```rust"));
        assert!(out.contains("let x=1;"));
        assert!(out.ends_with("```\n\n"));
    }

    #[test]
    fn render_block_body_todo_and_divider() {
        let todo = json!({ "type": "to_do", "to_do": { "checked": true, "rich_text": [{ "plain_text": "done" }] } });
        assert_eq!(render_block_body("to_do", &todo), "- [x] done\n");

        let todo2 = json!({ "type": "to_do", "to_do": { "checked": false, "rich_text": [{ "plain_text": "todo" }] } });
        assert_eq!(render_block_body("to_do", &todo2), "- [ ] todo\n");

        assert_eq!(
            render_block_body("divider", &json!({"type":"divider","divider":{}})),
            "---\n\n"
        );
    }
}
