//! 邮件模块：IMAP 收件（list / read / search）+ SMTP 发件（send）+ keyring 凭证（login）。
//!
//! 流程：配置文件存账户元数据（host/port/username）→ `everyday mail login` 存密码到
//! 系统密钥环 → `everyday mail list/read/search/send` 自动读取密码连接。
//! 密码绝不落盘 config.toml。

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::TryStreamExt;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use crate::config::{Config, MailAccount};
use crate::error::{AgentError, Result};
use crate::modules::{parse_simple_args, ActionDoc, Executor};
use crate::output::Output;

pub struct EmailModule {
    config: Arc<Config>,
}

impl EmailModule {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Executor for EmailModule {
    fn name(&self) -> &'static str {
        "mail"
    }

    fn description(&self) -> &'static str {
        "Email management (IMAP/SMTP): folders, list, read, search, send, login."
    }

    fn actions(&self) -> Vec<ActionDoc> {
        vec![
            ActionDoc::new("folders", "List all mailbox folders", "everyday mail folders [--account NAME]"),
            ActionDoc::new("list", "List messages (recursively across all folders by default)", "everyday mail list [--unread] [--limit N] [--folder NAME] [--no-recursive] [--account NAME]"),
            ActionDoc::new("read", "Read a single message", "everyday mail read <uid> [--folder NAME] [--account NAME]"),
            ActionDoc::new("search", "Search messages (recursively across all folders by default)", "everyday mail search --query Q [--limit N] [--folder NAME] [--no-recursive] [--account NAME]"),
            ActionDoc::new("send", "Send a message", "everyday mail send --to ADDR --subject S --body TEXT [--cc ADDR] [--account NAME]"),
            ActionDoc::new("login", "Store password in system keyring", "everyday mail login [--account NAME]"),
        ]
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let account = self
            .config
            .mail_account(flags.get("account").map(|s| s.as_str()))?;

        match action {
            // login 不需要密码，只需账户元数据 + 交互输入
            "login" => mail_login(account).await,
            _ => {
                let password = get_password(account)?;
                match action {
                    "folders" => mail_folders(account, &password).await,
                    "list" => mail_list(account, &password, &flags).await,
                    "read" => mail_read(account, &password, &flags, &positional).await,
                    "search" => mail_search(account, &password, &flags).await,
                    "send" => mail_send(account, &password, &flags).await,
                    other => Err(AgentError::UnknownAction(format!("mail {other}"))),
                }
            }
        }
    }
}

// ============ keyring 凭证 ============

/// 从系统密钥环读取账户密码。
fn get_password(account: &MailAccount) -> Result<String> {
    let service = Config::keyring_service("mail", &account.name);
    let entry = keyring::Entry::new(&service, &account.username)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    entry.get_password().map_err(|e| {
        AgentError::Auth(format!(
            "no password in keyring for mail account '{}': {e}. \
             Run `everyday mail login --account {}` to store it.",
            account.name, account.name
        ))
    })
}

/// 交互式输入密码并存入系统密钥环。
async fn mail_login(account: &MailAccount) -> Result<Output> {
    let service = Config::keyring_service("mail", &account.name);
    let entry = keyring::Entry::new(&service, &account.username)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    // rpassword 是同步的，放进 spawn_blocking 避免阻塞运行时。
    let username = account.username.clone();
    let account_name = account.name.clone();
    let password = tokio::task::spawn_blocking(move || {
        rpassword::prompt_password(format!("Password for {username}: "))
    })
    .await
    .map_err(|e| AgentError::Other(format!("join password prompt: {e}")))?
    .map_err(|e| AgentError::Other(format!("read password: {e}")))?;

    entry
        .set_password(&password)
        .map_err(|e| AgentError::Auth(format!("keyring set: {e}")))?;
    Ok(Output::text(format!(
        "password stored for mail account '{account_name}'"
    )))
}

// ============ IMAP 连接 ============

// async-imap 基于 futures 的 AsyncRead/AsyncWrite，而 tokio-rustls 实现的是 tokio 的，
// 用 tokio-util compat 桥接。
type ImapSession = async_imap::Session<Compat<TlsStream<TcpStream>>>;

