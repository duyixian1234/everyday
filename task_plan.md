# Everyday 开发计划

**项目：** Everyday — The Rust-powered hands for your AI Agent
**范围：** 以 `agents.md`「范围与定位」节为权威说明（原 PRD.md 已移除）
**启动时间：** 2026-07-08
**当前状态：** v0.17.2 已发布 — Task 用户自定义命令与 cron 调度（[F017](./docs/adr/F017-task-module.md)，Phase 23 完成：`[tasks.<name>]` 配置 + `task add/run/list/remove/history` + daemon 独立 cron 调度循环，非破坏性 patch；集成测试收尾修复 CLI 解析冲突与调度加固）；v0.17.1 已发布 — 日期序号 ID（[R021](./docs/adr/R021-date-sequence-id.md)：`{前缀}{YYYYMMDD}-{PID}-{当日序号}` 取代纳秒 hex，全前缀 n/t/b/m/ev/mc/ri 统一，CLI 用户可读可重输；质量门禁中发现「纯内存计数器」跨进程撞号缺陷（CLI 每次命令新进程、同日同前缀必撞 SQLite 主键），修订为带 PID 段）；v0.17.0 已发布 — Daemon 常驻自动同步（[F016](./docs/adr/F016-daemon-sync-scheduler.md)，Phase 22 完成，GOAI 破例升 minor）；v0.16.2 已发布 — R020 修订（[R020](./docs/adr/R020-env-credential-fallback.md) amendment：`[auth] env_credentials` 经进程级镜像对 `mail list`/`cal`/`sync` 等 no-Config hot path 生效，双通道全模块一致）；v0.16.1 已发布 — 日志迁移收尾（[F015](./docs/adr/F015-leveled-logging-tracing.md)：warning 站点全迁 tracing、mcp serve `_error` 系、README 契约段）；v0.16.0 已发布 — 默认日志静音 + `-v`/`-vv` 显式开启（[F015](./docs/adr/F015-leveled-logging-tracing.md)，tracing 分级日志，R001 形状不变）；v0.15.0 已发布 — MCP server 模块落地（[F014](./docs/adr/F014-mcp-module.md)，`everyday mcp serve` / `mcp tools`，stdio + rmcp 3.x）；v0.14.0 已发布（[R020](./docs/adr/R020-env-credential-fallback.md)）；Notion provider 已移除（[R019](./docs/adr/R019-remove-notion-provider.md)，note/todo/bookmark 仅本地 SQLite）。本文档历史阶段的 Notion 描述仅作决策记录，不代表当前能力。
**文件维护规则：** 阶段计划 + 错误表 + 设计决策摘要；禁止保留任务执行细节
（子任务清单、完成小结、中途修复明细）。
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

### Phase 16: 架构深化 Phase 1 — Quick Wins（P2c / P2a / P6）[complete]
按 ADR [F012](./docs/adr/F012-architecture-deepening-phase.md) 落地第一阶段三项低风险改进（已并入 v0.11.0-rc）：P2c `Config::validate()` 加载时语义校验；P2a `AccountProvider` trait 统一账户解析（替代 R007 宏，旧 `X_account()` 委托保留）；P6 `TypedValue` + `Output::TypedRecords` 保类型输出（mail list uid/unread、memory list/history）。Phase 3（P3–P5）按 F012 时间线后续启动。

