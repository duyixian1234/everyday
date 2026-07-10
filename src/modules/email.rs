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
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use crate::config::{Config, MailAccount};
use crate::error::{AgentError, Result};
use crate::modules::{ActionDoc, Executor, parse_simple_args};
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
            ActionDoc::new(
                "folders",
                "List all mailbox folders",
                "everyday mail folders [--account NAME]",
            ),
            ActionDoc::new(
                "list",
                "List messages (recursively across all folders by default)",
                "everyday mail list [--unread] [--limit N] [--folder NAME] [--no-recursive] [--account NAME]",
            ),
            ActionDoc::new(
                "read",
                "Read a single message (searches all folders by default)",
                "everyday mail read <uid> [--folder NAME] [--no-recursive] [--account NAME]",
            ),
            ActionDoc::new(
                "search",
                "Search messages (recursively across all folders by default)",
                "everyday mail search --query Q [--limit N] [--folder NAME] [--no-recursive] [--account NAME]",
            ),
            ActionDoc::new(
                "send",
                "Send a message",
                "everyday mail send --to ADDR --subject S --body TEXT [--cc ADDR] [--account NAME]",
            ),
            ActionDoc::new(
                "login",
                "Store password in system keyring",
                "everyday mail login [--account NAME]",
            ),
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
    let rows = folders
        .into_iter()
        .map(|f| vec![decode_imap_utf7(&f)])
        .collect();
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
        // select_folder 兼容原始编码名与解码后的中文名（用户 --folder 输入）
        if select_folder(session, folder).await.is_err() {
            continue; // 跳过无法选中的文件夹
        }
        let uids = match search_uids(session, search_query).await {
            Ok(u) => u,
            Err(_) => continue, // 单个文件夹搜索失败不致命
        };
        // 显示用解码后的中文名，select 用原始编码名
        let display_folder = decode_imap_utf7(folder);
        // 每个文件夹取最近 limit 条作为全局候选，不提前 break —— 确保所有文件夹都参与
        let rows = fetch_summaries(session, uids, limit, &display_folder).await?;
        all_rows.extend(rows);
    }
    // 全局按邮件日期降序（跨文件夹 UID 不连续，日期更准确）
    all_rows.sort_by(|a, b| {
        let da = a.get(2).and_then(|s| parse_mail_date(s));
        let db = b.get(2).and_then(|s| parse_mail_date(s));
        match (da, db) {
            (Some(da), Some(db)) => db.cmp(&da),
            (Some(_), None) => std::cmp::Ordering::Less, // 有日期的排前
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
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
    let limit: usize = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let mut session = imap_connect(account, password).await?;
    let folders = resolve_folders(&mut session, flags).await?;
    let query = if unread { "UNSEEN" } else { "ALL" };
    let rows = collect_across_folders(&mut session, folders, query, limit).await?;
    session.logout().await.ok();

    Ok(Output::records(
        vec![
            "uid".into(),
            "folder".into(),
            "date".into(),
            "from".into(),
            "subject".into(),
        ],
        rows,
    ))
}

/// 读取单封邮件完整内容。
/// - `--folder NAME`：仅在该文件夹查
/// - 默认（递归）：遍历所有文件夹，返回首个命中该 UID 的邮件
/// - `--no-recursive`：仅 INBOX
///
/// 注意：IMAP UID 仅在单个文件夹内唯一，跨文件夹不唯一。`mail list` 默认递归
/// 所有文件夹，故 `mail read` 不带 `--folder` 时也递归查找，保证 list 给出的
/// uid 总能被 read 读到。
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

    let mut session = imap_connect(account, password).await?;
    // 与 list/search 一致：默认递归所有文件夹，--folder 指定单个，--no-recursive 仅 INBOX
    let folders = resolve_folders(&mut session, flags).await?;

    // 遍历文件夹逐个尝试 uid_fetch，返回首个命中。UID 不存在的文件夹返回空集（不报错）。
    let mut last_err: Option<AgentError> = None;
    let mut found: Option<(async_imap::types::Fetch, String)> = None;
    for folder in &folders {
        if select_folder(&mut session, folder).await.is_err() {
            continue; // 跳过无法 SELECT 的文件夹（如 \NoSelect）
        }
        match session.uid_fetch(uid.to_string(), "(UID BODY[])").await {
            Ok(stream) => match stream.try_collect::<Vec<_>>().await {
                Ok(fetches) => {
                    if let Some(f) = fetches.into_iter().next() {
                        found = Some((f, decode_imap_utf7(folder)));
                        break;
                    }
                    // 该文件夹无此 UID，继续下一个
                }
                Err(e) => last_err = Some(AgentError::Network(format!("fetch collect: {e}"))),
            },
            Err(e) => last_err = Some(AgentError::Network(format!("fetch: {e}"))),
        }
    }
    session.logout().await.ok();

    let (fetch, folder_name) = found.ok_or_else(|| match last_err {
        Some(e) => e,
        None => AgentError::Other(format!(
            "no message with uid {uid} (searched {} folder{})",
            folders.len(),
            if folders.len() == 1 { "" } else { "s" }
        )),
    })?;
    let body = fetch
        .body()
        .ok_or_else(|| AgentError::Other("message has no body".into()))?;

    let parsed =
        mailparse::parse_mail(body).map_err(|e| AgentError::Other(format!("parse mail: {e}")))?;
    let subject = header_value(&parsed, "Subject");
    let from = header_value(&parsed, "From");
    let date = header_value(&parsed, "Date");
    let text = extract_body(&parsed);

    Ok(Output::Records {
        headers: vec!["field".into(), "value".into()],
        rows: vec![
            vec!["subject".into(), subject],
            vec!["from".into(), from],
            vec!["date".into(), date],
            vec!["folder".into(), folder_name],
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
    let query = flags.get("query").ok_or_else(|| {
        AgentError::InvalidArgument("usage: everyday mail search --query Q".into())
    })?;
    let limit: usize = flags
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let mut session = imap_connect(account, password).await?;
    let folders = resolve_folders(&mut session, flags).await?;
    // IMAP SEARCH TEXT "query" —— 转义双引号与反斜杠
    let escaped = query.replace('\\', "\\\\").replace('"', "\\\"");
    let search = format!("TEXT \"{escaped}\"");
    let rows = collect_across_folders(&mut session, folders, &search, limit).await?;
    session.logout().await.ok();

    Ok(Output::records(
        vec![
            "uid".into(),
            "folder".into(),
            "date".into(),
            "from".into(),
            "subject".into(),
        ],
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

    use lettre::message::{Mailbox, header::ContentType};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let from: Mailbox = account.username.parse().map_err(|e| {
        AgentError::InvalidArgument(format!("invalid from address '{}': {e}", account.username))
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

/// 提取邮件正文：优先 text/plain；为空则回退到 text/html 并转纯文本。
/// 解决营销/通知类邮件（HTML-only）正文为空的问题。
fn extract_body(parsed: &mailparse::ParsedMail) -> String {
    if let Some(plain) = find_body_by_type(parsed, "text/plain")
        && !plain.trim().is_empty()
    {
        return plain;
    }
    if let Some(html) = find_body_by_type(parsed, "text/html") {
        return html_to_text(&html);
    }
    parsed.get_body().unwrap_or_default()
}

/// 递归查找第一个指定 Content-Type 的叶子部分的正文。
fn find_body_by_type(part: &mailparse::ParsedMail, mime: &str) -> Option<String> {
    if part.ctype.mimetype == mime
        && let Ok(body) = part.get_body()
    {
        return Some(body);
    }
    for sub in &part.subparts {
        if let Some(body) = find_body_by_type(sub, mime) {
            return Some(body);
        }
    }
    None
}

/// HTML → 纯文本：去标签、跳过 script/style、块级元素转换行、
/// 解码常见实体（&amp; &lt; &gt; &quot; &apos; &#39; &nbsp;）、折叠空白。
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    let mut skip_content = false; // 处于 script/style 内，丢弃文本
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut name = String::new();
            // '<' 之后若紧跟 '/' 则为闭合标签
            let closing = matches!(chars.peek(), Some(&'/'));
            if closing {
                chars.next();
            }
            while let Some(&nc) = chars.peek() {
                // 标签名在 '>', '/'（自闭合如 <br/>）, 或空白处结束
                if nc == '>' || nc == '/' || nc.is_whitespace() {
                    break;
                }
                name.push(nc);
                chars.next();
            }
            // 跳到 '>' 结束标签
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '>' {
                    break;
                }
            }
            match name.to_ascii_lowercase().as_str() {
                "script" | "style" => skip_content = !closing,
                "br" => out.push('\n'),
                "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if closing => {
                    out.push('\n');
                }
                _ => {}
            }
        } else if !skip_content {
            if c == '&' {
                out.push_str(&decode_entity(&mut chars));
            } else {
                out.push(c);
            }
        }
    }
    collapse_whitespace(&out)
}

/// 从 chars 当前位置（紧跟在 '&' 之后）读取一个 HTML 实体到 ';' 或空白。
/// 已知实体返回解码字符；未知原样返回（含前导 '&')。
fn decode_entity(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut ent = String::new();
    let mut terminated = false;
    while let Some(&c) = chars.peek() {
        if c == ';' {
            chars.next();
            terminated = true;
            break;
        }
        if c.is_whitespace() || c == '<' || c == '&' {
            break;
        }
        ent.push(c);
        chars.next();
        if ent.len() > 10 {
            break; // 安全上限，防畸形输入
        }
    }
    let with_amp = format!("&{ent}{}", if terminated { ";" } else { "" });
    match ent.as_str() {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" | "#39" => "'".into(),
        "nbsp" => "\u{a0}".into(),
        _ => with_amp,
    }
}

/// 折叠空白：行内连续空白压成单空格，连续空行压成单空行，去首尾空白。
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_blank = false;
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank && !out.is_empty() {
                out.push('\n');
                prev_blank = true;
            }
            continue;
        }
        let mut buf = String::with_capacity(trimmed.len());
        let mut in_ws = false;
        for c in trimmed.chars() {
            if c.is_whitespace() {
                in_ws = true;
            } else {
                if in_ws {
                    buf.push(' ');
                    in_ws = false;
                }
                buf.push(c);
            }
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&buf);
        prev_blank = false;
    }
    out.trim().to_string()
}

/// 解码 IMAP UTF-7 文件夹名（RFC 3501 §5.1.3）为可读 UTF-8。
///
/// 规则：`&` 起始、`-` 结尾的段是 modified base64 编码的 UTF-16BE；`&-` 表示字面 `&`；
/// 其余字符透传。用 `char` 迭代以正确处理 UTF-8（用户可能直接传入中文名，无 `&` 段）。
/// 例如 `&UXZO1mWHTvZZOQ-/Github&kBp35Q-` → `其他文件夹/Github通知`。
fn decode_imap_utf7(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            let mut segment = String::new();
            let mut found_terminator = false;
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '-' {
                    found_terminator = true;
                    break;
                }
                segment.push(nc);
            }
            if !found_terminator {
                // 无结束 '-'，原样输出
                out.push('&');
                out.push_str(&segment);
                break;
            }
            if segment.is_empty() {
                out.push('&'); // &- → 字面 &
            } else if let Some(decoded) = decode_modified_base64_utf16(segment.as_bytes()) {
                out.push_str(&decoded);
            } else {
                // 解码失败，保留原始段
                out.push('&');
                out.push_str(&segment);
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// modified base64（`,` 替代 `/`，无 padding）→ UTF-16BE → String。
fn decode_modified_base64_utf16(b64: &[u8]) -> Option<String> {
    let raw = decode_base64_modified(b64)?;
    if raw.len() % 2 != 0 {
        return None;
    }
    let u16s: Vec<u16> = raw
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&u16s).ok()
}

/// modified base64 解码（无依赖，手写）。
fn decode_base64_modified(input: &[u8]) -> Option<Vec<u8>> {
    const TABLE: [i8; 256] = build_b64_table();
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input {
        if c == b'=' {
            break;
        }
        let v = TABLE[c as usize];
        if v < 0 {
            continue;
        }
        buf = (buf << 6) | (v as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// 构建 base64 查找表（const fn，编译期计算）。`,` 映射到 63（modified base64 用 `,` 替 `/`）。
const fn build_b64_table() -> [i8; 256] {
    let mut t = [-1i8; 256];
    let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut i = 0;
    while i < alpha.len() {
        t[alpha[i] as usize] = i as i8;
        i += 1;
    }
    t[b',' as usize] = 63; // modified base64
    t
}

/// 解析 RFC 2822 邮件日期，用于跨文件夹按时间排序。容错：去掉括号注释如 "(UTC)"。
fn parse_mail_date(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let cleaned = s.split('(').next().unwrap_or("").trim_end();
    chrono::DateTime::parse_from_rfc2822(cleaned)
        .ok()
        .or_else(|| chrono::DateTime::parse_from_rfc2822(s).ok())
}

/// 选中文件夹：先直接尝试（INBOX / ASCII / 原始编码名），
/// 失败则遍历所有文件夹匹配解码后的中文名。兼容用户输入中文或原始名。
async fn select_folder(session: &mut ImapSession, folder: &str) -> Result<()> {
    if session.select(folder).await.is_ok() {
        return Ok(());
    }
    let all = list_all_folders(session).await?;
    for f in &all {
        if decode_imap_utf7(f) == folder {
            session
                .select(f)
                .await
                .map_err(|e| AgentError::Network(format!("select '{f}': {e}")))?;
            return Ok(());
        }
    }
    Err(AgentError::Other(format!(
        "folder '{folder}' not found (tried direct select and decoded-name match)"
    )))
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
    fn imap_utf7_ascii_passthrough() {
        assert_eq!(decode_imap_utf7("INBOX"), "INBOX");
        assert_eq!(decode_imap_utf7("Sent Messages"), "Sent Messages");
    }

    #[test]
    fn imap_utf7_chinese_passthrough() {
        // 用户直接传入中文名（无 & 段），应原样透传不破坏 UTF-8
        assert_eq!(
            decode_imap_utf7("其他文件夹/Github通知"),
            "其他文件夹/Github通知"
        );
    }

    #[test]
    fn imap_utf7_ampersand_escape() {
        // &- 表示字面 &
        assert_eq!(decode_imap_utf7("A&-B"), "A&B");
    }

    #[test]
    fn imap_utf7_single_chinese_char() {
        // "你" = U+4F60 → UTF-16BE 4F 60 → modified base64 "T2A"
        assert_eq!(decode_imap_utf7("&T2A-"), "你");
    }

    #[test]
    fn imap_utf7_mixed_chinese_and_ascii() {
        // "其他文件夹" 前缀 + "/Github"
        let decoded = decode_imap_utf7("&UXZO1mWHTvZZOQ-/Github&kBp35Q-");
        assert!(
            decoded.chars().any(|c| c as u32 > 127),
            "expected Chinese chars in: {decoded}"
        );
        assert!(decoded.contains("Github"));
    }

    #[test]
    fn imap_utf7_no_terminator_fallback() {
        // 无结束 '-'，原样输出不 panic
        assert_eq!(decode_imap_utf7("test&abc"), "test&abc");
    }

    #[test]
    fn imap_utf7_roundtrip_known() {
        // "你好" → UTF-16BE 4F60 597D → base64: 4F 60 59 → 010011 110110 000001 011001 = T 2 B Z
        // 剩余 7D → 011111 01(pad) = f Q → "T2BZfQ"
        assert_eq!(decode_imap_utf7("&T2BZfQ-"), "你好");
    }

    #[test]
    fn parse_mail_date_rfc2822() {
        assert!(parse_mail_date("Wed, 08 Jul 2026 08:29:31 +0000 (UTC)").is_some());
        assert!(parse_mail_date("Wed, 1 Jul 2026 16:55:11 +0800").is_some());
        assert!(parse_mail_date("invalid date").is_none());
    }

    #[test]
    fn parse_mail_date_orders_correctly() {
        let earlier = parse_mail_date("Wed, 01 Jul 2026 00:55:26 -0700").unwrap();
        let later = parse_mail_date("Wed, 08 Jul 2026 08:29:31 +0000").unwrap();
        assert!(later > earlier);
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

    #[test]
    fn html_to_text_strips_tags() {
        assert_eq!(html_to_text("<p>Hello <b>world</b></p>"), "Hello world");
    }

    #[test]
    fn html_to_text_block_newlines() {
        let html = "<p>One</p><p>Two</p><div>Three</div>";
        assert_eq!(html_to_text(html), "One\nTwo\nThree");
    }

    #[test]
    fn html_to_text_br_becomes_newline() {
        assert_eq!(html_to_text("a<br>b<br/>c"), "a\nb\nc");
    }

    #[test]
    fn html_to_text_skips_script_style() {
        let html = "<style>body{}</style>x<script>alert(1)</script>y";
        // script/style 内容被丢弃，但标签本身不引入换行
        assert_eq!(html_to_text(html), "xy");
    }

    #[test]
    fn html_to_text_decodes_entities() {
        assert_eq!(html_to_text("a &amp; b &lt; c &gt; d"), "a & b < c > d");
        assert_eq!(html_to_text("&quot;hi&quot; &#39;ok&#39;"), "\"hi\" 'ok'");
    }

    #[test]
    fn html_to_text_unknown_entity_preserved() {
        assert_eq!(html_to_text("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(html_to_text("&unknown;"), "&unknown;");
    }

    #[test]
    fn html_to_text_collapses_whitespace() {
        let html = "<p>  spaced   out  </p>\n\n\n<p>next</p>";
        // 源中多个空行 → 压成单个空行做段落分隔；行内连续空白压成单空格
        assert_eq!(html_to_text(html), "spaced out\n\nnext");
    }

    #[test]
    fn html_to_text_utf8_preserved() {
        assert_eq!(html_to_text("<p>你好 <b>世界</b></p>"), "你好 世界");
    }

    #[test]
    fn extract_body_prefers_plain_text() {
        let raw = b"Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain body\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<p>html body</p>\r\n\
--b--\r\n";
        let parsed = mailparse::parse_mail(raw).unwrap();
        assert_eq!(extract_body(&parsed).trim(), "plain body");
    }

    #[test]
    fn extract_body_falls_back_to_html() {
        // text/plain 为空 → 回退 html 并去标签
        let raw = b"Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<p>html <b>only</b> body</p>\r\n\
--b--\r\n";
        let parsed = mailparse::parse_mail(raw).unwrap();
        assert_eq!(extract_body(&parsed), "html only body");
    }

    #[test]
    fn find_body_by_type_html_only_single_part() {
        let raw = b"Content-Type: text/html\r\n\r\n<p>hi</p>";
        let parsed = mailparse::parse_mail(raw).unwrap();
        assert_eq!(
            find_body_by_type(&parsed, "text/html"),
            Some("<p>hi</p>".into())
        );
        assert_eq!(find_body_by_type(&parsed, "text/plain"), None);
    }
}
