# ADR K001: Memory module — agent's own notebook (append-only triple store)

**Status:** Accepted
**Date:** 2026-07-14

## Context

AI agents working with Everyday need to persist stable, structured facts about the user, projects, and the world — preferences, relationships, project metadata, technical knowledge. These facts are not timestamped events (which Timeline already captures) and not unstructured prose (which Note stores). They are **timeless assertions** that an agent wants to recall, refine, and reason over.

The original proposal was a `(subject, predicate, object)` triple store with optional `confidence` and `source` metadata, persisted in a local SQLite file. The proposal passed the F003 module-scope gate ([F003](F003-module-scope-external-integration.md)) by providing:

1. **Typed CLI surface** — agent invokes `everyday memory add user prefers rust` rather than `sqlite3 memory.db "INSERT..."`, eliminating shell-quoting and parameter-order fragility.
2. **Stable JSON contract** — `--json` output schema is fixed; agents depend on it for decision trees. `sqlite3 -json` output drifts with table changes.
3. **First-class confidence / source** — shell SQL has no first-class concept of confidence or provenance; embedding them in JSON output and querying them semantically (`memory get user --source explicit`) is the value-add.

Three semantic-model candidates were considered for handling re-adds of the same triple:

- **A. Pure upsert** — `UPDATE SET confidence=?, source=?, ...` on duplicate. Loses history; agent cannot introspect "I used to think X, now Y."
- **B. Append-only versions** — `INSERT` new row on duplicate; current state derived via `MAX(created_at) WHERE deleted_at IS NULL`. Preserves full version history.
- **C. Split current + history tables** — current row updated in place, history appended. Two-table writes, more complex.

The project's existing append-only discipline ([L001](L001-append-only-event-log.md), ops-log, mail cache K1 retention per [M003](M003-envelope-cache.md)) established precedent for **never mutating historical rows**. Model B aligns with this precedent and serves agent self-reflection needs. Model A breaks the pattern; Model C adds complexity without benefit.

## Decision

**Module: `memory`** (new top-level module, registered in `ModuleRegistry` via `Executor` trait).

**Storage:** `~/.config/everyday/memory.db` — independent SQLite file, NOT sharing `timeline.db`. See [K004](K004-memory-single-instance.md) for the storage-decoupling rationale.

**Data model:** Append-only versions of `(subject, predicate, object)` triples. Each `memory add` creates a new row; re-adding the same triple inserts a new version row with a new `id` and updated `created_at`. Current state is a window-function view over the table.

**Schema:**

```sql
CREATE TABLE memory (
    id          TEXT PRIMARY KEY,           -- short UUID
    subject     TEXT NOT NULL,
    predicate   TEXT NOT NULL,
    object      TEXT NOT NULL,
    confidence  REAL NOT NULL DEFAULT 1.0,  -- [0.0, 1.0], validated
    source      TEXT,                       -- free-text provenance label
    created_at  TEXT NOT NULL,              -- RFC3339 UTC, millisecond precision
    deleted_at  TEXT                        -- RFC3339 UTC, NULL = active
);
CREATE INDEX ix_memory_spo_created
    ON memory(subject, predicate, object, created_at DESC);
CREATE INDEX ix_memory_subject_created
    ON memory(subject, created_at DESC);
CREATE INDEX ix_memory_subject_predicate
    ON memory(subject, predicate, created_at DESC);
CREATE INDEX ix_memory_created_at
    ON memory(created_at DESC);
```

**Soft delete:** `memory delete <S> <P> <O>` sets `deleted_at = now()` on the **current-state row only** (the row with `MAX(created_at) WHERE deleted_at IS NULL AND subject=? AND predicate=? AND object=?`). A subsequent `delete` on an already-deleted current state → `AgentError::InvalidArgument("already deleted")`. Deleting a triple that has no current state (never existed or already fully deleted) → `AgentError::InvalidArgument("triple not found or already deleted")`.

**Resurrection:** `add` after delete creates a new row (append-only). History naturally shows the original row (with `deleted_at`) followed by the resurrection row. No separate `undelete` command in v1 — `add` is the resurrection verb.

**Current-state query:** uses SQLite 3.25+ window function for clarity:

```sql
SELECT * FROM (
    SELECT *, ROW_NUMBER() OVER (
        PARTITION BY subject, predicate, object
        ORDER BY created_at DESC
    ) AS rn
    FROM memory
    WHERE deleted_at IS NULL AND subject = ?
) WHERE rn = 1
ORDER BY created_at DESC;
```

**v1 commands (7):**

| Command | Behavior |
|---|---|
| `add <S> <P> <O> [--confidence N] [--source LABEL]` | Append a row |
| `get <SUBJECT>` | Current state of all triples with this subject |
| `relation <SUBJECT> <PREDICATE>` | Current state of all triples matching (subject, predicate) |
| `list [--limit N]` | All current-state rows (default cap 100) |
| `delete <S> <P> <O>` | Soft-delete current state row |
| `graph <SUBJECT> [--depth N] [--include-deleted]` | Forward-only recursive traversal — see [K002](K002-memory-graph-query.md) |
| `history <S> <P> <O>` | Full version history (including deleted rows) |

