# ADR L002: Calendar window-refresh exception to the append model

**Status:** Accepted
**Date:** 2026-07-11

## Context

[ADR L001](L001-append-only-event-log.md) established the append-only log model for all Timeline sources. Calendar events are fundamentally different:

- A calendar event is a **future projection** ("a meeting is scheduled for Friday 14:00"), not a **past occurrence** ("an email was received").
- Calendar events get **rescheduled** (start time changes) and **deleted** by the user.
- With pure append + natural key `(source, ref_id, event_type, timestamp)`: a rescheduled event (new start time → new timestamp) creates a new row; the old row (old timestamp) remains. The log now shows the **same meeting twice** — a ghost record at the old time.

Pure append is unusable for Calendar.

## Decision

**Calendar uses window-refresh sync mode, not append.**

- During sync, the orchestrator first `DELETE FROM events WHERE source = 'cal' AND timestamp BETWEEN window.from AND window.to`, then inserts the current snapshot returned by the CalProvider.
- All other sources (mail / rss / todo / note / bookmark) remain pure append.
- The cal window includes a 7-day lookahead (`[last_sync, now + 7 days]`) so `timeline today` / `timeline week` can show upcoming meetings.
- The provider must apply the window filter itself before returning events — see [C003](C003-cal-provider-window-filter.md).

## Alternatives considered

### Tombstone + reschedule event types

- Record `scheduled` events; on reschedule, write a new `rescheduled` event + tombstone the old one.
- Adds complexity: tombstone semantics, query-time filtering, and CalDAV doesn't expose "when was this rescheduled" so the reschedule timestamp can't be trusted.

### Pure append accepting ghost records

- Accept that rescheduled meetings show twice.
- Unacceptable for user experience: `timeline today` would show the same meeting at both old and new times.

### Upsert for cal only (hybrid)

- Upsert cal events by `(source, ref_id, event_type)` without timestamp. Essentially state-mirror for cal, breaking the append model's consistency.
- Window-refresh achieves the same result (current snapshot) more cleanly.

## Consequences

- Calendar sync is not idempotent in the append sense — it's delete-then-insert. Re-syncing the same window is safe (deletes old rows, inserts same snapshot).
- The DELETE scope is strictly bounded by the sync window's timestamp range, never touching events outside it.
- This is the **only** source with a non-append sync mode. The exception is isolated to one provider.
- The provider and the orchestrator must agree on the same window — see [C003](C003-cal-provider-window-filter.md).

## Cross-references

- The append model that cal is exempt from: [L001](L001-append-only-event-log.md).
- The provider contract that enforces the window: [C003](C003-cal-provider-window-filter.md).
- The orchestrator that drives the delete-then-insert: [L009](L009-best-effort-sync.md).