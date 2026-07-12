# Progress Log — Everyday

> 当前状态 + **ADR 时间序索引** + 发版流水。所有决策性叙述见
> [docs/adr/](./docs/adr/README.md)；按主题查找见 [findings.md](./findings.md)。
> 本文件的维护规则见 [`.rules/01-workflow.md`](./.rules/01-workflow.md)
> §"Finishing a task"。

## 当前状态

- **v0.8.0 已发布**：Phase 12 凭据 / `login` 逻辑收拢到顶层 `auth` 模块。
  删除 `mail` / `cal` / `note` / `todo` / `bookmark` 各自 `login` 子命令及本地 provider
  的 no-op `login`，改为统一 `everyday auth login|logout|verify|list --module <mod>`
  （[R013](./docs/adr/R013-auth-module-consolidation.md) 收拢总设计 / [R014](./docs/adr/R014-auth-verify-opt-in.md)
  verify 显式可选 / [R015](./docs/adr/R015-auth-credential-io.md) 非交互输入契约）。
  模块内部凭据读取改走 `auth::get_credential`；`--verify` 存后显式验证，
  默认只存。`auth verify` / `list` 对 local/sqlite、rss 短路 `not_required`。
  250 tests / clippy `-D warnings` 零警告 / fmt clean。破坏性变更：各模块 `login` 已移除。
- **v0.7.0 已发布**：Phase 11 跨模块统一搜索 `everyday search query "<q>"`
  落地，新增 `search` 模块（[S001–S006](./docs/adr/S001-search-architecture.md)）。
  Searchable 适配器覆盖 note / todo / bookmark / rss（新增本地条目缓存表）
  / cal（full-pull + in-memory GLOB）；best-effort 并发扇出，per-module cap 50，
  global cap 20，空结果 exit 0；warning 走 stderr（`--json` 结构化）。
  241 tests / clippy `-D warnings` 零警告 / fmt clean。
- **v0.7.0 已发布**：tag `v0.7.0`，跨模块统一搜索（search 模块 + Searchable/Registry），ADR S001–S006。
- **v0.6.2 已发布**：tag `v0.6.2`，修复 Rust 1.97 stable clippy
  `doc_lazy_continuation` + `doc_overindented_list_items` 两 lint 阻塞 CI 的问题
  （`src/modules/calendar.rs:10` 补 2 空格缩进、`src/modules/todo.rs:14` 由 14 空格
  缩至 2 空格）。纯注释 / 文档格式 patch，无功能性改动。
- **v0.6.1 已发布**：tag `v0.6.1`，修复 timeline `--from` 单独给定被静默回退
  preset 的问题。详见 [ADR L013](./docs/adr/L013-from-explicit-error.md)。
- **v0.6.0 已发布**：Mail Cache 落地 + CLI 重构（clap 子命令树 + 移除
  help-registry）。详见 [ADR M002–M005](./docs/adr/M002-imap-connection-pool.md) 与
  [ADR F007](./docs/adr/F007-clap-subcommand-tree.md)。
- **模块**：`mail` / `cal` / `rss` / `note` / `todo` / `bookmark` / `timeline`
  / `config` / **`search`**（9 个，走统一 Executor trait）均可用。
- **本地 provider 默认**：[note](./docs/adr/N001-notion-note-module.md) /
  [todo](./docs/adr/T001-notion-todo-module.md) /
  [bookmark](./docs/adr/B001-bookmark-dual-provider.md) 三模块默认走本地
  SQLite，Notion 显式声明。
- **Timeline**：append-only event log + ops-log AOP 统一 6 个 source 的事件捕获，
  详见 [L001–L013](./docs/adr/L001-append-only-event-log.md)。
- **质量门禁**：`cargo build` ✅ / `cargo clippy --all-targets -- -D warnings` ✅
  零警告 / `cargo test`（具体数字见各版本发版行）/ `cargo fmt --check` ✅；
  CI 三平台 + aarch64 mac 全绿（[F006](./docs/adr/F006-ci-release-github-only.md)）。

## ADR 时间序索引

按 ADR 时间倒序排列。完整列表见
[docs/adr/README.md](./docs/adr/README.md)。

| 日期 | 系列 | ADR | 摘要 |
| --- | --- | --- | --- |
| 2026-07-12 | R | [R013–R015](./docs/adr/R013-auth-module-consolidation.md) | 凭据 / `login` 逻辑收拢到顶层 `auth` 模块；verify 显式可选；非交互输入契约 |
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
| **v0.8.0** | `v0.8.0` | 凭据 / `login` 逻辑收拢到顶层 `auth` 模块（破坏性：移除各模块 `login`） | [R013–R015](./docs/adr/R013-auth-module-consolidation.md) |
| **v0.7.0** | `v0.7.0` | 跨模块统一搜索：`everyday search` + Searchable/Registry | [S001–S006](./docs/adr/S001-search-architecture.md) |
| **v0.6.2** | `v0.6.2` | 修 Rust 1.97 clippy 注释 lint 阻塞 CI | （纯格式 patch，无新 ADR） |
| **v0.6.1** | `v0.6.1` | 修 timeline `--from` 单独给定被静默回退 | [L013](./docs/adr/L013-from-explicit-error.md) |
| **v0.6.0** | `v0.6.0` | Mail Cache 落地 + clap 子命令化 + 移除 help-registry | [M002–M005](./docs/adr/M002-imap-connection-pool.md), [F007](./docs/adr/F007-clap-subcommand-tree.md), [R012](./docs/adr/R012-config-executor-trait.md) |
| **v0.5.0** | `v0.5.0` | Timeline 统一事件层 + 4 处修补 | [L001–L013](./docs/adr/L001-append-only-event-log.md) |
| **v0.4.0** | `v0.4.0` | bookmark 模块 + 模块分层 + Justfile + cargo fmt 门槛 | [B001](./docs/adr/B001-bookmark-dual-provider.md), [F006](./docs/adr/F006-ci-release-github-only.md) |
| **v0.3.0** | `v0.3.0` | note/todo 本地 SQLite provider + 默认 local | [F005](./docs/adr/F005-default-provider-local.md) |
| **v0.2.0** | `v0.2.0` | todo Notion + 共享 notion-client | [T001](./docs/adr/T001-notion-todo-module.md), [F004](./docs/adr/F004-shared-notion-client.md) |
| **v0.1.0** | `v0.1.0` | 初始发布：mail / cal / rss / note + CI | [F001](./docs/adr/F001-cli-shape.md), [F002](./docs/adr/F002-multi-account-keyring.md), [M001](./docs/adr/M001-imap-stack.md), [C001](./docs/adr/C001-caldav-stack.md) |

发版流程步骤见
[`.rules/01-workflow.md`](./.rules/01-workflow.md) §"Release (runbook summary)"。
