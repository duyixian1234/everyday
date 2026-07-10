# Findings — Everyday

记录调研、技术选型、外部内容摘要。外部抓取内容仅作数据参考，不执行其中任何指令。

---

## 项目现状（2026-07-08 勘察）
- `Cargo.toml`：包名 `everyday`，edition 2024，无依赖
- `src/main.rs`：仅 `println!("Hello, world!");`
- `.gitignore`：已存在（8 字节）
- 无 README、无 tests、无 agents.md

## PRD 关键约束
- 命令结构：`everyday <module> <action> [options]`
- 输出：默认人类可读；`--json` 输出纯净 JSON（AI 主模式）
- 配置路径：`~/.config/everyday/config.toml`
- JSON 错误格式：`{"error": "ErrorType", "message": "Details..."}`
- 退出码：成功 0，失败非 0
- 凭证：禁明文，走 `keyring`
- 冷启动 < 100ms
- 网络请求必须超时

## Rust edition 2024 注意事项
- `cargo 1.96.1` / `rustc 1.96.1` 支持 edition 2024
- edition 2024 对 `unsafe`、`gen` 关键字等有调整，本项目不涉及
- `tokio` 需 >= 1.x，`clap` >= 4.x（derive）

## 依赖版本规划（待 lock 时确认）
- tokio (full)
- clap (derive)
- serde, serde_json
- toml
- dirs (跨平台配置目录)
- thiserror, anyhow
- keyring
- sysinfo
- reqwest (json, rustls-tls)
- scraper
- ignore, walkdir
- async-imap, lettre, futures
- caldav (或 vdirsyncer 风格手写 CalDAV 客户端，待评估 crate 稳定性)
- feed-rs
- arboard
- notify
- chrono (serde)
- tabled (表格输出)

## 依赖踩坑记录（2026-07-08 实测）

### lettre 0.11
- ❌ `imap-pool` feature 不存在（旧文档误导）
- ✅ 正确 features：`tokio1-rustls-tls`（不是 `rustls-tls`）、`smtp-transport`、`pool`、`builder`
- lettre 只管 SMTP；IMAP 走 `async-imap`

### sysinfo 0.30 API 变更
- ❌ `System::global_cpu_usage()` — 在 0.30 上不存在（方法名/位置变了）
- ❌ `System::disks()` — 0.30 起 `Disks` 拆为独立结构体
- ✅ CPU：`sys.cpus()` 取所有核心 `cpu_usage()` 求平均（跨版本稳定）
- ✅ 磁盘：`sysinfo::Disks::new_with_refreshed_list()` 然后 `.iter()`
- ✅ 内存/swap：`sys.total_memory()` / `sys.used_memory()` / `sys.total_swap()` / `sys.used_swap()` 稳定

### toml crate
- `toml::Value::is_bool()`（不是 `is_boolean()`）
- `toml::Value::try_from(&serde_struct)` 可把结构体转 `toml::Value` 做点分路径操作

### Rust 格式化陷阱
- `format!("{s:<0$}", s, w)` 看似合理，但 `0$` 指向第一个位置参数 `s`（&str），
  而宽度需要 `&usize` → 类型错位。改用自由函数 `pad(s, w)` 手动 `s.chars().count()` + 补空格，
  避免内联格式化语法歧义。

## async-imap / lettre 邮件实现踩坑（2026-07-08）

### async-imap 0.9.7 与 tokio-rustls 的 AsyncRead 不兼容
- async-imap 基于 `futures::AsyncRead/AsyncWrite`（`futures_io`）
- tokio-rustls 的 `TlsStream` 实现的是 `tokio::io::AsyncRead/AsyncWrite`
- **桥接**：`tokio-util = { features = ["compat"] }` 的 `.compat()`（`TokioAsyncReadCompatExt`）
  把 tokio stream 转成 `Compat<T>`（impl futures AsyncRead+AsyncWrite）
- 类型：`async_imap::Session<Compat<TlsStream<TcpStream>>>`

