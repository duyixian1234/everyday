//! 笔记模块：基于 Notion API 的笔记 / 知识库管理。
//!
//! 设计目标：屏蔽 Notion 繁琐的 Block 嵌套，向 Agent 暴露**纯文本/Markdown 追加**
//! 与**简化版属性操作**两个高层能力。
//!
//! 支持的 `action`：
//! - `login`   交互式把 Notion Integration Token 存入密钥环
//! - `search`  按标题关键词搜索页面 / 数据库
//! - `create`  在指定数据库新建一条记录（带标题与若干简化属性）
//! - `read`    读取页面正文，自动聚合为 Markdown（`--json` 下返回结构化对象）
//! - `append`  向页面末尾追加文本区块（支持 `--text` 或管道 stdin）
//! - `update`  修改页面属性（Meta 信息）
//! - `list`    列出指定数据库下的全部页面（标题 + 属性）
//!
//! 凭证安全：Token 仅存系统密钥环（service = `everyday/note/<account>`），绝不落盘 config。

use std::collections::HashMap;
use std::io::{IsTerminal, Read};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::config::NoteAccount;
use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor};
use crate::output::Output;

/// Notion REST API 基址。
const NOTION_API: &str = "https://api.notion.com/v1";
/// 使用的 Notion API 版本（固定，向后兼容）。
const NOTION_VERSION: &str = "2022-06-28";
/// 密钥环中存放 token 的条目用户名（与账户名无关，同 service 下唯一）。
const KEYRING_USER: &str = "token";

/// 递归渲染 block 的最大深度，防止异常数据导致无限展开。
const MAX_BLOCK_DEPTH: usize = 12;

/// `parse_args` 的返回类型别名（避免复杂元组类型直接内联）。
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
    fn name(&self) -> &'static str {
        "note"
    }

    fn description(&self) -> &'static str {
        "Note & knowledge-base (Notion): login, search, list, create, read, append, update."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new(
                "login",
                "Store Notion Integration Token in system keyring",
                "everyday note login [--account NAME]",
            ),
            ActionDoc::new(
                "search",
                "Search pages/databases by title",
                "everyday note search --query Q [--limit N]",
            ),
            ActionDoc::new(
                "create",
                "Create a page (record) in a database",
                "everyday note create --title T [--db ID] [--prop K:V ...]",
            ),
            ActionDoc::new(
                "read",
                "Read a page and render its content as Markdown",
                "everyday note read <page_id> [--account NAME]",
            ),
            ActionDoc::new(
                "append",
                "Append text/markdown blocks to a page",
                "everyday note append [page_id] --text TEXT",
            ),
            ActionDoc::new(
                "update",
                "Update a page's properties (metadata)",
                "everyday note update <page_id> --prop K:V ...",
            ),
            ActionDoc::new(
                "list",
                "List pages in a database",
                "everyday note list [--db ID] [--limit N]",
            ),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, multi, positional) = parse_args(args);
        let account = self
            .config
            .note_account(flags.get("account").map(|s| s.as_str()))?;

        match action {
            "login" => note_login(account).await,
            "search" => note_search(account, &flags).await,
            "create" => note_create(account, &flags, &multi).await,
            "read" => note_read(account, &flags, &positional).await,
            "append" => note_append(account, &flags, &positional).await,
            "update" => note_update(account, &flags, &positional, &multi).await,
            "list" => note_list(account, &flags).await,
            other => Err(AgentError::UnknownAction(format!("note {other}"))),
        }
    }
}

// ============ 参数解析 ============
//
// 与 `parse_simple_args` 不同，note 的 `--prop` 允许重复出现，且值内含冒号，
// 因此实现专用解析：单值 flag 取最后一次，重复 flag（如 prop）单独收集为有序列表。

/// 解析结果：`(单值 flags, 重复 flag 列表, 位置参数)`。
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

/// 把 flag 放入单值 map；`prop` 等重复 flag 同时放入 `multi` 列表。
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

// ============ 凭证（keyring） ============