/// 建立 IMAPS（implicit TLS, 993）连接并登录。
async fn imap_connect(account: &MailAccount, password: &str) -> Result<ImapSession> {
    // 安装 rustls ring crypto provider（重复安装返回 Err，忽略即可）。
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));

    let domain = &account.imap_host;
    let tcp = TcpStream::connect((domain.as_str(), account.imap_port))
        .await
        .map_err(|e| {
            AgentError::Network(format!(
                "imap connect to {domain}:{}: {e}",
                account.imap_port
            ))
        })?;
    let server_name = rustls::pki_types::ServerName::try_from(domain.clone())
        .map_err(|e| AgentError::Network(format!("invalid imap host '{domain}': {e}")))?;
    let tls_stream = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| AgentError::Network(format!("imap tls handshake: {e}")))?;

    let client = async_imap::Client::new(tls_stream.compat());
    let session = client
        .login(account.username.as_str(), password)
        .await
        .map_err(|e| AgentError::Auth(format!("imap login failed: {}", e.0)))?;
    Ok(session)
}

// ============ 动作实现 ============

/// 列出邮箱所有文件夹（IMAP LIST）。
async fn mail_folders(account: &MailAccount, password: &str) -> Result<Output> {
    let mut session = imap_connect(account, password).await?;
    let folders = list_all_folders(&mut session).await?;
    session.logout().await.ok();
    let rows = folders.into_iter().map(|f| vec![f]).collect();
    Ok(Output::records(vec!["folder".into()], rows))
}

/// 调用 IMAP LIST 列出所有文件夹名（过滤 \NoSelect）。
async fn list_all_folders(session: &mut ImapSession) -> Result<Vec<String>> {
    let names: Vec<async_imap::types::Name> = session
        .list(None, Some("*"))
        .await
        .map_err(|e| AgentError::Network(format!("list folders: {e}")))?
        .try_collect()
        .await
        .map_err(|e| AgentError::Network(format!("list collect: {e}")))?;
    let folders = names
        .iter()
        .filter(|n| {
            // 跳过标记为 \NoSelect 的文件夹（无法 SELECT）
            !n.attributes()
                .iter()
                .any(|a| matches!(a, async_imap::types::NameAttribute::NoSelect))
        })
        .map(|n| n.name().to_string())
        .collect();
    Ok(folders)
}

/// 根据命令行 flags 解析要遍历的文件夹列表。
/// - `--folder NAME`：仅该文件夹
/// - 默认（递归）：所有文件夹
/// - `--no-recursive`：仅 INBOX
async fn resolve_folders(
    session: &mut ImapSession,
    flags: &HashMap<String, String>,
) -> Result<Vec<String>> {
    if let Some(f) = flags.get("folder") {
        return Ok(vec![f.clone()]);
    }
    if flags.contains_key("no-recursive") {
        return Ok(vec!["INBOX".to_string()]);
    }
    list_all_folders(session).await
}

/// 跨多个文件夹收集邮件摘要，合并、按 UID 降序、截断到 limit。
/// 无法 SELECT 的文件夹（如 \NoSelect）会被跳过。
async fn collect_across_folders(
    session: &mut ImapSession,
    folders: Vec<String>,
    search_query: &str,
    limit: usize,
) -> Result<Vec<Vec<String>>> {
    let mut all_rows: Vec<Vec<String>> = Vec::new();
    for folder in &folders {
        if session.select(folder).await.is_err() {
            continue; // 跳过无法选中的文件夹
        }
        let uids = match search_uids(session, search_query).await {
            Ok(u) => u,
            Err(_) => continue, // 单个文件夹搜索失败不致命
        };
        let rows = fetch_summaries(session, uids, limit, folder).await?;
        all_rows.extend(rows);
        if all_rows.len() >= limit {
            break;
        }
    }
    // 全局按 UID 降序（不同文件夹 UID 可能重复，folder 列区分来源）
    all_rows.sort_by_key(|r| {
        Reverse(r.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0))
    });
    all_rows.truncate(limit);
    Ok(all_rows)
}

/// 列出邮件摘要（默认递归所有文件夹）。
async fn mail_list(
    account: &MailAccount,
    password: &str,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let unread = flags.contains_key("unread");
    let limit: usize = flags.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut session = imap_connect(account, password).await?;
    let folders = resolve_folders(&mut session, flags).await?;
    let query = if unread { "UNSEEN" } else { "ALL" };
    let rows = collect_across_folders(&mut session, folders, query, limit).await?;
    session.logout().await.ok();

    Ok(Output::records(
        vec!["uid".into(), "folder".into(), "date".into(), "from".into(), "subject".into()],
        rows,
    ))
}

