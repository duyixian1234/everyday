# ADR S004: Execution model — concurrent, best-effort, cap/limit, exit codes

**Status:** Accepted
**Date:** 2026-07-12

## Context
The aggregator fans out a query to multiple modules concurrently. We must define concurrency, failure handling, result capping, and process exit semantics.

## Decision
`SearchRegistry::query`:

1. **Filter** target modules by `--module` (default: all registered `Searchable` modules).
2. **Fan out concurrently** via `futures::join_all` over `Searchable::search`.
3. **Best-effort failure handling** (mirrors [L009](L009-best-effort-sync.md)): if a module's `search` returns `Err`, capture it into a `warnings` list and continue; return successful hits. **Only if every module fails** does `query` return `AgentError`.
   - Warnings surface per [R001](R001-thread-local-json-mode.md): `--json` → structured stderr `{"_warning": ...}`; text → `eprintln!`.
4. **Cap & limit:**
   - Per-module internal cap = **50** (bounds work per source).
   - Global `limit` default = **20**; applied after merge + group + sort.
5. **Exit code:** empty result → **exit 0** (consistent with `timeline` empty results), so agents do not mistake "no matches" for an error.

## Alternatives considered
- **Fail-whole (rejected):** any module error aborts the entire search with non-zero exit. A single source outage would blank an otherwise useful result.
- **No per-module cap (rejected):** one verbose module could dominate the merged list before the global limit.

## Consequences
- Partial source outages are visible as warnings, not failures.
- Result volume is bounded and predictable for agents.
