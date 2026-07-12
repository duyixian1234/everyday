# Everyday 开发计划

**项目：** Everyday — The Rust-powered hands for your AI Agent
**范围：** 以 `agents.md`「范围与定位」节为权威说明（原 PRD.md 已移除）
**启动时间：** 2026-07-08
**当前状态：** v0.7.0 已发布：Phase 11（跨模块统一搜索）落地，新增 `search` 模块（ADR S001–S006）。`everyday search query "<q>" [--module a,b,c] [--since 7d] [--limit N]`：searchable 适配器覆盖 note/todo/bookmark/rss（本地条目缓存表）+ cal（full-pull + in-memory GLOB），best-effort 并发扇出 + per-module cap 50 + global cap 20 + 空结果 exit 0，warning 走 stderr（`--json` 结构化）。mail 推迟 v1.1。241 tests / clippy 零警告 / fmt clean；v0.6.x 已发布：v0.6.2 修复 Rust 1.97 clippy 注释 lint 阻塞 CI（commit `dd2e786`）。

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
各模块实现 `Executor`；未实现/未知动作返回 `NotImplemented`/`UnknownAction`；注册到 `ModuleRegistry`。
> 注：初版曾含 `system`/`network`/`fs` 骨架，已于 2026-07-10 整体移除（见 `findings.md`「架构决策：移除 fs / net / sys 模块」）。

### Phase 6: 模块实现 [complete]
- `mail`（IMAP list/read/search + SMTP send + keyring login；文件夹递归 + IMAP UTF-7 中文解码）[2026-07-08]
- `calendar`（CalDAV：login/calendars/list/add/delete，libdav+icalendar）[2026-07-09]
- `rss`（feed-rs：follow/list/unfollow/digest/fetch）[2026-07-09]
- `note`（Notion 笔记/知识库：login/search/create/read/append/update/list）[2026-07-10]
- `notion-client` 共享 SDK + `todo`（Notion 待办：login/init-db/list/add/start/complete）[2026-07-10]

### Phase 7: 构建、测试、文档、发布 [complete]
全模块 `cargo build` / `clippy` / 单测 + 集成测试全绿；README + skills 文档与代码一致；CI（三平台 + aarch64 macOS）+ release workflow；**v0.4.0 已打 tag 发布**（相对 v0.3.0 增量：bookmark 模块 + 模块分层 shared/util + Justfile + README 国际化 + cargo fmt 门槛）。

### Phase 8: 中英文 README [complete]
根 `README.md` 与 `skills/README.md` 已改写为英文；完整中文文档保留为 `README_ZH.md`；`README.md` 与 `README_ZH.md` 顶部均加语言切换链接（English ↔ 简体中文）。

### Phase 9: Timeline 统一事件层 [complete]
按 `CONTEXT.md` + 9 个 ADR 实现：
- `src/modules/timeline.rs` TimelineModule + Executor + 5 actions（today/yesterday/week/month/sync）。
- `src/modules/timeline/{store,providers,orchestrator}.rs`：timeline.db + 6 source adapter + 按 source 分组并行 sync 编排。
- `src/ops_log.rs`：AOP dispatch hook 记录 notion 账户写操作。
- 6 模块（mail/cal/rss/note_local/todo_local/bookmark_local）暴露 `fetch_for_timeline(window)`。
- 顺手修：`gen_id` 同纳秒撞 ID（加 atomic counter）；`query_events` LIMIT 占位符缺 `?`（改字面整数）。
- 质量门禁全绿：173 tests / clippy `-D warnings` 零警告 / fmt clean。commit `2ce5055`。
- 4 处修补 + 发版 v0.5.0（commit `218f70b`，tag `v0.5.0` 已推 origin）。

### Phase 10: Mail Cache（envelope 缓存 + 并发 sync）[complete]
按 `CONTEXT.md` §Mail Cache + ADR [M002](docs/adr/M002-imap-connection-pool.md)–[M005](docs/adr/M005-staleness-auto-sync.md) 实现：
- `src/modules/email_cache.rs`：mail_cache.db 双表（envelopes 主键 `(account, folder, uid)`，folder_state 主键 `(account, folder)` 存 `uid_validity/max_uid/last_sync_at`）；`upsert_envelopes` 事务原子写 envelope + 前进水位；`clear_folder` 处理 UIDVALIDITY 失效；`is_stale` 阈值 15 分钟。
- `src/modules/email_pool.rs`：M=4 IMAP session 池 + `Arc<Semaphore>`；`PoolGuard` 借用归还，`invalidate` 标 dirty。
- `src/modules/email.rs::mail_list` 改造：开 cache → staleness 检查 → 必要时并发 sync（`sync_folders_concurrent` 跨 folder `join_all`）→ 查本地 envelope → 渲染表格。`search` / `read` / `send` 保持直连 IMAP 不变。
- K1 清理：只追加不删除（接受数据库膨胀）；F1 flags：sync 时刻快照（最坏 15 分钟滞后可接受）。
- 8 个 SQL 集成单测覆盖：upsert 写 + 水位前进、空 batch 仅前进 last_sync、upsert on conflict、clear_folder、UIDVALIDITY 失效模拟重置、unread 过滤、K1 ghost envelope 留存、date desc + limit。
- 质量门禁全绿：build ✅ / clippy `-D warnings` 零警告 ✅ / 196 tests passed (+15) ✅ / fmt clean ✅。
- 已发版：v0.6.0（Mail Cache）+ v0.6.1（timeline `--from` 静默回退修复，commit `52f6377`）+ v0.6.2（Rust 1.97 clippy 注释 lint 修复，commit `dd2e786`）均已推 origin（GitHub，非 cnb 镜像）。

