# Architecture Decision Records — Everyday

> Every nontrivial architectural decision for the Everyday CLI lives here.
> Each ADR captures the **Context**, **Decision**, **Alternatives considered**, and **Consequences**.
> ADRs cross-reference each other via relative markdown links; follow them to trace the full reasoning chain.

## How to read this index

ADRs are numbered by **module prefix**, not chronologically:

| Prefix   | Scope |
|----------|-------|
| `F00x`   | Foundation — cross-cutting concerns (CLI shape, accounts, scope, SDKs, CI) |
| `M00x`   | Mail module (IMAP, SMTP, envelope cache) |
| `C00x`   | Calendar module (CalDAV) |
| `N00x`   | Note module |
| `T00x`   | Todo module |
| `B00x`   | Bookmark module |
| `L00x`   | Timeline unified event layer |
| `R00x`   | Refactoring patterns (caveman review, 2026-07-11/12) |
| `K00x`   | Knowledge base — agent's own notebook (memory module) |
| `D00x`   | Device/WebDAV sync — cross-device file-level sync |

Status legend: **Accepted** = in production; **Superseded** = replaced by a later ADR (see the link).

---

## Foundation (F-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [F001](F001-cli-shape.md) | CLI command shape, Executor trait, Output, AgentError | Accepted | 2026-07-08 |
| [F002](F002-multi-account-keyring.md) | Multi-account configuration + OS keyring credentials | Accepted | 2026-07-08 |
| [F003](F003-module-scope-external-integration.md) | Module scope — external integration interface only (no fs/net/sys) | Accepted | 2026-07-10 |
| [F004](F004-shared-notion-client.md) | Shared Notion client SDK with 429 backoff retry | Superseded | 2026-07-10 |
| [F005](F005-default-provider-local.md) | Default provider is local SQLite for note/todo/bookmark | Accepted (rev. R019) | 2026-07-10 |
| [F006](F006-ci-release-github-only.md) | CI + GitHub-only release workflow (cnb mirror excluded) | Accepted | 2026-07-10 |
| [F007](F007-clap-subcommand-tree.md) | Data-driven clap subcommand tree via module_arg_spec | Accepted | 2026-07-12 |
| [F008](F008-rss-module.md) | RSS module — feed-rs based subscription aggregator | Accepted | 2026-07-09 |
| [F009](F009-performance-budget.md) | Performance budget — cold start < 100 ms, network timeouts, large-output streaming | Accepted | 2026-07-12 |
| [F010](F010-testing-requirements.md) | Testing requirements — mandatory coverage, mocks, CI behaviour, bug-fix discipline | Accepted | 2026-07-12 |
| [F012](F012-architecture-deepening-phase.md) | Architecture deepening phase — Phases 1–3 all implemented (P2c/P2a/P6, P1/P2b, P3/P4/P5) | Accepted | 2026-08-05 |
| [F013](F013-request-context-explicit-parameter.md) | Explicit-parameter `RequestContext` through `Executor::execute` + middleware (v0.12 breaking, supersedes F012 P4 thread-local form) | Accepted | 2026-08-05 |
| [F014](F014-mcp-module.md) | MCP module — everyday as an MCP server (stdio, rmcp 3.x), protocol projection of every (module, action) into MCP tools | Accepted | 2026-08-10 |
| [F015](F015-leveled-logging-tracing.md) | Leveled logging via tracing — default quiet (WARN), `-v`/`-vv` opt-in INFO/DEBUG, custom Layer preserving `{"_log"}` shapes | Accepted | 2026-08-10 |
| [F016](F016-daemon-sync-scheduler.md) | Daemon sync scheduler — resident `everyday daemon run` as the only role allowed to pull periodically (timeline + mail all-folders + rss); `--once`/`status`; state file; graceful shutdown | Accepted | 2026-08-13 |
| [F017](F017-task-module.md) | Task module — named no-shell command execution, SQLite history, bounded capture, process-tree timeout, and independent daemon cron loop | Accepted | 2026-08-18 |
| [F018](F018-agent-skill-distribution.md) | Agent skill distribution — `COMMANDS.md` as single inventory owner, CLI-projected drift check in CI, repo-as-live-source via link (copy keeps a staging gate but can diverge silently) | Accepted | 2026-09-12 |