/// 从密钥环读取 Notion Token。
fn get_token(account: &NoteAccount) -> Result<String> {
    let service = crate::config::Config::keyring_service("note", &account.name);
    let entry = keyring::Entry::new(&service, KEYRING_USER)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry.get_password().map_err(|e| {
        AgentError::Auth(format!(
            "no Notion token in keyring for note account '{}': {e}. \
             Run `everyday note login --account {}` to store it.",
            account.name, account.name
        ))
    })
}

/// 交互式输入 Token 并存入密钥环。
async fn note_login(account: &NoteAccount) -> Result<Output> {
    let service = crate::config::Config::keyring_service("note", &account.name);
    let entry = keyring::Entry::new(&service, KEYRING_USER)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    let account_name = account.name.clone();
    let prompt = format!(
        "Paste Notion Integration Token (ntn_...) for account '{}': ",
        account.name
    );
    // rpassword 为同步 API，放进 spawn_blocking 避免阻塞运行时。
    let password = tokio::task::spawn_blocking(move || rpassword::prompt_password(prompt))
        .await
        .map_err(|e| AgentError::Other(format!("join token prompt: {e}")))?
        .map_err(|e| AgentError::Other(format!("read token: {e}")))?;
    let token = password.trim().to_string();
    if token.is_empty() {
        return Err(AgentError::InvalidArgument(
            "token must not be empty".into(),
        ));
    }
    entry
        .set_password(&token)
        .map_err(|e| AgentError::Auth(format!("keyring set: {e}")))?;
    Ok(Output::text(format!(
        "Notion token stored for note account '{account_name}'"
    )))
}

// ============ HTTP 封装 ============

