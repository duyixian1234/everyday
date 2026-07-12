# ADR C002: Full pull + local date filter (no server time-range REPORT)

**Status:** Accepted
**Date:** 2026-07-09

## Context

`cal list` needs to show events in a user-requested time window (default: next 7 days; can be overridden). Two architectural shapes are possible:

- **Server-side filter:** issue a CalDAV `REPORT` with a `time-range` element; the server returns only matching events.
- **Full pull + local filter:** issue `GetCalendarResources` (no filter), parse all VEVENTs locally, apply the date predicate in Rust.

The decision is whether to trust server-side filtering or to do the work in the CLI.

## Decision

**Always pull the full calendar resource set per calendar, then filter and sort locally.**

- Use `caldav.request(GetCalendarResources)` (which returns `calendar-data` along with the resource list).
- Parse each resource into `icalendar::Calendar`, walk events, apply the date predicate using `DatePerhapsTime::date_naive()`.
- Sort by `NaiveDateTime` ascending before rendering.

No `chrono-tz` dependency is introduced; `chrono::Local` (already a project dep via Timeline) is reused.

## Alternatives considered

### Server-side `time-range` REPORT

- Less data on the wire.
- **In practice unreliable.** Server implementations disagree on whether `DTEND` is exclusive, how floating vs zoned times are interpreted, and whether recurrences are expanded. QQ Calendar and NetEase Calendar both produced visibly wrong windows during local testing.
- The "less data" win is small for personal calendars (hundreds to low thousands of events per year).

### Hybrid: server REPORT for the broad window, local refinement

- More complex; still depends on server correctness for the coarse window.
- Marginal benefit.
- Rejected.

### Cache events locally and filter against the cache

- Could be combined with the mail cache approach ([M003](M003-envelope-cache.md)).
- Considered for future work, not part of this ADR.

## Consequences

- `cal list` does more work on the client but always produces correct results regardless of server quirks.
- A typical personal calendar (hundreds of events per year) is small enough that the network cost is dominated by the CalDAV round-trips, not by the payload size.
- No new dependency (`chrono-tz` is unnecessary).
- The local-filter pipeline is unit-testable without an actual CalDAV server — important because the CI gate is hermetic.
- Future enhancement: a local cache (similar to `mail_cache.db`) would make repeat `cal list` calls network-free. Out of scope for this ADR.

## Cross-references

- The CalDAV stack used to pull: [C001](C001-caldav-stack.md).
- The window filter applied to Timeline events emitted from `cal`: [L002](L002-calendar-window-refresh.md), [L006](L006-utc-storage-local-query.md).
- The DST-boundary handling that the local filter relies on: [R004](R004-dst-boundary-dates.md).