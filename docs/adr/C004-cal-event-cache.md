# ADR C004: Calendar event cache — snapshot-replace, not append-only

**Status:** Accepted
**Date:** 2026-09-18

## Context

`cal` was the only remote-heavy module without a local cache. `mail` has an
envelope cache ([M003](M003-envelope-cache.md)) and `rss` has an item cache
([S005](S005-time-semantics-scope.md)), but every `cal` call pulled CalDAV live:
`CalDavBackend::list` issued a `GetCalendarResources` per calendar, and the
cross-module search provider (`CalSearchProvider`) went to the network on every
query.

That left `search` internally inconsistent. Of its seven providers, six read
local data; **`cal` was the sole provider that could fail or stall on the
network**. `mail` had been moved to a cache-first search path precisely to fix
this class of problem ([S007](S007-mail-search-local-cache.md)).

[C002](C002-full-pull-local-filter.md) already anticipated this cache and named
the intended shape:

> **Cache events locally and filter against the cache** — Could be combined with
> the mail cache approach (M003). Considered for future work, not part of this
> ADR.

Taking C002 at its word would mean copying M003. **That is the trap this ADR
exists to avoid**, and the central decision below is to *not* copy it.

## Decision

### Snapshot replace, not append-only

Each refresh **replaces a calendar's rows wholesale** (delete-then-insert),
rather than appending and never deleting.

M003's retention rule is explicit: **K1 — append-only, never physically
delete**, with "No reconcile, no TTL, no `DELETE` in the sync flow." Mail's
ghost envelopes are therefore permanent by design.

That design is correct for mail and **wrong for calendars**, because the two
hold different kinds of state:

- A mail envelope is an **immutable fact** — a message that arrived. A stale
  ghost envelope is *noise*: it may show a message twice or show one that was
  expunged, but the underlying event still happened.
- A calendar event is **mutable state** — the same UID gets rescheduled,
  renamed, or cancelled. A stale calendar row is not noise, it is **wrong
  information**: it displays a meeting that no longer exists, at a time it no
  longer occupies.

The timeline layer had already conceded exactly this point, independently, in
[L002](L002-calendar-window-refresh.md):

> Calendar events get **rescheduled** (start time changes) and **deleted** by
> the user. … Pure append is unusable for Calendar.

So cal is the one source that must never be append-only, and a cache feeding it
must not silently reintroduce the invariant the layer above rejected.

### Consequences of snapshot replace

- **No `gc` action.** `mail gc` exists precisely because K1 *does* produce
  ghosts ([M007](M007-mail-cache-gc.md)). Replacement leaves none, so a
  `cal cache gc` would have nothing to clean. Adding one for symmetry is exactly
  the "every command must justify its existence" failure [F003](F003-module-scope-external-integration.md)
  guards against. **(Dependency to remember: if this cache ever moves to
  incremental updates via ETag or RFC 6578 sync-token, ghosts become possible
  again and a `gc` must follow.)**
- **No incremental sync token.** Replacement re-reads each calendar in full, so
  the cache does not depend on `sync-token` support. That matters because
  libdav 0.11 defines the `SYNC_COLLECTION` / `SYNC_TOKEN` name constants but
  ships **no `SyncCollection` request type**; using it would mean hand-rolling
  RFC 6578 XML over `request_raw`. Not worth it for the benefit bought here.

### Identity is the UID, not the href

The primary key is `(account, calendar_href, uid)`.

`cal add` derives the resource filename from a nanosecond timestamp
(`event_filename`), so **a re-PUT of the same event changes its href while its
UID stays stable**. Keying on href would emit a phantom row on every edit — the
same ghost problem, reintroduced through the key rather than the retention rule.

### Full mirror + freshness watermark

The cache stores **every** event the server returns, not a time window, plus a
`last_sync_at` per `(account, calendar)`.

Cal currently carries three different notions of "window": `CalListOptions`
filters *after* the fetch, the timeline orchestrator computes a
`TimeWindow`, and the network layer applies none at all (C002). Storing a
window union would be wrong: `cal list` defaults to today-and-future while the
timeline's upper bound is a lookahead, so their intersection can **exclude
events the user is actively looking at**. Mirroring the server keeps the cache's
semantics minimal and leaves filtering to each consumer.

### Staleness: 15 minutes, hardcoded