/// 构建带超时与 UA 的 reqwest 客户端（复用 main.rs 安装的 rustls ring provider）。
fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(format!("everyday/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AgentError::Network(format!("build http client: {e}")))
}

/// 发起 Notion API 请求（JSON body），成功返回解析后的 JSON Value。
///
/// 状态码非 2xx 时：401/403 映射为 `Auth`，其余映射为 `Network`，并尽量从响应体
/// 提取 `message` 字段。
async fn notion_request(
    method: reqwest::Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<Value> {
    let client = build_client()?;
    let url = format!("{NOTION_API}{path}");
    let mut req = client
        .request(method, &url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Notion-Version", NOTION_VERSION)
        .header("Content-Type", "application/json");
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AgentError::Network(format!("notion request failed: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AgentError::Network(format!("read response body: {e}")))?;
    if !status.is_success() {
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| text.clone());
        if status == 401 || status == 403 {
            return Err(AgentError::Auth(format!(
                "Notion API auth failed ({}): {}",
                status, msg
            )));
        }
        return Err(AgentError::Network(format!(
            "Notion API error ({}): {}",
            status, msg
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| AgentError::Other(format!("parse notion response: {e}")))
}

async fn api_get(path: &str, token: &str) -> Result<Value> {
    notion_request(reqwest::Method::GET, path, token, None).await
}

async fn api_post(path: &str, token: &str, body: Value) -> Result<Value> {
    notion_request(reqwest::Method::POST, path, token, Some(body)).await
}

async fn api_patch(path: &str, token: &str, body: Value) -> Result<Value> {
    notion_request(reqwest::Method::PATCH, path, token, Some(body)).await
}

/// 分页拉取某 block 的全部子 block（用于 `read` 内容聚合）。
async fn fetch_all_blocks(token: &str, block_id: &str) -> Result<Vec<Value>> {
    let client = build_client()?;
    let url = format!("{NOTION_API}/blocks/{block_id}/children");
    let mut out: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut rb = client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Notion-Version", NOTION_VERSION)
            .query(&[("page_size", "100")]);
        if let Some(c) = &cursor {
            rb = rb.query(&[("start_cursor", c.as_str())]);
        }
        let resp = rb
            .send()
            .await
            .map_err(|e| AgentError::Network(format!("fetch blocks: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AgentError::Network(format!("read blocks body: {e}")))?;
        if !status.is_success() {
            return Err(AgentError::Network(format!(
                "Notion API error ({}) while fetching blocks: {}",
                status, text
            )));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AgentError::Other(format!("parse blocks: {e}")))?;
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

// ============ 富文本 / 标题提取 ============

/// 把 Notion rich_text 数组渲染为纯文本（不带格式）。
fn rich_text_plain(rt: &[Value]) -> String {
    rt.iter()
        .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

/// 把 Notion rich_text 数组渲染为带行内格式的 Markdown（bold/italic/code/strike + 链接）。
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

/// 从搜索/页面对象中抽取标题：
/// - database：顶层 `title` 数组
/// - page：在 `properties` 中找 type == "title" 的属性
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

// ============ 属性编码（create / update 共用） ============

/// 根据属性类型把字符串值编码为 Notion property value。
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
        // 未知类型（公式/关系/文件等）降级为富文本，避免直接报错阻断。
        _ => Ok(json!({ "rich_text": [{ "text": { "content": value } }] })),
    }
}

/// 把字符串解析为布尔（支持 true/false/yes/no/1/0，大小写不敏感）。
fn parse_bool(s: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(AgentError::InvalidArgument(format!(
            "not a boolean (expected true/false): {s}"
        ))),
    }
}

/// 在数据库 schema 中查找属性类型；找不到则报错并列出可用属性。
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

/// 在数据库 schema 中找到 title 类型的属性名（create 时用它承载 `--title`）。
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

/// 无需 schema 时的启发式编码（用于无 database 父级的独立页面 update）。
fn encode_property_heuristic(value: &str) -> Value {
    if let Ok(b) = parse_bool(value) {
        return json!({ "checkbox": b });
    }
    if let Ok(n) = value.parse::<f64>() {
        return json!({ "number": n });
    }
    json!({ "rich_text": [{ "text": { "content": value } }] })
}

/// 把页面 properties 简化为 `name -> 字符串值` 的 map（用于 read 的 JSON 输出）。
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

/// 把纯文本 / Markdown 文本切分为 Notion block 数组（Markdown-lite 解析）。
///
/// 支持的语法：```代码块```、`#/##/###` 标题、`- /*` 无序列表、`1.` 有序列表、
/// `> ` 引用、`---` 分割线、空行分隔段落。其余作为普通段落。
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

        // 代码块围栏。
        if trimmed.starts_with("```") {
            if !in_code {
                in_code = true;
                code_lang = trimmed.trim_start_matches('`').trim().to_string();
            } else {
                // 结束围栏：输出 code block。
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

        // 分割线。
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            flush_para(&mut para, &mut blocks);
            blocks.push(json!({ "object": "block", "type": "divider", "divider": {} }));
            i += 1;
            continue;
        }

        // 标题。
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

        // 引用。
        if let Some(rest) = trimmed.strip_prefix("> ") {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("quote", rest));
            i += 1;
            continue;
        }

        // 无序列表。
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("bulleted_list_item", rest));
            i += 1;
            continue;
        }

        // 有序列表。
        if let Some(rest) = trimmed
            .split_once(". ")
            .filter(|(n, _)| n.parse::<usize>().is_ok())
        {
            flush_para(&mut para, &mut blocks);
            blocks.push(block_text("numbered_list_item", rest.1));
            i += 1;
            continue;
        }

        // 空行：段落分隔。
        if trimmed.is_empty() {
            flush_para(&mut para, &mut blocks);
            i += 1;
            continue;
        }

        // 普通文本行：累积进段落。
        para.push(line.to_string());
        i += 1;
    }
    // 收尾。
    if in_code {
        // 未闭合的代码块：按纯文本兜底。
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

/// 构造带单一 rich_text 的 block（用于标题/列表/引用/段落）。
fn block_text(block_type: &str, text: &str) -> Value {
    json!({
        "object": "block",
        "type": block_type,
        block_type: { "rich_text": rt_from_str(text) }
    })
}

/// 由纯字符串构造 Notion rich_text 数组。
fn rt_from_str(s: &str) -> Vec<Value> {
    if s.is_empty() {
        return vec![];
    }
    vec![json!({ "type": "text", "text": { "content": s }, "plain_text": s })]
}

// ============ Block -> Markdown（递归聚合） ============

/// 把页面所有 block 递归渲染为 Markdown，作为 `read` 的正文聚合结果。
async fn blocks_to_markdown(token: &str, blocks: &[Value], depth: usize) -> Result<String> {
    let mut out = String::new();
    for b in blocks {
        let block_type = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let body = render_block_body(block_type, b);
        out.push_str(&body);

        // 递归子 block（toggle / 嵌套列表 / 列表列等）。
        if b.get("has_children").and_then(|h| h.as_bool()) == Some(true)
            && depth < MAX_BLOCK_DEPTH
            && let Some(id) = b.get("id").and_then(|i| i.as_str())
        {
            let children = fetch_all_blocks(token, id).await?;
            let child_md = Box::pin(blocks_to_markdown(token, &children, depth + 1)).await?;
            // 子列表缩进 2 空格，保持层级结构。
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

/// 渲染单个 block 的正文（不含子 block），返回带末尾换行的字符串。
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
        // 其余类型：尽力渲染其 rich_text，避免内容丢失。
        _ => {
            // 多数块类型把文本放在与 type 同名的子对象里。
            if let Some(obj) = b.get(block_type)
                && let Some(rt) = obj.get("rich_text").and_then(|r| r.as_array())
            {
                return format!("{}\n\n", rich_text_md(rt));
            }
            String::new()
        }
    }
}

/// 取出 block 子对象中的 rich_text 数组（无则空）。
fn rt_of(b: &Value, block_type: &str) -> Vec<Value> {
    b.get(block_type)
        .and_then(|o| o.get("rich_text"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 提取媒体类 block 的 URL 与说明文字（image/bookmark/file/embed/video/pdf/audio）。
fn media_url_and_caption(b: &Value, block_type: &str) -> (String, String) {
    let sub = match b.get(block_type) {
        Some(s) => s,
        None => return (String::new(), String::new()),
    };
    // 媒体内容在 sub["type"] 指向的子字段（external / file）里，或直接有 url。
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

// ============ 动作实现 ============

/// `note search --query Q [--limit N]`：按标题搜索页面/数据库。
async fn note_search(account: &NoteAccount, flags: &HashMap<String, String>) -> Result<Output> {
    let query = flags
        .get("query")
        .ok_or_else(|| AgentError::InvalidArgument("search requires --query <keyword>".into()))?;
    let limit: usize = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
        .min(100);
    let token = get_token(account)?;

    let body = json!({ "query": query, "page_size": limit });
    let resp = api_post("/search", &token, body).await?;
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

    // JSON 模式返回对象数组；文本模式返回表格。
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

/// `note list [--db ID] [--limit N]`：列出指定数据库下的页面。
///
/// 通过 `POST /databases/{id}/query` 分页拉取，自动截断到 `--limit`（默认 50，上限 100）。
/// 目标数据库优先 `--db`，否则取账户配置的 `default_database_id`。
async fn note_list(account: &NoteAccount, flags: &HashMap<String, String>) -> Result<Output> {
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
    let token = get_token(account)?;

    // 分页查询数据库下的页面。
    let client = build_client()?;
    let url = format!("{NOTION_API}/databases/{db_id}/query");
    let mut out: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut body = json!({ "page_size": 100u32 });
        if let Some(c) = &cursor {
            body["start_cursor"] = json!(c);
        }
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Notion-Version", NOTION_VERSION)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AgentError::Network(format!("query database: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AgentError::Network(format!("read query body: {e}")))?;
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| text.clone());
            if status == 401 || status == 403 {
                return Err(AgentError::Auth(format!(
                    "Notion API auth failed ({}): {}",
                    status, msg
                )));
            }
            return Err(AgentError::Network(format!(
                "Notion API error ({}) while querying database: {}",
                status, msg
            )));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AgentError::Other(format!("parse query response: {e}")))?;
        if let Some(results) = v.get("results").and_then(|r| r.as_array()) {
            out.extend(results.iter().cloned());
        }
        // 继续翻页直到没有更多、或已达到 limit（limit==0 表示不限制）。
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

/// `note create --title T [--db ID] [--prop K:V ...]`：在数据库中新建记录。
async fn note_create(
    account: &NoteAccount,
    flags: &HashMap<String, String>,
    multi: &[(String, String)],
) -> Result<Output> {
    let title = flags
        .get("title")
        .ok_or_else(|| AgentError::InvalidArgument("create requires --title <title>".into()))?;
    // 目标数据库：优先 --db，否则配置默认。
    let db_id = flags
        .get("db")
        .cloned()
        .or_else(|| account.default_database_id.clone())
        .ok_or_else(|| {
            AgentError::InvalidArgument(
                "no --db given and no default_database_id set for this account".into(),
            )
        })?;
    let token = get_token(account)?;

    // 取数据库 schema 以定位 title 属性名 + 校验 --prop 类型。
    let schema = api_get(&format!("/databases/{}", db_id), &token).await?;
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
    let created = api_post("/pages", &token, body).await?;
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

/// `note read [page_id]`：读取页面属性 + 正文，聚合为 Markdown。
async fn note_read(
    account: &NoteAccount,
    _flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    let page_id = resolve_page_id(account, positional)?;
    let token = get_token(account)?;

    let page = api_get(&format!("/pages/{}", page_id), &token).await?;
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

    // 正文：递归拉取全部 block 并聚合为 Markdown。
    let blocks = fetch_all_blocks(&token, &page_id).await?;
    let content = blocks_to_markdown(&token, &blocks, 0).await?;

    let json_out = json!({
        "id": page_id,
        "title": title,
        "url": url,
        "properties": Value::Object(props_str),
        "content": content,
    });

    // JSON 模式返回结构化对象；文本模式直接打印聚合后的 Markdown。
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

/// `note append [page_id] --text TEXT`：向页面末尾追加文本区块。
async fn note_append(
    account: &NoteAccount,
    flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    let page_id = resolve_page_id(account, positional)?;

    // 文本来源：--text 优先；否则从 stdin 读取（仅当 stdin 非终端，避免挂起）。
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

    let token = get_token(account)?;
    let blocks = text_to_blocks(&text);
    if blocks.is_empty() {
        return Err(AgentError::InvalidArgument(
            "nothing to append (no blocks produced)".into(),
        ));
    }

    // Notion 单次最多 100 个 block，超出分批追加。
    let mut appended = 0usize;
    for chunk in blocks.chunks(100) {
        let body = json!({ "children": chunk });
        api_patch(&format!("/blocks/{}/children", page_id), &token, body).await?;
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

/// `note update <page_id> --prop K:V ...`：修改页面属性。
async fn note_update(
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
    let token = get_token(account)?;

    // 尝试从页面父级拿到数据库 schema 以精确编码属性类型；
    // 无数据库父级（独立页面）时退化为启发式编码。
    let page = api_get(&format!("/pages/{}", page_id), &token).await?;
    let schema_opt: Option<Value> = match page
        .get("parent")
        .and_then(|p| p.get("database_id"))
        .and_then(|d| d.as_str())
    {
        Some(db_id) => api_get(&format!("/databases/{db_id}"), &token).await.ok(),
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
    let updated = api_patch(&format!("/pages/{}", page_id), &token, body).await?;
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

// ============ 小工具 ============

/// 从位置参数或账户默认配置解析 page_id。
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

/// 从 stdin 读取全部内容。
fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| AgentError::Io(e.to_string()))?;
    Ok(buf)
}

/// 探测当前渲染模式（JSON 全局 flag 已注入 `--json` 到 args，这里复用模块层的判别）。
fn mode_from_json() -> crate::output::RenderMode {
    // note 的动作内部直接决定输出形态，但 `search`/`read` 需要按模式分支。
    // 由于 main 已把 `--json` 注入 args 并被 parse_args 捕获到 flags，这里读取它。
    // 为避免重复传参，本函数通过检查进程参数中是否含 `--json` 简单判断。
    if std::env::args().any(|a| a == "--json") {
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
        // 单值 flag 取最后一次 prop 的值（仅用于回退，真实逻辑用 multi）。
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
