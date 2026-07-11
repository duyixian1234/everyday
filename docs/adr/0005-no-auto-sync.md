# ADR 0005: No auto-sync on query

**Status:** Accepted
**Date:** 2026-07-11

## Context

The original Timeline design specified: "默认每次执行 `timeline` 前自动执行一次 sync（可增加 `--no-sync` 跳过）".

This conflicts with a hard project constraint in `agents.md`:

> **性能预算：冷启动 < 100ms**

`timeline sync` touches three network sources (mail IMAP, cal CalDAV, rss HTTP fetches). A single mail account's IMAP SEARCH can take 1-3 seconds on domestic networks. Three remote sources serialized easily exceed 10 seconds. Auto-syncing before every `timeline` query would make every query take seconds, and repeatedly hit remote APIs (rate-limit risk).

## Decision

**Timeline queries never auto-sync. Queries read SQLite only (millisecond-level).**

- `everyday timeline today` → reads `timeline.db`, returns in < 100ms.
- `everyday timeline sync` → explicit sync command that pulls from all providers.
- `everyday timeline today --sync` → opt-in: sync once, then query.
- No `--no-sync` flag needed (sync is opt-in, not opt-out).

## Alternatives considered

### Auto-sync by default with `--no-sync` opt-out (original design)

- Violates the 100ms cold-start budget.
- AI Agents query timeline frequently ("what happened today"); each query hitting three remote APIs is unreasonable.
- Service provider rate-limiting risk from repeated sync calls.

### Auto-sync with caching (sync if stale > N minutes)

- Adds cache invalidation complexity.
- Still unpredictable latency (first query after staleness threshold is slow).
- The explicit `timeline sync` + `--sync` flag achieves the same control with simpler semantics.

## Consequences

- Timeline data may be stale between syncs. This is by design — Timeline is a "refresh on demand" cache, like `git fetch` / `rss digest`.
- AI Agents should sync periodically (e.g., `timeline sync` every few hours) then query freely.
- First sync uses a 30-day lookback window (`--since` overridable); see [ADR 0009](0009-best-effort-sync.md) for sync execution model.
