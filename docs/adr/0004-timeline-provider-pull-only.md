# ADR 0004: TimelineProvider as separate trait + pull-only model

**Status:** Accepted
**Date:** 2026-07-11

## Context

Timeline needs to pull events from six existing modules (mail / cal / rss / todo / note / bookmark). Two architectural questions were coupled:

1. **Where do providers live?** In the timeline module reaching into module internals, or in each module exposing a provider?
2. **Push or pull?** The original design proposed both: remote sources pulled on sync, local modules pushed events on write.

These are coupled because the push model requires modules to depend on timeline's database, which affects where the provider logic lives.

## Decision

### Separate TimelineProvider trait (not on Executor)

Timeline defines its own `TimelineProvider` trait, separate from `Executor`:

```rust
#[async_trait]
pub trait TimelineProvider: Send + Sync {
    fn source(&self) -> &'static str;
    async fn sync(&self, window: &TimeWindow) -> Result<(Vec<TimelineEvent>, SyncMode)>;
}
```

- A `TimelineProviderRegistry` (independent from `ModuleRegistry`) holds provider instances.
- Providers are stateless: they receive a `TimeWindow` and return events. The orchestrator manages watermarks in `sync_state`.

### Pull-only model (no push, even for local modules)

All six sources are pulled by the sync orchestrator, including local modules (todo/note/bookmark). Local providers query their own SQLite tables during sync. Modules never write to timeline.db.

### Provider adapters in timeline module

Each module exposes a `fetch_for_timeline` data-access function. Timeline's `providers.rs` writes adapter implementations that call these functions and convert results to `TimelineEvent`. Dependency direction: timeline → modules (single-direction), modules don't depend on timeline types.

## Alternatives considered

### Push for local modules (original design)

- Local modules write events to timeline.db on every write operation.
- **Rejected**: breaks module independence (`agents.md` hard constraint: "new module = new file + one registration line"). Todo depending on timeline.db is architectural inversion — timeline is the consumer, not the depended-upon.
- Double-write consistency burden: push failure (db lock, disk full) requires pull compensation; if pull must exist anyway, push is redundant.
- Local SQLite queries are millisecond-level; pull latency is imperceptible.

### TimelineProvider method on Executor

- Add `async fn timeline_events()` to Executor with a default empty impl.
- **Rejected**: pollutes Executor with timeline concerns. Modules that don't participate carry the method. Executor is for user commands; timeline sync is an orthogonal internal concern.

### Timeline-internal providers reaching into module internals

- Timeline module directly imports IMAP/CalDAV/SQLite internals of each module.
- **Rejected**: breaks encapsulation. Timeline would need to know six different data access patterns, including the local vs notion provider split for todo/note/bookmark.

## Consequences

- `timeline today` reflects data as of the last `timeline sync`. Local module writes (e.g., `todo add`) won't appear until the next sync. This is acceptable: local SQLite queries are ~ms, and [ADR 0005](0005-no-auto-sync.md) defers sync to explicit invocation.
- Each module gains one `fetch_for_timeline` function — a clean, minimal coupling point.
- Notion accounts cannot be pulled from Notion API (no incremental history, rate limits). Resolved by [ADR 0007](0007-notion-ops-log.md).