## Mail (M-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [M001](M001-imap-stack.md) | IMAP stack — async-imap + tokio-rustls compat bridge + custom IMAP UTF-7 decoder + lettre SMTP | Accepted | 2026-07-08 |
| [M002](M002-imap-connection-pool.md) | IMAP connection pool M=4 with semaphore | Accepted | 2026-07-11 |
| [M003](M003-envelope-cache.md) | Envelope cache — dual-table SQLite design with K1 append-only retention | Accepted | 2026-07-11 |
| [M004](M004-uid-watermark-sync.md) | UID watermark + UIDVALIDITY incremental sync | Accepted | 2026-07-11 |
| [M005](M005-staleness-auto-sync.md) | Staleness-based auto-sync + flags snapshot + search bypass | Accepted | 2026-07-11 |
| [M006](M006-mail-search-cached.md) | `mail search --cached` — local envelope-cache search path (opt-in, typed output) | Accepted | 2026-08-21 |
| [M007](M007-mail-cache-gc.md) | `mail cache gc` — manual, server-reconciled ghost-envelope cleanup (daemon stays pull-only) | Accepted | 2026-08-21 |
| [M008](M008-mail-folder-key-canonicalization.md) | Canonical folder keys (raw modified-UTF-7) + Message-ID duplicate suppression in the envelope cache | Accepted | 2026-09-12 |

## Calendar (C-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [C001](C001-caldav-stack.md) | CalDAV stack — libdav + icalendar + hyper-rustls (ring provider), skip DNS SRV bootstrap | Accepted | 2026-07-09 |
| [C002](C002-full-pull-local-filter.md) | Full pull + local date filter (no server time-range REPORT) | Accepted | 2026-07-09 |
| [C003](C003-cal-provider-window-filter.md) | CalProvider::sync must honor the window argument | Accepted | 2026-07-11 |
| [C004](C004-cal-event-cache.md) | Calendar event cache — snapshot-replace (not append-only), UID-keyed, 15-min staleness, lazy refresh | Accepted | 2026-09-18 |

## Note (N-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [N001](N001-notion-note-module.md) | Note module — Notion API integration that shields block nesting | Superseded | 2026-07-10 |

## Todo (T-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [T001](T001-notion-todo-module.md) | Todo module — Notion API + shared notion-client (strongly-typed DTO) | Superseded | 2026-07-10 |
| [T002](T002-todo-delete-action.md) | Todo delete action — Notion archive + local physical delete (with title preservation) | Accepted | 2026-07-11 |

## Bookmark (B-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [B001](B001-bookmark-dual-provider.md) | Bookmark module — local SQLite (default) + Notion (with exact-match tag filter) | Superseded | 2026-07-10 |

