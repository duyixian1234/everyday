//! Mail module: IMAP receiving (list / read / search) + SMTP sending (send)
//! + keyring credentials (login). See [M001](../../docs/adr/M001-imap-stack.md).
//!
//! Flow: the config file stores account metadata (host/port/username) →
//! `everyday mail login` stores the password in the system keyring →
//! `everyday mail list/read/search/send` automatically reads it to connect.
//! The password never lands in config.toml.

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
use crate::modules::{Executor, parse_simple_args};
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
    fn description(&self) -> &'static str {
        "Email management (IMAP/SMTP): folders, list, read, search, send, login."
    }

    fn module_arg_spec(&self) -> crate::modules::ModuleArgSpec {
        use crate::modules::{ActionArgSpec, ArgKind, ArgSpec, ModuleArgSpec, Positional};
        static ACTIONS: &[ActionArgSpec] = &[
            ActionArgSpec {
                name: "folders",
                description: "列出所有文件夹",
                usage: "everyday mail folders [--account NAME]",
                args: &[],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "list",
                description: "列出邮件（走本地 envelope 缓存）",
                usage: "everyday mail list [--unread] [--limit N] [--folder NAME] [--no-recursive] [--sync] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "unread",
                        help: "仅未读",
                        kind: ArgKind::Bool,
                    },
                    ArgSpec {
                        name: "limit",
                        help: "条数上限",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "folder",
                        help: "限定文件夹",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "no-recursive",
                        help: "仅查 INBOX（不递归子文件夹）",
                        kind: ArgKind::Bool,
                    },
                    ArgSpec {
                        name: "sync",
                        help: "强制立即同步本地缓存",
                        kind: ArgKind::Bool,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "read",
                description: "读取邮件正文",
                usage: "everyday mail read <uid> [--folder NAME] [--no-recursive] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "id",
                        help: "邮件 UID（--uid 的替代写法）",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "folder",
                        help: "邮件所在文件夹",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "no-recursive",
                        help: "仅查 INBOX",
                        kind: ArgKind::Bool,
                    },
                ],
                positional: Positional::OptionalSingle,
            },
            ActionArgSpec {
                name: "search",
                description: "在服务器搜索邮件",
                usage: "everyday mail search --query Q [--limit N] [--folder NAME] [--no-recursive] [--account NAME]",
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
                    ArgSpec {
                        name: "folder",
                        help: "限定文件夹",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "no-recursive",
                        help: "仅查 INBOX",
                        kind: ArgKind::Bool,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "send",
                description: "发送邮件",
                usage: "everyday mail send --to ADDR --subject S --body TEXT [--cc ADDR] [--account NAME]",
                args: &[
                    ArgSpec {
                        name: "to",
                        help: "收件人地址",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "subject",
                        help: "主题",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "body",
                        help: "正文",
                        kind: ArgKind::Value,
                    },
                    ArgSpec {
                        name: "cc",
                        help: "抄送地址",
                        kind: ArgKind::Value,
                    },
                ],
                positional: Positional::None,
            },
            ActionArgSpec {
                name: "login",
                description: "保存邮箱凭证到系统 keyring",
                usage: "everyday mail login [--account NAME]",
                args: &[],
                positional: Positional::None,
            },
        ];
        ModuleArgSpec {
            name: "mail",
            description: self.description(),
            actions: ACTIONS,
        }
    }

    async fn execute(&self, action: &str, args: &[String]) -> Result<Output> {
        let (flags, positional) = parse_simple_args(args);
        let account = self
            .config
            .mail_account(flags.get("account").map(|s| s.as_str()))?;

        match action {
            // login needs no password, only account metadata + interactive input
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

// ============ keyring credentials ============

/// Read the account password from the system keyring.
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

/// Interactively prompt for the password and store it in the system keyring.
async fn mail_login(account: &MailAccount) -> Result<Output> {
    let service = Config::keyring_service("mail", &account.name);
    let entry = keyring::Entry::new(&service, &account.username)
        .map_err(|e| AgentError::Auth(format!("keyring entry: {e}")))?;
    // rpassword is sync; wrap in spawn_blocking to avoid blocking the runtime.
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

// ============ IMAP connection ============

// async-imap is built on futures' AsyncRead/AsyncWrite, while tokio-rustls
// implements tokio's; bridge them with tokio-util compat.
pub(crate) type ImapSession = async_imap::Session<Compat<TlsStream<TcpStream>>>;

/// Establish an IMAPS (implicit TLS, 993) connection and log in.
pub(crate) async fn imap_connect(account: &MailAccount, password: &str) -> Result<ImapSession> {
    // Install the rustls ring crypto provider (re-installing returns Err; ignore it).
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

// ============ action implementations ============

/// List all mailbox folders (IMAP LIST).
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

/// Call IMAP LIST to list all folder names (filtering out \NoSelect).
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
            // Skip folders marked \NoSelect (cannot be SELECTed)
            !n.attributes()
                .iter()
                .any(|a| matches!(a, async_imap::types::NameAttribute::NoSelect))
        })
        .map(|n| n.name().to_string())
        .collect();
    Ok(folders)
}

/// Resolve the folder list to traverse from CLI flags.
/// - `--folder NAME`: only that folder
/// - default (recursive): all folders
/// - `--no-recursive`: only INBOX
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

/// Collect mail summaries across multiple folders, merge them, sort by UID
/// descending and truncate to `limit`.
/// Folders that cannot be SELECTed (e.g. \NoSelect) are skipped.
async fn collect_across_folders(
    session: &mut ImapSession,
    folders: Vec<String>,
    search_query: &str,
    limit: usize,
) -> Result<Vec<Vec<String>>> {
    let mut all_rows: Vec<Vec<String>> = Vec::new();
    for folder in &folders {
        // select_folder accepts both the raw encoded name and the decoded Chinese name (user's --folder input)
        if select_folder(session, folder).await.is_err() {
            continue; // skip folders that cannot be selected
        }
        let uids = match search_uids(session, search_query).await {
            Ok(u) => u,
            Err(_) => continue, // a single folder's search failure is non-fatal
        };
        // display uses the decoded Chinese name; select uses the raw encoded name
        let display_folder = decode_imap_utf7(folder);
        // take the most recent `limit` per folder as global candidates; do not break early so every folder participates
        let rows = fetch_summaries(session, uids, limit, &display_folder).await?;
        all_rows.extend(rows);
    }
    // global sort by message date descending (cross-folder UIDs are not contiguous, so date is more accurate)
    all_rows.sort_by(|a, b| {
        let da = a.get(2).and_then(|s| parse_mail_date(s));
        let db = b.get(2).and_then(|s| parse_mail_date(s));
        match (da, db) {
            (Some(da), Some(db)) => db.cmp(&da),
            (Some(_), None) => std::cmp::Ordering::Less, // dated entries sort first
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    all_rows.truncate(limit);
    Ok(all_rows)
}

/// List message summaries (recursively across all folders by default).
///
/// Implementation per ADR [M002](../../docs/adr/M002-imap-connection-pool.md)–
/// [M005](../../docs/adr/M005-staleness-auto-sync.md):
/// 1. open `mail_cache.db`.
/// 2. resolve target folders (one ad-hoc IMAP session to get LIST).
/// 3. staleness check (any folder with `last_sync_at > 15min` or no watermark
///    → trigger sync).
/// 4. `--sync` flag forces an immediate sync.
/// 5. sync goes through `email_pool::Pool` (M=4) concurrently across folders,
///    writing envelopes + updating watermarks.
/// 6. query the local `envelopes` table and return.
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

    // 1. open the local cache
    let cache = email_cache::open().await?;

    // 2. resolve folders (one-shot ad-hoc session; list is not persisted)
    let mut list_session = imap_connect(account, password).await?;
    let folders = resolve_folders(&mut list_session, flags).await?;
    list_session.logout().await.ok();

    // 3. staleness check
    let now = chrono::Utc::now();
    let mut needs_sync = force_sync;
    if !needs_sync {
        for folder in &folders {
            match email_cache::get_folder_state(&cache, &account.name, folder).await? {
                None => {
                    // no watermark → first-time sync
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

    // 4. sync if needed (concurrent across folders, best-effort)
    let _sync_stats = if needs_sync {
        let pool = email_pool::Pool::new(account.clone(), password.to_string()).await?;
        let stats = sync_folders_concurrent(&pool, &cache, &account.name, &folders).await?;
        // sessions are dropped silently when the pool drops
        Some(stats)
    } else {
        None
    };

    // 5. query local envelopes
    let query = email_cache::EnvelopeQuery {
        folder: flags.get("folder").cloned(),
        unread_only: unread,
        since: None,
        limit: Some(limit),
    };
    let envelopes = email_cache::query_envelopes(&cache, &account.name, &query).await?;

    // 6. render table rows
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

/// Read a single message's full content.
/// - `--folder NAME`: look only in that folder
/// - default (recursive): walk all folders, return the first hit for the UID
/// - `--no-recursive`: only INBOX
///
/// Note: IMAP UIDs are unique only within a single folder, not across folders.
/// `mail list` recurses by default, so `mail read` without `--folder` also
/// recurses, guaranteeing any uid shown by list can be read.
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
    // consistent with list/search: recurse all folders by default, --folder picks one, --no-recursive is INBOX only
    let folders = resolve_folders(&mut session, flags).await?;

    // try uid_fetch per folder, return the first hit. Folders without the UID yield an empty set (no error).
    let mut last_err: Option<AgentError> = None;
    let mut found: Option<(async_imap::types::Fetch, String)> = None;
    for folder in &folders {
        if select_folder(&mut session, folder).await.is_err() {
            continue; // skip folders that cannot be SELECTed (e.g. \NoSelect)
        }
        match session.uid_fetch(uid.to_string(), "(UID BODY[])").await {
            Ok(stream) => match stream.try_collect::<Vec<_>>().await {
                Ok(fetches) => {
                    if let Some(f) = fetches.into_iter().next() {
                        found = Some((f, decode_imap_utf7(folder)));
                        break;
                    }
                    // this folder has no such UID; try the next
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

/// Search messages (recursively across all folders by default).
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
    // IMAP SEARCH TEXT "query" — escape double quotes and backslashes
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

/// Send a message (SMTP via lettre, STARTTLS).
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

// ============ helper functions ============

/// Run IMAP SEARCH, return UIDs in descending order (most recent first).
async fn search_uids(session: &mut ImapSession, query: &str) -> Result<Vec<u32>> {
    let set: std::collections::HashSet<u32> = session
        .uid_search(query)
        .await
        .map_err(|e| AgentError::Network(format!("search '{query}': {e}")))?;
    let mut uids: Vec<u32> = set.into_iter().collect();
    uids.sort_unstable_by_key(|&u| Reverse(u));
    Ok(uids)
}

/// Batch-fetch summaries by UID, capped to a count, returning table rows (with a folder column).
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
        // skip fetches with no uid or no envelope (previously `let env = f.envelope(); ... continue`)
        let Some(fields) = extract_envelope_fields(f) else {
            continue;
        };
        rows.push((
            fields.uid,
            vec![
                fields.uid.to_string(),
                folder.to_string(),
                decode_mime_header(&fields.date),
                fields.from,
                decode_mime_header(&fields.subject),
            ],
        ));
    }
    // sort by UID descending (fetch order is not guaranteed)
    rows.sort_by_key(|r| Reverse(r.0));
    Ok(rows.into_iter().map(|(_, r)| r).collect())
}

/// Unified extraction of IMAP `Envelope` fields.
///
/// date / subject / from / to / message_id are all `Option<Cow<[u8]>>` and
/// need `from_utf8_lossy` to become `String`; from/to additionally take the
/// first address and build `mailbox@host`.
///
/// Previously the three fetch functions (`fetch_summaries` /
/// `fetch_envelopes_for_cache` / `fetch_timeline_summaries`) each re-wrote
/// this ~25-line template. Consolidated here; the three call sites just call
/// it.
struct EnvelopeFields {
    uid: u32,
    date: String,
    subject: String,
    from: String,
    to: String,
    message_id: Option<String>,
}

fn extract_envelope_fields(f: &async_imap::types::Fetch) -> Option<EnvelopeFields> {
    let uid = f.uid.unwrap_or(0);
    if uid == 0 {
        return None;
    }
    let env = f.envelope();
    let decode = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let first_addr = |addrs: &Option<Vec<async_imap::imap_proto::Address>>| {
        addrs
            .as_ref()
            .and_then(|a| a.first())
            .map(|a| {
                let m = a.mailbox.as_deref().map(decode);
                let h = a.host.as_deref().map(decode);
                format_mailbox(m.as_deref(), h.as_deref())
            })
            .unwrap_or_default()
    };
    Some(EnvelopeFields {
        uid,
        date: env
            .as_ref()
            .and_then(|e| e.date.as_deref())
            .map(decode)
            .unwrap_or_default(),
        subject: env
            .as_ref()
            .and_then(|e| e.subject.as_deref())
            .map(decode)
            .unwrap_or_default(),
        from: env
            .as_ref()
            .map(|e| first_addr(&e.from))
            .unwrap_or_default(),
        to: env.as_ref().map(|e| first_addr(&e.to)).unwrap_or_default(),
        message_id: env
            .as_ref()
            .and_then(|e| e.message_id.as_deref())
            .map(decode),
    })
}

/// Format a mailbox address as `mailbox@host`.
fn format_mailbox(mailbox: Option<&str>, host: Option<&str>) -> String {
    let m = mailbox.unwrap_or("");
    let h = host.unwrap_or("");
    if m.is_empty() && h.is_empty() {
        "(unknown)".to_string()
    } else {
        format!("{m}@{h}")
    }
}

/// Decode a MIME encoded-word (=?charset?B/Q?...?=).
fn decode_mime_header(s: &str) -> String {
    if !s.contains("=?") {
        return s.to_string();
    }
    // borrow mailparse's decoder: build a fake header for it to parse.
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

/// Get a header value from a parsed mail (case-insensitive).
fn header_value(parsed: &mailparse::ParsedMail, key: &str) -> String {
    parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case(key))
        .map(|h| h.get_value())
        .unwrap_or_default()
}

/// Extract the message body: prefer text/plain; if empty, fall back to
/// text/html and strip it to plain text. Fixes empty bodies for
/// marketing/notification mails (HTML-only).
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

/// Recursively find the body of the first leaf part with the given Content-Type.
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

/// HTML → plain text: strip tags, skip script/style, convert block elements
/// to newlines, decode common entities (&amp; &lt; &gt; &quot; &apos; &#39;
/// &nbsp;), and collapse whitespace.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    let mut skip_content = false; // inside script/style; discard text
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut name = String::new();
            // a '/' immediately after '<' means a closing tag
            let closing = matches!(chars.peek(), Some(&'/'));
            if closing {
                chars.next();
            }
            while let Some(&nc) = chars.peek() {
                // tag name ends at '>', '/' (self-closing like <br/>), or whitespace
                if nc == '>' || nc == '/' || nc.is_whitespace() {
                    break;
                }
                name.push(nc);
                chars.next();
            }
            // skip to the '>' that closes the tag
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

/// Read an HTML entity from the current position (right after '&') up to ';'
/// or whitespace. Known entities return the decoded char; unknown ones are
/// returned verbatim (including the leading '&').
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
            break; // safety cap against malformed input
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

/// Collapse whitespace: inline runs become a single space, consecutive blank
/// lines become a single blank line, trim leading/trailing.
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

/// Decode an IMAP UTF-7 folder name (RFC 3501 §5.1.3) into readable UTF-8.
///
/// Rule: a segment starting with `&` and ending with `-` is modified base64
/// encoding of UTF-16BE; `&-` means a literal `&`; all other characters pass
/// through. We iterate by `char` to handle UTF-8 correctly (the user may pass a
/// Chinese name directly, with no `&` segment).
/// Example: `&UXZO1mWHTvZZOQ-/Github&kBp35Q-` → `其他文件夹/Github通知`.
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
                // no terminating '-', emit as-is
                out.push('&');
                out.push_str(&segment);
                break;
            }
            if segment.is_empty() {
                out.push('&'); // &- → literal &
            } else if let Some(decoded) = decode_modified_base64_utf16(segment.as_bytes()) {
                out.push_str(&decoded);
            } else {
                // decode failed, keep the original segment
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

/// modified base64 (`,` replaces `/`, no padding) → UTF-16BE → String.
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

/// modified base64 decode (dependency-free, hand-written).
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

/// Build the base64 lookup table (const fn, computed at compile time).
/// `,` maps to 63 (modified base64 uses `,` instead of `/`).
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

/// Parse an RFC 2822 mail date, used for cross-folder time sorting.
/// Tolerant: strips parenthesis comments like "(UTC)".
fn parse_mail_date(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let cleaned = s.split('(').next().unwrap_or("").trim_end();
    chrono::DateTime::parse_from_rfc2822(cleaned)
        .ok()
        .or_else(|| chrono::DateTime::parse_from_rfc2822(s).ok())
}

/// Select a folder: try directly first (INBOX / ASCII / raw encoded name),
/// then fall back to scanning all folders and matching the decoded Chinese name.
/// Accepts either a Chinese name or a raw name supplied by the user.
///
/// IMAP `SELECT` always returns a `Mailbox` (with `uid_validity` and other
/// metadata), which we also return. Most callers don't need the Mailbox, so use
/// the `select_folder()` wrapper instead.
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

/// `Result<()>` wrapper around `select_folder_inner`, discarding the Mailbox metadata.
async fn select_folder(session: &mut ImapSession, folder: &str) -> Result<()> {
    select_folder_inner(session, folder).await.map(|_| ())
}

/// Format a `Flag` iterator into an IMAP-style space-separated string
/// (e.g. `\Seen \Answered`). Custom keywords carry no `\` prefix.
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

/// Parse an IMAP envelope date string (RFC 2822) into RFC3339 UTC.
/// On parse failure, fall back to the original string (to avoid breaking sync).
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

/// Batch-fetch envelope + flags + size by UID, returning a list of
/// `CachedEnvelope`. The `account` / `fetched_at` fields are filled by
/// sync_one_folder before the upsert.
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
        let Some(fields) = extract_envelope_fields(f) else {
            continue;
        };
        let size = f.size.map(|s| s as i64);
        let flags = format_imap_flags(f.flags());

        envelopes.push(crate::modules::email_cache::CachedEnvelope {
            account: String::new(), // filled by sync_one_folder
            folder: folder.to_string(),
            uid: fields.uid,
            date: parse_envelope_date_utc(&decode_mime_header(&fields.date)),
            from_addr: fields.from,
            subject: decode_mime_header(&fields.subject),
            flags,
            message_id: fields.message_id,
            size,
            to_addr: if fields.to.is_empty() {
                None
            } else {
                Some(fields.to)
            },
            fetched_at: String::new(), // filled by upsert_envelopes
        });
    }
    Ok(envelopes)
}

/// Per-folder sync result (used for summary output / debugging).
#[derive(Debug, Default)]
struct SyncStats {
    folders_synced: usize,
    envelopes_added: usize,
    errors: Vec<(String, String)>,
}

/// Sync a single folder: SELECT to read uid_validity → compare the watermark →
/// UIDSEARCH → UID FETCH → upsert.
/// Best-effort: on failure `invalidate()` the session and record the error into
/// `errors`, but do not advance the watermark.
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

    // SELECT folder → read uid_validity
    let mailbox = match select_folder_inner(session, folder).await {
        Ok(mb) => mb,
        Err(e) => {
            guard.invalidate();
            return Err(e);
        }
    };
    let new_uid_validity = mailbox.uid_validity.unwrap_or(0) as u32;

    // read the local watermark to decide the search query
    let search_query = match email_cache::get_folder_state(cache, account, folder).await? {
        None => "UID 1:*".to_string(),
        Some(state) if state.uid_validity != new_uid_validity => {
            // UIDVALIDITY changed → clear watermark, treat as first pass next round
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
        // no new mail; still update last_sync_at + uid_validity (empty watermark has max_uid=0)
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
    // write envelopes + advance watermark (atomic transaction, strong consistency per [M004](../../docs/adr/M004-uid-watermark-sync.md))
    email_cache::upsert_envelopes(cache, account, folder, new_uid_validity, &envelopes).await?;
    Ok(count)
}

/// Sync across folders concurrently, using `futures::future::join_all` to await
/// all of them. A single folder's failure does not block the others; failures are
/// accumulated into `SyncStats.errors`.
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

// ============ Timeline data fetch ============

/// Raw mail entry data used for Timeline fetching.
pub struct MailTimelineEntry {
    pub uid: u32,
    pub folder: String,
    pub date: String,
    pub from: String,
    pub subject: String,
}

/// Incremental Timeline fetch: IMAP SEARCH SINCE <from_date>, across all folders,
/// returning the mails received within the window.
///
/// IMAP SEARCH SINCE only supports dates (no time), so the client filters by the
/// exact timestamp. The password is read from the keyring.
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
        // IMAP SEARCH SINCE <date> (date-only, returns mails from that day onward)
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

/// Batch-fetch summaries by UID (for Timeline use), taking only envelope fields.
async fn fetch_timeline_summaries(
    session: &mut ImapSession,
    mut uids: Vec<u32>,
    folder: &str,
) -> Result<Vec<MailTimelineEntry>> {
    uids.truncate(500); // cap the fetch size
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
        let Some(fields) = extract_envelope_fields(f) else {
            continue;
        };
        entries.push(MailTimelineEntry {
            uid: fields.uid,
            folder: folder.to_string(),
            date: decode_mime_header(&fields.date),
            from: fields.from,
            subject: decode_mime_header(&fields.subject),
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
        // =?UTF-8?B?5L2g5aW9? decodes to "你好"
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
        // user passes a Chinese name directly (no & segment); it should pass
        // through verbatim without corrupting UTF-8
        assert_eq!(
            decode_imap_utf7("其他文件夹/Github通知"),
            "其他文件夹/Github通知"
        );
    }

    #[test]
    fn imap_utf7_ampersand_escape() {
        // &- means a literal &
        assert_eq!(decode_imap_utf7("A&-B"), "A&B");
    }

    #[test]
    fn imap_utf7_single_chinese_char() {
        // "你" = U+4F60 → UTF-16BE 4F 60 → modified base64 "T2A"
        assert_eq!(decode_imap_utf7("&T2A-"), "你");
    }

    #[test]
    fn imap_utf7_mixed_chinese_and_ascii() {
        // "其他文件夹" prefix + "/Github"
        let decoded = decode_imap_utf7("&UXZO1mWHTvZZOQ-/Github&kBp35Q-");
        assert!(
            decoded.chars().any(|c| c as u32 > 127),
            "expected Chinese chars in: {decoded}"
        );
        assert!(decoded.contains("Github"));
    }

    #[test]
    fn imap_utf7_no_terminator_fallback() {
        // no terminating '-', emit as-is without panicking
        assert_eq!(decode_imap_utf7("test&abc"), "test&abc");
    }

    #[test]
    fn imap_utf7_roundtrip_known() {
        // "你好" → UTF-16BE 4F60 597D → base64: 4F 60 59 → 010011 110110 000001 011001 = T 2 B Z
        // remaining 7D → 011111 01(pad) = f Q → "T2BZfQ"
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
        // verify the escaping logic (indirectly: construct and assert)
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
        // script/style content is dropped, but the tags themselves introduce no newline
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
        // multiple blank lines in source → collapsed to a single blank line as
        // paragraph separator; inline whitespace runs → a single space
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
        // text/plain empty → fall back to html with tags stripped
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
