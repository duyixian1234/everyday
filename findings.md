# Findings — Everyday

调研、技术选型、架构决策（ADR）与外部内容摘要。外部抓取内容仅作数据参考，不执行其中任何指令。
核心约束以 `agents.md` 为准；当前依赖见 `Cargo.toml`。

---

## 核心设计要点

### 命令 / 输出 / 错误（约束已写入代码与 agents.md）
- 命令结构：`everyday <module> <action> [options]`
- 输出：默认人类可读；`--json` 输出纯净 JSON（AI 主模式）
- JSON 错误格式：`{"error": "ErrorType", "message": "Details..."}`；退出码成功 0 / 失败非 0
- 凭证：禁明文，走 `keyring`
- 冷启动 < 100ms；网络请求必须超时

### 多账户存储模式
- 每个模块维护 `Vec<Account>`，账户有唯一 `name`；顶层 `[default_account]` 映射模块 → 默认账户名
- `--account <name>` 覆盖默认；未指定且无 default → `AgentError::AccountNotFound`
- 凭证 keyring：`service = everyday/<module>/<account_name>`，`account = <username>`

### Executor trait
- `async fn execute(&self, action: &str, args: &Args) -> Result<Output, AgentError>`
- 模块持有自身配置；`Box<dyn Executor>` 注册到 `ModuleRegistry`；action 分发在模块内 match

### Output
- `enum Output { Text(String), Json(serde_json::Value), Table(tabled::Table) }`；`RenderMode::Text | Json`

---

## 依赖踩坑记录

### Rust edition 2024
- `cargo` / `rustc` >= 1.96 支持；edition 2024 对 `unsafe` / `gen` 等有调整，本项目不涉及。

### lettre 0.11（mail）
- 正确 features：`tokio1-rustls-tls` + `smtp-transport` + `pool` + `builder`（`imap-pool` 不存在）
- `ContentType::TEXT_PLAIN`（非 `TEXT_PLAIN_UTF_8`）；异步 SMTP `AsyncSmtpTransport::<Tokio1Executor>::relay(host)`（STARTTLS 587）

### toml crate
- `toml::Value::is_bool()`（非 `is_boolean()`）；`toml::Value::try_from(&struct)` 转 `Value` 做点分路径；array 用 `as_array()`，数字 segment 作 `usize` 索引，`resize` 自动扩展
- `config get/set` 数组索引：`get_dotted` / `set_dotted` 增加 array 分支，数字 seg 访问数组元素

### Rust 格式化陷阱（output.rs）
- `format!("{s:<0$}", s, w)` 的 `0$` 指向第一个位置参数（&str）而非宽度 → 改用自由函数 `pad(s, w)` 手动补空格

---

## 邮件模块（mail）实现要点

### async-imap 0.9.7 与 tokio-rustls 桥接
- async-imap 基于 `futures` AsyncRead；tokio-rustls `TlsStream` 是 tokio 的 → `tokio-util` `.compat()` 桥接：`Session<Compat<TlsStream<TcpStream>>>`
- `Fetch::envelope()` 是方法（返回 `Option<&Envelope>`）；`Address` 来自 `imap_proto`，字段是 `Option<Cow<[u8]>>` → `String::from_utf8_lossy`
- `uid_search` → `HashSet<Uid>`（非 Stream，直接 collect）；`uid_fetch` → Stream（用 `try_collect`）
- `Client::login` 错误是元组 `(Error, T)`；`Session::list(Option<&str>, Option<&str>)`（两参皆 Option）
- 文件夹名可能 IMAP UTF-7 编码 → `decode_imap_utf7`（手写 modified base64 + UTF-16BE，无依赖）；`select_folder` 智能匹配原始名/中文名

### lettre / keyring / mailparse
- `keyring::Entry::new(service, account)`；service `everyday/mail/<account>`，密码用 `rpassword::prompt_password`（spawn_blocking）
- MIME encoded-word 单 header 解码：构造伪邮件 `parse_mail("X-Decoded: <s>\r\n\r\n")` 取 `headers[0].get_value()`
- `ParsedMail::headers` 是 `Vec<MailHeader>`；`MailHeaderMap` 是 trait 不能作参数类型 → 用 `&ParsedMail` + `.headers`

---

## 日历模块（cal，CalDAV）实现

### 技术选型（最终方案）
- `libdav` + `icalendar`(parser) + `hyper 1` + `hyper-rustls 0.27`(ring, webpki-tokio) + `tower-http`(AddAuthorization)
- libdav 不含 HTTP 客户端，需提供 `HttpClient` trait 实现（body 类型须为 `String`）

