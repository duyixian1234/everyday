# ADR K004: Memory single-instance storage — no account column

**Status:** Accepted
**Date:** 2026-07-14

## Context

Most existing modules support multi-account configurations:

- `mail` / `cal` — multi-account by design (work / personal)
- `note` / `todo` / `bookmark` — dual-provider (local SQLite default, Notion optional) × multi-account
- `timeline` — `account` is a first-class nullable schema column ([L003](L003-account-first-class-column.md))
- `rss` / `search` — single instance

The question for `memory`: should it follow the multi-account pattern, or be a single global instance?

The user's reasoning during design was that memory is "agent's own notebook" — the agent invoking `everyday memory add` is the sole writer in current CLI usage. Multi-account would add schema complexity without immediate use case.

## Decision

**`memory.db` is a single global instance** at `~/.config/everyday/memory.db`. **No `account` column** in the schema. **`auth` module is not touched** by memory.

| Decision | Rationale |
|---|---|
| No account column | Current CLI is single-agent per invocation; no concrete multi-agent scenario justifies the schema cost |
| No per-account config blocks | `~/.config/everyday/config.toml` does not gain a `[memory.accounts]` section |
| Multi-agent isolation by **subject naming convention** | Program does not enforce; documented in `skills/everyday-cli/SKILL.md` so agents agree on a convention |
| No `auth::get_credential` calls | Memory has no external service; credentials are not applicable |
| Single global `MemorySearchProvider` | Mirrors `rss` / `search` single-instance pattern (see [K003](K003-memory-searchable.md)) |

**Subject naming convention** (documented in `skills/everyday-cli/SKILL.md`, not enforced in code):

```
user                       # bare subject for the human user
project-everyday           # project entity
tech:rust                  # domain-prefixed: technology knowledge
agent:self                 # agent's own self-description (rare)
team:backend:alice         # hierarchical: team > sub-team > person
```

The convention is hierarchical by colons. Programs do not parse or validate the structure; agents agree on a shared vocabulary. This is more flexible than a schema-level account column because:

- An agent can name subjects at any granularity (per-user, per-project, per-team, per-domain).
- Cross-agent shared facts (`user`, `project-everyday`) live in the same table without artificial partition.
- Renaming a subject is just `delete old + add new`; no account-migration logic.

**Why not multi-account columns:**

- Schema cost: an additional TEXT NOT NULL column + indices + per-command account-filter flag — added to every query path.
- Config complexity: `[memory.accounts] [[memory.accounts.entries]]` sections, `[default_account]` entries, keyring service names like `everyday/memory/<account>` — boilerplate with no current consumer.
- F003 philosophy ([F003](F003-module-scope-external-integration.md)): account abstraction exists to model **external services** with credentials (IMAP, CalDAV, Notion). Memory has no external service.
- `auth` module consolidation ([R013](R013-auth-module-consolidation.md)): if memory ever needs accounts, the consolidation work already provides the framework. v1 deliberately defers this.

**Why no `auth` module touch:**

- Memory has no credentials. There is nothing to store in keyring.
- `auth login --module memory` does not exist in v1.
- `everyday memory` CLI never calls `auth::get_credential` or `auth::verify`.

## Alternatives considered

### Multi-account with explicit `--account` flag (rejected)

Mirror mail/cal: `[[memory.accounts]]` in config, `--account agent-a` flag on every command. Schema gains `account TEXT NOT NULL` column. Rejected because (a) no current consumer, (b) subject naming convention already provides flexible isolation, (c) F003 says account abstraction is for external services.

### Per-agent config directory isolation (rejected)

`~/.config/everyday/memory-{agent-name}.db`. Filesystem-level partitioning. Rejected because (a) the CLI doesn't have a notion of "current agent" yet, (b) backup/migration becomes harder, (c) cross-agent fact sharing is desired.

### Subject whitelist / vocabulary enforcement (rejected)

Validate that subjects match a regex like `^[a-z][a-z0-9-]+(:[a-z][a-z0-9-]+)*$`. Rejected because (a) the user explicitly said "memory 是由具体的 agent 发起的，不应在程序中限制" — convention lives in `skill.md`, not in code, (b) closed vocabularies inhibit agent evolution.

### Make memory call `auth::*` for future-proofing (rejected)

Even without credentials today, calling auth establishes the module's "external service" framing. Rejected because it would create dead code paths and confuse readers about memory's nature.

## Consequences

- Schema is minimal (8 columns, no `account`).
- `~/.config/everyday/memory.db` is the only memory file — `find ~/.config -name "memory*.db"` returns at most one path.
- Cross-agent fact sharing works by default: two agents writing `(user, prefers, rust)` write to the same row's version history.
- Multi-agent isolation is a convention, not a guarantee. If two agents have conflicting conventions, they may collide on the same subject. Mitigation: documented convention + agent-side discipline.
- `auth login --module memory` does not exist. If a user tries `everyday auth list` and looks for memory, it's absent — documented in `auth`'s help text.
- Future migration to multi-account is possible (add column + backfill default) but explicitly deferred to v2.

## Cross-references

- [F002](F002-multi-account-keyring.md) — multi-account convention (not adopted for memory)
- [F003](F003-module-scope-external-integration.md) — module scope; memory is local-only, no external integration
- [R013](R013-auth-module-consolidation.md) — `auth` module; memory does not call it
- [L003](L003-account-first-class-column.md) — `account` column precedent in timeline (not adopted for memory)
- [K001](K001-memory-module.md) — main memory module decision
- [K003](K003-memory-searchable.md) — single global `MemorySearchProvider` (follows from this ADR)