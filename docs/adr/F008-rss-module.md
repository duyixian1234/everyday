# ADR F008: RSS module — feed-rs based subscription aggregator

**Status:** Accepted
**Date:** 2026-07-09

## Context

RSS was the first "external integration" module added after mail and calendar. It also pre-dates most of the abstractions that arrived later (note/todo/bookmark). Three design questions had to be answered:

1. **Library.** Parse Atom and RSS 2.0 (and the long tail of RSS dialects) without hand-rolling a parser.
2. **State.** A subscription list is user state — it has to persist across runs and survive config edits.
3. **Failure handling.** Feeds die, redirect, rate-limit, and return malformed XML. The CLI must degrade gracefully rather than fail the entire `digest` if one feed is broken.

## Decision

### Library: `feed-rs`

- Crate: `feed-rs` (a fast, format-agnostic parser covering RSS 0.9x, 1.0, 2.0, Atom 0.3, 1.0).
- No dependency on `reqwest`'s higher-level features; the module owns its fetch + parse pipeline.

### Actions

```
everyday rss follow   <feed_url> [--tags t1,t2]
everyday rss unfollow <feed_url>
everyday rss list     [--tag T]
everyday rss digest   [--since 7d] [--limit 50]
everyday rss fetch    <feed_url>     # raw fetch + parse, no subscription state
```

- `follow` / `unfollow` / `list` operate on the subscription list.
- `digest` produces a unified table across all followed feeds; the only network call is the per-feed fetch.
- `fetch` is a stateless debug aid: parse one URL, print entries, exit. Useful for diagnosing a broken feed without polluting the subscription list.

### Subscription state

- Stored in `~/.config/everyday/rss.db` (single table: `subscriptions(url PRIMARY KEY, title, tags, added_at, last_fetched_at, last_status)`).
- Tags are stored as a JSON array; filtering by tag uses SQLite's JSON path or a simple `LIKE` match (acceptable for tens to hundreds of feeds).

### Best-effort fetch

- A single broken feed (timeout, 5xx, malformed XML) **does not abort** the entire `digest`.
- Per-feed failures are reported in the output; successful feeds still render.
- See [L009](L009-best-effort-sync.md) for the same pattern in Timeline.

### `--json` position semantics

- The original implementation allowed `--json` to appear anywhere on the command line. This was fragile: clap's `trailing_var_arg` swallowed later flags. Fixed early.
- The fix lives in the data-driven clap tree now: see [F007](F007-clap-subcommand-tree.md).

## Alternatives considered

### Hand-roll Atom / RSS parsing

- Rejected: RSS dialects multiply, and a parser bug becomes a security issue.

### Use `rss` crate (Atom-only cousin)

- Rejected: `feed-rs` covers both protocols and the long tail; using two crates would be silly.

### Subscribe via OPML import

- Considered: OPML import is a nice-to-have, not a core capability.
- Deferred: future work, not blocking v0.1.

### Stream the digest

- For very large digests, streaming avoids materializing everything in memory.
- Deferred: digests are bounded by `--limit`; default 50 entries per feed × handful of feeds stays small.

## Consequences

- The subscription DB is one more local store to manage, but it stays simple (one table).
- The best-effort fetch pattern means `digest` is reliable even when the user's feed list has stale entries.
- The `fetch` action is a useful debug primitive without polluting state.
- The `--json` semantics for this module established a project-wide rule later enforced by [R005](R005-parse-simple-args.md).

## Cross-references

- RSS feeds are a Timeline source: [L004](L004-timeline-provider-pull-only.md).
- The best-effort execution model it shares with Timeline: [L009](L009-best-effort-sync.md).

## Amendment (2026-08-19) — digest 摘要列 + `--since`；fetch 双入口

**Status:** Accepted (amended)
**Version:** v0.17.6 (non-breaking)

`digest` 与 `fetch` 的区分度不足：除范围维度（digest 跨源聚合 / fetch 单源）外，两者输出几乎同构，且 digest 名为"早报摘要"却无摘要列（`RssEntryRow` 从未携带 summary，尽管缓存与 timeline 已在使用该字段）。本次修订将差异轴扩展为二维（范围 + 信息量），并纠正本 ADR 与实现的既有漂移。

### 决策变更

1. **digest 输出加摘要列**：`feed / title / summary / published / author / link`。文本模式 summary 截断 ~80 字符（表格可读），JSON 给完整 ~200 字符（截断规则与 `EntryForCache` 一致）。摘要来源 = feed 条目自带的 summary/description 直出，**不做外部 LLM 生成**（保持 RSS 零鉴权、本地工具定位）。
2. **digest 新增 `--since`**：复用 timeline 的时长解析（`30m` / `7d`，也接受日期），按 `published` 过滤。有 `--since` 时无 published 的条目剔除（与 `fetch_for_timeline` 的窗口语义一致）；无 `--since` 时保留（排末位）。补齐 ADR 原文承诺但从未实现的 `--since 7d`。
3. **digest 数据源改为缓存优先**：默认读本地 `rss_items` 缓存（毫秒级）；`--fresh` 强刷实时抓取并更新缓存；表空或过滤后无结果时回退实时抓取（保证输出非空）。**无 staleness 阈值**——新鲜度由 daemon（常驻时）与 `--fresh`（手动时）负责，查询路径零网络（L005 精神）。`--category` 过滤时先从 config 解析匹配的 feed 名集合再查缓存（缓存表无 category 列）。
4. **fetch 双入口**：`rss fetch --name N`（从订阅列表解析 URL，写缓存——等价 daemon 单源拉取，daemon 复用 fetch 内部逻辑不变）+ `rss fetch <url>`（位置参数，stateless 调试——任意 URL 直接抓取解析，不查订阅列表、不写缓存）。`--name` 路径与既有调用面（skill 文档、自动化脚本）完全兼容。

### 既有漂移纠正（本次一并记录）

- **订阅存储**：ADR 原文写 `~/.config/everyday/rss.db`（subscriptions 表）；实际实现是 config.toml 的 `[[rss.feeds]]`（name/url/category），条目缓存另存 `rss-items.db`。以实际实现为准。
- **标签字段**：ADR 原文 `--tags`；实际实现为 `--category`。以实际实现为准。
- **fetch 参数**：ADR 原文 `fetch <feed_url>`（stateless）；实际实现一度只有 `--name`。本次修订为双入口共存，两者兼得。

### Consequences

- digest 成为"聚合阅读视图"（宽 + 深），fetch 保持"单源抓取/调试"（窄 + 精简），区分度由一维变二维。
- JSON rows 加 summary 字段 = 非破坏性（加字段）；新增 fetch 位置参数 = 非破坏性（旧命令原样可用）→ patch 版本 v0.17.6。
- 文档同步面：README×2、docs/commands×2、skill COMMANDS.md、module_arg_spec 帮助文本。