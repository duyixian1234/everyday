# ADR C003: CalProvider::sync must honor the window argument

**Status:** Accepted
**Date:** 2026-07-11

## Context

`CalProvider::sync(window: &TimeWindow)` is the adapter from Calendar to Timeline. The window argument tells the provider which time range to return events for. Early implementations ignored it and returned **all** events from the calendar, then relied on the orchestrator to filter at insert time.

This was wrong for two reasons:

1. **Cost.** Pulling every event for every sync wastes server round-trips and CPU on parse + filter for events that the orchestrator would discard anyway.
2. **Semantics.** The natural key includes `timestamp` — if two syncs return the same event with two different parsed timestamps, the orchestrator may insert duplicates under different keys. Honoring the window keeps the sync incremental.

This was surfaced and fixed during the 2026-07-11/12 caveman-style review (commit `b124048`).

## Decision

**`CalProvider::sync` MUST apply the `TimeWindow` filter before returning events.**

- Filter events whose `DTSTART` (or the equivalent parsed `DateTime`) falls inside `[window.from, window.to]`.
- `cal` is the only Timeline source where the provider has the raw event stream — the orchestrator cannot recover the discarded events later.
- The orchestrator's window-refresh DELETE is still scoped to `[window.from, window.to]` (see [L002](L002-calendar-window-refresh.md)); the provider's filter and the orchestrator's DELETE must use the same window, otherwise ghost rows persist.

## Alternatives considered

### Push all events, let orchestrator filter

- Status quo before this fix.
- Wastes server round-trips and CPU.
- Allowed ghost rows when window semantics drifted between provider and orchestrator.
- Rejected.

### Add a `Strict` / `Loose` flag to `sync`

- Adds config surface for a contract that should be unconditional.
- Rejected.

## Consequences

- Provider and orchestrator must use the **same** window value. The orchestrator passes `window` to `CalProvider::sync`; no drift is possible.
- For sync windows smaller than the full calendar (e.g. `[now, now + 7 days]`), fewer events traverse the wire and through the parser.
- A unit test now asserts that `CalProvider::sync` returns only events inside the window.
- The fix is one of the "contract"-category items from the caveman review — see [R-series](README.md#refactoring-patterns-r-series).

## Cross-references

- The window-refresh DELETE that operates on the same window: [L002](L002-calendar-window-refresh.md).
- The orchestrator that drives the sync: [L009](L009-best-effort-sync.md).
- The pull model that makes this contract possible: [L004](L004-timeline-provider-pull-only.md).