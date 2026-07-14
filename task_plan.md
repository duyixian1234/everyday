# Everyday 开发计划

**项目：** Everyday — The Rust-powered hands for your AI Agent
**范围：** 以 `agents.md`「范围与定位」节为权威说明（原 PRD.md 已移除）
**启动时间：** 2026-07-08
**当前状态：** v0.10.0 已发布（Phase 15：memory 模块落地，append-only 三元组 + 当前态视图 + graph + Searchable）。
**文件维护规则：** 阶段计划 + 错误表 + 设计决策摘要；禁止保留任务执行细节
（子任务清单、完成小结、中途修复明细）——见 [governance.md](./governance.md) §4.1。
详细 ADR 全文见 [docs/adr/](./docs/adr/README.md)。

---

## 总体目标

打造高性能、内存安全的本地 CLI 工具集，作为 AI Agent 的"数字双手"。统一命令结构 `everyday <module> <action> [options]`，支持 Text / JSON 双输出模式，JSON 为 AI 交互主模式。

---

## 阶段规划

### Phase 1: 项目地基与文档 [complete]
基础架构：`Cargo.toml`（包名 `everyday`，edition 2024）、`agents.md` 协作规范、`src/` 骨架（cli/config/error/output/modules）。

### Phase 2: 配置系统（多账户） [complete]
`config.rs` 加载与多账户合并、`everyday config set/get/list/path/init`、凭证走 keyring（config 只存元数据）、`config.example.toml`。

### Phase 3: 核心抽象 [complete]
`AgentError` 统一错误 + JSON 格式；`Output` 结构体（Text/Json/Table）；`Executor` trait + `ModuleRegistry`。

### Phase 4: CLI 框架 [complete]
`cli.rs`（clap derive，扁平结构 + 子命令帮助预扫描）、`main.rs`（解析 → 配置 → 查找 → 执行 → 渲染 → 退出码）。

### Phase 5: 模块骨架 [complete]
各模块实现 `Executor`；未实现/未知动作返回 `NotImplemented`/`UnknownAction`；注册到 `ModuleRegistry`。初版曾含 `system`/`network`/`fs` 骨架，已在 Phase 6 之前整体移除（[F003](./docs/adr/F003-module-scope-external-integration.md)）。

### Phase 6: 模块实现 [complete]
`mail`（IMAP list/read/search + SMTP send + keyring login） / `calendar`（CalDAV：login/calendars/list/add/delete） / `rss`（feed-rs：follow/list/unfollow/digest/fetch） / `note`（Notion 笔记/知识库） / `todo`（Notion 待办 + 共享 notion-client）落地。

### Phase 7: 构建、测试、文档、发布 [complete]
全模块 `cargo build` / `clippy` / 单测 + 集成测试全绿；README + skills 文档与代码一致；CI（三平台 + aarch64 macOS）+ release workflow；**v0.4.0 已发布**（bookmark 模块 + 模块分层 shared/util + Justfile + README 国际化 + cargo fmt 门槛）。

### Phase 8: 中英文 README [complete]
根 `README.md` 与 `skills/README.md` 改写为英文；完整中文文档保留为 `README_ZH.md`；两侧顶部均加语言切换链接。

### Phase 9: Timeline 统一事件层 [complete]
按 `CONTEXT.md` + 9 个 ADR（[L001–L012](./docs/adr/L001-append-only-event-log.md)）实现。`src/modules/timeline.rs` + `timeline/{store,providers,orchestrator}.rs` + `src/ops_log.rs` AOP hook；6 模块暴露 `fetch_for_timeline(window)`；Cal 例外为窗口刷新。**v0.5.0 已发布**。

### Phase 10: Mail Cache（envelope 缓存 + 并发 sync）[complete]
按 ADR [M002](./docs/adr/M002-imap-connection-pool.md)–[M005](./docs/adr/M005-staleness-auto-sync.md) 实现。`src/modules/email_cache.rs`（mail_cache.db 双表 + K1 append-only）+ `src/modules/email_pool.rs`（M=4 + Arc<Semaphore>）；`mail list` 改造为 cache → staleness → 并发 sync → 本地 envelope；search/read/send 仍直连 IMAP。**v0.6.0**（+ v0.6.1 [L013](./docs/adr/L013-from-explicit-error.md) + v0.6.2 Rust 1.97 clippy 注释 lint 修复）均已发布。

