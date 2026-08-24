# ADR M006: `mail search --cached` — local envelope-cache search path

**Status:** Accepted
**Date:** 2026-08-21

## Context

`mail search --query Q` currently runs IMAP `SEARCH TEXT "Q"` over each folder directly
([M005](M005-staleness-auto-sync.md) §3). It does **not** consult the local envelope cache.
This is asymmetric with `mail list`, which is cache-first with staleness-driven auto-sync.

Meanwhile the cross-module `everyday search` already answers mail queries from the local
`mail_cache.db` via `MailSearchProvider` ([S007](S007-mail-search-local-cache.md)), using the
token-OR case-insensitive GLOB semantics of [S003](S003-query-semantics.md) / [R008](R008-sql-glob-not-like.md).

So today an agent asking "search my mail" gets a fast local answer from `everyday search`,
but `everyday mail search` forces a slow network round-trip to the IMAP server — even when the
query only needs subject/from, which the cache already covers. M005 explicitly deferred a local
escape hatch: *"Future escape hatch: `mail search --cached` for a local-only `LIKE` path — out of
scope for this ADR."* This ADR closes that gap.

## Decision

Add a `--cached` flag to `mail search`. Semantics:

1. **`--cached` is explicit opt-in.** The default `mail search --query Q` is unchanged — it still
   runs IMAP `SEARCH TEXT` so the full-fidelity search (subject + body + headers) is always one
   command away. `--cached` selects the local fast path.

2. **Local query over the envelope cache.** Reuse the existing `search_envelopes` path
   (`subject` / `from_addr` / `to_addr`, per-token OR, case-insensitive GLOB, metacharacter tokens
   skipped) — the exact semantics the cross-module `MailSearchProvider` already uses
   ([S007](S007-mail-search-local-cache.md) / [S003](S003-query-semantics.md) / [R008](R008-sql-glob-not-like.md)).
   This keeps one shared search implementation and one documented semantic, rather than a second
   divergent local matcher.

3. **Staleness-driven auto-sync, mirroring `mail list`.** When the target folder(s) are stale
   (> 15 minutes per [M005](M005-staleness-auto-sync.md)), `--cached` triggers one sync round first,
   then searches local. `--sync` forces a sync regardless. So `--cached` is *not* a strict
   zero-network promise on a cold/stale cache — it is the same cache-first model as `list`.

4. **Recognized recall divergence, documented.** The local path covers subject/from/to only. It
   cannot match body or arbitrary headers that IMAP `SEARCH TEXT` covers. This is the same accepted
   divergence already recorded in [S003](S003-query-semantics.md) and [M005](M005-staleness-auto-sync.md).
   Agents needing full-fidelity recall must use the default (IMAP) path.

5. **Typed output for `--cached`.** The local path renders with the F012 P6 typed records
   (`uid` numeric, `unread` boolean), matching `mail list`'s typed rendering. The default IMAP path
   keeps its historical plain-string contract unchanged (`render_search`). Rationale: the local path
   is list-like and agent-facing; typed values are easier for agents to consume, and the default
   path's legacy contract is preserved for compatibility.

6. **Scoping flags.** `--account`, `--folder`, and `--limit` behave consistently with `mail list`.

## Alternatives considered

### `--cached` as default with live fallback
Mirrors `rss digest`'s cache-first + fallback-to-live model. Rejected: it would make the *default*
`mail search` silently lose body recall unless the user knew to add a `--live` flag. The asymmetry
must be explicit, not implicit.

### Make local search the only path (drop IMAP search)
Rejected: would permanently remove body/header search. Breaking and a silent capability loss.

### A second bespoke local matcher
Rejected: would create two divergent search semantics to document and maintain. Reusing
`search_envelopes` keeps the single source of truth.

## Consequences

- `mail search --cached` is now a fast local answer for subject/from queries, consistent with
  `mail list` and cross-module `search`.
- The `mail search` / `mail list` asymmetry narrows: both have a cache-first path, and both share
  the staleness model.
- Default `mail search` (IMAP) is unchanged, preserving full-fidelity body/header recall and the
  legacy JSON/plain-string contract.
- CLI help, `docs/commands*`, the skill reference, and the CLI contract must note the new `--cached`
  flag. `mail` action set is unchanged (additive flag only — non-breaking).
- The cache may serve results that are up to one staleness window behind server state for `flags`
  and envelope membership; documented trade-off carried over from [M005](M005-staleness-auto-sync.md).

## Cross-references

- Local cache storage + query helpers: [M003](M003-envelope-cache.md), `email_cache.rs`.
- Staleness threshold + auto-sync model: [M005](M005-staleness-auto-sync.md).
- Watermark/UIDVALIDITY sync that populates the cache: [M004](M004-uid-watermark-sync.md).
- Cross-module mail search already on the cache: [S007](S007-mail-search-local-cache.md).
- Shared query semantics: [S003](S003-query-semantics.md), [R008](R008-sql-glob-not-like.md).
- Typed records (P6): [F012](F012-architecture-deepening-phase.md).