### Phase 11: 跨模块统一搜索（Search）[complete]
按 ADR [S001](docs/adr/S001-search-architecture.md)–[S006](docs/adr/S006-search-module-cli.md) 落地：
- `src/search.rs`：`Searchable` trait（`#[async_trait]` 包装以 dyn-compat）+ `SearchQuery` / `Hit` / `SearchOutcome` + `SearchRegistry`（并发扇出 `join_all`，best-effort，per-module cap 50，global cap 20，empty exit 0）。
- `src/modules/search.rs`：`SearchModule` 实现 Executor，单 action `query`；`everyday search query "<q>" [--module a,b,c] [--since 7d] [--limit N] [--json]`。
- v1 searchable 适配器：note/todo/bookmark（local SQLite GLOB，[R008](docs/adr/R008-sql-glob-not-like.md)）；rss（新增 `~/.config/everyday/rss-items.db` 本地条目缓存表，由 `digest`/`fetch` 同步写入，[S005](docs/adr/S005-time-semantics-scope.md)）；cal（`fetch_for_timeline` 全量拉取 + in-memory GLOB，[C002](docs/adr/C002-full-pull-local-filter.md)）；mail 推迟 v1.1。
- 查询语义：空白切 token，多词 **OR**，大小写不敏感 GLOB 子串 `lower(col) GLOB lower('*token*')`（[S003](docs/adr/S003-query-semantics.md)）。
- 执行模型：best-effort（失败模块进 `SearchWarning` 仅 stderr 输出；仅全失败才报 AgentError，[L009](docs/adr/L009-best-effort-sync.md)/[R001](docs/adr/R001-thread-local-json-mode.md)）；warning 文案：`--json` → stderr 结构化 `{"_warning": ...}`，text → `eprintln!`。
- ts：每模块自定义主时间，全局 `ts desc`；note=updated_at, todo=updated_at（fallback created_at），bookmark=created_at，rss=published，cal=event_start（[S005](docs/adr/S005-time-semantics-scope.md)）。
- 复用：[S006](docs/adr/S006-search-module-cli.md) 共享 `util::datetime::parse_since`（来自 [L012](docs/adr/L012-since-query-flag.md)）和 `timeline::parse_source_list`（验证 `--module`）。
- 质量门禁：build / clippy `-D warnings` / 241 tests / fmt clean；已发布 v0.7.0。
- 提交：5 个原子 commit — search core、note、todo、bookmark、rss cache + searchable、cal、search module + registry 接入。

### Phase 12: 凭据 / login 逻辑收拢到顶层 auth 模块 [pending]
按 Grill 设计（ADR [R013](docs/adr/R013-auth-module-consolidation.md) 收拢总设计 / [R014](docs/adr/R014-auth-verify-opt-in.md) verify 显式可选 / [R015](docs/adr/R015-auth-credential-io.md) 非交互输入契约）。目标：删除 5 个模块的 `login` 子命令与各自 keyring 读写，统一由顶层 `auth` 命令 + `src/modules/auth.rs` 接管凭据全生命周期；`--verify` 显式触发真实认证，默认只存凭据。

设计要点（Grill 已拍板，Q1–Q7）：
- 接口：`everyday auth <login|logout|verify|list> --module <mod> [--account <name>] [--password <pwd> | --token <tok>] [--verify]`；复用全局 `--account`，缺省回退 module 默认账户。
- 策略解析：`resolve_strategy(module, account) -> AuthStrategy {Password, Token, None}`，纯从 Config 派生；keyring user 由策略决定（Password→`account.username`，Token→`"token"`），keyring **service 格式冻结** `everyday/<module>/<account>`（F002 不动）。
- verify 复用现有连接原语（`email::imap_connect` / cal 连接 / `notion_client`），local / rss 短路返回 `not_required`。
- 非交互输入走 `--password`/`--token`（argv，绝不读 env）；flag 缺省回退 `rpassword` 交互；JSON 模式静默不回显。
- 破坏性变更：移除各模块 `login`；CHANGELOG / ADR 标注 breaking。

