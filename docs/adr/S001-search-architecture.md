# ADR S001: Search architecture — Searchable trait + SearchRegistry

**Status:** Accepted
**Date:** 2026-07-12

## Context
Phase 11 introduces cross-module unified search (`everyday search`), letting an AI agent issue one query across all integrated modules. The aggregator must fan out a query to every participating module and merge the results into a single normalized list. Two architectures were considered:

1. **Reuse the existing `Executor` dispatch** — each module exposes a `search` action returning JSON hits; the aggregator calls `ModuleRegistry::dispatch(module, search_cmd)` and normalizes the JSON output.
2. **Add a dedicated `Searchable` trait** alongside `Executor`.

The project already has a stable `Executor` + `ModuleRegistry` dispatch model ([F001](F001-cli-shape.md), [F007](F007-clap-subcommand-tree.md)), but `search` is a distinct *capability* with its own typed contract (`Hit`), not a generic action.

## Decision
Add `src/search.rs` containing:

- `trait Searchable: Send + Sync` with `fn module_name(&self) -> &'static str` and `async fn search(&self, q: &SearchQuery, cfg: &Config) -> Result<Vec<Hit>, AgentError>`. Use the **native `async fn in trait`** feature (stable in Rust 2024 edition) — do **not** add the `async_trait` crate.
- `struct SearchRegistry { providers: Vec<Arc<dyn Searchable>> }` with `register` and `query`.
- Modules implement **both** `Executor` (existing) and `Searchable` (new). At bootstrap, the same module `Arc` structs are registered into both `ModuleRegistry` and `SearchRegistry`.

The aggregator (`SearchRegistry::query`) is independent of `Executor`; it never goes through the generic action path.

## Alternatives considered
- **Pure `Executor`-dispatch (rejected):** no new trait, minimal churn, but offers no compile-time guarantee that a module supports search, and forces search semantics through the generic `Args → Output` path, diluting the typed `Hit` contract.
- **`Searchable` as a supertrait of `Executor` (rejected):** would force *every* module to implement search, including `config` and (until v1.1) `mail`, violating the incremental v1 scope.

## Consequences
- Each searchable module gains a `Searchable` impl (accepted churn; see [S005](S005-time-semantics-scope.md) for v1 scope).
- `mail` does **not** implement `Searchable` in v1.1; the registry simply omits it until then.
- Search logic is fully decoupled from the CLI action dispatch, easing future additions of non-module search sources.