### Phase 17: 架构深化 Phase 2 — P1 CLI/business 分离 + P2b config 子集 [complete]
按 ADR [F012](./docs/adr/F012-architecture-deepening-phase.md) 落地第二阶段（已并入 v0.11.0-rc）：
- **P1**：`cli_action!`/`flag!` 宏压缩全 11 模块 ArgSpec（净 -321 行，mail `module_arg_spec` 129→61）；mail/cal/rss 建 service-layer trait（`MailBackend`/`CalBackend`/`RssBackend` + domain 类型 + `for_account` DI 工厂 + `dispatch()` 唯一 Output 触点 + Mock backend 直测）；note/todo/bookmark 沿用 R016 `*Backend`；timeline/search/auth/config/memory 仅宏化 ArgSpec（已渲染分离 + 直测，未建独立 ModuleService trait，留待 Phase 3 接线）。**Phase 3 接线完成（commit `fe1ab51`）**：memory/search/timeline/auth 补 `MemoryBackend`/`SearchBackend`/`TimelineBackend`/`AuthBackend` trait + `for_config` 工厂 + Mock 直测，execute 一律收敛为 `dispatch()` 唯一 Output 触点；config 保留 Executor 直实现（纯文件 IO、无 domain 层，service trait 属 Speculative Generality，理由记录于 F012）。
- **P2b**：业务模块注入 config 子集（`MailModuleConfig` 等，`impl_module_config!` 宏），`ModuleRegistry::build` 经 `Config::X_module_config()` 切片；`for_account()` 弃 Config 参数，凭据走 `auth::get_credential_with_user()`；timeline/search/auth 保留 `Arc<Config>`（跨模块编排器）。
- Phase 3（P3 lifecycle / P4 request context / P5 middleware）按 F012 时间线后续启动。

### Phase 18: 架构深化 Phase 3 — lifecycle / request context / middleware [complete]
按 ADR [F012](./docs/adr/F012-architecture-deepening-phase.md) 落地第三阶段（已并入 v0.11.0-rc）：
- **P3**：`Executor` 加 `initialize()`/`health_check()`/`shutdown()` 默认实现 + `HealthStatus`；`ModuleRegistry::{initialize_all, health_check_all, shutdown_all}`；根级 `everyday health` 命令（text/JSON 双输出，exit 0/1）；mail/memory/timeline override health_check（仅本地 DB 探测，无网络）；main.rs 在 dispatch 前后接线 initialize/shutdown。
- **P4**：`shared::request_context` — `RequestContext { request_id, deadline, caller }` 非破坏性 thread-local 传播（v0.12 才改显式参数 breaking）；`generate_request_id()` = `cli-<nanos>-<pid>`；main.rs 每命令设置 + 完成后清除。
- **P5**：`shared::middleware` — `Middleware` trait（before/after/on_error）+ `run_with_middleware`；默认 `LoggingMiddleware`（stderr 输出 request_id/module/action/elapsed，JSON 模式输出结构化 `_log` 行）；main.rs dispatch 走 middleware 链，模块零侵入。

### Phase 19: v0.12 — P4 显式参数化 RequestContext（breaking）[complete]
按 ADR [F013](./docs/adr/F013-request-context-explicit-parameter.md) 落地 F012 P4 的 v0.12 breaking 半程：`RequestContext` 改为显式参数贯穿 `Executor::execute` 与 middleware 栈，thread-local 传播整体移除。**破坏性**：自定义 `Executor` 实现者需加 `ctx: &RequestContext` 参数；迁移指南见 F013 §Migration guide。

### Phase 20: env 凭据回退（R020，opt-in）[complete]
按 ADR [R020](./docs/adr/R020-env-credential-fallback.md) 为 R015 开受控例外：系统无 keyring 后端时允许 opt-in 从环境变量读凭据（`[auth] env_credentials = true` 或 `EVERYDAY_ENV_CREDENTIALS=1` 双通道开关；`EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` 命名；读取链 keyring→env→报错；`auth list` 第四态 `env`；login 仍写 keyring、logout 对 env 凭据提示 unset）。默认行为不变。

### Phase 21: MCP server 模块（F014）[complete]
按 ADR [F014](./docs/adr/F014-mcp-module.md) 落地：`everyday mcp serve` 经 stdio 把每个 `(module, action)` 协议投影为 MCP tool `<module>_<action>`（rmcp 3.x）；`mcp tools` 打印 tool 清单 + JSON Schema；schema 复用 `module_arg_spec()` 单一事实来源；`mcp` 模块注入 `Arc<ModuleRegistry>`（构建期后置注入）；Mutex 串行化 tool 调用；会话内复用 registry + initialize/shutdown 钩子；JSON 文本输出 + `isError`；无 `[mcp]` 配置面。质量：stdio 端到端测试（tests/mcp_stdio.rs）锁定 stdout 仅 JSON-RPC + EOF 退出 0；CLI contract 覆盖 `mcp serve`/`tools`；README/README_ZH/skill 文档同步。

