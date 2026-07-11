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
use crate::modules::{email_cache, email_pool};
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
                "List messages from local cache (auto-sync if stale; --sync to force)",
                "everyday mail list [--unread] [--limit N] [--folder NAME] [--no-recursive] [--sync] [--account NAME]",
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
pub(crate) type ImapSession = async_imap::Session<Compat<TlsStream<TcpStream>>>;

/// 建立 IMAPS（implicit TLS, 993）连接并登录。
pub(crate) async fn imap_connect(account: &MailAccount, password: &str) -> Result<ImapSession> {
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
///
/// 实现按 ADR 0010-0013：
/// 1. 打开 `mail_cache.db`。
/// 2. 解析目标 folders（一次临时 IMAP session 拿 LIST）。
/// 3. staleness 检查（任一 folder `last_sync_at > 15min` 或无水位 → 触发 sync）。
/// 4. `--sync` flag 强制立即 sync。
/// 5. sync 走 `email_pool::Pool`（M=4）并发跨 folder 写 envelope + 更新水位。
/// 6. 查本地 `envelopes` 表返回。
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
    let force_sync = flags.contains_key("sync");

    // 1. 打开本地缓存
    let cache = email_cache::open().await?;

    // 2. 解析 folders（一次性临时 session；list 不持久化）
    let mut list_session = imap_connect(account, password).await?;
    let folders = resolve_folders(&mut list_session, flags).await?;
    list_session.logout().await.ok();

    // 3. staleness 检查
    let now = chrono::Utc::now();
    let mut needs_sync = force_sync;
    if !needs_sync {
        for folder in &folders {
            match email_cache::get_folder_state(&cache, &account.name, folder).await? {
                None => {
                    // 无水位 → 首次 sync
                    needs_sync = true;
                    break;
                }
                Some(state) if email_cache::is_stale(&state, now) => {
                    needs_sync = true;
                    break;
                }
                _ => {}
            }
        }
    }

    // 4. 必要时 sync（并发跨 folder，best-effort）
    let _sync_stats = if needs_sync {
        let pool = email_pool::Pool::new(account.clone(), password.to_string()).await?;
        let stats = sync_folders_concurrent(&pool, &cache, &account.name, &folders).await?;
        // pool drop 时 session 静默丢弃
        Some(stats)
    } else {
        None
    };

    // 5. 查本地 envelope
    let query = email_cache::EnvelopeQuery {
        folder: flags.get("folder").cloned(),
        unread_only: unread,
        since: None,
        limit: Some(limit),
    };
    let envelopes = email_cache::query_envelopes(&cache, &account.name, &query).await?;

    // 6. 渲染表格行
    let rows: Vec<Vec<String>> = envelopes
        .into_iter()
        .map(|e| {
            vec![
                e.uid.to_string(),
                decode_imap_utf7(&e.folder),
                e.date,
                e.from_addr,
                decode_mime_header(&e.subject),
            ]
        })
        .collect();

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
///
/// IMAP `SELECT` 始终返回 `Mailbox`（含 `uid_validity` 等元数据），
/// 一并返回。多数调用方不需要 Mailbox，用 `select_folder()` 包装即可。
async fn select_folder_inner(
    session: &mut ImapSession,
    folder: &str,
) -> Result<async_imap::types::Mailbox> {
    if let Ok(mb) = session.select(folder).await {
        return Ok(mb);
    }
    let all = list_all_folders(session).await?;
    for f in &all {
        if decode_imap_utf7(f) == folder {
            return session
                .select(f)
                .await
                .map_err(|e| AgentError::Network(format!("select '{f}': {e}")));
        }
    }
    Err(AgentError::Other(format!(
        "folder '{folder}' not found (tried direct select and decoded-name match)"
    )))
}

/// `select_folder_inner` 的 `Result<()>` 版本，丢弃 Mailbox 元数据。
async fn select_folder(session: &mut ImapSession, folder: &str) -> Result<()> {
    select_folder_inner(session, folder).await.map(|_| ())
}

/// 把 `Flag` 迭代器格式化为 IMAP 风格的空格分隔字符串（如 `\Seen \Answered`）。
/// 自定义 keyword 不带 `\` 前缀。
fn format_imap_flags<'a, I>(flags: I) -> String
where
    I: Iterator<Item = async_imap::types::Flag<'a>>,
{
    flags
        .map(|f| match f {
            async_imap::types::Flag::Seen => "\\Seen".to_string(),
            async_imap::types::Flag::Answered => "\\Answered".to_string(),
            async_imap::types::Flag::Flagged => "\\Flagged".to_string(),
            async_imap::types::Flag::Deleted => "\\Deleted".to_string(),
            async_imap::types::Flag::Draft => "\\Draft".to_string(),
            async_imap::types::Flag::Recent => "\\Recent".to_string(),
            async_imap::types::Flag::MayCreate => "\\*".to_string(),
            async_imap::types::Flag::Custom(k) => k.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 把 IMAP envelope date 字符串（RFC 2822）解析为 RFC3339 UTC。
/// 解析失败时回退原字符串（避免 sync 中断）。
fn parse_envelope_date_utc(raw: &str) -> String {
    if let Some(cleaned) = raw.split('(').next() {
        let cleaned = cleaned.trim();
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(cleaned) {
            return dt.with_timezone(&chrono::Utc).to_rfc3339();
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(raw) {
            return dt.with_timezone(&chrono::Utc).to_rfc3339();
        }
    }
    raw.to_string()
}

/// 按 UID 批量 fetch envelope + flags + size，返回 `CachedEnvelope` 列表。
/// `account` / `fetched_at` 字段由 sync_one_folder 在 upsert 前填。
async fn fetch_envelopes_for_cache(
    session: &mut ImapSession,
    uids: &[u32],
    folder: &str,
) -> Result<Vec<crate::modules::email_cache::CachedEnvelope>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let uid_set = uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fetches: Vec<async_imap::types::Fetch> = session
        .uid_fetch(&uid_set, "(UID ENVELOPE FLAGS RFC822.SIZE)")
        .await
        .map_err(|e| AgentError::Network(format!("fetch envelope: {e}")))?
        .try_collect()
        .await
        .map_err(|e| AgentError::Network(format!("fetch envelope collect: {e}")))?;

    let mut envelopes = Vec::with_capacity(fetches.len());
    for f in &fetches {
        let uid = f.uid.unwrap_or(0);
        if uid == 0 {
            continue;
        }
        let env = f.envelope();
        let raw_date = env
            .and_then(|e| e.date.as_deref())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let raw_subject = env
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
        let to = env
            .and_then(|e| e.to.as_ref().and_then(|a| a.first()))
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
        let message_id = env
            .and_then(|e| e.message_id.as_deref())
            .map(|b| String::from_utf8_lossy(b).into_owned());
        let size = f.size.map(|s| s as i64);
        let flags = format_imap_flags(f.flags());

        envelopes.push(crate::modules::email_cache::CachedEnvelope {
            account: String::new(), // 由 sync_one_folder 填
            folder: folder.to_string(),
            uid,
            date: parse_envelope_date_utc(&decode_mime_header(&raw_date)),
            from_addr: from,
            subject: decode_mime_header(&raw_subject),
            flags,
            message_id,
            size,
            to_addr: if to.is_empty() { None } else { Some(to) },
            fetched_at: String::new(), // 由 upsert_envelopes 填
        });
    }
    Ok(envelopes)
}

/// 单 folder sync 结果（用于汇总输出 / 调试）。
#[derive(Debug, Default)]
struct SyncStats {
    folders_synced: usize,
    envelopes_added: usize,
    errors: Vec<(String, String)>,
}

/// 单 folder sync：SELECT 拿 uid_validity → 比对水位 → UIDSEARCH → UID FETCH → upsert。
/// best-effort：失败时 `invalidate()` session + 计入 errors，水位不前进。
async fn sync_one_folder(
    pool: &email_pool::Pool,
    cache: &sqlx::SqlitePool,
    account: &str,
    folder: &str,
) -> Result<usize> {
    let mut guard = match pool.acquire().await {
        Ok(g) => g,
        Err(e) => return Err(e),
    };
    let session = guard.session()?;

    // SELECT folder → uid_validity
    let mailbox = match select_folder_inner(session, folder).await {
        Ok(mb) => mb,
        Err(e) => {
            guard.invalidate();
            return Err(e);
        }
    };
    let new_uid_validity = mailbox.uid_validity.unwrap_or(0) as u32;

    // 读本地水位，决定 search query
    let search_query = match email_cache::get_folder_state(cache, account, folder).await? {
        None => "UID 1:*".to_string(),
        Some(state) if state.uid_validity != new_uid_validity => {
            // UIDVALIDITY 失效 → 清空水位，下一轮当首次处理
            email_cache::clear_folder(cache, account, folder).await?;
            "UID 1:*".to_string()
        }
        Some(state) => format!("UID {}:*", state.max_uid + 1),
    };

    // UIDSEARCH
    let uids = match search_uids(session, &search_query).await {
        Ok(u) => u,
        Err(e) => {
            guard.invalidate();
            return Err(e);
        }
    };

    if uids.is_empty() {
        // 无新邮件，仍更新 last_sync_at + uid_validity（空水位 max_uid=0）
        email_cache::upsert_envelopes(cache, account, folder, new_uid_validity, &[]).await?;
        return Ok(0);
    }

    // UID FETCH ENVELOPE + FLAGS + SIZE
    let envelopes = match fetch_envelopes_for_cache(session, &uids, folder).await {
        Ok(e) => e,
        Err(e) => {
            guard.invalidate();
            return Err(e);
        }
    };

    let count = envelopes.len();
    // 写 envelope + 更新水位（事务原子，ADR 0012）
    email_cache::upsert_envelopes(cache, account, folder, new_uid_validity, &envelopes).await?;
    Ok(count)
}

/// 并发跨 folder sync：使用 `futures::future::join_all` 等待全部结束。
/// 单 folder 失败不阻塞其他，结果汇入 `SyncStats.errors`。
async fn sync_folders_concurrent(
    pool: &email_pool::Pool,
    cache: &sqlx::SqlitePool,
    account: &str,
    folders: &[String],
) -> Result<SyncStats> {
    let mut stats = SyncStats::default();
    let futures = folders.iter().map(|folder| {
        let pool = pool.clone();
        async move {
            (
                folder.clone(),
                sync_one_folder(&pool, cache, account, folder).await,
            )
        }
    });
    let results = futures::future::join_all(futures).await;
    for (folder, result) in results {
        match result {
            Ok(n) => {
                stats.folders_synced += 1;
                stats.envelopes_added += n;
            }
            Err(e) => {
                stats.errors.push((folder, format!("{e}")));
            }
        }
    }
    Ok(stats)
}

// ============ Timeline 数据拉取 ============

/// Timeline 拉取用：邮件条目原始数据。
pub struct MailTimelineEntry {
    pub uid: u32,
    pub folder: String,
    pub date: String,
    pub from: String,
    pub subject: String,
}

/// Timeline 增量拉取：IMAP SEARCH SINCE <from_date>，跨所有文件夹，
/// 返回窗口内收到的邮件。
///
/// IMAP SEARCH SINCE 只支持日期（无时间），客户端侧按精确 timestamp 过滤。
/// 需要从 keyring 读取密码。
pub async fn fetch_for_timeline(
    account: &MailAccount,
    from: chrono::DateTime<chrono::Utc>,
    _to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<MailTimelineEntry>> {
    let password = get_password(account)?;
    let mut session = imap_connect(account, &password).await?;
    let folders = list_all_folders(&mut session).await?;

    let from_date = chrono::DateTime::parse_from_rfc3339(&from.to_rfc3339())
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|_| chrono::Utc::now().date_naive());

    let mut all_entries: Vec<MailTimelineEntry> = Vec::new();

    for folder in &folders {
        if select_folder(&mut session, folder).await.is_err() {
            continue;
        }
        // IMAP SEARCH SINCE <date>（date-only，返回当天及以后的邮件）
        let search = format!("SINCE {from_date}");
        let uids = match search_uids(&mut session, &search).await {
            Ok(u) => u,
            Err(_) => continue,
        };
        let display_folder = decode_imap_utf7(folder);
        let entries = fetch_timeline_summaries(&mut session, uids, &display_folder).await?;
        all_entries.extend(entries);
    }
    session.logout().await.ok();
    Ok(all_entries)
}

/// 按 UID 批量 fetch 摘要（timeline 用），只取 envelope 字段。
async fn fetch_timeline_summaries(
    session: &mut ImapSession,
    mut uids: Vec<u32>,
    folder: &str,
) -> Result<Vec<MailTimelineEntry>> {
    uids.truncate(500); // 防止过大 fetch
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let uid_set = uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fetches: Vec<async_imap::types::Fetch> = session
        .uid_fetch(uid_set, "(UID ENVELOPE)")
        .await
        .map_err(|e| AgentError::Network(format!("fetch: {e}")))?
        .try_collect()
        .await
        .map_err(|e| AgentError::Network(format!("fetch collect: {e}")))?;

    let mut entries = Vec::with_capacity(fetches.len());
    for f in &fetches {
        let uid = f.uid.unwrap_or(0);
        if uid == 0 {
            continue;
        }
        let env = f.envelope();
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
        entries.push(MailTimelineEntry {
            uid,
            folder: folder.to_string(),
            date: decode_mime_header(&date),
            from,
            subject: decode_mime_header(&subject),
        });
    }
    Ok(entries)
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
