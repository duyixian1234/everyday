# ADR S006: Search module CLI — `query` action + flags

**Status:** Accepted
**Date:** 2026-07-12

## Context
How does a user or agent invoke unified search? The CLI is a data-driven clap tree keyed by `module_arg_spec` ([F007](F007-clap-subcommand-tree.md)), and the established shape is `everyday <module> <action> [options]` ([F001](F001-cli-shape.md)).

## Decision
- `search` is a **first-class module** (like `timeline` and `config`), with a single action `query`.
- Invocation:
  ```
  everyday search "<query>" [--module a,b,c] [--json] [--limit N] [--since 7d]
  ```
- **Reuse, do not duplicate:** the `--since` parser from [L012](L012-since-query-flag.md) and the source-filter parser from [L013](L013-from-explicit-error.md) are shared with the timeline module.
- Output via the existing `Output` enum: `Table` for text, `Json` for agents ([F001](F001-cli-shape.md)).
- Flags:
  - `--module` — comma-separated allow-list (maps to `SearchRegistry::query` filter).
  - `--since` — relative (`7d`) or date (`YYYY-MM-DD`), per [L012](L012-since-query-flag.md).
  - `--limit` — overrides the global default (20, see [S004](S004-execution-model.md)).
  - `--json` — JSON mode (thread-local, [R001](R001-thread-local-json-mode.md)).

## Alternatives considered
- **Top-level `everyday search` not modeled as a module (rejected):** breaks the clap data-driven tree consistency of [F007](F007-clap-subcommand-tree.md) and the `module → action` shape of [F001](F001-cli-shape.md).

## Consequences
- One additional entry in `module_arg_spec` and the module registry.
- Agents use the identical `--json` contract they already use for other modules.
