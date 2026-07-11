# ADR 0002: Calendar window-refresh exception to append model

**Status:** Accepted
**Date:** 2026-07-11

## Context

[ADR 0001](0001-append-only-event-log.md) established the append-only log model for all Timeline sources. However, Calendar events are fundamentally different from other sources:

- A calendar event is a **future projection** ("a meeting is scheduled for Friday 14:00"), not a **past occurrence** ("an email was received").
- Calendar events get **rescheduled** (start time changes) and **deleted** by the user.
- With pure append + natural key `(source, ref_id, event_type, timestamp)`: a rescheduled event (new start time → new timestamp) creates a new row. The old row (old timestamp) remains. The log now shows the **same meeting twice** — a ghost record at the old time.

This makes pure append unusable for Calendar.

## Decision

**Calendar uses window-refresh sync mode, not append.**

- During sync, the orchestrator first `DELETE FROM events WHERE source='cal' AND timestamp BETWEEN window.from AND window.to`, then inserts the current snapshot returned by the CalProvider.
- All other sources (mail / rss / todo / note / bookmark) remain pure append.
- The cal window includes a 7-day lookahead (`[last_sync, now + 7 days]`) so `timeline today` / `timeline week` can show upcoming meetings.

## Alternatives considered

### Tombstone + reschedule event types

Record `scheduled` events, and on reschedule write a new `rescheduled` event + tombstone the old one. Adds complexity: tombstone semantics, query-time filtering of tombstoned events, and CalDAV doesn't expose "when was this rescheduled" so we can't timestamp the reschedule accurately.

### Pure append accepting ghost records

Accept that rescheduled meetings show twice. Unacceptable for user experience — `timeline today` would show the same meeting at both old and new times.

### Upsert for cal only (hybrid)

Upsert cal events by `(source, ref_id, event_type)` without timestamp. This is essentially state-mirror for cal, breaking the append model's consistency. Window-refresh achieves the same result (current snapshot) more cleanly.

## Consequences

- Calendar sync is not idempotent in the append sense — it's a delete-then-insert. Re-syncing the same window is safe (deletes old rows, inserts same snapshot).
- The delete scope is strictly bounded by the sync window's timestamp range, so it never touches events outside the synced window.
- This is the **only** source with a non-append sync mode. The exception is isolated to one provider.
