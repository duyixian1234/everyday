# ADR L003: Account as first-class nullable schema column

**Status:** Accepted
**Date:** 2026-07-11

## Context

Everyday supports multiple accounts per module (e.g. `work` and `personal` mail). Timeline aggregates events across accounts. The `source` field (`mail` / `cal` / `rss` / ...) identifies the module but not the account.

Three options for representing account identity in Timeline events:

1. **Account in metadata JSON.** `source = "mail"`, `metadata = {"account": "work"}`. Filtering requires `json_extract`.
2. **Account encoded in source.** `source = "mail:work"`. Prefix matching for filtering.
3. **Account as first-class column.** `source` stays a flat enum, `account` is a separate nullable column.

A concrete problem: `ref_id` (e.g. IMAP UID) is unique per-account but may collide across accounts. Work's UID 100 and personal's UID 100 are different emails.

## Decision

**Account is a first-class nullable column in the `events` table, included in the natural key.**

```sql
account TEXT  -- NULL for rss (no account concept)
```

Natural key: `(source, COALESCE(account, ''), ref_id, event_type, timestamp)`.

- `--source mail,todo` filters by enum (`WHERE source IN (...)`).
- `--account work` filters independently (`WHERE account = 'work'`).
- RSS has no account concept; `account` is NULL. `COALESCE(account, '')` in the unique index ensures NULLs participate in dedup correctly (SQLite treats multiple NULLs as distinct in UNIQUE constraints).

## Alternatives considered

### Account in metadata JSON

- SQLite `json_extract` filtering is slower and less ergonomic than a column.
- `ref_id` cross-account collision still requires account in the natural key — if it's in the key, it should be a column, not buried in JSON.

### Source-encoded account (`mail:work`)

- `source` is no longer a stable enum; it's a composite string.
- `--source mail` requires prefix matching (`WHERE source LIKE 'mail%'`), breaking the clean `IN (...)` filter.
- Inconsistent with the rest of Everyday's architecture where `source` / `module` is always a flat enum.

## Consequences

- Schema has one extra column. Local single-account modules (todo / note / bookmark default `personal`) fill it with the configured account name.
- RSS provider writes NULL for account; queries that don't filter on account naturally include RSS events.
- The `--account` global flag is reused for timeline queries with a different semantic ("filter display" vs "select operation account"). This is acceptable because timeline doesn't execute account operations — `--account work` in timeline context unambiguously means "show work account events".

## Cross-references

- The natural key this column is part of: [L001](L001-append-only-event-log.md).
- The pull model that fills `account` per provider: [L004](L004-timeline-provider-pull-only.md).
- Multi-account semantics across modules: [F002](F002-multi-account-keyring.md).