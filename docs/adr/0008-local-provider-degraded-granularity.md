# ADR 0008: Local provider degraded event granularity

**Status:** Accepted
**Date:** 2026-07-11

## Context

[ADR 0001](0001-append-only-event-log.md) established the append-only log model where each state transition is a separate event. However, local module SQLite tables store **current state snapshots**, not transition history:

- `todos` table: `id, title, status, due, priority, created_at` — no `updated_at` or status-change timestamps.
- `notes` table: has `updated_at`, but only the latest — if a note was updated 3 times between syncs, only the most recent `updated_at` is visible.
- Deletions: rows are physically removed, leaving no trace for the provider to query.

This creates a fundamental tension: the append-log wants one event per transition, but local SQLite stores one row per entity with current state only.

## Decision

**Accept "latest-state snapshot" granularity for local providers. Multiple transitions between syncs are collapsed into one event reflecting the current state.**

### Specific changes

1. **Add `updated_at` column to `todos` table** (migration via `ALTER TABLE`). `set_status` updates this column. Note table already has `updated_at`.

2. **Todo provider** generates:
   - `created` event for new todos (`created_at > last_sync`).
   - Current-status-mapped event when `updated_at > last_sync` (e.g., `status = "Done"` → `completed` event with `timestamp = updated_at`).
   - If a todo went Todo → In Progress → Done between syncs, only a single `completed` event is generated (the `started` transition is lost).

3. **Note provider** generates:
   - `created` for new notes (`created_at > last_sync`).
   - `updated` when `updated_at > last_sync` (one event per sync, even if multiple updates occurred).

4. **Deletions are not supported in v1.** The todo module currently has no `delete` action. When added in the future, it should use soft-delete (`deleted_at` column) so the provider can query `WHERE deleted_at > last_sync` to generate `deleted` events.

5. **Notion accounts are unaffected** — ops-log ([ADR 0007](0007-notion-ops-log.md)) records every CLI-initiated transition at execution time, so full granularity is preserved for notion accounts.

## Alternatives considered

### Full transition history (status log table)

Add `todo_status_log(id, todo_id, from_status, to_status, changed_at)` and have `todo_local.rs::set_status` write a log entry each transition.

- **Rejected**: requires invasive changes to `todo_local.rs` write path, making local modules maintain their own ops-log. This contradicts [ADR 0004](0004-timeline-provider-pull-only.md)'s pull model and the AOP philosophy ([ADR 0007](0007-notion-ops-log.md)).
- Significant engineering cost for marginal value.

### Diff snapshot (provider maintains last-seen state)

Provider stores last sync's snapshot per `ref_id`, diffs current state against it, generates events for changed fields.

- **Rejected**: doesn't solve the core problem (multiple transitions still collapse to one diff). Adds snapshot management complexity (a `snapshot` table in timeline.db) for no granularity improvement over the simpler approach.

## Consequences

- `started` (In Progress) events may be missing for local todo accounts if the todo transitioned through In Progress and was completed before the next sync.
- `created` and `completed` — the two most valuable anchors for Review — are preserved.
- Note "updated" frequency reflects sync frequency, not actual edit frequency. This is acceptable: timeline measures "when was activity detected", not "when did every keystroke happen".
- The `todos` table schema change (`updated_at` column) requires migration logic in `todo_local.rs::open()` (idempotent `ALTER TABLE ADD COLUMN ... IF NOT EXISTS` equivalent — SQLite lacks `IF NOT EXISTS` for `ALTER TABLE`, so use a check or `CREATE TABLE` with the new column for fresh DBs + migration for existing).