/// 读取单封邮件完整内容。
async fn mail_read(
    account: &MailAccount,
    password: &str,
    flags: &HashMap<String, String>,
    positional: &[String],
) -> Result<Output> {
    let uid_str = positional
        .first()
        .or_else(|| flags.get("id"))
        .ok_or_else(|| AgentError::InvalidArgument("usage: everyday mail read <uid>".into()))?;
    let uid: u32 = uid_str
        .parse()
        .map_err(|_| AgentError::InvalidArgument("uid must be a number".into()))?;
    let folder = flags
        .get("folder")
        .cloned()
        .unwrap_or_else(|| "INBOX".to_string());

    let mut session = imap_connect(account, password).await?;
    session
        .select(&folder)
        .await
        .map_err(|e| AgentError::Network(format!("select {folder}: {e}")))?;

    let fetches: Vec<async_imap::types::Fetch> = session
        .uid_fetch(uid.to_string(), "(UID BODY[])")
        .await
        .map_err(|e| AgentError::Network(format!("fetch: {e}")))?
        .try_collect()
        .await
        .map_err(|e| AgentError::Network(format!("fetch collect: {e}")))?;
    session.logout().await.ok();

    let fetch = fetches
        .into_iter()
        .next()
        .ok_or_else(|| AgentError::Other(format!("no message with uid {uid}")))?;
    let body = fetch
        .body()
        .ok_or_else(|| AgentError::Other("message has no body".into()))?;

    let parsed = mailparse::parse_mail(body)
        .map_err(|e| AgentError::Other(format!("parse mail: {e}")))?;
    let subject = header_value(&parsed, "Subject");
    let from = header_value(&parsed, "From");
    let date = header_value(&parsed, "Date");
    let text = parsed.get_body().unwrap_or_default();

    Ok(Output::Records {
        headers: vec!["field".into(), "value".into()],
        rows: vec![
            vec!["subject".into(), subject],
            vec!["from".into(), from],
            vec!["date".into(), date],
            vec!["body".into(), text],
        ],
    })
}

/// 搜索邮件（默认递归所有文件夹）。
async fn mail_search(
    account: &MailAccount,
    password: &str,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let query = flags
        .get("query")
        .ok_or_else(|| AgentError::InvalidArgument("usage: everyday mail search --query Q".into()))?;
    let limit: usize = flags.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20);

    let mut session = imap_connect(account, password).await?;
    let folders = resolve_folders(&mut session, flags).await?;
    // IMAP SEARCH TEXT "query" —— 转义双引号与反斜杠
    let escaped = query.replace('\\', "\\\\").replace('"', "\\\"");
    let search = format!("TEXT \"{escaped}\"");
    let rows = collect_across_folders(&mut session, folders, &search, limit).await?;
    session.logout().await.ok();

    Ok(Output::records(
        vec!["uid".into(), "folder".into(), "date".into(), "from".into(), "subject".into()],
        rows,
    ))
}

/// 发送邮件（SMTP via lettre，STARTTLS）。
async fn mail_send(
    account: &MailAccount,
    password: &str,
    flags: &HashMap<String, String>,
) -> Result<Output> {
    let to = flags
        .get("to")
        .ok_or_else(|| AgentError::InvalidArgument("--to <addr> is required".into()))?;
    let subject = flags
        .get("subject")
        .ok_or_else(|| AgentError::InvalidArgument("--subject <text> is required".into()))?;
    let body = flags
        .get("body")
        .ok_or_else(|| AgentError::InvalidArgument("--body <text> is required".into()))?;

    use lettre::message::{header::ContentType, Mailbox};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let from: Mailbox = account.username.parse().map_err(|e| {
        AgentError::InvalidArgument(format!(
            "invalid from address '{}': {e}",
            account.username
        ))
    })?;
    let to_mb: Mailbox = to
        .parse()
        .map_err(|e| AgentError::InvalidArgument(format!("invalid to address '{to}': {e}")))?;

    let mut builder = Message::builder().from(from).to(to_mb);
    if let Some(cc) = flags.get("cc") {
        let cc_mb: Mailbox = cc
            .parse()
            .map_err(|e| AgentError::InvalidArgument(format!("invalid cc address '{cc}': {e}")))?;
        builder = builder.cc(cc_mb);
    }
    let email = builder
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.clone())
        .map_err(|e| AgentError::InvalidArgument(format!("build email: {e}")))?;

    let transport = AsyncSmtpTransport::<Tokio1Executor>::relay(&account.smtp_host)
        .map_err(|e| AgentError::Network(format!("smtp relay '{}': {e}", account.smtp_host)))?
        .port(account.smtp_port)
        .credentials(Credentials::new(
            account.username.clone(),
            password.to_string(),
        ))
        .build();
    transport
        .send(email)
        .await
        .map_err(|e| AgentError::Network(format!("smtp send: {e}")))?;

    Ok(Output::text(format!("sent to {to}")))
}

// ============ 辅助函数 ============

