# Progress Log — Everyday

> 当前状态 + **ADR 时间序索引** + 发版流水。决策性叙述见
> [docs/adr/](./docs/adr/README.md)；按 ADR 编号前缀查找见
> `docs/adr/README.md` 的索引节。
> 本文件的维护规则见 [`.rules/01-workflow.md`](./.rules/01-workflow.md)
> §"Finishing a task"。

## 当前状态

每行 ≤ 1 句话；详细任务执行细节、子任务清单、完成小结一律不进本文件。

- **v0.17.1 已发布** — **日期序号 ID**（[R021](./docs/adr/R021-date-sequence-id.md)）：`{前缀}{YYYYMMDD}-{PID}-{当日序号}`（如 `n20260814-1a2b-001`）取代纳秒 hex，全前缀 n/t/b/m/ev/mc/ri 统一、按天重置、按前缀独立计数；质量门禁暴露「纯内存计数器」跨进程撞号缺陷（CLI 每次命令新进程，同日第二次写同前缀撞 SQLite 主键；memory end-to-end 测试对真实全局 DB 复现）→ 修订为带 PID 段恢复跨进程唯一；旧格式 id 共存不迁移，引用精确字符串匹配；CLI 帮助/错误文本 `<page_id>` → `<id>`（`default_page_id` 配置字段保留）。非破坏性。
- **v0.17.0 已发布** — **Daemon 常驻自动同步**（[F016](./docs/adr/F016-daemon-sync-scheduler.md)，GOAI 破例升 minor）：`everyday daemon run` 前台常驻为唯一允许周期性拉取的角色（timeline run_sync + mail 服务器全部文件夹缓存 + rss 拉取，顺序执行、完成后 sleep、失败照跑）；`[daemon]` 节（enabled/interval_seconds/sources，serde(default) 向后兼容）；状态文件 `daemon-state.json`（pid/running/周期时间/各源结果，原子写三时机）+ pid 存活防重入（`tasklist` CSV 引号 PID 匹配，语言无关）；`daemon status`（文本+JSON）；`--once` 输出对齐 timeline sync（顶层 ok + 每源对象）；`daemon.log` 固定 INFO 文件日志（不随 -v）；优雅退出统一 `graceful_shutdown`（--once/SIGINT/SIGTERM/Ctrl+C 汇合）；常驻 stdout 静默。L005/D003 铁律不变。
- **v0.16.2 已发布** — **R020 修订：`[auth] env_credentials` 对 no-Config hot path 生效**：`mail list` / `cal` / `sync` 等调用点（`get_credential_with_user`，P2b 无完整 Config）此前只认 `EVERYDAY_ENV_CREDENTIALS` 环境开关，config 字段无效；现 main 启动时把 config 字段镜像到进程级开关（`sync_env_credentials_from_config`），双通道对全部模块生效（[R020](./docs/adr/R020-env-credential-fallback.md) amendment）。非破坏性。
- **v0.16.1 已发布** — **日志迁移收尾**（[F015](./docs/adr/F015-leveled-logging-tracing.md)）：T2-T4 完成——warning 站点全迁 tracing（`_warning`+`warning_text` 机制，text 逐字节 / JSON 形状不变；auto_sync 成功降 info 随 `-v`；timeline JSON 模式结构化）；mcp serve 迁移（`_error` 第三系）；README 双语契约段更新。非破坏性。
- **v0.16.0 已发布** — **默认日志静音 + `-v`/`-vv` 显式开启**（[F015](./docs/adr/F015-leveled-logging-tracing.md)）：引入 tracing + tracing-subscriber，默认 WARN（warning/error 可见、中间件进度静音），`-v`=INFO 恢复 `[req] module action ok in Nms` / `{"_log"}` 行（R001 形状不变），`-vv`=DEBUG 预留；`LoggingMiddleware` 无条件留栈靠 LevelFilter 静音；仅渲染 everyday target；14 处 eprintln 全量迁移；Layer 单测 + 二进制级契约测试锁定形状。非破坏性。
- **v0.15.0 已发布** — **MCP server 上线**（[F014](./docs/adr/F014-mcp-module.md)）：`everyday mcp serve` 经 stdio 把每个 `(module, action)` 投影为 MCP tool `<module>_<action>`（schema 复用 `module_arg_spec`，单一事实来源）；`mcp tools` 打印 tool 清单调试；rmcp 3.x + Mutex 串行 + 会话复用 registry + stdout 仅 JSON-RPC；写操作 tool 带 `[WRITE]` 标记；stdio 端到端测试锁定协议卫生；README/README_ZH/skill 文档已同步。非破坏性。
- **v0.13.1 已发布** — 工程质量工具栈批 1 + 批 2 落地（[G001](./docs/adr/G001-quality-tools-suite.md)）：CI 测试换 nextest（junit 报告）、typos/git-cliff/cargo-deny 门禁、CLI contract 测试层；sync 并行 tmp 目录竞争修复（Unix 可见、Windows 掩盖）；release 流水线换 cargo-dist（installer 脚本 + Sigstore attestation）。无新功能、无破坏。
- **v0.14.0 已发布** — env 凭据回退（[R020](./docs/adr/R020-env-credential-fallback.md)）：为 R015 开受控例外——keyring 后端不可用（headless/CI/沙箱）时，opt-in 后可从环境变量读凭据。`[auth] env_credentials = true` 或 `EVERYDAY_ENV_CREDENTIALS=1` 双通道开关；变量 `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD`；读取链 keyring → env → 报错；`auth list` 新增第四态 `env`；`logout` 对 env 来源凭据提示 `unset`；login 始终写 keyring。默认行为不变（R015 仍生效）。非破坏性。
- **v0.13.0 已发布** — Notion provider 移除（[R019](./docs/adr/R019-remove-notion-provider.md)，note/todo/bookmark 仅本地 SQLite）；**WebDAV 设备同步上线**（[D001–D003](./docs/adr/D001-webdav-file-sync.md)）：`everyday sync` 双向文件级同步（4 个用户 DB + config.toml，LWW 冲突副本），`auth` 支持 `--module webdav`，写命令后可选 auto_sync（D003，默认关）。
- **v0.12.0 已发布** — F012 P4 显式参数化 RequestContext（[F013](./docs/adr/F013-request-context-explicit-parameter.md)）：`Executor::execute` + middleware 钩子加 `&RequestContext`，thread-local 移除；破坏性（自定义 Executor 迁移指南见 F013）。
- **v0.11.0-rc 已发布** — 架构深化三阶段落地（[F012](./docs/adr/F012-architecture-deepening-phase.md)）：Phase 1（P6 TypedValue / P2c Config 校验 / P2a AccountProvider）+ Phase 2（P1 CLI/business 分离 + P2b config 子集）+ Phase 3（P3 lifecycle `everyday health` / P4 RequestContext / P5 Middleware）。
- **v0.9.0 已发布** — 跨模块统一搜索 v1.1 收口：`mail` Searchable 走本地 envelope 缓存（[S007](./docs/adr/S007-mail-search-local-cache.md)）。
- **v0.8.1 已发布** — 动作层 Backend DI 重构（[R016–R018](./docs/adr/R016-action-backend-di.md)）。
- **v0.8.0 已发布** — 凭据 / `login` 收拢到顶层 `auth` 模块（[R013–R015](./docs/adr/R013-auth-module-consolidation.md)；破坏性）。
- **v0.7.0 已发布** — 跨模块统一搜索 `everyday search`（[S001–S006](./docs/adr/S001-search-architecture.md)）。
- **v0.6.x 已发布** — Mail Cache 落地（[M002–M005](./docs/adr/M002-imap-connection-pool.md)）+ timeline `--from` 显式报错（[L013](./docs/adr/L013-from-explicit-error.md)）+ Rust 1.97 clippy 注释 lint 修复。
- **模块**：`mail` / `cal` / `rss` / `note` / `todo` / `bookmark` / `timeline` / `memory` / `config` / `search`（10 个，走 Executor trait）。
- **本地 provider 唯一**：`note` / `todo` / `bookmark` 三模块仅本地 SQLite（Notion provider 已移除，[R019](./docs/adr/R019-remove-notion-provider.md)，v0.13.0）。
- **Timeline**：append-only event log 统一 6 source 事件捕获（[L001–L013](./docs/adr/L001-append-only-event-log.md)）。
- **质量门禁**：`cargo build` / `cargo clippy --all-targets -- -D warnings` 零警告 / `cargo test` / `cargo fmt --check` 全绿；CI 三平台 + aarch64 mac 全绿（[F006](./docs/adr/F006-ci-release-github-only.md)）。
- **工程质量工具栈（[G001](./docs/adr/G001-quality-tools-suite.md)）**：CI 测试换 nextest（junit 报告，本地仍 `cargo test` ~4s）；typos 拼写门禁；git-cliff 发版时生成 CHANGELOG.md；cargo-deny 合规审计（389 crates 零 copyleft，allow 加 Unicode-3.0/0BSD/CDLA-Permissive-2.0；4 个 unmaintained 显式接受并记录）；**CLI contract 测试层**（tests/cli_contract.rs 锁顶层命令集/模块 action 集/config 结构——防 v0.8/v0.12/v0.13 类破坏）；semver-checks 因纯 bin 无公共 API 被否。
- **批 2：release 流水线换 cargo-dist**（[G001](./docs/adr/G001-quality-tools-suite.md) 批 2 落地）：release.yml 由 `dist generate` 机器生成（勿手改，源在 dist-workspace.toml）；4 平台归档 tar.xz/zip + shell/powershell 安装脚本 + sha256.sum + **Sigstore attestation**（`gh attestation verify` 可验）；README 安装节更新（installer 为推荐通道）。sccache/cargo-cache 评估结论=不引入（Swatinem/rust-cache 已覆盖、cargo-cache 是磁盘卫生工具）；tabled 已是最新（proc-macro-error 无升级路径，保留在 G001 接受清单）。

