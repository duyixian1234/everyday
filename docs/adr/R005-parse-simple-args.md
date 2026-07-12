# ADR R005: `parse_simple_args` — single-dash tokens are values, double-dash tokens are flags

**Status:** Accepted
**Date:** 2026-07-11

## Context

Modules parse `Vec<String>` arguments via a helper called `parse_simple_args`. The original classification rule:

> A token starting with `-` is a flag; the next token is its value.

This breaks for negative numbers:

```bash
everyday todo list --limit -1
```

The helper sees `--limit` (flag), then `-1` (next token, "flag" because it starts with `-`), then returns `("limit", Some(""))` — `--limit` is set to empty string, not `-1`. The user wanted to limit to "the last 1 todo" (or similar) and got an unrelated default.

Other breakage points:

- `everyday todo add --tags -rust,-cli` — `-rust` is read as a flag, not as a tag.
- `everyday mail list --from -2h` — `-2h` is treated as a flag (or worse, mis-split).

## Decision

**A flag is a token starting with `--`. A single-dash token (e.g. `-1`, `-2h`, `-rust`) is always a value.**

Updated `parse_simple_args` rule:

1. If the previous token was a known flag expecting a value → consume this token as the value, regardless of leading dashes.
2. If this token starts with `--` → it is a flag (its value is the next token, if any, unless the flag was declared `ArgKind::Bool` — see [F007](F007-clap-subcommand-tree.md)).
3. Otherwise (single-dash or no dash) → it is a positional value (or a `ArgKind::Multi` accumulator element).

The helper is unit-tested with the cases above and several others (negative limit, negative time relative, single-letter tag).

The fix is one of the "risk"-category items from the 2026-07-11/12 review — see commit `265f902`.

## Alternatives considered

### Use clap for everything

- The original path that [F007](F007-clap-subcommand-tree.md) replaced.
- Modules still want `Vec<String>` semantics inside `execute`; clap is the dispatch layer, not the per-action parser.
- Reconsidered: clap's typed matches **are** what we want for the boundary; `parse_simple_args` is what the modules use internally after clap hands them flags + positionals.

### Detect "looks like a number" before deciding

- `"-1".parse::<i64>().is_ok()` → value.
- Brittle: `-2h` parses as `-2` then chokes.
- Rejected.

### Always require `--limit=N` (no space)

- Forces the user to type the equals sign.
- More invasive than necessary.
- Rejected.

### Forbid negative values in flags

- `--limit` is allowed negative in `--json` to mean "all"; this is a documented convention.
- Rejected: the convention exists for a reason.

## Consequences

- `everyday todo list --limit -1` now works as documented.
- `parse_simple_args` matches the user's intuition: single-dash = value, double-dash = flag.
- The helper is the canonical post-clap parser for module actions; [F007](F007-clap-subcommand-tree.md) feeds it a `Vec<String>` and the helper classifies correctly.
- New flags that expect negative values (or relative durations like `-2h`) work without special-casing.

## Cross-references

- The clap subcommand tree that hands flags to this parser: [F007](F007-clap-subcommand-tree.md).
- The thread-local JSON mode that `--json` flows through: [R001](R001-thread-local-json-mode.md).
- The query argument resolution that uses these conventions: [L012](L012-since-query-flag.md), [L013](L013-from-explicit-error.md).