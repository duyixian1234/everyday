# ADR R018: Backend domain types + in-memory mock backends

**Status:** Accepted
**Date:** 2026-07-12

## Context

[R016](R016-action-backend-di.md) decided backends return **domain types, not
`Output`** (grill option 6a). Two sub-questions remain: how strongly typed the
domain representation is, and whether the DI payoff is locked in with a
regression guard (grill option 7a).

Currently each action helper builds `Output` inline (often via
`serde_json::json!`). After the refactor the module owns rendering, so the
backend must hand back structured data the module can render for both text and
`--json`.

## Decision

### Typed domain structs (per module, minimal)

Serde-derived structs (for `--json` rendering); notion and local impls return
the **same** type:

- **note**:
  - `NoteSummary { id: String, title: String, url: Option<String>, updated: String }`
    — `list` / `search` return `Vec<NoteSummary>`.
  - `NoteDetail { id: String, title: String, content: String }`
    — `read` / `create` / `append` / `update` return `NoteDetail`.
- **todo**: `TodoItem { id, title, status, .. }` — actions return `TodoItem` /
  `Vec<TodoItem>`.
- **bookmark**: `BookmarkItem { id_or_url, title, url, tags }` — actions return
  `BookmarkItem` / `Vec<BookmarkItem>`.

Notion-JSON→struct conversion lives in `notion.rs`; SQLite-row→struct conversion
lives in `local.rs`. Both sides are forced to the same contract by the trait.

### Module owns rendering

`note/mod.rs` (and siblings) convert the domain struct → `Output` (text table /
`--json`). The inline render code currently in each action helper is extracted
into the module's render section.

### In-memory mock backends (regression guard)

Add `MockNoteBackend` / `MockTodoBackend` / `MockBookmarkBackend` — a
`Vec`-backed in-memory store (under `#[cfg(test)]` or a `testkit` module).
Action-layer unit tests **inject** them to prove:

1. The action path has **zero** `NotionClient` dependency (the seam holds).
2. Behaviour is **provider-agnostic** (same `Output` for mock vs real backend
   given equivalent domain data).

This is the explicit acceptance evidence for the DI refactor and prevents the
seam from silently regressing.

### Error type

Backend methods return the existing `Result<T>` = `AgentError`. Notion errors
are mapped at the `NotionNoteBackend` boundary; SQLite errors at the
`LocalNoteBackend` boundary.

## Alternatives considered

### Return `serde_json::Value` (option 6b)

- Notion returns JSON as-is; local builds a same-shape `Value`. Least boilerplate.
- Loses type safety; local/notion shapes can drift; test doubles return opaque
  `Value`.
- **Rejected.**

### No mocks (option 7b)

- Refactor ships, existing integration tests stay green.
- DI payoff has no regression guard; a future change could re-leak `NotionClient`
  into the module unnoticed.
- **Rejected.**

## Consequences

- Slightly more code (domain structs) but explicit, testable contracts.
- Action-layer logic becomes unit-testable **without** keyring / network →
  faster CI, consistent with [F010](F010-testing-requirements.md).
- Local impls must be refactored to return domain types (today they build
  `Output` / rows directly) — part of the R016 implementation checklist.

## Cross-references

- The trait + factory these types flow through: [R016](R016-action-backend-di.md)
- The directory layout they live in: [R017](R017-backend-layout-scope.md)
- `Output` / `AgentError` the module renders to / errors unify to: [F001](F001-cli-shape.md)
- Mandatory testing + mock discipline: [F010](F010-testing-requirements.md)