## Timeline (L-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [L001](L001-append-only-event-log.md) | Append-only event log as the unified model | Accepted | 2026-07-11 |
| [L002](L002-calendar-window-refresh.md) | Calendar window-refresh exception to the append model | Accepted | 2026-07-11 |
| [L003](L003-account-first-class-column.md) | Account as first-class nullable schema column | Accepted | 2026-07-11 |
| [L004](L004-timeline-provider-pull-only.md) | TimelineProvider as separate trait + pull-only model | Accepted | 2026-07-11 |
| [L005](L005-no-auto-sync.md) | No auto-sync on query — query/sync separation | Accepted | 2026-07-11 |
| [L006](L006-utc-storage-local-query.md) | UTC timestamp storage with local-timezone query | Accepted | 2026-07-11 |
| [L007](L007-notion-ops-log.md) | Notion provider via local ops-log with AOP dispatch hook | Superseded | 2026-07-11 |
| [L008](L008-local-provider-degraded-granularity.md) | Local provider degraded event granularity (latest-state snapshot) | Accepted | 2026-07-11 |
| [L009](L009-best-effort-sync.md) | Best-effort sync with per-provider watermarks + grouped parallel | Accepted | 2026-07-11 |
| [L010](L010-ops-log-provider.md) | OpsLogProvider — project ops-log rows into the events table | Superseded | 2026-07-11 |
| [L011](L011-aop-handles-output-text.md) | AOP ops-log hook must parse Output::Text variant | Accepted | 2026-07-11 |
| [L012](L012-since-query-flag.md) | `--since` flag in query path (date + relative duration) | Accepted | 2026-07-11 |
| [L013](L013-from-explicit-error.md) | Timeline `--from` solo explicit error (resolve_query_range) | Accepted | 2026-07-12 |

## Refactoring patterns (R-series)

Refactoring patterns and structural decisions — caveman-review fixes from 2026-07-11/12 plus later dependency-injection refactors. These ADRs document **reusable patterns** the codebase must follow going forward.

