# ADR R024: Local provider soft delete — `deleted_at` + a current-state view

**Status:** Accepted
**Date:** 2026-09-18

## Context

`todo delete` was a physical `DELETE FROM todos` (`src/modules/todo/local.rs`).
The row was gone: no record it had existed, no way to recover it, and no way for
the timeline to report the deletion.

Two ADRs had already planned the change and named the mechanism:

- [L008](L008-local-provider-degraded-granularity.md): *"There is no `deleted_at`
  column yet; **soft-delete is deferred**. … Future soft-delete work would add a
  `deleted_at` column and have the provider generate `deleted` events."*
- [T002](T002-todo-delete-action.md): *"A **future option once `todo add` gets a
  recovery workflow. Not now.**"*

Neither was implemented. `CONTEXT.md` even records the intent — bookmark "将来加
delete 时改为软删除（`deleted_at` 列）".

Two facts about the starting position matter for the decision below:

1. **The delete path had no test at all.** `todo delete` was never exercised by
   any test: the dispatch arm, `render_delete`, and `MockTodoBackend::delete`
   were all uncovered, and the `deleted` field on the mock was never assigned.
   A behaviour change here could not be caught by a regression.
2. **`memory` is the repo's only shipped soft-delete store** — and its
   implementation is a **partial** model, useful in one half and misleading in
   the other. It has `CREATE VIEW current_state_view ... WHERE deleted_at IS NULL`
   (`memory/store.rs`) as a single source of truth for the current state, shared
   by every read path. But it declares `deleted_at` **only inside
   `CREATE TABLE`**, with no column-existence check and no `ALTER TABLE`, so a
   `memory.db` created before that column existed **never gained it**.

## Decision

### Soft delete in `todo`, with the column and semantics reusable by `bookmark`

`todo` is the only module with a delete action today (`note` and `bookmark` have
none). `deleted_at TEXT` is added to `todos` and `delete` stamps it instead of
removing the row. `bookmark` gains `delete` in Phase 28 and reuses this column
shape and these semantics rather than inventing its own; **`note` is out of
scope** — adding a delete action to a module that has never had one is new
feature surface with no requirement behind it, which is precisely the mistake
[K005](K005-memory-module-deprecation.md) recorded about `memory`.

### Reads go through a view; writes carry their own guard

A `active_todos` view (`SELECT ... FROM todos WHERE deleted_at IS NULL`) is the
one place "live row" is defined, and every read path queries it:

| Read path | Query target |
|---|---|
| `list` | `active_todos` |
| `search_for_search` (Searchable) | `active_todos` |
| `fetch_for_timeline` | `active_todos` |

Writes cannot be protected by a view, so they carry the condition explicitly:

| Write path | Guard |
|---|---|
| `set_status` (`start`/`complete`) | `WHERE id = ?3 AND deleted_at IS NULL` |
| `delete` pre-read | `WHERE id = ?1 AND deleted_at IS NULL` |
| `delete` stamp | `WHERE id = ?2 AND deleted_at IS NULL` |

This split is the decision. The alternative — adding `deleted_at IS NULL` to
each of roughly twenty read queries — relies on nobody ever forgetting one, and
a forgotten guard is **invisible**: the leaked row looks like an ordinary row
and no test fails. Putting the predicate in the view makes the read side
structurally correct, and confines the hand-written guards to the few write
sites where they are unavoidable. (This mirrors `memory`'s view, which is the
part of that module worth copying.)

### Migrate the column; do not repeat `memory`'s omission

`deleted_at` is added by an idempotent column-existence check plus a conditional
`ALTER TABLE`, the pattern `todo` already used for `updated_at`. It is **not**
declared only in `CREATE TABLE` — doing that is what left old `memory.db` files
without the column, and a schema that only new databases get is not a schema.

### No recovery action in this change

`T002`'s trigger condition — *"once `todo add` gets a recovery workflow"* — is
**ambiguous**, and this ADR settles the reading: **soft delete lands before a
recovery workflow, and a recovery action is a separate proposal.** The
sentence can be read as "do not soft-delete until recovery exists" or as
"soft delete is the prerequisite for recovery"; the second is what shipped,
because the first inverts the dependency — retention is what *makes* recovery
possible, and deleting data irreversibly while waiting for a recovery feature
destroys exactly what that feature would need.

No `todo restore` ships here. It is new user-visible surface that no ADR
requests, and it immediately raises questions this change cannot answer — what
state does a restored todo return to, and what does a todo that was `started`
before deletion come back as? Leaving it unbuilt keeps those questions where
they belong (a future proposal) instead of half-answering them in code.

### `deleted` timeline events are the point of retaining the row

A soft-deleted todo is hidden from every read path, including
`fetch_for_timeline`. Without a `deleted` event, the timeline would show that
todo **frozen at its last pre-deletion state** — a `In Progress` entry for a
task the user deleted. That is not missing data, it is **wrong information**,
and it is the same failure mode the calendar cache
([C004](C004-cal-event-cache.md)) exists to avoid.

