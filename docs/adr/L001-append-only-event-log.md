# ADR L001: Append-only event log as the unified model

**Status:** Accepted
**Date:** 2026-07-11

## Context

Timeline is Everyday's unified event layer, aggregating events from Mail / Calendar / RSS / Todo / Note / Bookmark. The foundational question was whether Timeline should be:

- An **append-only log**: events are immutable records of "something happened at time T", never modified. Current state is derived by replaying an entity's event sequence.
- A **state mirror**: events are upserted by a dedup key (`source + ref_id + event_type`), so each entity has at most one event row reflecting its latest state.

The original design brief contained an internal contradiction: "回答过去发生了什么" (answering what happened in the past) implied an immutable log, but the dedup key `source + ref_id + event_type` implied mutable upsert.

## Decision

**Adopt the append-only log model.**

- Events are immutable once written. The natural key for idempotent deduplication is `(source, COALESCE(account, ''), ref_id, event_type, timestamp)`.
- The same `ref_id` can have multiple events (a todo's `created` and `completed` are separate rows).
- Current state is derived by sequential replay of an entity's events, not stored directly.

## Alternatives considered

### State mirror (upsert by `source + ref_id + event_type`)

- Loses transition timestamps: when a todo goes Todo → In Progress → Done, the upsert keeps only the final state, losing *when* transitions happened.
- Digest / Review features need transition times ("when did you complete this todo this week?").
- Mail "read", note "updated", calendar "rescheduled" are discrete temporal events that lose meaning when collapsed to current state.

## Consequences

- Calendar events are "future projections" not "past occurrences" — they get rescheduled / deleted, conflicting with immutability. Resolved by [L002](L002-calendar-window-refresh.md) (cal is the sole exception).
- Local SQLite tables store current state, not transition history. Resolved by [L008](L008-local-provider-degraded-granularity.md) — accept latest-state snapshot granularity.
- Re-syncing the same window is idempotent via the natural key (timestamp included); `INSERT OR IGNORE` on the unique index handles duplicates safely.
- All consumers (queries, digests, reviews) replay events to derive state — none of them maintain a parallel mirror table.

## Cross-references

- The natural key includes `account` as a first-class column: [L003](L003-account-first-class-column.md).
- The pull model that fills this log: [L004](L004-timeline-provider-pull-only.md).
- Query/sync separation that protects the log: [L005](L005-no-auto-sync.md).
- The sole non-append source: [L002](L002-calendar-window-refresh.md).