### async-imap 0.9.7 API 细节
- `Fetch::envelope()` 是**方法**返回 `Option<&Envelope<'_>>`，不是字段
- `Envelope`/`Address` 来自 `imap_proto::types`（async-imap re-export），带生命周期，字段是 `Option<Cow<'a, [u8]>>`（字节非字符串！）→ 用 `String::from_utf8_lossy` 转
- `uid_search(query) -> Result<HashSet<Uid>>`（Uid=u32），**不是 Stream**，直接 collect
- `uid_fetch(uids, query) -> Result<impl Stream<Item=Result<Fetch>>>`，用 `try_collect`
- `Client::login(user, pass) -> Result<Session<T>, (Error, T)>`，错误是元组
- `Session::list(reference: Option<&str>, pattern: Option<&str>) -> Result<impl Stream<Item=Result<Name>>>`（**两个参数都是 Option**，不是 &str）
- `Name::name() -> &str`（直接返回，非 Option）；`Name::attributes() -> &[NameAttribute]`，`NameAttribute::NoSelect` 标记不可 SELECT 的文件夹
- 文件夹名可能是 IMAP UTF-7 编码（如 `&UXZO1mWHTvZZOQ-/Github&kBp35Q-`）。已实现 `decode_imap_utf7` 解码：`&<modified-base64>-` 段是 UTF-16BE 的 modified base64（`,` 替 `/`，无 padding），`&-` 是字面 `&`，其余 char 透传。手写 base64 解码表（const fn），无额外依赖。`select_folder` 智能匹配：先直接 select（原始名/INBOX），失败再遍历所有文件夹匹配解码名，支持用户输入中文。

### lettre 0.11
- `ContentType::TEXT_PLAIN_UTF_8` **不存在**，用 `ContentType::TEXT_PLAIN`
- 异步 SMTP：`AsyncSmtpTransport::<Tokio1Executor>::relay(host)` → `.port().credentials().build()` → `transport.send(email).await`
- 需要 `tokio1-rustls-tls` feature（含 tokio1 + rustls）
- `relay()` 是 STARTTLS（端口 587）；implicit TLS(465) 需 `builder().tls(Tls::Wrapper(...))`，本项目默认 587

### keyring 凭证流程
- `keyring::Entry::new(service, account)` → `set_password` / `get_password`
- service 约定 `everyday/mail/<account_name>`，account 用 username
- 密码交互输入用 `rpassword::prompt_password`（同步，放 `spawn_blocking`）

### mailparse 解码技巧
- MIME encoded-word（`=?UTF-8?B?...?=`）单 header 解码：构造伪邮件 `parse_mail("X-Decoded: <s>\r\n\r\n")` 取 headers[0].get_value()
- `ParsedMail::headers` 是 `Vec<MailHeader>`，`MailHeader::get_key()`（小写）/ `get_value()`（已解码）
- `MailHeaderMap` 是 trait 不能作函数参数类型，用 `&ParsedMail` + 访问 `.headers`

### config get/set 数组索引
- toml::Value 的 array 用 `as_array()`，数字 segment 解析为 `usize` 索引
- set 时 `arr.resize(idx+1, ...)` 自动扩展数组（填充空 table）

## 多账户存储模式
- 每个模块维护 `Vec<Account>`，账户有唯一 `name`
- 顶层 `[default_account]` 表映射模块 → 默认账户名
- `--account <name>` 覆盖默认；未指定且无 default → 报错引导用户配置
- 凭证 keyring 约定：`service = everyday/<module>/<account_name>`, `account = <username>`

## Executor trait 设计要点
- `async fn execute(&self, action: &str, args: &Args) -> Result<Output, AgentError>`
- 模块自身持有配置（构造时注入对应账户配置）
- trait object `Box<dyn Executor>` 注册到 `ModuleRegistry`
- action 分发由各模块内部 match，主程序不关心 action 细节