### Phase 22: Daemon 自动同步（F016）[done]
按 ADR [F016](./docs/adr/F016-daemon-sync-scheduler.md) 落地常驻自动同步（v0.17.0）：`everyday daemon run [--once] [--sources ...]` / `daemon status`；`[daemon]` 配置节（enabled / interval_seconds / sources，全字段 `serde(default)` 向后兼容，`enabled=false` 时 run 报错退出）；同步周期 = timeline `run_sync`（复用 orchestrator）+ mail 全文件夹增量缓存（每周期 IMAP LIST 发现，含 Sent/Trash/Junk/Drafts）+ rss 缓存拉取，顺序执行、完成后 sleep 防重叠；状态文件 `daemon-state.json`（pid / running / 周期时间 / sources 结果，退出保留 final 状态）；pid 存活判定 + 防重入；优雅退出统一 `graceful_shutdown` 路径（`--once` 完成 / SIGINT / SIGTERM / Ctrl+C 汇合）；日志：常驻 stdout 静默、stderr 随 `-v`、文件日志 `daemon.log` 固定 INFO 不轮转；`cli_contract` / `config.example.toml` / README / skill reference 同步；`docs/daemon.md` 三平台服务注册示例（nssm / launchd / systemd，Q12 不写 SCM 集成代码）。质量：`--once` 集成测试验证完整退出路径 + 状态落盘；单元测试注入 signal/interval 覆盖调度循环与优雅退出；R001 回归（verbose_logging / mcp_stdio）全绿。

### Phase 23: Task 用户自定义命令与 cron 调度（F017）[done]
按 ADR [F017](./docs/adr/F017-task-module.md) 落地 `[tasks.<name>]` 配置、`task add/run/list/remove/history`、无 shell 进程树超时终止与 64 KiB 分流捕获、`task.db` 审计历史，以及与同步周期独立的 daemon cron 循环（含 `--once`）。

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
| Windows 上 Linux target typecheck 缺 `x86_64-linux-gnu-gcc`，`ring` build script 无法运行 | 保留 Unix 专属代码的 cfg 单测/CI 覆盖；本机完成 Windows 全测试与 all-target clippy |
| QQ CalDAV 不支持 current-user-principal（PROPFIND 404） | `find_current_user_principal` 失败时降级用 `base_url` 作 calendar home set |
| libdav `bootstrap_via_service_discovery` fallback DNS SRV（QQ 无 SRV，os error 10054） | `CalDavClient::new(webdav)` 跳过 bootstrap，手动 `find_context_path` 只做 well-known 重定向 |
| Rust 1.97 clippy `doc_lazy_continuation` / `doc_overindented_list_items` deny-by-default 阻塞 CI | `///` 注释里以 `-`/`*`/`+` 开头的列表项续行必须 2 空格缩进；rustfmt 不重排 doc，含 doc 改动必须本地跑 clippy |
| `Drop` 中 `tokio::spawn` 后 runtime 已关闭 → panic + session 丢失 | 探测 `tokio::runtime::Handle::try_current()`，无则直接还 session |
| `Local.from_local_datetime(&ndt).unwrap()` DST 边界 panic | spring-forward gap 用 `.earliest()`，fall-back ambiguous 用 `.latest()` |
| `parse_simple_args` 把 `-1` / `-X` 误判为 flag | 单破折号 token 永远当值；双破折号 `--XXX` 才是 flag |

---

## Phase 状态汇总

- Phase 1–18 全部 complete；Phase 19–20 已并入 v0.13/v0.14 系列（Phase 20 = R020 env 凭据回退，随 **v0.14.0** 发布）。详见上文「阶段规划」。
- 当前最新发布：v0.14.0。
- 历史发版一览见 [progress.md](./progress.md) §发版流水。