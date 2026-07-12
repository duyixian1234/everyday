# ADR R008: Use SQL `GLOB`, not `LIKE`, for token-boundary flag matching

**Status:** Accepted
**Date:** 2026-07-11

## Context

The mail cache stores IMAP flags as a space-separated string (`\Seen \Answered \Flagged`). Code that checks for a specific flag used `LIKE`:

```sql
WHERE flags LIKE '%\Seen%'
```

The intent is "the envelope has the `\Seen` flag set". The bug:

- `LIKE '%\Seen%'` matches `\Seen`, `\SeenSomething`, and `\MySeen`.
- More precisely: it matches anywhere the substring `\Seen` appears — including in flag names that merely contain it.

For the actual IMAP flag namespace this is unlikely to bite (flag names are mostly single words), but the pattern is fragile. Any future flag like `\SeenReply` would silently match.

## Decision

**Use SQL `GLOB` for token-boundary flag matching.**

```sql
WHERE ' ' || flags || ' ' GLOB '* \Seen *'
```

The pattern prepends and appends a space to the flag string, then matches `* \Seen *` against the result. Since `GLOB` is anchored by `*`, this matches only when `\Seen` is surrounded by spaces (or string boundaries — which the prepended/appended spaces provide).

`GLOB` is SQLite's pattern-matching operator with stricter semantics than `LIKE`:
- `*` matches any sequence (like `LIKE '%'`).
- `?` matches one character.
- The match is case-sensitive (LIKE is case-insensitive by default for ASCII).
- No SQL injection risk because the pattern is hard-coded in Rust code, not user-supplied.

The fix is one of the "nit"-category items from the 2026-07-11/12 review — see commit `3d77bb5`.

## Alternatives considered

### Store flags as a separate join table

- Most correct.
- One row per envelope-flags pair; flags become first-class rows.
- Schema migration for existing caches; more code; more JOINs.
- Rejected: the flag list is small and rarely queried.

### Store flags as JSON and use `json_each`

- Allows set-style queries.
- SQLite's JSON functions are fast enough but the API is heavier than needed.
- Rejected: same complexity cost as the join table.

### Continue with `LIKE` and accept the substring-match risk

- Status quo.
- Rejected: the fix is one line; no reason not to.

### Add a `flags_set` virtual column computed via `REPLACE(flags, ' ', ',')`

- Overkill for this scope.

## Consequences

- Flag matching is now token-boundary: `\Seen` matches only when it's a complete flag.
- Future flag names like `\SeenReply` are correctly excluded.
- The query cost is identical to `LIKE` (both scan the column).
- The pattern is documented in a helper (`fn has_flag_sql(flag: &str) -> String`) so new queries don't have to re-derive it.

## Cross-references

- The envelope cache being queried: [M003](M003-envelope-cache.md).
- The flags snapshot semantics that motivate this query: [M005](M005-staleness-auto-sync.md).
- The companion argument-parsing discipline that pairs with strict SQL matching: [R005](R005-parse-simple-args.md).