# ADR L010: OpsLogProvider — project ops-log rows into the events table

**Status:** Accepted
**Date:** 2026-07-11

## Context

After [L007](L007-notion-ops-log.md) established that notion accounts are served by reading `~/.config/everyday/ops-log.db`, the missing piece was the actual `TimelineProvider` adapter that turns `ops_log` rows into `TimelineEvent` values.

Without it, `timeline sync` for a `provider = "notion"` todo account would query the local SQLite `todos` table (the only source the provider was wired to), then the timeline would either miss the write operations entirely or emit them with `updated_at` granularity rather than the per-action granularity the ops-log captures.

## Decision

**Add an `OpsLogProvider` to the timeline `TimelineProviderRegistry`.**

For each notion account of `todo` / `note` / `bookmark`, the provider:

1. Opens `~/.config/everyday/ops-log.db`.
2. Queries `SELECT * FROM ops_log WHERE module = ? AND account = ? AND occurred_at > ?` (the `?` is the per-`(source, account)` watermark from `sync_state`).
3. Maps each row to a `TimelineEvent`:
   - `source` ← `module` (`todo` / `note` / `bookmark`).
   - `event_type` ← `action` (e.g. `add` → `created`, `complete` → `completed`, `start` → `started`, `delete` → `deleted`).
   - `timestamp` ← `occurred_at`.
   - `ref_id` ← `ref_id`.
   - `title` ← `title`.
   - `metadata` ← the JSON `metadata` column, parsed.
4. Returns the events with `SyncMode::Append` (the ops-log itself is append-only).

The provider sits alongside the local SQLite provider in the registry (see [R011](R011-add-dual-providers-macro.md) for the dual-provider build macro).

## Alternatives considered

### Map ops-log in the orchestrator directly, skip the provider abstraction

- Faster to write; bypasses the `TimelineProvider` trait.
- Rejected: every source must speak the same trait for the orchestrator's parallelism and watermark handling to apply uniformly. Bypassing the trait creates a special case.

### Replace local SQLite reads with ops-log reads for local accounts too

- Would unify the data path.
- Rejected: local accounts have no ops-log entries (the AOP hook only fires for `provider = "notion"`). Local providers must keep querying their own tables.

## Consequences

- Notion accounts now produce timeline events at full per-action granularity — `created` / `started` / `completed` / `deleted` for todos, `created` / `updated` for notes, `added` for bookmarks.
- The ops-log remains a single shared store; the provider is a thin adapter.
- The dual-provider build (`add_dual_providers!`) ensures both providers coexist in the registry per account — see [R011](R011-add-dual-providers-macro.md).
- The provider honors the same watermark discipline as every other source ([L009](L009-best-effort-sync.md)).

## Cross-references

- The ops-log this reads from: [L007](L007-notion-ops-log.md).
- The local provider whose granularity this complements: [L008](L008-local-provider-degraded-granularity.md).
- The dual-provider build macro: [R011](R011-add-dual-providers-macro.md).
- The trait it implements: [L004](L004-timeline-provider-pull-only.md).