# ADR S003: Query semantics — tokenize OR, case-insensitive GLOB

**Status:** Accepted
**Date:** 2026-07-12

## Context
The user/agent supplies a free-text `raw` query. We must define how it is tokenized, matched, and combined across heterogeneous backends (SQLite `GLOB` for local modules, IMAP `SEARCH` for mail in v1.1).

## Decision
- **Tokenize** `raw` by whitespace into tokens.
- **Combine tokens with OR** (recall-first): a hit matches if *any* token matches. This favors search recall; an `--and` flag for precision is deferred to v2.
- **Match each token as a case-insensitive substring** using `lower(column) GLOB lower('*token*')`. The double `lower()` satisfies [R008](R008-sql-glob-not-like.md) (GLOB for token-boundary matching) while making the match case-insensitive (SQLite `GLOB` is otherwise case-sensitive).
- **Per-module mapping:**
  - `note` / `todo` / `bookmark` (local SQLite): `GLOB` over title/body/url columns.
  - `cal`: full-pull then `GLOB` over summary/description ([C002](C002-full-pull-local-filter.md)), honoring the window ([C003](C003-cal-provider-window-filter.md)).
  - `rss`: `GLOB` over cached item title/summary (new local cache, see [S005](S005-time-semantics-scope.md)).
  - `mail` (v1.1): map each token to IMAP `SEARCH TEXT token`. Note IMAP `SEARCH` combines multiple criteria with AND by default — this OR/AND divergence between local GLOB and IMAP is documented and accepted for v1.1; unification is a v2 concern.

## Alternatives considered
- **AND default (rejected):** higher precision but lower recall; search is recall-sensitive, and `--and` can be added later without breaking the OR default.
- **Phrase-only, no token split (rejected):** fails common multi-word queries.

## Consequences
- OR default may return noisier result sets; mitigated by future `--and` and `--sort relevance` ([S005](S005-time-semantics-scope.md)).
- Local modules share one GLOB helper; mail's IMAP mapping is isolated to v1.1.
