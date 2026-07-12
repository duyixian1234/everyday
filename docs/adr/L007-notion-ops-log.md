# ADR L007: Notion provider via local ops-log with AOP dispatch hook

**Status:** Accepted
**Date:** 2026-07-11

## Context

Todo / Note / Bookmark modules support two providers: `local` (SQLite, default) and `notion` (Notion API). Timeline's pull model ([L004](L004-timeline-provider-pull-only.md)) requires providers to query data sources for events.

For **local** accounts, the timeline provider queries the module's SQLite table — trivial, millisecond-level.

For **notion** accounts, querying the Notion API for incremental events is problematic:

- Notion API has no "modified since X" filter — requires fetching all pages and client-side filtering by `last_edited_time`.
- Pagination + rate limiting (429 with `Retry-After`).
- **Status transition history is not available**: Notion stores current status, not when a todo transitioned Todo → Done. The timeline cannot know *when* a todo was completed, only that it currently is.

## Decision

**Notion accounts are served from a local ops-log, not the Notion API.**

### Ops-log database

A unified `~/.config/everyday/ops-log.db` records CLI-initiated write operations on notion accounts:

```sql
CREATE TABLE ops_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    module TEXT NOT NULL,      -- todo / note / bookmark
    account TEXT NOT NULL,
    action TEXT NOT NULL,      -- add / complete / start / delete / create / update
    ref_id TEXT NOT NULL,
    title TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    occurred_at TEXT NOT NULL  -- RFC3339 UTC
);
```

The notion TimelineProvider queries `ops_log WHERE module=? AND account=? AND occurred_at > ?` and maps rows to `TimelineEvent`. See [L010](L010-ops-log-provider.md) for the provider implementation.

### AOP dispatch hook (module-zero-intrusion)

Ops-log writing is decoupled from modules via a **dispatch-layer hook** in `main.rs::run()`:

```
module.execute(action, args) → success? → ops_log::maybe_log_op(module, action, account, &output) → finalize(output)
```

`src/ops_log.rs` encapsulates all logic:

1. Only `todo` / `note` / `bookmark` write actions are logged.
2. Only `provider = "notion"` accounts are logged (local accounts are pulled from SQLite directly).
3. `ref_id` and `title` are extracted from the module's JSON `Output`. The hook must handle `Output::Text` too — see [L011](L011-aop-handles-output-text.md).
4. Write failures are non-blocking but surfaced — see [R006](R006-ops-log-surfacing.md).

Modules are completely untouched — they don't know ops-log exists.

### Explicit limitation

**Changes made outside the CLI (notion.so web UI, other Notion clients) are not captured.** Only CLI-initiated operations appear in the timeline. This is an intentional trade-off.

## Alternatives considered

### Query Notion API for notion accounts

- No incremental filter → fetch all pages every sync, client-side filter by `last_edited_time`.
- Status transition history unavailable (Notion stores current status, not change timestamps).
- Rate-limit and pagination complexity.
- Rejected: poor performance, missing data (transition timestamps), high implementation cost.

### Skip notion accounts (v1 only supports local)

- Notion users get no timeline at all.
- Rejected: the ops-log approach provides partial coverage (CLI-initiated ops) with minimal cost.

### Push from modules (each notion module writes ops-log on every action)

- Violates module independence (same reasoning as [L004](L004-timeline-provider-pull-only.md)'s rejection of push).
- The AOP hook achieves the same result without touching module code.

## Consequences

- Notion account timeline only reflects CLI operations, not external changes. Users are informed via sync output ("notion provider: ops-log based, external changes not captured").
- The ops-log is append-only and grows unbounded. v1 has no cleanup; future work may add retention (e.g. 90-day TTL).
- The AOP hook parses `Output` to extract `ref_id` / `title`. Some actions (e.g. `todo complete`) didn't include `title` originally — [T002](T002-todo-delete-action.md) ensures the delete path pre-fetches the title.
- The hook must parse `Output::Text` for default text mode — see [L011](L011-aop-handles-output-text.md).

## Cross-references

- The pull model this serves: [L004](L004-timeline-provider-pull-only.md).
- The provider that reads the ops-log: [L010](L010-ops-log-provider.md).
- The AOP hook implementation detail: [L011](L011-aop-handles-output-text.md), [R006](R006-ops-log-surfacing.md).
- The local alternative that fills in the granularity gap: [L008](L008-local-provider-degraded-granularity.md).