### Phase 11: 跨模块统一搜索（Search）[complete]
按 ADR [S001](./docs/adr/S001-search-architecture.md)–[S006](./docs/adr/S006-search-module-cli.md) 落地。`src/search.rs`：`Searchable` trait + `SearchQuery`/`Hit`/`SearchOutcome` + `SearchRegistry`（best-effort 并发扇出，per-module cap 50，global cap 20）。v1 适配器：note/todo/bookmark（本地 SQLite GLOB，[R008](./docs/adr/R008-sql-glob-not-like.md)）/ rss（新增本地条目缓存表）/ cal（full-pull + in-memory GLOB）；mail 推迟 v1.1。**v0.7.0 已发布**。

### Phase 12: 凭据 / login 逻辑收拢到顶层 auth 模块 [complete]
按 ADR [R013](./docs/adr/R013-auth-module-consolidation.md) 收拢总设计 / [R014](./docs/adr/R014-auth-verify-opt-in.md) verify 显式可选 / [R015](./docs/adr/R015-auth-credential-io.md) 非交互输入契约。统一 `everyday auth login|logout|verify|list --module <mod>`；删除 5 个模块 `login` 子命令 + 各 provider no-op `login`；模块内部凭据读取改走 `auth::get_credential`；keyring service 冻结 `everyday/<module>/<account>`（[F002](./docs/adr/F002-multi-account-keyring.md) 不动）。**v0.8.0 已发布**（破坏性：移除各模块 `login`）。

### Phase 13: 动作层 Backend 依赖倒置重构（note/todo/bookmark）[complete]
按 ADR [R016](./docs/adr/R016-action-backend-di.md) 总设计 / [R017](./docs/adr/R017-backend-layout-scope.md) 目录布局与范围 / [R018](./docs/adr/R018-backend-domain-mocks.md) domain 类型与 Mock。引入 `NoteBackend` / `TodoBackend` / `BookmarkBackend` trait（每动作一方法，返回 typed domain，绝不返回 `Output`）；`for_account` 工厂集中 provider 分支 + token 读取 + `NotionClient` 构造（仅工厂内一次）。三模块动作层：零 `NotionClient` 引用、零 provider 分支、零 keyring 读取。加 in-memory `Mock*Backend`（DI 回归护栏）；目录布局 `xxx/{mod.rs, backend.rs, notion.rs, local.rs}`，模块对外路径不变。**v0.8.1 已发布**（非破坏性内部重构）。

### Phase 14: 跨模块统一搜索 v1.1 收口 — Mail Searchable 走本地 envelope 缓存 [complete]
按 ADR [S007](./docs/adr/S007-mail-search-local-cache.md) 落地。`MailSearchProvider` 扫 `mail_cache.db`（非 live IMAP `SEARCH`）；复用 [S003](./docs/adr/S003-query-semantics.md) + [R008](./docs/adr/R008-sql-glob-not-like.md)：tokens 空白切，单 token OR 跨 `subject|from_addr|to_addr`，大小写不敏感 GLOB，metacharacter token 跳过。单全局 provider；`Hit::id = "{account}:{folder}:{uid}"` 供 agent 经 `mail read` 回写。**v0.9.0 已发布**（非破坏性）。

### Phase 15: Memory 模块（agent's own notebook）[complete]
按 ADR [K001](./docs/adr/K001-memory-module.md)–[K004](./docs/adr/K004-memory-single-instance.md) 设计 + 实现。`src/modules/memory/{mod,store,actions,search}.rs`；append-only `(subject, predicate, object)` 三元组 + confidence/source 元数据 + soft delete；独立 `~/.config/everyday/memory.db`；v1 命令集 `add / get / relation / list / delete / graph / history`（7 个）；参与 `everyday search`（当前态 GLOB 适配器，K003）；graph 前向 BFS 深度 1..=5（K002）；无 account 列、无 `auth` 模块触及（K004）。**v0.10.0 已发布**。

---

