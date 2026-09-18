# ADR F020: Per-hit staleness — `Hit.stale`, not a provider-level warning

**Status:** Accepted
**Date:** 2026-09-18

## Context

Phase 27 made `cal` local-first ([C004](C004-cal-event-cache.md)): its search
provider now reads a local event cache that is refreshed lazily and may serve a
snapshot up to `STALENESS_THRESHOLD_SECS` (15 minutes) old. That closed a real
inconsistency — `cal` had been the only one of seven search providers that hit
the network on every query — but it introduced a new property: **a `cal` hit can
now be older than the live truth.**

[C004](C004-cal-event-cache.md) requires that the cache's consumers be able to
tell. Without that, an agent has no way to distinguish "the calendar has no such
event" from "the calendar cache has not caught up", and those call for different
responses — the first is an answer, the second is a reason to re-query.

The question this ADR settles is **where that signal lives**.

## Decision

### Staleness rides on the hit

`Hit` (`src/search.rs`) gains:

```rust
/// True when this hit came from a local snapshot that is known to be older
/// than its refresh threshold. `false` for hits read from the source of
/// truth, and for every provider with no cached tier.
pub stale: bool,
```

Every provider sets it. The six that read authoritative local data (note, todo,
bookmark, rss, mail, memory) always set `false`; `cal` sets it from the cache's
own freshness check. `everyday search --json` emits the field per hit.

### Why per-hit rather than the existing `_warning` channel

The repo already has a provider-level warning channel: `SearchOutcome.warnings`,
rendered as `{"_warning": "search_provider_failed", ...}`. Reusing it was the
cheaper option and was the initial proposal. It is rejected for three reasons.

1. **The warning channel is per-*failure*, and staleness is not a failure.** A
   warning says "this provider did not contribute". A stale hit *did*
   contribute a result — just an older one. Encoding staleness as a warning
   would tell a consumer to disregard a hit that may well be correct, and the
   `_warning` text (`warning: search provider 'cal' failed: …`) would be
   factually wrong.
2. **The signal is per-hit, not per-provider.** A single query fans out across
   accounts and calendars; one account's cache can be fresh while another's is
   stale. A provider-level warning cannot express that, so it would have to
   degrade to "something somewhere may be old", which is not actionable.
3. **Warnings are easy to miss and hard to parse.** They ride outside the result
   array. A consumer that reads `hits` gets no signal at all, and the whole
   point is that a consumer *of the hits* needs to know.

### `stale` is a flag, not a timestamp

A `stale: bool` rather than an `as_of: DateTime<Utc>` or an `age_secs: u64`.

The boolean is what a consumer must act on: it decides whether to re-query. A
timestamp invites the consumer to invent its own threshold — a second staleness
policy, in a consumer, disagreeing with the provider's. The provider owns the
threshold (C004 fixes it at 15 minutes, not configurable, per
[M005](M005-staleness-auto-sync.md)), so the provider should publish the verdict
rather than the inputs to a verdict a consumer would have to re-derive.

If a future consumer genuinely needs the age — for display, say — that is an
additive change to make when a consumer exists ([K005](K005-memory-module-deprecation.md)'s
lesson about speculative surface).

### `false` is the default for authoritative providers

The six non-cached providers set `stale: false` unconditionally, and they are
not being inaccurate: their data **is** the source of truth (note/todo/bookmark
read their own SQLite DBs; mail and rss read caches that the module itself
treats as authoritative for search; memory reads the current-state view). Only
`cal` has a cache whose accuracy is explicitly time-bounded, because only `cal`
has a remote source of truth it is mirroring.

This distinction — "local DB that *is* the data" versus "local cache of remote
data" — is the same one that decides what WebDAV syncs
([D001](D001-webdav-file-sync.md)) and what `CachedCalBackend` wraps.

## Alternatives considered

### Reuse the `_warning` channel
- No contract change; the channel already exists.
- Rejected: see above — wrong granularity, and it mislabels a success as a
  failure.

### `as_of` / `age_secs` on the hit
- More information; a consumer could render "15 minutes old".
- Rejected: no consumer needs it today, and it invites a second, consumer-side
  staleness policy. Additive later if a real consumer appears.

### A module-level `stale` map in the response envelope
- Keeps `Hit` unchanged; one flag per module.
- Rejected: still cannot express per-account/per-calendar freshness, and it is
  a second shape to learn for one provider's benefit.

### Do nothing; document the 15-minute bound
- Zero code.
- Rejected: an agent that cannot tell "no such event" from "cache is behind"
  will make the wrong call, and the failure is silent. `cal` is the module
  agents most often query for *imminent* events, where a 15-minute lag matters
  most.

## Consequences

- `everyday search --json` gains a `stale` key on every hit. This is an
  **additive** contract change: no field is removed or renamed, so a consumer
  that ignores unknown keys is unaffected. `docs/commands*.md` and the skill
  reference document it.
- `Hit` has seven construction sites (one per provider) plus the test helper;
  all are updated in the same change, so `stale` cannot be forgotten silently —
  the struct has no `Default`, so a missing field is a compile error. That is
  deliberate: a `#[derive(Default)]` here would let a new provider ship with a
  wrong staleness answer that no test catches.
- Staleness describes the **snapshot**, not the hit's own age: an event that
  started an hour ago is not "stale" if the cache that produced it is fresh.
  The two are independent, and this ADR only claims the former.
- The flag says nothing about the *other* reason a cal result may differ from a
  live query — a refresh that failed and fell back to the old snapshot. That
  fallback is what makes the result old, so the same flag covers it: a failed
  refresh leaves a stale cache, and the cache reports stale.
- Future providers with a cached tier set `stale` from their own freshness
  check rather than inventing a new mechanism.

## Cross-references

- Implements the surfacing requirement of [C004](C004-cal-event-cache.md) §C-6.
- Related: [S002](S002-hit-normalization.md) — the `Hit` shape this extends;
  [S005](S005-time-semantics-scope.md) — `ts` is the entity's own time, which
  this ADR deliberately does **not** conflate with snapshot freshness.
- Related: [M005](M005-staleness-auto-sync.md) — the staleness threshold
  discipline `cal` inherits.
- Related: [K005](K005-memory-module-deprecation.md) — why no speculative
  timestamp field.
