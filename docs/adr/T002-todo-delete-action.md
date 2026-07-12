# ADR T002: Todo delete action — Notion archive + local physical delete (with title preservation)

**Status:** Accepted
**Date:** 2026-07-11

## Context

Until v0.5.0, the `todo` module had **no `delete` action**. The only way to remove a todo was via the Notion UI or by manually deleting the SQLite row. The lack of a CLI path made the Timeline view ([L007](L007-notion-ops-log.md)) inconsistent: a todo completed in Notion UI never appeared as a `completed` event in the timeline, and there was no way to delete a todo to test the soft-delete flow.

The design has to handle both providers symmetrically while keeping the title in the ops-log so the resulting timeline row is human-readable.

## Decision

**Add `everyday todo delete <id>`. The behavior depends on the active provider.**

### Notion provider

1. `GET /pages/<id>` → fetch the page; extract `properties["Name"]["title"][0]["plain_text"]` (or equivalent) as `title`.
2. `PATCH /pages/<id>` with `{"archived": true}`.
3. Emit `deleted todo '<title>' (id=<id>)` in both text and JSON modes.

### Local provider

1. `SELECT title FROM todos WHERE id = ?` — fetch the title.
2. `DELETE FROM todos WHERE id = ?` — physical delete.
3. Emit `deleted todo '<title>' (id=<id>)`.

### Why fetch the title before deleting?

The ops-log AOP hook ([L007](L007-notion-ops-log.md), [L011](L011-aop-handles-output-text.md)) records the deletion as a `deleted` event. The hook extracts `ref_id` and `title` from the module's `Output`. If the title is missing, the timeline row shows `todo ''` — useless for a Review or digest.

Fetching the title first is one extra `GET` (Notion) or `SELECT` (SQLite). Both are fast.

## Alternatives considered

### Soft-delete only (mark `deleted` but keep the row)

- Reversible.
- The local schema gains a `deleted_at` column; queries must filter it everywhere.
- A future option once `todo add` gets a recovery workflow. Not now.

### Hard delete on both providers

- For Notion this means `DELETE /pages/<id>`, which Notion actually treats as `archived: true` under the hood.
- For local this is `DELETE FROM todos`. Same outcome.
- Implementation: Notion path still needs the title-fetch step (same as archive path).

### Don't fetch the title; record `id` only

- Cheaper.
- Timeline rows become `deleted todo '' (id=abc123)` — useless.
- Rejected.

### Emit deletion via a separate `delete` action that bypasses ops-log

- Skips the AOP hook.
- Timeline loses the deletion event.
- Rejected.

## Consequences

- The CLI gains a way to delete todos without leaving the terminal.
- Timeline's `deleted` event type is exercised end-to-end for both providers (see [L007](L007-notion-ops-log.md) for the schema).
- The local provider still does a physical delete — this is consistent with `K1` (`note` and `bookmark` local providers don't have a delete either today; future soft-delete work should add a `deleted_at` column and update the timeline provider, per [L008](L008-local-provider-degraded-granularity.md)).
- The delete action emits a parseable text/JSON output that downstream tooling (including the AOP hook) can rely on.

## Cross-references

- The todo module this belongs to: [T001](T001-notion-todo-module.md).
- The ops-log that records the deletion: [L007](L007-notion-ops-log.md).
- The AOP hook that must parse `Output::Text`: [L011](L011-aop-handles-output-text.md).
- Soft-delete as a future extension path: [L008](L008-local-provider-degraded-granularity.md).