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

Status legend: **Accepted** = in production; **Superseded** = replaced by a later ADR (see the link).

---

## Foundation (F-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [F001](F001-cli-shape.md) | CLI command shape, Executor trait, Output, AgentError | Accepted | 2026-07-08 |
| [F002](F002-multi-account-keyring.md) | Multi-account configuration + OS keyring credentials | Accepted | 2026-07-08 |
| [F003](F003-module-scope-external-integration.md) | Module scope — external integration interface only (no fs/net/sys) | Accepted | 2026-07-10 |
| [F004](F004-shared-notion-client.md) | Shared Notion client SDK with 429 backoff retry | Accepted | 2026-07-10 |
| [F005](F005-default-provider-local.md) | Default provider is local SQLite for note/todo/bookmark | Accepted | 2026-07-10 |
| [F006](F006-ci-release-github-only.md) | CI + GitHub-only release workflow (cnb mirror excluded) | Accepted | 2026-07-10 |
| [F007](F007-clap-subcommand-tree.md) | Data-driven clap subcommand tree via module_arg_spec | Accepted | 2026-07-12 |
| [F008](F008-rss-module.md) | RSS module — feed-rs based subscription aggregator | Accepted | 2026-07-09 |
| [F009](F009-performance-budget.md) | Performance budget — cold start < 100 ms, network timeouts, large-output streaming | Accepted | 2026-07-12 |
| [F010](F010-testing-requirements.md) | Testing requirements — mandatory coverage, mocks, CI behaviour, bug-fix discipline | Accepted | 2026-07-12 |

## Mail (M-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [M001](M001-imap-stack.md) | IMAP stack — async-imap + tokio-rustls compat bridge + custom IMAP UTF-7 decoder + lettre SMTP | Accepted | 2026-07-08 |
| [M002](M002-imap-connection-pool.md) | IMAP connection pool M=4 with semaphore | Accepted | 2026-07-11 |
| [M003](M003-envelope-cache.md) | Envelope cache — dual-table SQLite design with K1 append-only retention | Accepted | 2026-07-11 |
| [M004](M004-uid-watermark-sync.md) | UID watermark + UIDVALIDITY incremental sync | Accepted | 2026-07-11 |
| [M005](M005-staleness-auto-sync.md) | Staleness-based auto-sync + flags snapshot + search bypass | Accepted | 2026-07-11 |

## Calendar (C-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [C001](C001-caldav-stack.md) | CalDAV stack — libdav + icalendar + hyper-rustls (ring provider), skip DNS SRV bootstrap | Accepted | 2026-07-09 |
| [C002](C002-full-pull-local-filter.md) | Full pull + local date filter (no server time-range REPORT) | Accepted | 2026-07-09 |
| [C003](C003-cal-provider-window-filter.md) | CalProvider::sync must honor the window argument | Accepted | 2026-07-11 |

## Note (N-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [N001](N001-notion-note-module.md) | Note module — Notion API integration that shields block nesting | Accepted | 2026-07-10 |

## Todo (T-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [T001](T001-notion-todo-module.md) | Todo module — Notion API + shared notion-client (strongly-typed DTO) | Accepted | 2026-07-10 |
| [T002](T002-todo-delete-action.md) | Todo delete action — Notion archive + local physical delete (with title preservation) | Accepted | 2026-07-11 |

## Bookmark (B-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [B001](B001-bookmark-dual-provider.md) | Bookmark module — local SQLite (default) + Notion (with exact-match tag filter) | Accepted | 2026-07-10 |

## Timeline (L-series)

| # | Title | Status | Date |
|---|-------|--------|------|
| [L001](L001-append-only-event-log.md) | Append-only event log as the unified model | Accepted | 2026-07-11 |
| [L002](L002-calendar-window-refresh.md) | Calendar window-refresh exception to the append model | Accepted | 2026-07-11 |
| [L003](L003-account-first-class-column.md) | Account as first-class nullable schema column | Accepted | 2026-07-11 |
| [L004](L004-timeline-provider-pull-only.md) | TimelineProvider as separate trait + pull-only model | Accepted | 2026-07-11 |
| [L005](L005-no-auto-sync.md) | No auto-sync on query — query/sync separation | Accepted | 2026-07-11 |
| [L006](L006-utc-storage-local-query.md) | UTC timestamp storage with local-timezone query | Accepted | 2026-07-11 |
| [L007](L007-notion-ops-log.md) | Notion provider via local ops-log with AOP dispatch hook | Accepted | 2026-07-11 |
| [L008](L008-local-provider-degraded-granularity.md) | Local provider degraded event granularity (latest-state snapshot) | Accepted | 2026-07-11 |
| [L009](L009-best-effort-sync.md) | Best-effort sync with per-provider watermarks + grouped parallel | Accepted | 2026-07-11 |
| [L010](L010-ops-log-provider.md) | OpsLogProvider — project ops-log rows into the events table | Accepted | 2026-07-11 |
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
| [R007](R007-config-account-macro.md) | Macro for `Config::X_account()` lookups (module-scope, not inside `impl`) | Accepted | 2026-07-11 |
| [R008](R008-sql-glob-not-like.md) | Use SQL `GLOB`, not `LIKE`, for token-boundary flag matching | Accepted | 2026-07-11 |
| [R009](R009-notion-common-local-module.md) | Common `local` module for shared Notion abstractions (login_flow, parse_tags, set_module_database_id) | Accepted | 2026-07-11 |
| [R010](R010-notion-local-account.md) | `NotionLocalAccount` merge + type alias (TodoAccount / BookmarkAccount) | Accepted | 2026-07-11 |
| [R011](R011-add-dual-providers-macro.md) | `add_dual_providers!` macro for `build_providers` (todo/note/bookmark) | Accepted | 2026-07-11 |
| [R012](R012-config-executor-trait.md) | `ConfigModule` goes through the `Executor` trait | Accepted | 2026-07-12 |
| [R013](R013-auth-module-consolidation.md) | Consolidate all credential/login logic into a top-level `auth` module + command | Accepted | 2026-07-12 |
| [R014](R014-auth-verify-opt-in.md) | `verify` is an explicit opt-in step, separate from credential storage | Accepted | 2026-07-12 |
| [R015](R015-auth-credential-io.md) | Non-interactive credential input via flags; secrets never read from environment | Accepted | 2026-07-12 |
| [R016](R016-action-backend-di.md) | Action-layer `Backend` trait + Dependency Inversion for note/todo/bookmark (kill `NotionClient` leak) | Accepted | 2026-07-12 |
| [R017](R017-backend-layout-scope.md) | Backend directory layout (L-B) + action-layer scope boundary | Accepted | 2026-07-12 |
| [R018](R018-backend-domain-mocks.md) | Backend domain types + in-memory mock backends (DI regression guard) | Accepted | 2026-07-12 |

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

---

## Conventions

- Every ADR has the same shape: `# ADR <id>: <title>`, then `**Status:**` / `**Date:**`, then `Context`, `Decision`, `Alternatives considered`, `Consequences`.
- Cross-references use the file-name form: `[L001](L001-append-only-event-log.md)`.
- New ADRs get the next free number in the relevant module prefix.
- When superseding, keep the old ADR and add `**Superseded by** [F0xx](...)`; do not delete history.