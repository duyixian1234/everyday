# ADR F005: Default provider is local SQLite for note/todo/bookmark

**Status:** Accepted
**Date:** 2026-07-10

## Context

`note` / `todo` / `bookmark` originally shipped with a single Notion-backed implementation. A local SQLite alternative was added later to:

- Remove the network round-trip on every read for users who don't need cross-device sync.
- Give the Agent a low-latency, offline-capable backend that fits its "first-class local" mental model.
- Enable the timeline event layer ([L001](L001-append-only-event-log.md)–[L013](L013-from-explicit-error.md)) which needs millisecond pull semantics.

The new question: **which provider should be the default?**

## Decision

**`provider = "local"` (SQLite) is the default for `note`, `todo`, and `bookmark`. The Notion backend remains available as `provider = "notion"` for users who explicitly want it.**

Mechanics:

- The account record schema gains an optional `provider` field. Absent → defaults to `"local"`.
- Config example templates are updated to show `provider = "local"` for the primary account.
- The Notion path is still first-class: `provider = "notion"` is fully supported and the docs still cover login/init-db flows.
- Backward compat: existing `config.toml` files without `provider` upgrade to `"local"` on first read. Users on Notion must add `provider = "notion"` to keep their behavior.

This change shipped in v0.3.0.

## Alternatives considered

### Default to Notion, opt-in local

- Pro: doesn't surprise existing users.
- Con: every new user has to discover and configure the local path before they get the fast path.
- Con: timeline's millisecond pull story is harder to sell if the default account is still network-bound.
- Rejected.

### Drop Notion entirely

- Pro: one codebase, less to test.
- Con: removes the cross-device sync option that some users explicitly want.
- Con: contradicts "external integration interface" — Notion users would lose their backend.
- Rejected.

### Default to Notion, offer local as a hidden flag

- Pro: no breaking change.
- Con: surfaces the feature poorly; users never discover it.
- Rejected.

### Auto-detect based on credential availability

- "If a Notion token is in keyring, use Notion; else use local."
- Rejected: too magical, fails to express intent, and keyring entries can be stale.

## Consequences

- A user cloning the repo and running `everyday note add "hello"` writes to `~/.config/everyday/note.db` without ever touching the network.
- The docs and the config example now show local-first; the Notion section is preserved as an explicit alternative.
- Timeline's notion provider (see [L007](L007-notion-ops-log.md)) is unaffected: it reads from the ops-log, not from the `provider` field of the running account.
- Future modules (if any) that target both backends should follow this pattern: `provider = "local"` default, `provider = "<remote>"` opt-in.

## Cross-references

- The local provider's degraded event granularity: [L008](L008-local-provider-degraded-granularity.md).
- Shared Notion client that backs the `"notion"` provider: [F004](F004-shared-notion-client.md).
- Module-level provider integration: [N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md), [B001](B001-bookmark-dual-provider.md).