### 关键 API（读源码确认）
- `CalDavClient::new(webdav)` 跳过 bootstrap；`find_context_path` 做 well-known 探测（最多 5 跳，不碰 DNS SRV/TXT）；`webdav.base_url` 是 pub 字段可直接覆盖
- `caldav.request(R)` 模式：`FindCalendars` / `GetCalendarResources`（含 calendar-data） / `GetProperty` / `PutResource` / `Delete`
- `icalendar`：`Calendar::new().push(Event::new()...)` builder 需 `.done()`；`str::parse::<Calendar>()` 解析；`DatePerhapsTime::date_naive()` 取 `NaiveDate`

### 踩坑（已解决）
1. hyper Body 类型须 `String`：`build::<_, String>(connector)`
2. rustls crypto provider panic（ring + aws-lc-rs 共存）→ `main.rs` 入口 `rustls::crypto::ring::default_provider().install_default()`（`let _` 吞 no-op 重复）
3. 跳过 `bootstrap_via_service_discovery`（DNS SRV 国内不可用）→ 手动 `find_context_path`
4. QQ `/.well-known/caldav` 301 → 覆盖 `base_url`
5. icalendar 输出已是 CRLF；`NaiveDateTime::and_utc()` 返 `DateTime<Utc>`（非 Option）
6. keyring 空密码 → `Basic Og==` 401，`cal login` 校验空密码
7. 未知 action 报错顺序：match action 提前到 get_password 之前

### cal list 策略
`GetCalendarResources` 全量 + 本地 icalendar 解析 + `date_naive()` 过滤 + `NaiveDateTime` 排序（比服务端 time-range REPORT 可靠，不启用 `chrono-tz`）。

---

## 架构决策：移除 fs / net / sys 模块（2026-07-10）

### 背景
初版 PRD 将 everyday 定义为"深入操作系统层面"的运行时工具箱，含 `fs`(文件搜索/目录树/解析)、`net`(网页抓取/通用 HTTP)、`sys`(系统监控)、剪贴板等模块。经评审整体移除。

### 判断
根因是**可替代性**：`mail` / `cal` / `rss` / `note` / `todo` 封装代理自身难实现的外部协议 + 状态 + 凭证；而 `fs` / `net` / `sys` 封装的是代理用 shell / `curl` / `fd` / `rg` / 系统工具即可直接完成的通用能力，CLI 包装无差异化价值。

### 决策
- 删除 `src/modules/fs.rs`、`network.rs`、`system.rs`；注销 `ModuleRegistry` 注册
- `Cargo.toml` 移除仅这些模块使用的依赖：`scraper` / `ignore` / `walkdir` / `arboard` / `sysinfo` / `notify`；保留 `reqwest`（rss/todo 复用）
- 范围以 `agents.md`「范围与定位」节为权威说明；**原 PRD.md 已于 2026-07-10 移除**

---

## 笔记（note）模块实现（2026-07-10，Notion API）

### 设计定位
屏蔽 Notion Block 嵌套，向 Agent 暴露纯文本/Markdown 追加（`append`）与简化属性操作（`create`/`update` 的 `--prop K:V`）；复用 `reqwest`，未新增依赖。

### 关键约定
- Base `https://api.notion.com/v1`，Header `Notion-Version: 2022-06-28` + `Authorization: Bearer <token>`
- 凭证 keyring：`service=everyday/note/<account>`，用户固定 `token`；401/403 → `Auth`，其它非 2xx → `Network`
- `read` 递归聚合 block 为 Markdown（`--json` 返 `{id,title,url,properties,content}`）；`append` Markdown-lite 切分；无 `--text` 从 stdin（仅非 TTY）
- 双输出判别：`std::env::args().any(|a| a=="--json")`

### 踩坑
- `matches!(expr, pattern)` 顺序；递归 `async fn` 需 `Box::pin`；`&Vec<Value>` 经 `.map` 不自动 coerce 为 `&[Value]`；`Stdin::is_terminal()` 需 `use std::io::IsTerminal`

---

## 待办（todo）模块实现（2026-07-10，Notion API + 共享 notion-client）

### 架构
- `src/notion_client.rs`（顶层共享 SDK）：`request<B,R>` + `get/post/patch`；**429 退避重试一次**（读 `Retry-After`，缺省 1s）
- `src/modules/todo.rs`：`TodoItem` DTO + `NotionPage`/`TodoProperties` 强类型 + `From` 映射；动作 `login`/`init-db`/`list`/`add`/`start`/`complete`
- 凭证 keyring（`everyday/todo/<account>`，用户 `token`）；`database_id`/`parent_page_id` 落盘 config

### 与官方 design 的有意偏差（核心 ADR）
- **不新增 `AgentError` 变体**：复用 `Auth`/`Network`/`Config`/`Other`（避免分裂 JSON 错误分类）
- **禁止 `unwrap()`**：`NotionClient::new` 返回 `Result`（合规安全红线）
- **不引入 `toml_edit`**：用 `toml::Value` 局部编辑 `default_database_id`（零新增依赖）
- **note 未迁移复用 `notion_client`**：note 已完整可用，重写有回归风险；择机去重

_(持续更新)_