## 关键设计决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 包名 | `everyday` | 项目约定 |
| 异步运行时 | `tokio` | 生态成熟 |
| CLI 解析 | `clap` (derive) | 类型安全 |
| 错误处理 | `thiserror` + `Result<T, AgentError>` | 统一错误类型，易序列化 |
| 配置格式 | TOML | 人类可读 |
| 凭证存储 | `keyring`（service=`everyday/<module>/<account>`） | 安全红线：禁明文，Token 绝不落盘 |
| 输出抽象 | `Output` enum (Text/Json/Table) + `Renderer` | 一处切换，全局生效 |
| 模块抽象 | `Executor` trait + `Box<dyn Executor>` | 主程序与模块解耦 |
| 模块范围 | 仅外部集成类（mail/cal/rss/note/todo）+ config | fs/net/sys 封装通用能力，代理可用 shell/curl/fd/rg 直接完成，已移除 |
| 错误处理（Notion） | 复用现有 `AgentError`（`Auth`/`Network`/`Config`/`Other`） | 设计文档建议新增变体，但与 note 映射重复、会分裂错误分类 |
| 非测试代码 | 禁止 `unwrap()`/`expect()` | 安全红线；`NotionClient::new` 改为返回 `Result` |
| 配置回写 | `toml` crate 的 `toml::Value` 局部编辑 | 不引入 `toml_edit`，零新增依赖 |
| HTTP 栈 | reqwest（rustls-tls）+ 共享 `notion_client` | note/todo/rss 复用，未引入新 HTTP 依赖 |

---

## 多账户 config.toml 设计草案

```toml
[default_account]
mail = "work"
calendar = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 465
username = "me@example.com"
# password 不存这里，走 keyring service="everyday/mail/work"

[[mail.accounts]]
name = "personal"
imap_host = "imap.gmail.com"
...

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/user"
username = "me"
```

---

## Errors Encountered

| Error | Resolution |
|-------|------------|
| lettre `imap-pool` feature 不存在 | 改为 `pool` + `tokio1-rustls-tls` + `builder` |
| `format!("{s:<0$}", s, w)` 位置参数错位 | 改用 `pad()` 自由函数手动补空格 |
| `toml::Value::is_boolean` 不存在 | 改为 `is_bool()` |
| clippy `needless_range_loop` | 用 `cells.iter().zip(widths.iter()).enumerate()` 替换 range 索引 |
| `mailparse` Envelope 字段是 `Cow<[u8]>` 非 `Cow<str>` | 用 `String::from_utf8_lossy` 转字符串 |
| async-imap 基于 `futures` AsyncRead，tokio-rustls 是 tokio 的 | `tokio-util` compat 桥接：`tls_stream.compat()` |
| `async_imap::types::Address` 路径不存在 | `Fetch::envelope()` 是方法（非字段），Address 来自 `imap_proto`，用类型推断避免命名 |
| `uid_search` 返回 `HashSet<u32>` 非 Stream | 直接 collect，不 try_collect |
| `mailparse::MailHeaderMap` 是 trait 不能作参数类型 | 改 `&mailparse::ParsedMail`，访问 `.headers` |
| `lettre` `ContentType::TEXT_PLAIN_UTF_8` 不存在 | 改 `ContentType::TEXT_PLAIN` |
| `config get/set` 不支持数组索引 | get_dotted/set_dotted 增加 array 分支，数字 seg 访问数组元素 |
| `http::Uri` 方法是 `host()` 非 `host_str()`（与 url::Url 混淆） | 改用 `base.host()` |
| `base` 被 `host` 借用后 move 到 `WebDavClient::new` | `host` 转 owned `String`（`.to_string()`）解除借用 |
| QQ CalDAV 不支持 current-user-principal（PROPFIND 404） | `find_current_user_principal` 失败时降级用 `base_url` 作 calendar home set |
| libdav `bootstrap_via_service_discovery` fallback DNS SRV（QQ 无 SRV，os error 10054） | `CalDavClient::new(webdav)` 跳过 bootstrap，手动 `find_context_path` 只做 well-known 重定向 |
| Rust 1.97 clippy `doc_lazy_continuation` / `doc_overindented_list_items` deny-by-default 阻塞 CI | `///` 注释里以 `-`/`*`/`+` 开头的列表项续行必须 2 空格缩进；rustfmt 不重排 doc，含 doc 改动必须本地跑 clippy |
| `Drop` 中 `tokio::spawn` 后 runtime 已关闭 → panic + session 丢失 | 探测 `tokio::runtime::Handle::try_current()`，无则直接还 session |
| `Local.from_local_datetime(&ndt).unwrap()` DST 边界 panic | spring-forward gap 用 `.earliest()`，fall-back ambiguous 用 `.latest()` |
| `parse_simple_args` 把 `-1` / `-X` 误判为 flag | 单破折号 token 永远当值；双破折号 `--XXX` 才是 flag |

---

## Phase 状态汇总

- Phase 1–14 全部 complete；详见上文「阶段规划」。
- 当前最新发布：v0.9.0（Phase 14）。
- 历史发版一览见 [progress.md](./progress.md) §发版流水。