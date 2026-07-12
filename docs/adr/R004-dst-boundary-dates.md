# ADR R004: DST-boundary date parsing — use `.earliest()` / `.latest()`, never `.unwrap()`

**Status:** Accepted
**Date:** 2026-07-11

## Context

`chrono`'s local-time conversion has two well-known edge cases:

1. **Spring-forward gap.** A local time like `2026-03-08T02:30:00` in `America/New_York` does not exist — clocks jump from 02:00 to 03:00.
2. **Fall-back overlap.** A local time like `2026-11-01T01:30:00` in `America/New_York` exists twice — once at UTC-4, once at UTC-5.

The naive code:

```rust
let dt: DateTime<Local> = Local.from_local_datetime(&ndt).unwrap();
```

panics on both. The codebase had three call sites that did this:

- `src/modules/timeline.rs` (twice)
- `src/modules/timeline/providers.rs` (twice)

All four were panic-on-DST-boundary bugs. They triggered for users in DST-observing timezones (most of North America, most of Europe) once or twice a year.

## Decision

**Every `Local.from_local_datetime(...)` call must be followed by `.earliest()` (spring-forward gap) or `.latest()` (fall-back overlap), never `.unwrap()`.**

The contract:

```rust
// Spring-forward gap: prefer the earliest valid moment (e.g. 03:00 EST becomes 03:00 EDT's UTC equivalent)
let dt = Local.from_local_datetime(&ndt).earliest();
// or
// Fall-back overlap: prefer the latest valid moment
let dt = Local.from_local_datetime(&ndt).latest();
```

`.earliest()` and `.latest()` return `Option<DateTime<Local>>`. If both are `None` (shouldn't happen but defensive), the caller propagates the `None` upstream; the orchestrator marks the event as skipped and continues.

The exact choice (earliest vs latest) depends on the semantic:

- For "user-typed meeting time", prefer `earliest` (the first occurrence is more likely to be the user's intent in spring-forward; the latter in fall-back is symmetric — both are defensible, both are documented).
- For "system-derived timestamp", prefer whatever chrono considers canonical.

In this codebase all four call sites use `earliest()` because the times come from user inputs (event start times, query bounds).

The fix is one of the "panic"-category items from the 2026-07-11/12 review — see commit `18c3840`.

## Alternatives considered

### Catch the panic from `unwrap()`

- The project rule forbids `unwrap()` in non-test code — see `agents.md`.
- Even with a `catch_unwind`, the value is lost.
- Rejected.

### Reject any local time that lands in a DST gap with an explicit error

- Strict; surfaces the issue to the user.
- Considered: but most production paths are UTC (Timeline events are stored UTC — see [L006](L006-utc-storage-local-query.md)). The DST gap only matters when converting from a local-time user input.
- Where it matters (query bounds, event times), `earliest()` is acceptable; the user can refine.
- Not rejected outright — could be added per-call if a future use case needs it.

### Store everything in UTC (already true) and skip local conversion

- The UTC storage is in place. The bug was in the local-time → UTC conversion at the query boundary.
- The conversion is unavoidable: users think in local time, the API must accept it.

## Consequences

- DST-boundary dates no longer panic. The four call sites are now panic-free.
- A user typing `2026-03-08T02:30` in `America/New_York` gets `2026-03-08T03:30 EDT` (earliest valid moment).
- The choice of `earliest` over `latest` is documented and consistent across call sites.
- A test asserts that `from_local_datetime(...).earliest()` produces a valid `DateTime<Local>` for known DST-gap inputs.
- The `.unwrap()` antipattern is now absent from this codebase's date parsing.

## Cross-references

- The UTC storage that makes most paths DST-safe: [L006](L006-utc-storage-local-query.md).
- The full-pool / cal-window path that converts local time at query time: [C002](C002-full-pull-local-filter.md), [L005](L005-no-auto-sync.md).
- The companion fix for `PoolGuard::Drop` panic: [R003](R003-pool-guard-drop.md).