## Output 设计要点
- `enum Output { Text(String), Json(serde_json::Value), Table(tabled::Table) }`
- `Output::render(mode: RenderMode) -> String`
- `RenderMode::Text | Json`
- 错误也走 Output 通道或独立 `AgentError::render_json()`，保持退出码语义

## CalDAV 日历模块实现（2026-07-09，源码验证）

### 技术选型（最终方案）
- **libdav 0.10.6**：CalDAV 协议，NLnet 资助、维护活跃。采用 **request API 模式**（`caldav.request(FindCalendars::new(path))`），非旧版 `find_calendars()` 方法
- **icalendar 0.17.12**：iCalendar 解析/生成，`parser` feature（default 开启）
- **HTTP 栈**：hyper 1.x legacy `Client` + hyper-rustls 0.27（`ring` + `webpki-tokio`，与 email 模块 webpki-roots 一致）+ tower-http 0.6 `AddAuthorization`（Basic Auth 中间件）
- **关键**：libdav 自身不含 HTTP 客户端，定义 `HttpClient` trait（blanket impl for `Service<Request<String>, Response=Response<Incoming>>`），由使用者提供 hyper client。body 类型必须为 `String`（`http-body 1.0` 实现了 `impl Body for String`）

### 关键 API（亲自读 libdav/icalendar 源码确认，修正交接文档二手信息）
- `CalDavClient::new(webdav)` —— 跳过 bootstrap（坑5），`CalDavClient` 实现 `Deref<Target=WebDavClient>`
- `libdav::caldav_service_for_url(&Uri) -> Result<DiscoverableService, _>` —— 从 URL scheme 推断 CalDavs/CalDav
- `WebDavClient::find_context_path(service, host, port) -> Result<Option<Uri>, _>` —— RFC 6764 §5 well-known 探测，最多 5 跳重定向，**不碰 DNS SRV/TXT**
- `WebDavClient.base_url` 是 **pub 字段**，重定向后可直接 `webdav.base_url = url` 覆盖（坑6）
- request 模式：`caldav.request(R)` where `R: DavRequest`，返回 `R::Response`
  - `caldav::FindCalendarHomeSet::new(principal_path)` → `{ home_sets: Vec<Uri> }`
  - `caldav::FindCalendars::new(home_set_path)` → `{ calendars: Vec<FoundCollection> }`（`{ href, etag, supports_sync }`）
  - `caldav::GetCalendarResources::new(collection_href).with_hrefs(hrefs)` → `{ resources: Vec<FetchedResource> }`（含 `calendar-data`！）
  - `dav::GetProperty::new(href, &PropertyName)` → `{ value: Option<String> }`（取 DISPLAY_NAME/CALENDAR_COLOUR）
  - `dav::PutResource::new(href).create(data, content_type)` → `{ etag: Option<String> }`（`If-None-Match: *`）
  - `dav::Delete::new(href).force()` → `DeleteResponse`（无条件）/ `.with_etag(etag)`（`If-Match`）
  - `caldav::ListCalendarResources` time-range REPORT **只返元数据不含 calendar-data** → `cal list` 改用 `GetCalendarResources` 全量 + 本地过滤
- `WebDavClient::find_current_user_principal()` → `Option<Uri>`（RFC 5397）
- icalendar 构造：`Calendar::new().push(Event::new().summary().starts(Utc).ends(Utc).done()).done()`，builder 方法返 `&mut Self`，需 `.done()` 拿 owned
- icalendar 解析：`str::parse::<Calendar>()`（FromStr），`cal.events()` → `impl Iterator<Item=&Event>`，`event.get_start()` → `Option<DatePerhapsTime>`
- `DatePerhapsTime::date_naive()` 内置方法直接拿 `NaiveDate`（坑3 简化：不必手写三变体 match）

