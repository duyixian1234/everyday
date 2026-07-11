# ADR 0009: Best-effort sync with per-provider watermarks

**Status:** Accepted
**Date:** 2026-07-11

## Context

`timeline sync` iterates all providers (mail×N accounts + cal×N + rss + todo×N + note×N + bookmark×N). Network providers can fail: mail IMAP timeout, cal CalDAV 401, rss feed 503.

Two execution-model questions:
1. **Failure handling**: if one provider fails, does the entire sync abort, or does it continue?
2. **Concurrency**: do providers run in parallel or sequentially?

## Decision

### Best-effort with per-provider watermarks

Each provider executes independently within a `try/catch`:
- **Success**: events are written, `sync_state.last_sync` is updated for that `(source, account)`.
- **Failure**: provider is skipped, watermark unchanged. Next sync will retry the same window.
- Sync overall returns success (not error) even if some providers failed.
- Failed providers are reported in sync output.

### Grouped parallel execution

Providers are grouped by `source`. Groups execute in parallel (`futures::join_all`). Within a group, multiple accounts execute sequentially.

- `mail[work]` and `mail[personal]` → sequential (same IMAP server risk of rate-limiting).
- `mail[*]` and `cal[*]` and `rss` and `todo[*]` → parallel (different servers).
- Local providers (todo/note/bookmark) are millisecond-level; sequential within group is imperceptible.

## Alternatives considered

### All-or-nothing (transactional sync)

Any provider failure aborts the entire sync and rolls back all changes.

- **Rejected**: one broken rss feed (503) blocks all mail/cal/todo data. A single bad source paralyzes the entire timeline.
- Watermark rollback for succeeded providers is wasteful (they'd re-pull the same window next time).

### Full parallel (all providers concurrently)

All providers including same-source multi-account run in parallel.

- **Rejected**: multiple IMAP connections to the same server risk rate-limiting / connection refusal. Same-source accounts often share infrastructure.

### Full sequential

All providers run one after another.

- **Rejected**: three network sources serialized = 5-10 seconds. Grouped parallel reduces to ~2-3 seconds (slowest source).

## Consequences

- Failed providers don't block successful ones. Watermarks advance independently.
- Failed provider's watermark is unchanged, so next sync retries the same `[last_sync, now]` window. For append-mode sources, `INSERT OR IGNORE` on the natural key ensures re-pulled events don't duplicate. For cal's window-refresh, the delete-then-insert is inherently idempotent.
- Sync output must clearly indicate failures so users know which sources are stale. Text mode shows `FAILED` with error reason; JSON mode includes `status` and `error` per provider.
- Grouped parallel requires `futures::join_all` (already a dependency). No new crates needed.