| # | Title | Status | Date |
|---|-------|--------|------|
| [R001](R001-thread-local-json-mode.md) | Thread-local `is_json()` instead of `std::env::args()` scan | Accepted | 2026-07-11 |
| [R002](R002-output-json-failure.md) | Output JSON serialization failure must not break the `--json` contract | Accepted | 2026-07-11 |
| [R003](R003-pool-guard-drop.md) | PoolGuard::Drop must guard `tokio::spawn` with `Handle::try_current()` | Accepted | 2026-07-11 |
| [R004](R004-dst-boundary-dates.md) | DST-boundary date parsing — use `.earliest()` / `.latest()`, never `.unwrap()` | Accepted | 2026-07-11 |
| [R005](R005-parse-simple-args.md) | `parse_simple_args` — single-dash tokens are values, double-dash tokens are flags | Accepted | 2026-07-11 |
| [R006](R006-ops-log-surfacing.md) | Surface ops-log write failures to the user | Accepted | 2026-07-11 |
| [R007](R007-config-account-macro.md) | Macro for `Config::X_account()` lookups (module-scope, not inside `impl`) | Superseded | 2026-07-11 |
| [R008](R008-sql-glob-not-like.md) | Use SQL `GLOB`, not `LIKE`, for token-boundary flag matching | Accepted | 2026-07-11 |
| [R009](R009-notion-common-local-module.md) | Common `local` module for shared abstractions (login_flow, parse_tags, set_module_database_id) | Accepted (rev. R019) | 2026-07-11 |
| [R010](R010-notion-local-account.md) | `NotionLocalAccount` merge + type alias (TodoAccount / BookmarkAccount) | Superseded | 2026-07-11 |
| [R011](R011-add-dual-providers-macro.md) | `add_dual_providers!` macro for `build_providers` (todo/note/bookmark) | Superseded | 2026-07-11 |
| [R012](R012-config-executor-trait.md) | `ConfigModule` goes through the `Executor` trait | Accepted | 2026-07-12 |
| [R013](R013-auth-module-consolidation.md) | Consolidate all credential/login logic into a top-level `auth` module + command | Accepted | 2026-07-12 |
| [R014](R014-auth-verify-opt-in.md) | `verify` is an explicit opt-in step, separate from credential storage | Accepted | 2026-07-12 |
| [R015](R015-auth-credential-io.md) | Non-interactive credential input via flags; secrets never read from environment | Accepted | 2026-07-12 |
| [R016](R016-action-backend-di.md) | Action-layer `Backend` trait + Dependency Inversion for note/todo/bookmark (kill `NotionClient` leak) | Accepted | 2026-07-12 |
| [R017](R017-backend-layout-scope.md) | Backend directory layout (L-B) + action-layer scope boundary | Accepted | 2026-07-12 |
| [R018](R018-backend-domain-mocks.md) | Backend domain types + in-memory mock backends (DI regression guard) | Accepted | 2026-07-12 |
| [R019](R019-remove-notion-provider.md) | Remove the Notion provider — note/todo/bookmark become local-only (v0.13 breaking) | Accepted | 2026-08-09 |
| [R020](R020-env-credential-fallback.md) | Environment-variable credential fallback (opt-in) — controlled exception to R015 | Accepted | 2026-08-09 |
| [R021](R021-date-sequence-id.md) | Date-sequence IDs — `{prefix}{YYYYMMDD}-{seq}` replaces the nanosecond-hex ID format (v0.17.1) | Accepted | 2026-08-14 |
| [R022](R022-config-editor.md) | Unified comment-preserving `ConfigEditor` — config module as the single writer of `config.toml` (closes F017's config-set lossy divergence) | Accepted | 2026-08-18 |
| [R023](R023-exit-code-on-request-context.md) | Exit code is a host concern on `RequestContext`, not an `Output` variant (removes `Output::ExitCode`) | Accepted | 2026-08-18 |

## Search (S-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [S001](S001-search-architecture.md) | Search architecture — Searchable trait + SearchRegistry | Accepted | 2026-07-12 |
| [S002](S002-hit-normalization.md) | Hit normalization & SearchQuery contract | Accepted | 2026-07-12 |
| [S003](S003-query-semantics.md) | Query semantics — tokenize OR, case-insensitive GLOB | Accepted | 2026-07-12 |
| [S004](S004-execution-model.md) | Execution model — concurrent, best-effort, cap/limit, exit codes | Accepted | 2026-07-12 |
| [S005](S005-time-semantics-scope.md) | Time semantics & module scope (v1 / v1.1, rss cache) | Accepted | 2026-07-12 |
| [S006](S006-search-module-cli.md) | Search module CLI — `query` action + flags | Accepted | 2026-07-12 |
| [S007](S007-mail-search-local-cache.md) | Mail search via local envelope cache (v1.1) | Accepted | 2026-07-14 |

## Knowledge base (K-series)

Append-only triple store for the agent's own notebook. Distinguishes from Timeline (timestamped events) and Note (unstructured prose) — see [`CONTEXT.md` §Memory](../../CONTEXT.md) for the conceptual boundary. **Frozen as of K005**: the module keeps working but takes no new features and is scheduled for removal in a later major version.

| # | Title | Status | Date |
|---|-------|--------|------|
| [K001](K001-memory-module.md) | Memory module — agent's own notebook (append-only triple store) | Accepted (rev. K005) | 2026-07-14 |
| [K002](K002-memory-graph-query.md) | Memory graph query — forward-only recursive traversal on current state | Accepted (rev. K005) | 2026-07-14 |
| [K003](K003-memory-searchable.md) | Memory participates in cross-module `Searchable` trait (current-state GLOB) | Accepted (rev. K005) | 2026-07-14 |
| [K004](K004-memory-single-instance.md) | Memory single-instance storage — no account column | Accepted (rev. K005) | 2026-07-14 |
| [K005](K005-memory-module-deprecation.md) | Deprecate the memory module — frozen, export-then-remove | Accepted | 2026-09-16 |

## Device sync (D-series)

Cross-device file-level sync to WebDAV (RFC 4918). Scope: bookmark/memory/note/todo SQLite DBs + config.toml. See [`CONTEXT.md` §WebDAV 设备同步](../../CONTEXT.md) for the glossary.

| # | Title | Status | Date |
|---|-------|--------|------|
| [D001](D001-webdav-file-sync.md) | WebDAV file-level sync — scope, semantics, LWW + conflict copy | Accepted | 2026-08-09 |
| [D002](D002-snapshot-hash-state.md) | Consistent snapshots (VACUUM INTO) + hash-driven state | Accepted | 2026-08-09 |
| [D003](D003-auto-sync-cli-boundary.md) | auto_sync — best-effort push after write commands, never on query | Accepted | 2026-08-09 |

## Quality tool suite (G-series)

Engineering-stack decisions: what tooling guards the release/CI pipeline and why.

| # | Title | Status | Date |
|---|-------|--------|------|
| [G001](G001-quality-tools-suite.md) | Quality tool suite — nextest/typos/git-cliff/cargo-deny + CLI contract tests; semver-checks rejected (pure bin); unmaintained deps accepted | Accepted | 2026-08-09 |

## Process and planning (F-series, continued)

| # | Title | Status | Date |
|---|-------|--------|------|
| [F019](F019-github-issues-as-plan-tracker.md) | GitHub Issues as the plan and progress tracker — `progress.md` / `task_plan.md` deleted; phases become milestones, work becomes issues, release log moves to `docs/releases.md` | Accepted | 2026-09-16 |

---

## ADR timeline

The tables above are grouped **by series prefix** — the right index when you know the
area but not the number. This one is **chronological** — the right index when you know
roughly *when* something landed. Follow the link for the full decision; the summary is
one line, never a substitute for the ADR.

| 日期 | 系列 | ADR | 摘要 |
| --- | --- | --- | --- |
| 2026-09-18 | C | [C004](C004-cal-event-cache.md) | cal 本地事件缓存：**快照替换**而非 append-only（M003 的 K1 对 cal 语义错误——邮件 ghost 是噪音、日历 ghost 是错误信息；L002 已独立论证 cal 必须 delete-then-insert）；主键 `(account, calendar_href, uid)`，用 UID 而非 href（`cal add` 的 href 是纳秒时间戳文件名，re-PUT 即变）；存全量镜像 + `last_sync_at`（不存时间窗，否则会删掉用户正在看的事件）； staleness 15 分钟硬编码不进 config（沿用 M005），取最旧日历水位；惰性刷新、**不动 daemon**、**不进 WebDAV sync**；因替换不产生幽灵行故**不做 gc**（未来转 ETag/sync-token 增量时需补）；修订 C002 的「future work」 |
| 2026-09-16 | F | [F019](F019-github-issues-as-plan-tracker.md) | GitHub Issues 成为唯一规划与进度追踪器：删除 `progress.md` / `task_plan.md`；Phase → milestone、任务 → 带 assignee 的 issue、设计决策 → ADR、约定 → `.rules`；发版流水移到 `docs/releases.md`（gh release 无法承载「主相关 ADR」列）、ADR 时间序索引与本文件合并；Errors 表并入 `.rules/07-dependency-pitfalls.md`；禁止在仓库根新增规划文件 |
| 2026-09-16 | K | [K005](K005-memory-module-deprecation.md) | memory 模块冻结：现有 7 动作不变、不再演进，排期后续大版本移除（不承诺版本号）；新增 `memory export` 读基表导出全量（含历史版本 + 软删除行，唯一能取出视图隐藏数据的动作），`{export_version, exported_at, count, facts[]}` 信封、恒定 JSON 不受 `--json` 影响、`ORDER BY id` 顺序稳定可复现但**非时间序**、空库 `count:0`；只读故 MCP 不带 `[WRITE]`；K001–K004 改 `Accepted (revised)` 而非 Superseded；无 `import`（非破坏性） |
| 2026-09-12 | F | [F018](F018-agent-skill-distribution.md) | Agent 技能分发：`COMMANDS.md` 为模块清单唯一持有者（`skills/README.md` 去重复表、去版本号，改指针）；以 `everyday mcp tools --json` 为机器可读契约，`just check-skill-drift` 断言全部投影动作有文档（排除 `{mcp, health}`）并接入 `just ci`；仓库经 `just skill-link` 以 junction/symlink 成为技能活源——复制方案保留 staging gate 但会静默分叉，故弃用 |
| 2026-09-12 | M | [M008](M008-mail-folder-key-canonicalization.md) | 邮件文件夹键归一化：解码器迁出为 `modules/imap_utf7.rs`（新增编码器 + `canonical_folder_key`，规范形 = raw modified-UTF-7、`INBOX` 大小写折叠）；7 个缓存入口在写入/查询前归一化 → `--folder` 中/英名写同一主键；`list`/`search` 输出按 (account, Message-ID) 去重（无 Message-ID 回退 (folder,uid)，绝不误并）；`query_envelopes` 兼容历史双拼写行；非破坏性 patch v0.18.2 |
| 2026-08-21 | M | [M006](M006-mail-search-cached.md) | `mail search --cached` 本地缓存搜索：显式 opt-in（默认仍走 IMAP `SEARCH TEXT` 保全 body 召回）；stale >15min 先同步再查本地；复用 `search_envelopes` token-OR GLOB；`--cached` 走 P6 类型化输出，默认路径保留历史纯字符串契约（v0.18.0 计划） |
| 2026-08-21 | M | [M007](M007-mail-cache-gc.md) | `mail cache gc` 幽灵清理：显式手动命令（daemon 保持纯拉取）；服务端对账判定幽灵，UIDVALIDITY 变更文件夹跳过；`--account`/`--folder` scoped 默认全库；对账成功推进水位（v0.18.1 计划） |
| 2026-08-19 | F | [F008](F008-rss-module.md)（amendment） | RSS digest/fetch 区分度：digest 摘要列 + `--since` + 缓存优先（`--fresh` 强刷、空/无结果回退实时）；fetch 双入口（`--name` 写缓存 / `<url>` stateless）；纠正既有漂移（订阅存 config.toml、`--category`、`--since` 补实现）；非破坏性 patch v0.17.6 |
| 2026-08-18 | R | [R023](R023-exit-code-on-request-context.md) | 退出码移到 RequestContext：`Output::ExitCode` 变体与 `with_exit_code()` 移除，`Output` 回归纯值类型；`task run` 改调 `ctx.set_exit_code(mirrored)`；`finalize(result, ctx, mode)` 在 CLI 边界读退出码；`RequestContext` 新增 `OnceLock<i32>` 退出码槽（内部可变，保持 `&` 流动且 Send+Sync）；MCP 直接渲染、无进程退出 |
| 2026-08-18 | R | [R022](R022-config-editor.md) | 统一保留注释的 ConfigEditor：config 模块成为 config.toml 唯一写入者（set_dotted / insert_task / remove_task，toml_edit load→mutate→原子写）；`config set` 也升级为保留注释 + 写入时按 path 校验 + 原子写，关闭 F017 的 config-set 丢失注释分歧；task/config_edit.rs 移除；config list 文本读侧展示原文件、JSON 契约不变 |
| 2026-08-18 | F | [F017](F017-task-module.md) | Task 模块：`[tasks.<name>]`、无 shell runner、task.db 历史、64 KiB 捕获、进程树超时终止、daemon 独立 cron 调度循环 |
| 2026-08-14 | R | [R021](R021-date-sequence-id.md) | 日期序号 ID：`{prefix}{YYYYMMDD}-{PID}-{seq}` 取代纳秒 hex（纯内存计数器跨进程撞号 → 加 PID 段恢复唯一）；全前缀统一、按天重置；旧格式共存不迁移 |
| 2026-08-13 | F | [F016](F016-daemon-sync-scheduler.md) | Daemon 自动同步：`everyday daemon run` 常驻进程为唯一允许周期性自动拉取的角色（timeline run_sync + mail 全文件夹缓存 + rss 拉取，顺序执行、完成后 sleep）；`[daemon]` 节（enabled/interval_seconds/sources 向后兼容）；状态文件 daemon-state.json + pid 防重入；`--once`/`status`；优雅退出统一 graceful_shutdown 路径；常驻 stdout 静默、文件日志固定 INFO；不写 SCM 集成代码（docs/daemon.md 三平台示例） |
| 2026-08-10 | F | [F015](F015-leveled-logging-tracing.md) | 分级日志（tracing）：默认 WARN 静音，`-v`=INFO（中间件进度），`-vv`=DEBUG 预留；全局 `-v/--verbose`（Count，仿 --json）；自定义 Layer 写 stderr，text 紧凑格式 + JSON `{"_log"}` 形状不变（R001）；middleware 无条件留栈靠 LevelFilter 静音；仅渲染 everyday target；14 处 eprintln 全量迁移 |
| 2026-08-10 | F | [F014](F014-mcp-module.md) | MCP 模块：everyday 作 MCP server（stdio，rmcp 3.x）；每个 (module, action) 协议投影为 MCP tool `<module>_<action>`（schema 复用 module_arg_spec）；JSON 文本输出 + isError；mcp 模块注入 Arc<ModuleRegistry>（后置注入）；Mutex 串行；serve/tools 动作；无配置面 |
| 2026-08-09 | R | [R020](R020-env-credential-fallback.md) | env 凭据回退（opt-in）：keyring 不可用时从 `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` 读凭据；双通道开关（config `[auth] env_credentials` / `EVERYDAY_ENV_CREDENTIALS=1`）；读取链 keyring→env→报错；`auth list` 第四态 `env`；logout 对 env 凭据提示 unset；login 仍写 keyring；R015 默认行为不变 |
| 2026-08-09 | G | [G001](G001-quality-tools-suite.md) | 工程质量工具栈：nextest(CI)/typos/git-cliff/cargo-deny + CLI contract 测试；semver-checks 因纯 bin 被否；4 unmaintained 依赖显式接受（G001 记录理由）；cargo-dist/sccache 留批 2 |
| 2026-08-09 | D | [D001–D003](D001-webdav-file-sync.md) | WebDAV 设备同步：D001 同步模型/范围/冲突（文件级 LWW + 冲突副本）/ D002 快照 + hash 状态（sync-state.json）/ D003 auto_sync CLI 边界（写命令 best-effort push，查询永不触发） |
| 2026-08-09 | R | [R019](R019-remove-notion-provider.md) | 移除 Notion provider：note/todo/bookmark 仅本地 SQLite；`provider="notion"` 加载即报错；ops-log/AOP 钩子/OpsLogProvider/共享客户端/令牌流全删；JSON 核心键保持稳定（v0.13.0 破坏性） |
| 2026-08-05 | F | [F013](F013-request-context-explicit-parameter.md) | P4 显式参数化 RequestContext：`Executor::execute` / middleware 钩子加 `&RequestContext`，thread-local 移除，破坏性（迁移指南内置） |
| 2026-08-05 | F | [F012](F012-architecture-deepening-phase.md) | 架构深化阶段：Phase 1（P6 TypedValue / P2c Config 校验 / P2a AccountProvider）+ Phase 2（P1 CLI/business 分离 + P2b config 子集）+ Phase 3（P3 lifecycle + P4 request context + P5 middleware）全部落地 |
| 2026-07-14 | K | [K001–K004](K001-memory-module.md) | Memory 模块设计完成（append-only 三元组 / 当前态视图 / graph / Searchable / 单实例） |
| 2026-07-14 | S | [S007](S007-mail-search-local-cache.md) | Mail 搜索走本地 envelope 缓存（非 live IMAP `SEARCH`），与 rss/cal 一致 |
| 2026-07-12 | R | [R013–R015](R013-auth-module-consolidation.md) | 凭据 / `login` 逻辑收拢到顶层 `auth` 模块；verify 显式可选；非交互输入契约 |
| 2026-07-12 | R | [R016–R018](R016-action-backend-di.md) | 动作层 Backend trait + DI：note/todo/bookmark 去除 `NotionClient` 直接泄漏；目录布局；domain 类型 + Mock 回归护栏 |
| 2026-07-12 | S | [S001–S006](S001-search-architecture.md) | 跨模块统一搜索：架构 / Hit 契约 / 查询语义 / 执行模型 / 时间语义与范围 / CLI |
| 2026-07-12 | F | [F009](F009-performance-budget.md) | 性能预算（冷启动 < 100 ms + 网络超时 + 大输出流式） |
| 2026-07-12 | F | [F010](F010-testing-requirements.md) | 测试要求（强制单测项 + mock + CI 行为） |
| 2026-07-12 | L | [L013](L013-from-explicit-error.md) | Timeline `--from` 单独给定显式报错 |
| 2026-07-12 | R | [R012](R012-config-executor-trait.md) | ConfigModule 走 Executor trait |
| 2026-07-12 | F | [F007](F007-clap-subcommand-tree.md) | clap 数据驱动子命令树（module_arg_spec） |
| 2026-07-11 | L | [L001–L012](L001-append-only-event-log.md) | Timeline 统一事件层全套 12 个决策 |
| 2026-07-11 | M | [M002–M005](M002-imap-connection-pool.md) | Mail Cache：连接池 / envelope 缓存 / UID 水位 / staleness |
| 2026-07-11 | C | [C003](C003-cal-provider-window-filter.md) | CalProvider::sync 必须遵循 window |
| 2026-07-11 | R | [R001–R011](R001-thread-local-json-mode.md) | caveman review 沉淀的 11 个重构模式 |
| 2026-07-10 | T | [T002](T002-todo-delete-action.md) | Todo `delete` action（Notion 归档 + 本地物理删除） |
| 2026-07-10 | B | [B001](B001-bookmark-dual-provider.md) | Bookmark：双 provider（local SQLite 默认 + Notion） |
| 2026-07-10 | N | [N001](N001-notion-note-module.md) | Note 模块屏蔽 Notion Block 嵌套 |
| 2026-07-10 | T | [T001](T001-notion-todo-module.md) | Todo 模块（共享 notion-client） |
| 2026-07-10 | F | [F004](F004-shared-notion-client.md) | 共享 Notion SDK + 429 退避重试 |
| 2026-07-10 | F | [F005](F005-default-provider-local.md) | note / todo / bookmark 默认本地 provider |
| 2026-07-10 | F | [F006](F006-ci-release-github-only.md) | CI + GitHub-only release（cnb 不推） |
| 2026-07-10 | F | [F003](F003-module-scope-external-integration.md) | 模块范围：仅外部集成（移除 fs / net / sys） |
| 2026-07-09 | C | [C001](C001-caldav-stack.md), [C002](C002-full-pull-local-filter.md) | CalDAV 技术栈 + 全量 + 本地过滤 |
| 2026-07-09 | F | [F008](F008-rss-module.md) | RSS 模块（feed-rs） |
| 2026-07-08 | F | [F001](F001-cli-shape.md) | CLI 语法 / Executor / Output / AgentError |
| 2026-07-08 | F | [F002](F002-multi-account-keyring.md) | 多账户 + OS keyring 凭证 |
| 2026-07-08 | M | [M001](M001-imap-stack.md) | IMAP/SMTP 技术栈（async-imap + lettre + 桥接） |

---

## Conventions

- Every ADR has the same shape: `# ADR <id>: <title>`, then `**Status:**` / `**Date:**`, then `Context`, `Decision`, `Alternatives considered`, `Consequences`.
- Cross-references use the file-name form: `[L001](L001-append-only-event-log.md)`.
- New ADRs get the next free number in the relevant module prefix.
- When superseding, keep the old ADR and add `**Superseded by** [F0xx](...)`; do not delete history.