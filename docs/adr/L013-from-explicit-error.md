# ADR L013: Timeline `--from` solo explicit error (resolve_query_range)

**Status:** Accepted
**Date:** 2026-07-12

## Context

The query path accepted `--from YYYY-MM-DD` and `--to YYYY-MM-DD` as independent flags, but the implementation required **both** to be present:

```rust
let from = parse(args.get("--from"), parse_from_date)?;
let to   = parse(args.get("--to"),   parse_to_date)?;
// silently fell back to preset if either was missing
```

Symptoms:

- `timeline --from 2026-07-99` (alone) → silently fell back to the preset window; the invalid date was ignored.
- `timeline --from 2026-07-12 --to 2026-07-01` (from > to) → silently produced an empty result set.
- The CLI help and v0.5.0 release notes implied both flags could be used independently. They couldn't.

This was the last "silent fallback" item from the 2026-07-11/12 review. It was fixed in v0.6.1 (commit `52f6377`).

## Decision

**Introduce `resolve_query_range(preset, from, to, since) -> Result<Range, AgentError>` and call it explicitly from `do_query`.**

Behavior:

- If neither `--from` nor `--to` is given → use the preset window.
- If only `--from` is given → `to = now()` (effectively "from this date forward").
- If only `--to` is given → `from = preset_start(now)` (the preset's start, not epoch).
- If both are given and `from > to` → `AgentError::InvalidArgument` with a clear message.
- If either date is unparseable → `AgentError::InvalidArgument` (no silent fallback).
- If `--since` is also given, it **overrides** the resolved range (sub-day precision from [L012](L012-since-query-flag.md) wins).

## Alternatives considered

### Require both `--from` and `--to`

- Forces the user to type more.
- Rejected: "from a date forward" is a common query.

### Allow `--from` alone, fall back to `now()` for `to`

- Matches the chosen behavior for `to`.
- Combined with the rest of the rule set, this is what shipped.

### Default `to` to preset's end instead of `now()`

- Surprising — preset semantics are about "yesterday" / "this week"; `--from yesterday` should reach into today.
- Rejected: `now()` matches user expectation.

## Consequences

- `timeline --from 2026-07-99` → explicit `InvalidArgument` with the date parse failure.
- `timeline --from 2026-07-12 --to 2026-07-01` → explicit `InvalidArgument` ("`from` must not be after `to`").
- `timeline --from 2026-07-01` → returns events from 2026-07-01 00:00 local to `now()` UTC.
- The query path is no longer susceptible to silent fallback. The same discipline is asserted by [F007](F007-clap-subcommand-tree.md)'s strict arg reading and [R005](R005-parse-simple-args.md)'s flag/value classification.
- The helper is unit-tested with 5 cases (no flags, only `--from`, only `--to`, both, inverted, invalid).

## Cross-references

- The query/sync separation principle: [L005](L005-no-auto-sync.md).
- The companion `--since` flag: [L012](L012-since-query-flag.md).
- The clap subcommand tree that surfaces these flags: [F007](F007-clap-subcommand-tree.md).
- The argument parser that hands them to the query path: [R005](R005-parse-simple-args.md).
- The shared thread-local JSON mode that affects how errors render: [R001](R001-thread-local-json-mode.md).