/// 执行 IMAP SEARCH，返回 UID 列表（降序，最近的在前）。
async fn search_uids(session: &mut ImapSession, query: &str) -> Result<Vec<u32>> {
    let set: std::collections::HashSet<u32> = session
        .uid_search(query)
        .await
        .map_err(|e| AgentError::Network(format!("search '{query}': {e}")))?;
    let mut uids: Vec<u32> = set.into_iter().collect();
    uids.sort_unstable_by_key(|&u| Reverse(u));
    Ok(uids)
}

/// 按 UID 批量 fetch 摘要，限制条数，返回表格行（含 folder 列）。
async fn fetch_summaries(
    session: &mut ImapSession,
    mut uids: Vec<u32>,
    limit: usize,
    folder: &str,
) -> Result<Vec<Vec<String>>> {
    uids.truncate(limit);
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let uid_set = uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fetches: Vec<async_imap::types::Fetch> = session
        .uid_fetch(uid_set, "(UID ENVELOPE FLAGS)")
        .await
        .map_err(|e| AgentError::Network(format!("fetch: {e}")))?
        .try_collect()
        .await
        .map_err(|e| AgentError::Network(format!("fetch collect: {e}")))?;

    let mut rows: Vec<(u32, Vec<String>)> = Vec::with_capacity(fetches.len());
    for f in &fetches {
        let uid = f.uid.unwrap_or(0);
        let env = f.envelope();
        // Envelope 字段是 Option<Cow<[u8]>>（IMAP 返回原始字节），需转 String。
        let date = env
            .and_then(|e| e.date.as_deref())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let subject = env
            .and_then(|e| e.subject.as_deref())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let from = env
            .and_then(|e| e.from.as_ref().and_then(|a| a.first()))
            .map(|a| {
                let m = a
                    .mailbox
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).into_owned());
                let h = a
                    .host
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).into_owned());
                format_mailbox(m.as_deref(), h.as_deref())
            })
            .unwrap_or_default();
        rows.push((
            uid,
            vec![
                uid.to_string(),
                folder.to_string(),
                decode_mime_header(&date),
                from,
                decode_mime_header(&subject),
            ],
        ));
    }
    // 按 UID 降序排列（fetch 顺序不保证）
    rows.sort_by_key(|r| Reverse(r.0));
    Ok(rows.into_iter().map(|(_, r)| r).collect())
}

/// 格式化邮箱地址为 `mailbox@host`。
fn format_mailbox(mailbox: Option<&str>, host: Option<&str>) -> String {
    let m = mailbox.unwrap_or("");
    let h = host.unwrap_or("");
    if m.is_empty() && h.is_empty() {
        "(unknown)".to_string()
    } else {
        format!("{m}@{h}")
    }
}

/// 解码 MIME encoded-word（=?charset?B/Q?...?=）。
fn decode_mime_header(s: &str) -> String {
    if !s.contains("=?") {
        return s.to_string();
    }
    // 借用 mailparse 解码：构造一个伪 header 让它解析。
    let fake = format!("X-Decoded: {s}\r\n\r\n");
    if let Ok(parsed) = mailparse::parse_mail(fake.as_bytes()) {
        for h in &parsed.headers {
            if h.get_key().eq_ignore_ascii_case("X-Decoded") {
                return h.get_value();
            }
        }
    }
    s.to_string()
}

/// 从已解析邮件中取指定 header 的值（不区分大小写）。
fn header_value(parsed: &mailparse::ParsedMail, key: &str) -> String {
    parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case(key))
        .map(|h| h.get_value())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_plain_header_unchanged() {
        assert_eq!(decode_mime_header("hello world"), "hello world");
    }

    #[test]
    fn decode_utf8_base64_subject() {
        // =?UTF-8?B?5L2g5aW9? = "你好"
        let s = "=?UTF-8?B?5L2g5aW9?=";
        assert_eq!(decode_mime_header(s), "你好");
    }

    #[test]
    fn header_value_case_insensitive() {
        let raw = b"Subject: Hi\r\n\r\nbody";
        let parsed = mailparse::parse_mail(raw).unwrap();
        assert_eq!(header_value(&parsed, "subject"), "Hi");
        assert_eq!(header_value(&parsed, "SUBJECT"), "Hi");
        assert_eq!(header_value(&parsed, "missing"), "");
    }

    #[test]
    fn format_mailbox_normal() {
        assert_eq!(
            format_mailbox(Some("me"), Some("example.com")),
            "me@example.com"
        );
    }

    #[test]
    fn format_mailbox_empty() {
        assert_eq!(format_mailbox(None, None), "(unknown)");
    }

    #[test]
    fn search_query_escapes_quotes() {
        // 验证转义逻辑（间接：构造与断言）
        let q = "he said \"hi\"";
        let escaped = q.replace('\\', "\\\\").replace('"', "\\\"");
        assert_eq!(escaped, "he said \\\"hi\\\"");
    }
}
