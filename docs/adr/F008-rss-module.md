# ADR F008: RSS module — feed-rs based subscription aggregator

**Status:** Accepted
**Date:** 2026-07-09

## Context

RSS was the first "external integration" module added after mail and calendar. It also pre-dates most of the abstractions that arrived later (note/todo/bookmark). Three design questions had to be answered:

1. **Library.** Parse Atom and RSS 2.0 (and the long tail of RSS dialects) without hand-rolling a parser.
2. **State.** A subscription list is user state — it has to persist across runs and survive config edits.
3. **Failure handling.** Feeds die, redirect, rate-limit, and return malformed XML. The CLI must degrade gracefully rather than fail the entire `digest` if one feed is broken.

## Decision

### Library: `feed-rs`

- Crate: `feed-rs` (a fast, format-agnostic parser covering RSS 0.9x, 1.0, 2.0, Atom 0.3, 1.0).
- No dependency on `reqwest`'s higher-level features; the module owns its fetch + parse pipeline.

### Actions

```
everyday rss follow   <feed_url> [--tags t1,t2]
everyday rss unfollow <feed_url>
everyday rss list     [--tag T]
everyday rss digest   [--since 7d] [--limit 50]
everyday rss fetch    <feed_url>     # raw fetch + parse, no subscription state
```

- `follow` / `unfollow` / `list` operate on the subscription list.
- `digest` produces a unified table across all followed feeds; the only network call is the per-feed fetch.
- `fetch` is a stateless debug aid: parse one URL, print entries, exit. Useful for diagnosing a broken feed without polluting the subscription list.

### Subscription state

- Stored in `~/.config/everyday/rss.db` (single table: `subscriptions(url PRIMARY KEY, title, tags, added_at, last_fetched_at, last_status)`).
- Tags are stored as a JSON array; filtering by tag uses SQLite's JSON path or a simple `LIKE` match (acceptable for tens to hundreds of feeds).

### Best-effort fetch

- A single broken feed (timeout, 5xx, malformed XML) **does not abort** the entire `digest`.
- Per-feed failures are reported in the output; successful feeds still render.
- See [L009](L009-best-effort-sync.md) for the same pattern in Timeline.

### `--json` position semantics

- The original implementation allowed `--json` to appear anywhere on the command line. This was fragile: clap's `trailing_var_arg` swallowed later flags. Fixed early.
- The fix lives in the data-driven clap tree now: see [F007](F007-clap-subcommand-tree.md).

## Alternatives considered

### Hand-roll Atom / RSS parsing

- Rejected: RSS dialects multiply, and a parser bug becomes a security issue.

### Use `rss` crate (Atom-only cousin)

- Rejected: `feed-rs` covers both protocols and the long tail; using two crates would be silly.

### Subscribe via OPML import

- Considered: OPML import is a nice-to-have, not a core capability.
- Deferred: future work, not blocking v0.1.

### Stream the digest

- For very large digests, streaming avoids materializing everything in memory.
- Deferred: digests are bounded by `--limit`; default 50 entries per feed × handful of feeds stays small.

## Consequences

- The subscription DB is one more local store to manage, but it stays simple (one table).
- The best-effort fetch pattern means `digest` is reliable even when the user's feed list has stale entries.
- The `fetch` action is a useful debug primitive without polluting state.
- The `--json` semantics for this module established a project-wide rule later enforced by [R005](R005-parse-simple-args.md).

## Cross-references

- RSS feeds are a Timeline source: [L004](L004-timeline-provider-pull-only.md).
- The best-effort execution model it shares with Timeline: [L009](L009-best-effort-sync.md).