## ADR 时间序索引

按 ADR 时间倒序排列。完整列表见 [docs/adr/README.md](./docs/adr/README.md)。

| 日期 | 系列 | ADR | 摘要 |
| --- | --- | --- | --- |
| 2026-08-13 | F | [F016](./docs/adr/F016-daemon-sync-scheduler.md) | Daemon 自动同步：`everyday daemon run` 常驻进程为唯一允许周期性自动拉取的角色（timeline run_sync + mail 全文件夹缓存 + rss 拉取，顺序执行、完成后 sleep）；`[daemon]` 节（enabled/interval_seconds/sources 向后兼容）；状态文件 daemon-state.json + pid 防重入；`--once`/`status`；优雅退出统一 graceful_shutdown 路径；常驻 stdout 静默、文件日志固定 INFO；不写 SCM 集成代码（docs/daemon.md 三平台示例） |
| 2026-08-10 | F | [F015](./docs/adr/F015-leveled-logging-tracing.md) | 分级日志（tracing）：默认 WARN 静音，`-v`=INFO（中间件进度），`-vv`=DEBUG 预留；全局 `-v/--verbose`（Count，仿 --json）；自定义 Layer 写 stderr，text 紧凑格式 + JSON `{"_log"}` 形状不变（R001）；middleware 无条件留栈靠 LevelFilter 静音；仅渲染 everyday target；14 处 eprintln 全量迁移 |
| 2026-08-10 | F | [F014](./docs/adr/F014-mcp-module.md) | MCP 模块：everyday 作 MCP server（stdio，rmcp 3.x）；每个 (module, action) 协议投影为 MCP tool `<module>_<action>`（schema 复用 module_arg_spec）；JSON 文本输出 + isError；mcp 模块注入 Arc<ModuleRegistry>（后置注入）；Mutex 串行；serve/tools 动作；无配置面 |
| 2026-08-09 | R | [R020](./docs/adr/R020-env-credential-fallback.md) | env 凭据回退（opt-in）：keyring 不可用时从 `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` 读凭据；双通道开关（config `[auth] env_credentials` / `EVERYDAY_ENV_CREDENTIALS=1`）；读取链 keyring→env→报错；`auth list` 第四态 `env`；logout 对 env 凭据提示 unset；login 仍写 keyring；R015 默认行为不变 |
| 2026-08-09 | G | [G001](./docs/adr/G001-quality-tools-suite.md) | 工程质量工具栈：nextest(CI)/typos/git-cliff/cargo-deny + CLI contract 测试；semver-checks 因纯 bin 被否；4 unmaintained 依赖显式接受（G001 记录理由）；cargo-dist/sccache 留批 2 |
| 2026-08-09 | D | [D001–D003](./docs/adr/D001-webdav-file-sync.md) | WebDAV 设备同步：D001 同步模型/范围/冲突（文件级 LWW + 冲突副本）/ D002 快照 + hash 状态（sync-state.json）/ D003 auto_sync CLI 边界（写命令 best-effort push，查询永不触发） |
| 2026-08-09 | R | [R019](./docs/adr/R019-remove-notion-provider.md) | 移除 Notion provider：note/todo/bookmark 仅本地 SQLite；`provider="notion"` 加载即报错；ops-log/AOP 钩子/OpsLogProvider/共享客户端/令牌流全删；JSON 核心键保持稳定（v0.13.0 破坏性） |
| 2026-08-05 | F | [F013](./docs/adr/F013-request-context-explicit-parameter.md) | P4 显式参数化 RequestContext：`Executor::execute` / middleware 钩子加 `&RequestContext`，thread-local 移除，破坏性（迁移指南内置） |
| 2026-08-05 | F | [F012](./docs/adr/F012-architecture-deepening-phase.md) | 架构深化阶段：Phase 1（P6 TypedValue / P2c Config 校验 / P2a AccountProvider）+ Phase 2（P1 CLI/business 分离 + P2b config 子集）+ Phase 3（P3 lifecycle + P4 request context + P5 middleware）全部落地 |
| 2026-07-14 | K | [K001–K004](./docs/adr/K001-memory-module.md) | Memory 模块设计完成（append-only 三元组 / 当前态视图 / graph / Searchable / 单实例） |
| 2026-07-14 | S | [S007](./docs/adr/S007-mail-search-local-cache.md) | Mail 搜索走本地 envelope 缓存（非 live IMAP `SEARCH`），与 rss/cal 一致 |
| 2026-07-12 | R | [R013–R015](./docs/adr/R013-auth-module-consolidation.md) | 凭据 / `login` 逻辑收拢到顶层 `auth` 模块；verify 显式可选；非交互输入契约 |
| 2026-07-12 | R | [R016–R018](./docs/adr/R016-action-backend-di.md) | 动作层 Backend trait + DI：note/todo/bookmark 去除 `NotionClient` 直接泄漏；目录布局；domain 类型 + Mock 回归护栏 |
| 2026-07-12 | S | [S001–S006](./docs/adr/S001-search-architecture.md) | 跨模块统一搜索：架构 / Hit 契约 / 查询语义 / 执行模型 / 时间语义与范围 / CLI |
| 2026-07-12 | F | [F009](./docs/adr/F009-performance-budget.md) | 性能预算（冷启动 < 100 ms + 网络超时 + 大输出流式） |
| 2026-07-12 | F | [F010](./docs/adr/F010-testing-requirements.md) | 测试要求（强制单测项 + mock + CI 行为） |
| 2026-07-12 | L | [L013](./docs/adr/L013-from-explicit-error.md) | Timeline `--from` 单独给定显式报错 |
| 2026-07-12 | R | [R012](./docs/adr/R012-config-executor-trait.md) | ConfigModule 走 Executor trait |
| 2026-07-12 | F | [F007](./docs/adr/F007-clap-subcommand-tree.md) | clap 数据驱动子命令树（module_arg_spec） |
| 2026-07-11 | L | [L001–L012](./docs/adr/L001-append-only-event-log.md) | Timeline 统一事件层全套 12 个决策 |
| 2026-07-11 | M | [M002–M005](./docs/adr/M002-imap-connection-pool.md) | Mail Cache：连接池 / envelope 缓存 / UID 水位 / staleness |
| 2026-07-11 | C | [C003](./docs/adr/C003-cal-provider-window-filter.md) | CalProvider::sync 必须遵循 window |
| 2026-07-11 | R | [R001–R011](./docs/adr/R001-thread-local-json-mode.md) | caveman review 沉淀的 11 个重构模式 |
| 2026-07-10 | T | [T002](./docs/adr/T002-todo-delete-action.md) | Todo `delete` action（Notion 归档 + 本地物理删除） |
| 2026-07-10 | B | [B001](./docs/adr/B001-bookmark-dual-provider.md) | Bookmark：双 provider（local SQLite 默认 + Notion） |
| 2026-07-10 | N | [N001](./docs/adr/N001-notion-note-module.md) | Note 模块屏蔽 Notion Block 嵌套 |
| 2026-07-10 | T | [T001](./docs/adr/T001-notion-todo-module.md) | Todo 模块（共享 notion-client） |
| 2026-07-10 | F | [F004](./docs/adr/F004-shared-notion-client.md) | 共享 Notion SDK + 429 退避重试 |
| 2026-07-10 | F | [F005](./docs/adr/F005-default-provider-local.md) | note / todo / bookmark 默认本地 provider |
| 2026-07-10 | F | [F006](./docs/adr/F006-ci-release-github-only.md) | CI + GitHub-only release（cnb 不推） |
| 2026-07-10 | F | [F003](./docs/adr/F003-module-scope-external-integration.md) | 模块范围：仅外部集成（移除 fs / net / sys） |
| 2026-07-09 | C | [C001](./docs/adr/C001-caldav-stack.md), [C002](./docs/adr/C002-full-pull-local-filter.md) | CalDAV 技术栈 + 全量 + 本地过滤 |
| 2026-07-09 | F | [F008](./docs/adr/F008-rss-module.md) | RSS 模块（feed-rs） |
| 2026-07-08 | F | [F001](./docs/adr/F001-cli-shape.md) | CLI 语法 / Executor / Output / AgentError |
| 2026-07-08 | F | [F002](./docs/adr/F002-multi-account-keyring.md) | 多账户 + OS keyring 凭证 |
| 2026-07-08 | M | [M001](./docs/adr/M001-imap-stack.md) | IMAP/SMTP 技术栈（async-imap + lettre + 桥接） |

