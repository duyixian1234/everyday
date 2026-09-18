# Release History — Everyday

> One row per release: the tag, a summary of what shipped, and the **primary ADR**
> that governs it. The release runbook that appends here lives in
> [`.rules/01-workflow.md`](../.rules/01-workflow.md) §"Release (runbook summary)".
>
> Why this file exists rather than relying on `gh release list`: the runbook appends
> to it inside the release commit (reviewed in the diff next to `Cargo.toml`), and
> GitHub Releases cannot carry the primary-ADR column. See
> [F019](./adr/F019-github-issues-as-plan-tracker.md).
>
> Per-commit history lives in `CHANGELOG.md` (`git cliff --unreleased --tag vX.Y.Z
> --prepend CHANGELOG.md`); tags and artifacts live on GitHub Releases.

| 版本 | tag | 摘要 | 主相关 ADR |
| --- | --- | --- | --- |
| **v0.20.0** | `v0.20.0` | Phase 27「缓存与离线一致性」，修的是同一表面下的**语义不一致**而非加功能。**cal 本地事件缓存**：`~/.config/everyday/cal-cache.db`，**快照替换**而非追加（刻意反转 M003 的 K1——日历事件是可变状态，陈旧行是错误信息而非噪音）、主键 `(account, calendar_href, uid)`（用 UID 而非 href，因 `cal add` 的 href 带纳秒时间戳、re-PUT 即变）、存全量镜像 + `last_sync_at`、staleness 15 分钟硬编码、惰性刷新、刷新失败沿用旧快照；`cal list` / search provider / timeline 共用该缓存，`search` 据此对 cal 变为离线可用；**不做 gc**（替换不产生幽灵行）；不进 WebDAV sync。`cal list --json` 新增 **UID** 列。**todo 软删除**：`deleted_at` 列 + `active_todos` 视图承载读路径（使泄漏结构性不可能）、写路径显式守卫、幂等列迁移（不重复 `memory/store.rs` 漏迁移的错误）；不提供 `restore`；WebDAV 文件级 LWW 语义**显式不变**。新增 `deleted` timeline 事件（此前无生产者——T002/L008 假定的 ops-log hook 已随 R019 删除）。**`everyday search --json` 每条命中新增 `stale` 字段**（`Hit.stale`，仅 cal 会为 true）。新增 `bookmark search`（GLOB over title/url/tags，复用跨模块 matcher）。契约新增一个 action、`cal list` 与 `search` 输出新增字段，均为**加性变更** | [C004](./adr/C004-cal-event-cache.md), [R024](./adr/R024-local-provider-soft-delete.md), [F020](./adr/F020-hit-staleness.md) |
| **v0.19.0** | `v0.19.0` | memory 模块冻结（现有 7 动作行为不变、不再演进、排期后续大版本移除但本版**不移除**）+ 新增 `everyday memory export`：读基表全量导出（含历史版本与软删除行，唯一能取出视图隐藏数据的动作），`{export_version, exported_at, count, facts[]}` 信封、恒定 JSON 不受 `--json` 影响、`ORDER BY id` 稳定可复现但**非时间序**、空库 `count:0`、无 `import`；K001–K004 改 `Accepted (revised)`。同期依赖现代化：15 个 major（toml 1 / reqwest 0.13 / sqlx 0.9 / toml_edit 0.25 / async-imap 0.11 / keyring 4 / libdav 0.11 / thiserror 2 / dirs 7 / sha2 0.11 / webpki-roots 1 / tower-http 0.7 / mailparse 0.17 / croner 4 / windows-sys 0.61）+ 删除 3 个无用依赖。非破坏性 | [K005](./adr/K005-memory-module-deprecation.md) |
| **v0.18.2** | `v0.18.2` | 邮件缓存文件夹键归一化 + 读取侧去重：`--folder` 中文名与 raw modified-UTF-7 名映射到同一主键（`INBOX` 大小写折叠），`mail list`/`search` 按 (account, Message-ID) 去重，消除「同一封邮件重复投递」幻影；`modules/imap_utf7.rs` 编解码器 + 一次性缓存迁移工具 | [M008](./adr/M008-mail-folder-key-canonicalization.md) |
| **v0.18.1** | `v0.18.1` | IMAP 会话池归还竞态修复：池改 mutex 空闲队列 + `Notify`（归还同步完成后再唤醒）、失效会话后台重建、重建成功前不可用；daemon 聚合文件夹级同步错误 | [M002](./adr/M002-imap-connection-pool.md), [R003](./adr/R003-pool-guard-drop.md) |
| **v0.18.0** | `v0.18.0` | Mail 补遗：`mail search --cached` 本地缓存搜索（显式 opt-in、stale 同步、P6 类型化输出、复用 search_envelopes_scoped）；`mail gc` 服务端对账幽灵清理（UIDVALIDITY 变更跳过、对账后推进水位、daemon 保持纯拉取） | [M006](./adr/M006-mail-search-cached.md), [M007](./adr/M007-mail-cache-gc.md) |
| **v0.17.7** | `v0.17.7` | cal 日程时区修复：`cal list` 对 UTC 存储事件（DTSTART 带 Z）按本地墙钟输出（`date_perhaps_time_to_naive` / `format_date_perhaps_time` 对 `Utc` 变体转 `Local`），修复日程显示早 8 小时，同时修正 UTC 事件排序错位 | [L006](./adr/L006-utc-storage-local-query.md)（补丁） |
| **v0.17.6** | `v0.17.6` | RSS digest/fetch 区分度：digest 摘要列（文本 80/JSON 200 截断）+ `--since` 时间窗（复用 timeline 时长解析）+ 本地 rss-items 缓存优先（`--fresh` 强刷、空/无结果回退实时）；fetch 双入口（`--name` 订阅源写缓存 / `<url>` stateless 调试不写缓存）；新增 `TypedValue::TruncatedText`；CLI 帮助与 docs/commands×2/skill 命令表同步 | [F008](./adr/F008-rss-module.md)（amendment） |
| **v0.17.5** | `v0.17.5` | `task run --json` 输出契约收口 + 捕获编码修复：`--json` 只捕获不回声（stdout 唯一 `_result` 信封）；捕获记录 UTF-8 优先 + GBK 兜底解码，修复 `ipconfig` GBK 输出在 JSON/task.db 里的 U+FFFD 乱码；新增 encoding_rs 依赖 | [F017](./adr/F017-task-module.md) |
| **v0.17.4** | `v0.17.4` | 配置与输出架构重构收口：ConfigEditor 统一 config.toml 唯一写入者（保留注释 + 原子写 + 按 path 校验，config set/task add-remove 走同一 seam）；退出码从 `Output` 移到 `RequestContext`（Output 回归纯值，MCP 不受影响）；task 执行深化到模块接口 + 调度器合并 Scheduler context；CLI 形状与 JSON 契约未变 | [R022](./adr/R022-config-editor.md), [R023](./adr/R023-exit-code-on-request-context.md) |
| **v0.17.3** | `v0.17.3` | Task run Windows 控制台非 UTF-8 输出修复：透传改用 OS 句柄直写（WriteFile），GBK 控制台输出不再崩溃；新增非 UTF-8 中继回归测试 | [F017](./adr/F017-task-module.md) |
| **v0.17.2** | `v0.17.2` | Task 用户自定义命令与 cron 调度：`[tasks.<name>]` 配置、`task add/run/list/remove/history`、无 shell 派生 + 进程树超时终止 + 64 KiB 捕获、task.db 审计历史、daemon 独立 30s cron 调度循环（不积压）、`--once` 接线；集成测试修复 CLI 解析冲突与调度加固 | [F017](./adr/F017-task-module.md) |
| **v0.17.1** | `v0.17.1` | 日期序号 ID：`{前缀}{YYYYMMDD}-{PID}-{当日序号}` 取代纳秒 hex，全前缀 n/t/b/m/ev/mc/ri 统一、按天重置、按前缀独立计数；PID 段恢复跨进程唯一（纯内存计数器会撞 SQLite 主键）；旧格式 id 共存不迁移 | [R021](./adr/R021-date-sequence-id.md) |
| **v0.17.0** | `v0.17.0` | Daemon 常驻自动同步：`everyday daemon run` 前台常驻为唯一允许周期性拉取的角色（timeline run_sync + mail 全文件夹缓存 + rss 拉取，顺序执行）；`[daemon]` 节（serde(default) 向后兼容）；状态文件 + pid 存活防重入；`daemon status`；优雅退出统一 `graceful_shutdown` | [F016](./adr/F016-daemon-sync-scheduler.md) |
| **v0.16.2** | `v0.16.2` | R020 修订：`[auth] env_credentials` 对 no-Config hot path 生效——main 启动时把 config 字段镜像到进程级开关（`sync_env_credentials_from_config`），双通道对全部模块生效 | [R020](./adr/R020-env-credential-fallback.md) |
| **v0.16.1** | `v0.16.1` | 日志迁移收尾：warning 站点全迁 tracing（`_warning`+`warning_text`，text 逐字节/JSON 形状不变，auto_sync 成功降 info，timeline JSON 结构化）；mcp serve 迁移（`_error` 第三系）；README 双语契约段 | [F015](./adr/F015-leveled-logging-tracing.md) |
| **v0.16.0** | `v0.16.0` | 默认日志静音 + `-v`/`-vv` 显式开启：引入 tracing + 自定义 Layer（text 紧凑格式 + JSON `{"_log"}` 形状不变，R001）；默认 WARN、`-v`=INFO、`-vv`=DEBUG；middleware 留栈靠 LevelFilter 静音；14 处 eprintln 全量迁移；契约测试锁定形状 | [F015](./adr/F015-leveled-logging-tracing.md) |
| **v0.15.0** | `v0.15.0` | MCP server 上线：`everyday mcp serve`（rmcp 3.x stdio）把每个 (module, action) 协议投影为 MCP tool `<module>_<action>`，schema 复用 module_arg_spec；`mcp tools` 调试输出；`[WRITE]` 写标记；stdout 仅 JSON-RPC | [F014](./adr/F014-mcp-module.md) |
| **v0.14.0** | `v0.14.0` | env 凭据回退（opt-in）：keyring 不可用时从 `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` 读凭据；双通道开关（config `[auth] env_credentials` / `EVERYDAY_ENV_CREDENTIALS=1`）；`auth list` 第四态 `env` | [R020](./adr/R020-env-credential-fallback.md) |
| **v0.13.1** | `v0.13.1` | 工程质量工具栈批 1 + 批 2：CI 测试换 nextest（junit 报告）、typos/git-cliff/cargo-deny 门禁、CLI contract 测试层；sync 并行 tmp 目录竞争修复；release 流水线换 cargo-dist（installer 脚本 + Sigstore attestation） | [G001](./adr/G001-quality-tools-suite.md) |
| **v0.13.0** | `v0.13.0` | Notion provider 移除（note/todo/bookmark 仅本地 SQLite，破坏性）+ **WebDAV 设备同步**：`everyday sync` 双向文件级同步（4 个用户 DB + config.toml，VACUUM INTO 快照 + SHA-256 变更检测，LWW 冲突副本），`auth --module webdav`，写命令后 opt-in auto_sync；单动作模块（sync/search）可省略 action | [R019](./adr/R019-remove-notion-provider.md), [D001–D003](./adr/D001-webdav-file-sync.md) |
| **v0.12.0** | `v0.12.0` | P4 显式参数化 RequestContext：`Executor::execute` / middleware 钩子加 `&RequestContext`，thread-local 移除（破坏性，迁移指南内置） | [F013](./adr/F013-request-context-explicit-parameter.md) |
| **v0.11.0-rc** | `v0.11.0-rc` | 架构深化三阶段：P6 TypedValue / P2c Config 校验 / P2a AccountProvider / P1 CLI/business 分离 / P2b config 子集 / P3 lifecycle（`everyday health`）/ P4 RequestContext / P5 Middleware | [F012](./adr/F012-architecture-deepening-phase.md) |
| **v0.10.0** | `v0.10.0` | Memory 模块落地：append-only `(subject, predicate, object)` 三元组 + 当前态视图 + graph + Searchable | [K001–K004](./adr/K001-memory-module.md) |
| **v0.9.0** | `v0.9.0` | 跨模块统一搜索 v1.1 收口：`mail` Searchable 走本地 envelope 缓存 | [S007](./adr/S007-mail-search-local-cache.md) |
| **v0.8.1** | `v0.8.1` | 动作层 Backend DI 重构：note/todo/bookmark 去 `NotionClient` 直接引用 | [R016–R018](./adr/R016-action-backend-di.md) |
| **v0.8.0** | `v0.8.0` | 凭据 / `login` 收拢到顶层 `auth` 模块（破坏性：移除各模块 `login`） | [R013–R015](./adr/R013-auth-module-consolidation.md) |
| **v0.7.0** | `v0.7.0` | 跨模块统一搜索：`everyday search` + Searchable/Registry | [S001–S006](./adr/S001-search-architecture.md) |
| **v0.6.2** | `v0.6.2` | 修 Rust 1.97 clippy 注释 lint 阻塞 CI | （纯格式 patch，无新 ADR） |
| **v0.6.1** | `v0.6.1` | 修 timeline `--from` 单独给定被静默回退 | [L013](./adr/L013-from-explicit-error.md) |
| **v0.6.0** | `v0.6.0` | Mail Cache 落地 + clap 子命令化 + 移除 help-registry | [M002–M005](./adr/M002-imap-connection-pool.md), [F007](./adr/F007-clap-subcommand-tree.md), [R012](./adr/R012-config-executor-trait.md) |
| **v0.5.0** | `v0.5.0` | Timeline 统一事件层 + 4 处修补 | [L001–L013](./adr/L001-append-only-event-log.md) |
| **v0.4.0** | `v0.4.0` | bookmark 模块 + 模块分层 + Justfile + cargo fmt 门槛 | [B001](./adr/B001-bookmark-dual-provider.md), [F006](./adr/F006-ci-release-github-only.md) |
| **v0.3.0** | `v0.3.0` | note/todo 本地 SQLite provider + 默认 local | [F005](./adr/F005-default-provider-local.md) |
| **v0.2.0** | `v0.2.0` | todo Notion + 共享 notion-client | [T001](./adr/T001-notion-todo-module.md), [F004](./adr/F004-shared-notion-client.md) |
| **v0.1.0** | `v0.1.0` | 初始发布：mail / cal / rss / note + CI | [F001](./adr/F001-cli-shape.md), [F002](./adr/F002-multi-account-keyring.md), [M001](./adr/M001-imap-stack.md), [C001](./adr/C001-caldav-stack.md) |