**No semantic validation:** the program does not validate that a triple is "fact-shaped" (e.g., it will accept `memory add user sent-email-to alice` without complaint). Semantic correctness is the agent's responsibility; rules go in `skills/everyday-cli/SKILL.md`. This matches the philosophy that memory is "agent's own notebook" — the program is a durable store, not a fact-checker.

**No `auth` module touch:** memory never calls `auth::*`. See [K004](K004-memory-single-instance.md).

**Empty-state semantics:** `get` / `relation` / `list` / `graph` on nonexistent or empty input return empty results (no error), matching Timeline's empty-state behavior. `delete` on nonexistent current state is an error.

**Output formats:**
- JSON: `{ "facts": [...], "count": N }` envelope; each fact has `id`, `subject`, `predicate`, `object`, `confidence`, `source`, `created_at`. `deleted_at` only appears in `history` output.
- Text: tabular (add/get/relation/list/delete/history) or markdown indented tree (graph).

**Errors:** use existing `AgentError::InvalidArgument` for all semantic violations (confidence range, depth range, delete on nonexistent/already-deleted). No new error variants. See [F001](F001-cli-shape.md).

## Alternatives considered

### Pure upsert (rejected)

`UPDATE` on duplicate `(subject, predicate, object)`. Simple, but loses history. Agent cannot answer "why did I stop believing user prefers java?" The project's append-only philosophy was set explicitly in [L001](L001-append-only-event-log.md); breaking it here would set bad precedent.

### Split current + history tables (rejected)

`memory_current` updated in place, `memory_history` appended. Two-table transactional writes; current-state reads are O(1) but history queries need a JOIN. The mail cache already uses this pattern ([M003](M003-envelope-cache.md)) but its rationale (read-heavy list, write-once) does not apply to memory (read-write balanced). Adds complexity without proportional gain.

### Schema-enforced semantic validation (rejected)

Constrain `predicate` to a closed set (e.g., `prefers`, `knows`, `uses`). Rejected because (a) the agent's mental model of "what predicates exist" evolves, (b) validation rules belong in `skill.md` not in SQLite CHECK constraints, (c) the user explicitly stated "memory 是由具体的 agent 发起的，不应在程序中限制" during design.

### Subject / predicate / object as ID references (rejected)

Replace string fields with foreign-key references to entity tables. Rejected because (a) triples are free-form; entities emerge from usage patterns, (b) FK constraints prevent the agent from storing raw observations before they've been normalized.

## Consequences

- Memory module joins the nine existing modules (`mail` / `cal` / `rss` / `note` / `todo` / `bookmark` / `timeline` / `config` / `search`).
- `MemoryModule` implements `Executor`, registered in `ModuleRegistry` like other modules.
- `MemorySearchProvider` implements `Searchable` (see [K003](K003-memory-searchable.md)) so memory participates in `everyday search`.
- `memory.db` grows monotonically — soft-deleted rows accumulate. v1 has no `cleanup` command; users can `cp memory.db memory.db.bak` and rebuild by re-adding facts.
- Append-only model means `confidence` evolution is recorded (0.5 from conversation → 0.9 from explicit confirmation are two rows). `memory get` shows the latest; `memory history` shows the evolution.
- `deleted_at` defaults to filtering out deleted rows from all current-state queries; `--include-deleted` opt-in for `graph`; `history` always shows everything.

## Deferred to v2

Captured for follow-up but **explicitly out of scope for v1**:

- `memory undelete-by-id <id>` — resurrect a specific historical row by its `id` (different semantics from "add new version")
- `memory search <query>` — semantic / embedding-based retrieval
- `memory merge <S> <P>` — conflict resolution when multiple current-state rows disagree on the same key
- `memory expire <S> <P> <O> --ttl DURATION` — TTL-based automatic expiry
- `memory cleanup` — physical GC of `deleted_at IS NOT NULL` rows
- `memory stats` — library size / top subjects / distribution metrics
- Multi-instance / per-agent partition if v1 single-instance proves limiting (see [K004](K004-memory-single-instance.md))

## Cross-references

- [F003](F003-module-scope-external-integration.md) — module scope gate (F003-justified)
- [F001](F001-cli-shape.md) — CLI shape, `Executor` trait, `AgentError`, `Output`
- [L001](L001-append-only-event-log.md) — append-only philosophy
- [L007](L007-notion-ops-log.md) — ops-log AOP pattern (not used by memory; cited as design parallel)
- [S001](S001-search-architecture.md) — `Searchable` trait integration
- [K002](K002-memory-graph-query.md) — graph query semantics
- [K003](K003-memory-searchable.md) — Searchable adapter
- [K004](K004-memory-single-instance.md) — single-instance storage rationale