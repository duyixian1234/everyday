# ADR K002: Memory graph query — forward-only recursive traversal on current state

**Status:** Accepted
**Date:** 2026-07-14

## Context

`memory get <SUBJECT>` returns direct 1-hop facts (all predicates and objects under a given subject). For richer queries like "what technologies does the user's project-everyday depend on, transitively?", the agent needs a multi-hop traversal.

The user's proposal showed:

```
$ everyday memory graph user

user
 |
 +-- prefers --> rust
 |
 +-- works_on --> everyday
                  |
                  +-- uses --> mongodb
```

This raises several semantic questions:

1. **Direction** — forward-only (subject → object), or also reverse (object → subjects that point to it)?
2. **State** — current state only, or full version history?
3. **Deleted handling** — show soft-deleted edges, or hide them by default?
4. **Depth bound** — what default and maximum?
5. **Cycle handling** — how to prevent infinite traversal on cycles like `(A, knows, B)` + `(B, knows, A)`?
6. **Output shape** — tree (markdown) vs. nested (JSON) vs. adjacency list?

## Decision

`memory graph <SUBJECT> [--depth N] [--include-deleted]`

| Parameter | Decision | Rationale |
|---|---|---|
| Direction | **Forward only** (subject → object) | Matches natural triple order; reverse queries handled by separate `backlinks` command (deferred to v2) |
| State | **Current state only** (`MAX(created_at) WHERE deleted_at IS NULL`) | Aligns with `memory get` behavior; full history served by `memory history` |
| Deleted edges | **Hidden by default**; `--include-deleted` opt-in | Consistent with Timeline soft-delete default filtering |
| Depth | **Default 2, max 5** | Depth 2 covers common use cases; 5 prevents prompt explosion (10k nodes possible at high connectivity) |
| Cycle handling | **Visited set**; revisit attempts silently skip | Prevents infinite loop; matches BFS convention |
| Required argument | **`<SUBJECT>` mandatory** | Full-graph query is O(n) and rarely useful to agents |
| Text output | **Markdown indented list** | Plain-text parseable, machine-and-human readable |
| JSON output | **Nested tree** (`{subject, predicates: [{name, objects: [{name, predicates: [...]}]}]}`) | Mirrors text shape; agent recursive traversal natural |

**Depth validation:** `--depth` must be integer in `[1, 5]`. Out of range → `AgentError::InvalidArgument`. Default is 2.

**Cycle detection:** maintain a visited set keyed by `(subject, predicate, object)` of already-rendered edges. When the recursive step would revisit, the branch terminates silently (no warning). This keeps output deterministic and bounded.

**Multi-subject edges:** if a (subject, predicate) maps to many objects, all are listed in order of `created_at DESC` (consistent with `memory get`).

**Empty / unknown subject:** if the subject does not exist or has no current-state facts, the output is just the subject name with no children — not an error. Mirrors `memory get` empty-state behavior.

**Implementation:** recursive CTE in SQLite is feasible but adds complexity; preferred implementation is **Rust-side BFS** for clarity, given the small graph sizes typically queried (depth ≤ 5, expected fan-out < 100 per node). Each level issues a `WHERE subject = ? AND deleted_at IS NULL` query using `ix_memory_subject_created`.

## Alternatives considered

### Bidirectional (subject ↔ object) (rejected for v1)

Useful for "who else knows X?" queries. But adds semantic ambiguity (the same predicate could be traversed in either direction with different intent) and doubles the recursion branching factor. Deferred to v2 as a separate `memory backlinks <OBJECT>` command, which is the cleanest decomposition.

### Full history in graph (rejected)

Show every version of every edge. Bloats output; agent decision-making should reflect "what is true now", not "everything that ever was". `memory history` already covers per-triple version tracking.

### Unbounded depth (rejected)

`--depth` without an upper bound. A real graph with even modest connectivity reaches thousands of nodes at depth 4. Hard cap of 5 protects both the agent's prompt budget and SQLite query latency.

### Recursive CTE (rejected for clarity)

`WITH RECURSIVE graph(...) AS (...)` is doable in SQLite 3.8.3+. Considered for performance, but Rust-side BFS with prepared-statement reuse is easier to read, test, and evolve. Performance is not the bottleneck at depth ≤ 5.

## Consequences

- `memory graph` adds visible value beyond `memory get` (multi-hop) but stays scoped — no reverse direction in v1.
- Cycle handling means the same `(S, P, O)` appears at most once in the output, even if reachable by multiple paths. Loses path information in exchange for bounded output size.
- `--include-deleted` flag exists but is rare in practice; deleted edges are usually noise.
- Depth cap of 5 may surprise users expecting unlimited recursion; documented in CLI help and `skill.md`.
- Reverse direction queries require `memory backlinks` (v2). If v1 users hit this gap, the workaround is `memory get <OBJECT>` to see what facts mention that object as a subject — partial coverage.

## Cross-references

- [K001](K001-memory-module.md) — main memory module decision
- [S003](S003-query-semantics.md) — token semantics (memory graph does NOT use search tokens; the input is a single subject)