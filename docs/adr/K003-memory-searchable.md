# ADR K003: Memory participates in cross-module `Searchable` trait (current-state GLOB)

**Status:** Accepted
**Date:** 2026-07-14

## Context

The unified `everyday search` aggregator (introduced in [S001](S001-search-architecture.md), with mail adapter added in [S007](S007-mail-search-local-cache.md)) currently includes six of the project's eight modules: `note`, `todo`, `bookmark`, `rss`, `cal`, `mail`. The remaining modules (`config`, and the proposed `memory`) raise the question of which must participate.

For `memory` specifically, the user explicitly deferred semantic / embedding-based search to a future extension. But the **text-search** form of search — GLOB on string fields — is cheap and consistent with other modules. The question is whether to implement `Searchable` for memory at all in v1.

Two positions:

- **Don't implement.** Memory is structured (s, p, o). Agents should query it precisely (`memory get <SUBJECT>` or `memory relation <S> <P>`), not via fuzzy text search. Adding memory to `everyday search` would dilute hit relevance with structured facts that don't share the "prose snippet" semantic other modules provide.
- **Do implement.** Consistency with other modules. Agents issuing `everyday search "rust"` expect memory facts about rust to be included. The implementation is small (~10 lines: GLOB on subject/predicate/object of current state).

The cost of implementing is low; the cost of not implementing (inconsistent aggregator behavior) is higher. Adopt the implement path.

## Decision

Implement `Searchable` for `memory` in v1, with the following contract:

| Aspect | Decision | Rationale |
|---|---|---|
| Adapter location | `src/search.rs` alongside other adapters | Single-file consistency with existing adapters |
| Match scope | Current state only (`MAX(created_at) WHERE deleted_at IS NULL`) | Mirrors `memory get` semantics; deleted facts excluded |
| Match fields | `subject OR predicate OR object` (three fields, OR-merged per token) | Triple is the natural search unit |
| Match semantics | GLOB per [S003](S003-query-semantics.md) + [R008](R008-sql-glob-not-like.md) | Same convention as other modules |
| Token splitting | Whitespace; OR across tokens; case-insensitive GLOB | Reuse [S003](S003-query-semantics.md) tokenizer |
| `Hit.id` | `"memory:" + row.id` (e.g., `memory:a1b2c3`) | Allows agent to drill into `memory get` / `memory history` by id |
| `Hit.title` | `"{subject} {predicate} {object}"` (single-line summary) | Readable in hit lists without parsing |
| `Hit.snippet` | `""` (empty) | Memory is structured; no prose to excerpt |
| Metacharacter handling | Skip per [S003](S003-query-semantics.md) | Match project-wide convention |

**Query example:**

```
$ everyday search "rust"
```

hits would include `memory:a1b2c3` (from `(user, prefers, rust)`) alongside note/todo/bookmark/mail hits, ranked by aggregator's existing cap/limit logic ([S004](S004-execution-model.md)).

**Adapter implementation sketch:**

```rust
pub struct MemorySearchProvider;

#[async_trait::async_trait]  // already in src/search.rs convention
impl Searchable for MemorySearchProvider {
    fn module_name(&self) -> &'static str { "memory" }

    async fn search(&self, q: &SearchQuery, _cfg: &Config) -> Result<Vec<Hit>, AgentError> {
        let tokens = q.tokens();  // already split per S003
        let mut hits = Vec::new();
        for token in tokens {
            let pattern = globify(token);
            // SELECT id, subject, predicate, object FROM current_state_view
            //   WHERE subject GLOB ?1 OR predicate GLOB ?2 OR object GLOB ?3
            // current_state_view is the ROW_NUMBER() OVER (PARTITION BY s,p,o ORDER BY created_at DESC) = 1 view
            // build hits...
        }
        Ok(dedupe_and_cap(hits, q.limit))
    }
}
```

Registration: `MemorySearchProvider` is registered unconditionally in `SearchRegistry::build()` (no per-account registration needed — see [K004](K004-memory-single-instance.md)).

## Alternatives considered

### Don't implement Searchable (rejected)

Cleaner v1, but creates inconsistency: 8 modules, 6/7 participate (mail was the late addition in [S007]). Agent queries like `everyday search "user prefers rust"` would miss memory facts, surprising the agent. The implementation cost is low (~10 lines + a few tests); the consistency gain is high.

### Implement against history (rejected)

Allow `everyday search` to match deleted or superseded versions. Bloats hit lists with facts the agent has already moved past. History is a per-triple concern, handled by `memory history`. Search hits should reflect "what is true now".

### Embedding-based search (rejected for v1)

Semantic / vector search is what the user explicitly listed as a v2 extension ("memory search：语义检索 / embedding 索引增强召回能力"). v1 Searchable adapter is text-only; embedding integration would require a new trait extension (e.g., `EmbeddingSearchable`) — out of scope for v1.

### Per-account MemorySearchProvider (rejected)

Mirror the per-account pattern of note/todo/bookmark. But memory is single-instance with no account column (see [K004](K004-memory-single-instance.md)). One global provider suffices.

## Consequences

- `memory` becomes the 7th `Searchable` provider (alongside note/todo/bookmark/rss/cal/mail).
- Memory search hits include the row id in `Hit.id`, enabling drill-down via `memory get` / `memory history`.
- Empty `Hit.snippet` is intentional and consistent with the structured nature of memory facts; downstream UI must not assume non-empty snippet.
- The `current_state_view` is the same window-function view used by `memory get` / `relation` / `list` / `graph` — single source of truth for "what is current", embedded as a SQLite VIEW in the schema.
- If v2 adds embedding search, a parallel `EmbeddingSearchable` trait can be added without modifying this `Searchable` adapter. Traits compose, not replace.

## Cross-references

- [S001](S001-search-architecture.md) — `Searchable` trait + `SearchRegistry`
- [S002](S002-hit-normalization.md) — `Hit` contract (id/title/snippet)
- [S003](S003-query-semantics.md) — token semantics + GLOB usage
- [S004](S004-execution-model.md) — concurrent best-effort execution
- [S007](S007-mail-search-local-cache.md) — most recent `Searchable` adapter precedent
- [R008](R008-sql-glob-not-like.md) — GLOB vs. LIKE for token-boundary matching
- [K001](K001-memory-module.md) — main memory module decision
- [K002](K002-memory-graph-query.md) — graph query (different semantic; does not use Searchable)
- [K004](K004-memory-single-instance.md) — single-instance storage (no per-account provider)