## 发版流水

每个发版对应一组 ADR 与对应 commit。详细 commit 历史见 `git log --grep`。

| 版本 | tag | 摘要 | 主相关 ADR |
| --- | --- | --- | --- |
| **v0.16.1** | `v0.16.1` | 日志迁移收尾：warning 站点全迁 tracing（_warning+warning_text，text 逐字节/JSON 形状不变，auto_sync 成功降 info，timeline JSON 结构化）；mcp serve 迁移（_error 第三系）；README 双语契约段 | [F015](./docs/adr/F015-leveled-logging-tracing.md) |
| **v0.16.0** | `v0.16.0` | 默认日志静音 + `-v`/`-vv` 显式开启：引入 tracing + 自定义 Layer（text 紧凑格式 + JSON `{"_log"}` 形状不变，R001）；默认 WARN、`-v`=INFO、`-vv`=DEBUG；middleware 留栈靠 LevelFilter 静音；14 处 eprintln 全量迁移；契约测试锁定形状 | [F015](./docs/adr/F015-leveled-logging-tracing.md) |
| **v0.15.0** | `v0.15.0` | MCP server 上线：`everyday mcp serve`（rmcp 3.x stdio）把每个 (module, action) 协议投影为 MCP tool `<module>_<action>`，schema 复用 module_arg_spec；`mcp tools` 调试输出；`[WRITE]` 写标记；stdout 仅 JSON-RPC | [F014](./docs/adr/F014-mcp-module.md) |
| **v0.14.0** | `v0.14.0` | env 凭据回退（opt-in）：keyring 不可用时从 `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` 读凭据；双通道开关（config `[auth] env_credentials` / `EVERYDAY_ENV_CREDENTIALS=1`）；`auth list` 第四态 `env` | [R020](./docs/adr/R020-env-credential-fallback.md) |
| **v0.13.0** | `v0.13.0` | Notion provider 移除（note/todo/bookmark 仅本地 SQLite，破坏性）+ **WebDAV 设备同步**：`everyday sync` 双向文件级同步（4 用户 DB + config.toml，VACUUM INTO 快照 + SHA-256 变更检测，LWW 冲突副本），`auth --module webdav`，写命令后 opt-in auto_sync（D003）；单动作模块（sync/search）可省略 action | [R019](./docs/adr/R019-remove-notion-provider.md), [D001–D003](./docs/adr/D001-webdav-file-sync.md) |
| **v0.12.0** | `v0.12.0` | P4 显式参数化 RequestContext：`Executor::execute` / middleware 钩子加 `&RequestContext`，thread-local 移除（破坏性，迁移指南内置） | [F013](./docs/adr/F013-request-context-explicit-parameter.md) |
| **v0.11.0-rc** | `v0.11.0-rc` | 架构深化三阶段：P6 TypedValue / P2c Config 校验 / P2a AccountProvider / P1 CLI/business 分离 / P2b config 子集 / P3 lifecycle（`everyday health`）/ P4 RequestContext / P5 Middleware | [F012](./docs/adr/F012-architecture-deepening-phase.md) |
| **v0.10.0** | `v0.10.0` | Memory 模块落地：append-only `(subject, predicate, object)` 三元组 + 当前态视图 + graph + Searchable | [K001–K004](./docs/adr/K001-memory-module.md) |
| **v0.9.0** | `v0.9.0` | 跨模块统一搜索 v1.1 收口：`mail` Searchable 走本地 envelope 缓存 | [S007](./docs/adr/S007-mail-search-local-cache.md) |
| **v0.8.1** | `v0.8.1` | 动作层 Backend DI 重构：note/todo/bookmark 去 `NotionClient` 直接引用 | [R016–R018](./docs/adr/R016-action-backend-di.md) |
| **v0.8.0** | `v0.8.0` | 凭据 / `login` 收拢到顶层 `auth` 模块（破坏性：移除各模块 `login`） | [R013–R015](./docs/adr/R013-auth-module-consolidation.md) |
| **v0.7.0** | `v0.7.0` | 跨模块统一搜索：`everyday search` + Searchable/Registry | [S001–S006](./docs/adr/S001-search-architecture.md) |
| **v0.6.2** | `v0.6.2` | 修 Rust 1.97 clippy 注释 lint 阻塞 CI | （纯格式 patch，无新 ADR） |
| **v0.6.1** | `v0.6.1` | 修 timeline `--from` 单独给定被静默回退 | [L013](./docs/adr/L013-from-explicit-error.md) |
| **v0.6.0** | `v0.6.0` | Mail Cache 落地 + clap 子命令化 + 移除 help-registry | [M002–M005](./docs/adr/M002-imap-connection-pool.md), [F007](./docs/adr/F007-clap-subcommand-tree.md), [R012](./docs/adr/R012-config-executor-trait.md) |
| **v0.5.0** | `v0.5.0` | Timeline 统一事件层 + 4 处修补 | [L001–L013](./docs/adr/L001-append-only-event-log.md) |
| **v0.4.0** | `v0.4.0` | bookmark 模块 + 模块分层 + Justfile + cargo fmt 门槛 | [B001](./docs/adr/B001-bookmark-dual-provider.md), [F006](./docs/adr/F006-ci-release-github-only.md) |
| **v0.3.0** | `v0.3.0` | note/todo 本地 SQLite provider + 默认 local | [F005](./docs/adr/F005-default-provider-local.md) |
| **v0.2.0** | `v0.2.0` | todo Notion + 共享 notion-client | [T001](./docs/adr/T001-notion-todo-module.md), [F004](./docs/adr/F004-shared-notion-client.md) |
| **v0.1.0** | `v0.1.0` | 初始发布：mail / cal / rss / note + CI | [F001](./docs/adr/F001-cli-shape.md), [F002](./docs/adr/F002-multi-account-keyring.md), [M001](./docs/adr/M001-imap-stack.md), [C001](./docs/adr/C001-caldav-stack.md) |

发版流程步骤见 [`.rules/01-workflow.md`](./.rules/01-workflow.md)
§"Release (runbook summary)"。