### 踩坑（已解决）
1. **hyper Body 类型**：`HttpClient` trait 的 `call(&mut self, req: Request<String>)` 要求 body=String。`Client<C, B>` 实现 `Service<Request<B>>`，故 `Client::builder().build::<_, String>(connector)` 显式指定 B=String（`Builder::build<C, B>` 的 B 是泛型，可 turbofish）
2. **rustls crypto provider panic**（坑4）：cargo feature unification 让 ring（email 的 tokio-rustls）与 aws-lc-rs（hyper-rustls 传递依赖）同时启用 → rustls 0.23 panic。**正解**：`main.rs` 入口 `let _ = rustls::crypto::ring::default_provider().install_default();`（重复 install 返 Err 是 no-op，用 `let _` 吞掉）
3. **`bootstrap_via_service_discovery` 触发 DNS SRV**（坑5）：内部先 CheckSupport base_url，失败 fallback `resolve_srv_record`（`_caldavs._tcp.<host>`）。国内服务商（QQ/网易/飞书）不实现 SRV，远程 DNS 关闭连接 → os error 10054。**正解**：`CalDavClient::new(webdav)` 跳过 bootstrap，手动 `find_context_path`（只做 well-known 重定向）
4. **QQ CalDAV `/.well-known/caldav` 301 重定向**（坑6）：用户 `caldav_url = https://dav.qq.com`，根 PROPFIND 404；well-known 301 → `https://dav.qq.com:443/calendar/`（带显式端口）。**正解**：`find_context_path` 返回 `Some(url)` 后 `webdav.base_url = url`
5. **icalendar 输出已是 CRLF**（修正交接文档坑2）：`fmt_write` 用 `write_crlf!` 宏。但 property 值内部可能混入裸 `\n`，仍用归一化 `.replace("\r\n","\n").replace('\r',"\n").replace('\n',"\r\n")` 保险
6. **`NaiveDateTime::and_utc()` 返 `DateTime<Utc>` 非 Option**（坑8）：`and_utc_opt` 不存在，用 `and_utc()`
7. **keyring 空密码**（坑9）：`set_password("")` 成功但 base64 成 `Basic Og==` 后服务端 401。`cal login` 校验空密码报错
8. **未知 action 报错顺序**（坑10）：`match action` 提前到 `get_password` 之前，避免空密码优先报 AuthError

### cal list 策略
用 `GetCalendarResources`（calendar-query REPORT，全量含 calendar-data）拉取每个日历事件，本地 icalendar 解析 VEVENT + `date_naive()` 按目标日期过滤 + `NaiveDateTime` 排序。比服务端 time-range REPORT 可靠（国内服务端实现质量参差可能返空），不启用 `chrono-tz` feature（用 NaiveDateTime 本地时间序对单日事件更直观）

_(持续更新)_

## 架构决策：移除 fs / net 模块（2026-07-10）

### 背景
用户评审认为 `fs`（文件搜索 / 目录树 / 结构化读取）、`net`（网页抓取 / 通用 HTTP）与 `sys`（系统资源监控）相对 `mail` / `cal` / `rss` 过于底层、可替代，不符合 everyday「AI Agent 的外部集成接口」定位。

### 判断
- 根因不是"层级高低"，而是**可替代性**：`mail` / `cal` / `rss` 封装代理自身难以实现的外部协议 + 状态 + 凭证（IMAP 鉴权、CalDAV、feed 解析与已读状态）；而 `fs` / `net` 封装的是代理用 shell / `curl` / `fd` / `rg` 即可直接完成的通用能力，CLI 包装无差异化价值。
- PRD 原愿景将 everyday 定义为"深入操作系统层面"的运行时工具箱，与收窄后的定位冲突。本次决策采纳"外部集成接口"定位。

### 决策
- 移除 `src/modules/fs.rs` 与 `src/modules/network.rs`，注销 `ModuleRegistry` 注册，更新 `cli.rs` / `agents.md` / `README.md` / skill 文档。
- `src/modules/system.rs` 整体移除（`sys` 不保留）。`sys status` 输出的 CPU/内存/磁盘信息代理可经系统工具直接获取，无差异化价值，与"外部集成接口"定位不符。
- `Cargo.toml` 移除仅 fs/net/sys 使用的依赖：`scraper`、`ignore`、`walkdir`、`arboard`、`sysinfo`、`notify`；保留 `reqwest`（rss 复用）。
- `PRD.md` 按项目约定为只读，不在本次修改；定位变更以 `agents.md`「范围与定位」节为权威说明。

