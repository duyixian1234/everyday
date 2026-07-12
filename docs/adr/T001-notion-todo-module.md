# ADR T001: Todo module — Notion API + shared notion-client (strongly-typed DTO)

**Status:** Accepted
**Date:** 2026-07-10

> **Update (2026-07-12):** Credential & `login` logic consolidated into the top-level `auth` module. This module's `login` subcommand is removed; `todo` now calls `auth::get_credential`. See [R013](R013-auth-module-consolidation.md) (and [R014](R014-auth-verify-opt-in.md) / [R015](R015-auth-credential-io.md)).

## Context

The `todo` module mirrors `note`'s shape but for tasks. Same Notion backend, similar account/keyring model, but the data model is different:

- Each todo has a **status** (Todo / In Progress / Done) and optional **due date** / **priority**.
- The Agent needs to flip status, list overdue items, and complete tasks — operations that `note` doesn't model.
- The Notion API returns pages as a loose JSON object; we need a strongly-typed DTO so `set_status`, `complete`, `start` don't string-match property names.

Three constraints:

1. **Reuse the shared Notion client** ([F004](F004-shared-notion-client.md)).
2. **Strong typing** of Notion's loose JSON into `TodoItem` / `TodoProperties` with `From` conversions.
3. **Same keyring convention** as `note` ([N001](N001-notion-note-module.md)) for symmetry.

## Decision

### Actions

```
everyday todo login                          # prompt for token, save to keyring
everyday todo init-db                        # create the Notion database "Todos" with Status / Due / Priority
everyday todo list   [--status S] [--tag T]  # list todos
everyday todo add    --title T [--prop K:V]  # create a todo
everyday todo start  <id>                    # status → In Progress
everyday todo complete <id>                  # status → Done
everyday todo delete <id>                    # see T002 — archive / physical delete
```

- `Status` is a Notion **select** property (not status), so it round-trips through the API cleanly. (An earlier implementation used a `status` property; Notion's filtering semantics differed enough to break filter queries — that fix is recorded in the v0.2.0 changelog.)
- Strong typing: `TodoItem { id, title, status, due, priority, tags }` plus a `NotionPage { id, properties: Map<String, NotionValue> }` wrapper and a `From` mapping between them. No string matching against Notion property names outside the `From` impl.
- The shared `NotionClient` handles auth headers, 429 backoff (one retry), and error mapping uniformly.

### Keyring convention

- Same as `note`: `service = "everyday/todo/<account>"`, `account = "token"`.

### Local provider

- Since [F005](F005-default-provider-local.md), the **default** provider is `local` (SQLite). Notion remains available via `provider = "notion"`.

### Build providers (notion + local)

- See [R011](R011-add-dual-providers-macro.md) — the `add_dual_providers!` macro ensures both providers can coexist per account.

## Alternatives considered

### Reuse note's notion helpers verbatim

- Considered early: a generic `notion_module` framework.
- Rejected: the DTOs are too different; "title only" doesn't work for todos with `Status`, `Due`, `Priority`. Forcing generic code would lose type safety.

### Hand-write JSON parsing for Notion responses

- Quick to start, painful to maintain. Notion's JSON nesting makes ad-hoc parsing fragile.
- Rejected.

### Implement status as a `status` property (Notion's newer type)

- More recent Notion API supports a dedicated `status` type with built-in states.
- Tempting but: select works with all Notion API versions and provides identical filtering semantics for our needs.
- Rejected: select is enough.

### Add due-date / priority to `note` instead

- They don't belong there.
- Rejected.

## Consequences

- A todo is a strongly-typed Rust value end-to-end; the Agent never sees raw Notion JSON.
- `complete` / `start` are single-property patches, not magic.
- Shared client means a Notion 429 fix benefits `todo` immediately.
- Default-local means new users get a fast offline todo backend without setting up Notion.
- Timeline's notion-source projection of todo events comes via the ops-log hook ([L007](L007-notion-ops-log.md)) for the Notion provider and via the local provider's `updated_at` column for the local provider.

## Cross-references

- The shared client: [F004](F004-shared-notion-client.md).
- The local default: [F005](F005-default-provider-local.md).
- The delete action: [T002](T002-todo-delete-action.md).
- The Timeline projection: [L007](L007-notion-ops-log.md), [L008](L008-local-provider-degraded-granularity.md).
- The dual-provider macro: [R011](R011-add-dual-providers-macro.md).
- The merged `NotionLocalAccount`: [R010](R010-notion-local-account.md).