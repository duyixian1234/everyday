# ADR S007: Mail search via local envelope cache (v1.1)

**Status:** Accepted
**Date:** 2026-07-14

## Context

ADR [S005](S005-time-semantics-scope.md) deferred `mail` search to v1.1
pending an IMAP `SEARCH` decision. Two paths exist:

1. **Live IMAP `SEARCH`** on every query.
2. **Local envelope cache** (`mail_cache.db`) already populated by
   `everyday mail sync`.

We need to pick a path that respects the search module's
local-first / best-effort contract ([S004](S004-execution-model.md))
and the cold-start budget ([F009](F009-performance-budget.md)).

## Decision

Mail search **scans `mail_cache.db` locally** — the same envelope
cache written by `everyday mail sync` (see [M003](M003-envelope-cache.md),
[M004](M004-uid-watermark-sync.md), [M005](M005-staleness-auto-sync.md)).
It does **not** issue a live IMAP `SEARCH` per query.

- **Query shape:** free-text tokens OR'd across
  `subject | from_addr | to_addr`, case-insensitive GLOB substring
  (see [S003](S003-query-semantics.md), [R008](R008-sql-glob-not-like.md)).
  Metacharacter tokens (`* ? [ ]`) are skipped to avoid injection.
- **Empty input:** empty query / empty token set → zero hits, exit 0
  (consistent with the aggregator's best-effort behavior).
- **Cache miss or stale cache:** the search returns fewer or zero hits.
  Mail search **never auto-syncs**; freshness is the caller's
  responsibility (`everyday mail sync` or `mail list`).
- **Result shape:** each `Hit` carries
  `id = {account}:{folder}:{uid}` so an agent can resolve the message
  through `everyday mail read --account <a> --folder <f> <uid>`. `ts`
  is the message date (per-module primary time per [S005](S005-time-semantics-scope.md)).
- **Registration:** the `mail` module exposes a single global
  `MailSearchProvider` registered by `SearchModule::build_registry`
  when `config.mail.accounts` is non-empty. Like `rss` and `cal`, the
  provider scans all accounts/folders in one shot; the aggregator's
  merge + global `--limit` cap still apply.

## Alternatives considered

- **Live IMAP `SEARCH` per query (rejected):** violates the
  best-effort / cold-start budget ([S004](S004-execution-model.md),
  [F009](F009-performance-budget.md)). Adds a network round trip per
  query; partial fan-out failures across providers would also have to
  absorb IMAP errors as best-effort warnings, which is muddy.
- **Hybrid: cache first, IMAP fallback on miss (rejected):** silent
  fallback hides sync bugs (consistent with the explicit-error
  direction set in [L013](L013-from-explicit-error.md)). Stale caches
  are the caller's signal to run `mail sync`.
- **Account-scoped provider instances (rejected):** redundant — the
  provider scans the cache table, which is keyed by account.
  Registering N providers would just complicate the registry without
  changing scan cost.

## Consequences

- Mail search depends on `mail sync` having run at least once. Cold
  start (no cache yet) returns zero hits.
- Per-account fan-out collapses into one provider; per-module cap
  (default 50, see [S004](S004-execution-model.md)) applies after the
  scan, then the global `--limit` cap truncates the merged list.
- The `mail` module joins the v1.1 searchable set alongside
  `note`, `todo`, `bookmark`, `rss`, and `cal`.
