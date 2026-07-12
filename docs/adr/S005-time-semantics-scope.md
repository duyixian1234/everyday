# ADR S005: Time semantics & module scope (v1 / v1.1, rss cache)

**Status:** Accepted
**Date:** 2026-07-12

## Context
Different modules have different notions of "the time" for an item, and not every module is in v1 scope. `rss` currently has **no local item storage** (subscriptions live in config; items are live-fetched on `digest`/`fetch`), so search needs a storage decision.

## Decision
- **Primary `ts` per module** (UTC, [L006](L006-utc-storage-local-query.md)):
  - `mail` → message date
  - `cal` → **event start time** (future events sort by when they occur)
  - `note` → last updated
  - `todo` → created / completed
  - `rss` → publish time
  - `bookmark` → added time
- **v1 scope:** `note`, `todo`, `bookmark` (local SQLite `GLOB`), `rss` (new local item cache), `cal` (full-pull `GLOB`).
- **`rss` gains a local item cache table** (SQLite), populated by `sync`/`digest`. Search queries this cache (offline-capable, consistent with the other local modules). Live-fetch-on-search was rejected (slow, rate-limit risk).
- **`mail` is deferred to v1.1** (IMAP `SEARCH`, see [S003](S003-query-semantics.md)). It does not implement `Searchable` until then.
- **Fusion (v1):** group hits by module, sort `ts desc` within the merged list. `--sort relevance` is deferred to v2 (avoiding false-precision ranking).

## Alternatives considered
- **`rss` live-fetch on search (rejected):** simple but slow and rate-limit prone.
- **`rss` deferred to v1.1 (rejected):** user wants `rss` searchable in v1 with a cache.
- **`cal` uses added/sync time (rejected):** future events would mix with history; event-start is the intuitive timeline anchor.

## Consequences
- v1 scope +1 SQLite table for `rss`.
- Future events sink in `ts desc` ordering (accepted).
- `mail` search is a clearly-scoped v1.1 follow-up.
