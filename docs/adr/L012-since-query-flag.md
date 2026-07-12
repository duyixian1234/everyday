# ADR L012: `--since` flag in query path (date + relative duration)

**Status:** Accepted
**Date:** 2026-07-11

## Context

`timeline <preset>` queries support `today` / `yesterday` / `week` / `month`. The original CLI surface advertised a `--since <date>` flag for ad-hoc queries, but the implementation only honored it in the **sync** path. Querying with `--since` silently fell back to the preset window.

This was caught end-to-end during v0.5.0 testing: agents reading the help text expected `--since` to filter the query result; it didn't.

A second concern: durations like "last 30 minutes" or "last 2 hours" are common ad-hoc queries. A date-only `--since` is too coarse for those.

## Decision

**`--since` is honored in the query path. It accepts two formats:**

1. **Date**: `YYYY-MM-DD` (interpreted as local-day start, `00:00:00`).
2. **Relative duration**: `Nm` / `Nh` / `Nd` where `N` is a positive integer.

| Format | Example | Meaning |
|--------|---------|---------|
| `YYYY-MM-DD` | `--since 2026-07-01` | since local midnight on 2026-07-01 |
| `Nm` | `--since 30m` | last 30 minutes (sub-day precision) |
| `Nh` | `--since 2h` | last 2 hours |
| `Nd` | `--since 7d` | last 7 days |

The parser (`parse_since_utc`) returns the UTC `DateTime<Utc>`; the query then uses `[since, now]` as the effective window.

If `--since` is combined with a preset, the preset is ignored in favor of the explicit window. This is the principle established by [L013](L013-from-explicit-error.md): explicit flags override presets.

## Alternatives considered

### Only support `--since` in sync, never in query

- Status quo before this fix.
- Help text was lying.
- Rejected: explicit CLI surface must work.

### Support both `--since` and `--until` (independent)

- `--until` adds a second dimension and a second parsing failure mode.
- Considered: simpler to add `--from` / `--to` instead — see [L013](L013-from-explicit-error.md).
- `--since` covers the common case; `--from` / `--to` covers the full-range case. Both can coexist.

### Always require ISO 8601 with timezone

- More precise but harder for users.
- The two formats above cover both human typing (`30m`) and date entry (`2026-07-01`).
- Rejected: ISO 8601 only would lose the relative-duration ergonomics.

## Consequences

- `--since` in `timeline today --since 30m` returns events from the last 30 minutes instead of today's window.
- The parser is a small dedicated function with 4 unit tests covering each format and the empty/invalid cases.
- Sub-day precision is preserved (30m doesn't snap to the day boundary).
- The query path now has two flags that override presets: `--since` and `--from` / `--to` (see [L013](L013-from-explicit-error.md)). The query argument resolution must handle their interaction; today, `--from` / `--to` takes precedence if both are given.

## Cross-references

- The query/sync separation principle this lives in: [L005](L005-no-auto-sync.md).
- The companion `--from` / `--to` explicit handling: [L013](L013-from-explicit-error.md).
- The clap subcommand tree that exposes the flag: [F007](F007-clap-subcommand-tree.md).