The event vocabulary already reserved the value (`CONTEXT.md` lists todo's
event types as `created`/`started`/`completed`/`deleted`) and the `events` table
places no enum constraint on `event_type`, so **no schema change is needed**.
Generating it is a separate ticket (t6).

Worth recording, because two ADRs currently claim otherwise: **T002 and L008
both assume an "ops-log AOP hook" emits `deleted` events. That mechanism no
longer exists** — `src/ops_log.rs` was deleted wholesale with the Notion
provider ([R019](R019-remove-notion-provider.md)). Under the pull model the
event can only be derived by the provider from table state. Those two ADRs are
factually stale on this point and are corrected as part of this work.

### WebDAV sync semantics are explicitly unchanged

**Soft delete does not make `deleted_at` a tombstone, and file-level sync is not
made deletion-aware.** This is a deliberate non-decision, recorded so it is not
mistaken for an oversight.

What the code actually does today: the sync namespace is a fixed whitelist
(`sync/mod.rs`), transfer is whole-file with `VACUUM INTO` snapshots and LWW
arbitration by mtime, and there is **no tombstone, no `Delete` variant in
`SyncAction`, and no DELETE method on the client** — row-level merge was
explicitly rejected in [D001](D001-webdav-file-sync.md).

Two consequences follow, and both are accepted:

- If device A deletes and pushes, device B's next sync **pulls the file and the
  row is gone there too** — the deletion propagates, unproblematically.
- If both sides changed, mtime LWW arbitrates. If the local copy wins, **a row
  deleted on the other device comes back.** With soft delete the row is still
  in the file, so this is a resurrection of a hidden row rather than data loss.

Implementing tombstone-aware arbitration is **rejected**: it would reintroduce
the row-level merge D001 ruled out, and it would need a conflict model for
"deleted here, edited there" that nothing in this project currently requires.
Writing the limitation down costs one paragraph and stops the next ADR from
designing against a mechanism that does not exist — which is exactly how T002
and L008 came to describe an ops-log hook that had already been deleted.

## Alternatives considered

### Guard every read query with `deleted_at IS NULL`
- No new schema object; each query is self-contained.
- Rejected: it is the same predicate duplicated ~20 times, and the failure mode
  is silent. A missed guard returns deleted rows and nothing goes red. The
  memory leak of a wrong-but-passing query is not worth the avoided view.

### `DELETE` with a separate `deleted_todos` archive table
- Base table stays clean; reads need no change at all.
- Rejected: two tables must be kept consistent on every write, and restoring
  means moving rows back. `deleted_at` on the row keeps the data and its
  deletion together, and needs no second write to stay coherent.

### Emit a runtime warning when a deleted todo is addressed
- Surfaces the state loudly.
- Rejected: `todo` is consumed through `--json`, where warnings ride along as a
  `_warning` key that a strict parser must then handle. A deleted todo is not
  addressable, so the existing `InvalidArgument` already says so; adding a
  warning to a correct error is noise. (Same reasoning as K005's rejection of a
  runtime deprecation warning.)

### Ship `todo restore` alongside
- Completes the story in one change.
- Rejected: see "No recovery action" above. It is new surface with an unresolved
  state model, and no ADR asks for it.

### Make sync arbitration tombstone-aware
- Would make deletion authoritative across devices.
- Rejected: contradicts D001's explicit rejection of row-level merge and opens a
  conflict model nothing requires. See "WebDAV sync semantics" above.

## Consequences

- `todo delete` is now reversible in principle: the row survives with
  `deleted_at` set. Nothing in-tree restores it yet.
- Deleted todos are invisible to `list`, `search`, and the timeline, and cannot
  be `start`ed, `complete`d, or deleted again — each returns `InvalidArgument`,
  identical to a nonexistent id.
- The `todos` table grows monotonically: nothing purges soft-deleted rows. This
  is accepted for personal-scale data, and is the explicit cost of retention.
- Existing databases gain `deleted_at` on next open. Rows predating the column
  are live (`deleted_at` is NULL), so **nothing is retroactively hidden**.
- The delete path now has a regression net, which it had never had.
- `bookmark` can reuse the column, the view shape, and the guard placement when
  it gains `delete` in Phase 28.
- `memory/store.rs`'s migration omission is now documented as a counter-example
  rather than an implicit pattern to copy.

## Cross-references

- Implements: [L008](L008-local-provider-degraded-granularity.md) (soft delete +
  provider-generated `deleted` events) and [T002](T002-todo-delete-action.md)
  (the delete action and its triggers). Both are corrected on the ops-log point.
- Precedent (view only): [K005](K005-memory-module-deprecation.md) /
  `memory/store.rs` — `current_state_view` is the model for read-path
  encapsulation; its missing migration is the counter-example.
- Constrains: [D001](D001-webdav-file-sync.md) — file-level LWW is unchanged and
  row-level merge stays rejected.
- Related: [C004](C004-cal-event-cache.md) — the same "a stale row is wrong
  information" argument; [F003](F003-module-scope-external-integration.md) —
  why no speculative restore action.