子任务（每项为独立可编译 commit，遵循 one-commit-one-task；每 commit 必过 `cargo build` + `cargo test` + `clippy --all-targets -- -D warnings` + `cargo fmt --check`）：

| ID | 子任务 | 关键改动（文件:行引用当前位置） | 建议 commit |
|----|--------|----------------------------------|-------------|
| T12.1 | 脚手架：新建 `auth` 模块 + 注册 | 新增 `src/modules/auth.rs`：`AuthStrategy` + `resolve_strategy` + `store/get/delete_credential` + `AuthModule`(Executor，解析 4 actions 与 `--module/--account/--password/--token/--verify`)；`main.rs`/`cli.rs` 的 `ModuleRegistry` 注册 `auth` | `feat(auth): add auth module skeleton` |
| T12.2 | `auth login` / `logout`（凭据存储与删除） | 实现 `store_credential`/`delete_credential` 落地 + 非交互 flag 与交互回退；keyring user 按策略派生（Password→username，Token→`"token"`） | `feat(auth): implement login/logout credential store` |
| T12.3 | `auth verify` + `login --verify` | 按策略分发：mail/cal 复用 `imap_connect`/cal 连接真连；notion 复用 `notion_client`；local/rss 短路 `not_required` | `feat(auth): implement verify (opt-in)` |
| T12.4 | `auth list` | 以 config 为主枚举账户，探测 keyring 状态，输出每行 `{"module","account","status"}`，`status ∈ {stored, missing, not_required}` | `feat(auth): implement list` |
| T12.5 | 迁移 mail → `auth::get_credential` | `email.rs`：`get_password` 调用点(189/278/1307) 改 `auth::get_credential`；删除 `get_password`(206)/`mail_login`(220)/`login` action 臂(165/187) 及 `name:"login"` 注册(165) | `refactor(mail): use auth::get_credential` |
| T12.6 | 迁移 calendar → `auth::get_credential` | `calendar.rs`：调用点(176/761) 改 `auth::get_credential`；删除 `get_password`(193)/`cal_login`(207)/`login` 臂(174) 及 `name:"login"` 注册(64) | `refactor(cal): use auth::get_credential` |
| T12.7 | 迁移 note/todo/bookmark(notion) → `auth::get_credential` | 删除 `local.rs::login_notion`(110) 及 note/todo/bookmark 的 `X_login` + `login` 臂(63/187, 176/294, 137/227) 与 `name:"login"` 注册(63/176/137)；notion token 取用改 `auth::get_credential(..., keyring_user="token")` | `refactor(note,todo,bookmark): use auth for notion token` |
| T12.8 | 移除 local provider `login` no-op | 删 `note_local.rs:85`/`todo_local.rs:68`/`bookmark_local.rs:56` 的 `login` 及对应 `login` 臂(175/278/218)；auth verify 对 local 返回 `not_required` | `refactor(local): drop login no-op` |
| T12.9 | auth 模块单测 | `resolve_strategy` 全组合（mail/cal=Password, note/todo/bookmark notion=Token, 三者 local=None, rss=None）；`store/get/delete` 往返（临时 keyring entry 并清理）；verify 短路；`list` 状态映射 | `test(auth): add unit tests` |
| T12.10 | 清理过时注释与死引用 | 移除指向旧 login/keyring 流程的注释（note 257 / todo 323 / bookmark 257 的 `See login_notion`；`get_password` 周边）；确认 `ops_log.rs:458` 断言 `!LOGGED_ACTIONS.contains("login")` 仍有效（login 不再被 AOP 记录） | `refactor: remove stale login/keyring comments` |
| T12.11 | 用户文档同步 | `skills/README.md` + `skills/everyday-cli/references/COMMANDS.md` 增 `auth` 用法、标注 module `login` 已移除(breaking)；`README.md`/`README_ZH.md` 命令参考补 auth 节；`just check-links` 通过 | `docs(auth): user-facing command reference` |

验收（Phase 完成条件）：
- `everyday auth login --module mail --account work --password <p> --verify` 存+验一体成功；`everyday auth verify --module note` 读已存 token 重验成功；`everyday auth list` 输出三态。
- 全模块不再有 `<module> login` 子命令；`cargo build` / `clippy -D warnings` / `test` / `fmt --check` 全绿。
- 遗留 7 篇 ADR（M001/C001/N001/T001/B001/R009/F002）已在 Grill 阶段标注「已收拢至 auth」，本阶段无需再改。

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

## Errors Encountered（仍相关项）

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

---

## Phase 状态汇总
- Phase 1: complete
- Phase 2: complete
- Phase 3: complete
- Phase 4: complete
- Phase 5: complete
- Phase 6: complete
- Phase 7: complete
- Phase 8: complete
- Phase 9: complete
- Phase 10: complete
- Phase 11: complete (search module landed; released as v0.7.0)
- Phase 12: pending (auth module consolidation; design ADRs R013/R014/R015 done, implementation T12.1–T12.11 pending)