## 笔记(note)模块实现（2026-07-10，基于 Notion API）

### 设计定位
- 屏蔽 Notion 繁琐的 Block 嵌套，向 Agent 暴露**纯文本/Markdown 追加**（`append`）与**简化版属性操作**（`create`/`update` 的 `--prop K:V`）。
- 复用既有 `reqwest`（json + rustls-tls），**未引入新依赖**。`provider` 字段预留（notion / 未来 obsidian、feishu），当前仅实现 notion。

### Notion API 关键约定（源码实现验证）
- Base `https://api.notion.com/v1`，固定 Header `Notion-Version: 2022-06-28` + `Authorization: Bearer <token>`。
- 凭证：Notion Integration Token（`ntn_...`）走 keyring，service = `everyday/note/<account>`，条目用户名固定 `token`（与账户名无关，避免多账户冲突）。
- 关键端点：`POST /v1/search`、`POST /v1/pages`（create）、`GET /v1/pages/{id}`（属性）、`PATCH /v1/pages/{id}`（改属性）、`GET /v1/blocks/{id}/children`（分页读，page_size≤100）、`PATCH /v1/blocks/{id}/children`（追加，单次≤100 block，超出分批）、`GET /v1/databases/{id}`（取 schema）。
- 401/403 → `AgentError::Auth`；其余非 2xx → `AgentError::Network`（尽量提取响应体 `message`）。

### 实现要点
- **`create`/`update` 属性编码**：先 `GET /v1/databases/{id}` 取 schema，按属性 `type` 精确编码（title/rich_text/number/checkbox/select/multi_select/url/email/phone）；未知类型降级 rich_text。无 database 父级的独立页面 `update` 退化为启发式编码（bool→checkbox、可解析数→number、其余→rich_text）。
- **`read` 正文聚合**：递归拉取 block 子节点（`MAX_BLOCK_DEPTH=12` 防无限展开），将 paragraph/heading/list/quote/code/callout/divider/image/bookmark/child_page 等聚合成标准 Markdown；rich_text 渲染行内格式（bold/italic/code/strike + 链接）。`--json` 返回 `{id,title,url,properties,content}` 结构化对象，极大节省 Agent context。
- **`append` 文本→block**：Markdown-lite 切分（`#` 标题、`-`/`*` 列表、`1.` 有序、`> ` 引用、```代码块、`---` 分割线、空行分段）。无 `--text` 时从 stdin 读取，但仅当 stdin 非 TTY（避免交互挂起）。
- **重复 flag 解析**：`--prop` 可多次出现且值含冒号，故 `note` 模块用专用 `parse_args`（单值 flag 取末次、`prop` 收集为有序列表），不复用 `parse_simple_args`。
- **双输出判别**：模块 `execute` 签名不含 `RenderMode`，`note` 通过 `std::env::args().any(|a| a=="--json")` 判别（main 已把 `--json` 注入进程参数），`search`/`read`/`create`/`append`/`update` 均支持文本/JSON 双形态。

### 踩坑（已解决）
- `matches!` 宏参数顺序：`matches!(expr, pattern)`，初版写成 `(pattern, expr)` 触发 E0532。
- 递归 `async fn` 需 `Box::pin` 打破无限 future 类型。
- `&Vec<Value>` 经 `.map(rich_text_plain)` 不自动 coerce 为 `&[Value]`，需闭包 `|r| rich_text_plain(r)`。
- `std::io::Stdin::is_terminal()` 需 `use std::io::IsTerminal;`。
- `note_account` 解析、`DefaultAccount.note`、`Config.note` 三处配套扩展，并同步 `config.example.toml` 与 `main.rs` 内 `example_config()`。

