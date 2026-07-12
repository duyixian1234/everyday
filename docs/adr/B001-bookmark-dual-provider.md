# ADR B001: Bookmark module — local SQLite (default) + Notion (with exact-match tag filter)

**Status:** Accepted
**Date:** 2026-07-10

## Context

The `bookmark` module was the last of the three Notion-backed modules to be designed. It followed `note` / `todo` but added one wrinkle: **tag filtering**. Users want to list bookmarks by tag (`bookmark list --tag rust`). Notion's tag property is a multi-select; a SQL `LIKE` over a comma-separated string doesn't match tags cleanly. Local SQLite, by contrast, can express tag membership as a join.

The two providers also had to coexist per-account — the same `add_dual_providers!` pattern used by `note` and `todo` (see [R011](R011-add-dual-providers-macro.md)).

## Decision

### Actions

```
everyday bookmark login                                       # notion only — no-op for local
everyday bookmark init-db                                     # notion: create the "Bookmarks" database
everyday bookmark add    --url U --title T [--tags t1,t2]
everyday bookmark list   [--tag T]
```

### Local provider (default per [F005](F005-default-provider-local.md))

- Two tables:
  - `bookmarks(id INTEGER PK, url TEXT NOT NULL, title TEXT NOT NULL, created_at TEXT NOT NULL)`
  - `bookmark_tags(bookmark_id INTEGER FK, tag TEXT NOT NULL, PRIMARY KEY (bookmark_id, tag))`
- `list --tag T` becomes `SELECT b.* FROM bookmarks b JOIN bookmark_tags t ON t.bookmark_id = b.id WHERE t.tag = ? ORDER BY b.created_at DESC` — **exact-match** tag filter, no fuzzy substring search.
- The schema is normalized so adding a tag filter does not require `LIKE '%rust%'` over a JSON column.

### Notion provider

- `init-db` creates a database "Bookmarks" with `Title` (title), `URL` (url), `Tags` (multi_select).
- `add` creates a page; tags passed as `--tags rust,cli` are split on commas and written to `Tags`.
- `list --tag T` filters via the multi_select `equals` query.
- No fuzzy matching — same exact-match semantic as local.

### Account record

- `[[bookmark.accounts]]` in `config.toml`, with optional `provider = "local"` (default) or `provider = "notion"`.
- Keyring: `service = "everyday/bookmark/<account>"`, account = `token` (Notion only).
- `default_database_id` is written back to config after `init-db` runs successfully.

### Build providers

- Uses the same `add_dual_providers!` macro as `note` and `todo` (see [R011](R011-add-dual-providers-macro.md)).

## Alternatives considered

### Tags stored as a comma-separated string in a single column

- Cheap to write.
- `list --tag rust` becomes `WHERE tags LIKE '%rust%'` — matches `rusty` and `rust-cli` too. Wrong.
- Could use `LIKE '%,rust,%'` after adding sentinels, but the query is fragile and order-dependent.
- Rejected.

### Single `tags TEXT` column with FTS5 virtual table for search

- More capability, more complexity.
- Not needed for the current `--tag T` exact-match use case.
- Rejected.

### Local provider without the join table (store tags as JSON)

- Modern SQLite has JSON path queries; the schema stays one table.
- Loses indexability of `tag = ?` — full table scan per filter.
- Rejected.

### Drop the Notion provider; keep local only

- Loses the cross-device sync option.
- Conflicts with the dual-provider pattern established for `note` and `todo`.
- Rejected.

## Consequences

- `--tag T` is **exact-match** on both providers — the user knows that `T=rust` won't match `rust-cli`.
- The two-table local schema is small and indexes cleanly; per-row overhead is one extra join.
- The macro-level `add_dual_providers!` keeps the provider construction uniform across `note`, `todo`, `bookmark`.
- A future enhancement could add `bookmark search --query Q` (FTS5 over `title` + `url`); out of scope today.
- The Notion provider inherits all of [F004](F004-shared-notion-client.md)'s 429 backoff and error semantics.

## Cross-references

- The local default for new accounts: [F005](F005-default-provider-local.md).
- The shared Notion client: [F004](F004-shared-notion-client.md).
- The dual-provider build macro: [R011](R011-add-dual-providers-macro.md).
- The merged `NotionLocalAccount`: [R010](R010-notion-local-account.md).
- The shared Notion abstractions consolidated into `local`: [R009](R009-notion-common-local-module.md).
- Timeline's notion-source projection (via ops-log): [L007](L007-notion-ops-log.md).