`STALENESS_THRESHOLD_SECS = 15 * 60`, **not exposed in config** — the same
discipline M005 set for mail ("15 minutes is hardcoded. Not exposed as a
config."). A never-synced account is always stale, so a fresh user gets a
refresh on first query.

Staleness is judged on the **oldest** calendar's watermark, since a refresh
covers all calendars anyway.

### Refresh is lazy; the daemon is untouched

`cal list` and the search provider refresh when stale, on demand — the same
lazy shape [M005](M005-staleness-auto-sync.md) gave mail. The daemon is **not**
given a cal step.

[F016](F016-daemon-sync-scheduler.md) makes the daemon the only role allowed to
pull periodically, but M005 already established that a *query* path may sync
lazily without violating [L005](L005-no-auto-sync.md). Adding a daemon step
would expand the blast radius from three files to six-plus
(`cycle`/`CycleResult`/`ActionResult`/state/logging/docs) to buy only a warmer
cache.

### Not synced across devices

`cal-cache.db` is excluded from the WebDAV namespace. `sync/mod.rs` states the
rule in its module docs — "Derived caches (mail_cache / rss-items / timeline)
are never synced" — and a rebuildable, device-local cache has no business
propagating stale rows to another machine. A test asserts the exclusion
alongside the existing `timeline.db` assertion.

## Alternatives considered

### Copy M003 exactly (append-only, K1 retention)
- Consistent with the mail cache; one retention model to learn.
- Rejected: it retains cancelled and rescheduled events forever. Mail's ghost is
  noise; cal's ghost is a lie. L002 already reached this conclusion for the
  timeline layer, and a cache that reintroduces the problem underneath it would
  make the layer above incoherent.

### ETag-based incremental update
- Cheaper on the wire: libdav's `GetCalendarResources` already requests
  `getetag` and parses it into `FetchedResourceContent { data, etag }`, but the
  current code reads only `.data` and discards the etag.
- Rejected **for now**: under snapshot replace the etag has no consumer, and
  adding it would be speculative surface of exactly the kind
  [K005](K005-memory-module-deprecation.md) was written about. It also does not
  compose with the unstable-href problem without the UID key landing first.
  Revisit if the full re-read ever becomes a measured cost.

### RFC 6578 sync-collection via `request_raw`
- True incremental: fetch only what changed.
- Rejected: libdav 0.11 has no `SyncCollection` request type, so this means
  hand-written XML marshalling plus sync-token lifecycle management — a class of
  protocol code this codebase has never carried, for a benefit the snapshot
  approach already delivers (an offline, consistent search path).

### Store a time window instead of a full mirror
- Smaller cache.
- Rejected: it would delete rows the user is looking at. `cal list` defaults to
  today-and-future and the timeline's bound is a lookahead; the intersection is
  not what either consumer reads.

### Add `cal cache gc` for symmetry with mail
- Consistent action surface.
- Rejected: there is nothing for it to collect. See "No `gc` action" above.

### Add a daemon refresh step
- Warmer cache, and consistent with the daemon's other pull steps.
- Rejected: costs six-plus files to pre-warm what a lazy check already handles.

## Consequences

- `search` now has **seven local-first providers** instead of six plus a
  network call, and works offline for `cal`.
- `cal list` first-call latency does **not** improve — replacement re-reads the
  full calendar set. What improves is repeat-query cost and offline
  availability. **If perceived `cal list` latency is the actual pain point, the
  answer is the ETag path above, not this ADR.**
- Search results can now be **staler than a live call** would have been; the
  staleness watermark is what lets a caller tell. Surfacing it is t4's job.
- The cache can be deleted at any time with no data loss — it is a mirror of
  server state, and the next query rebuilds it.
- `everyday health` reports the cache db's openability before the keyring
  credential, matching mail's shape, and still performs **no** network probe.
- `memory/store.rs` remains the repo's only other soft-delete-style store; this
  ADR deliberately does not imitate its missing column migration (see R024, the
  local-provider soft-delete ADR landing with Phase 27's todo work), only its
  view-encapsulation idea where relevant.

## Cross-references

- Amends: [C002](C002-full-pull-local-filter.md) — the "considered for future
  work" caching item is now implemented here.
- Constrains: [L002](L002-calendar-window-refresh.md) — the timeline's
  window-refresh semantics are why this cache may not be append-only.
- Pattern source: [M003](M003-envelope-cache.md) (schema/watermark shape) and
  [M004](M004-uid-watermark-sync.md) (rows + watermark in one transaction),
  mirrored in structure and deliberately inverted in retention.
- Contrast: [M007](M007-mail-cache-gc.md) — why mail needs `gc` and cal does not.
- Related: [M005](M005-staleness-auto-sync.md) (staleness discipline),
  [S007](S007-mail-search-local-cache.md) (cache-first search precedent),
  [F003](F003-module-scope-external-integration.md) (why no `gc` action),
  [F016](F016-daemon-sync-scheduler.md) (why the daemon is untouched).
