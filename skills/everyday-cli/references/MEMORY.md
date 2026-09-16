# everyday CLI — Memory Notebook (agent's own)

> **⚠️ Deprecated / frozen (K005).** The `memory` module takes no new features and is
> scheduled for removal in a later major version. Everything below still works exactly
> as documented — read it as the semantics of a live, if frozen, module.
>
> **Before it is removed, export your data**: `everyday memory export > mem-backup.json`
> dumps the whole table losslessly (including superseded versions and soft-deleted rows,
> which no other action can retrieve). There is no `import`.
>
> Decision record: [K005](../../../docs/adr/K005-memory-module-deprecation.md).

Semantics and conventions for the `memory` module. Loaded on demand by the `everyday-cli` skill when persisting or recalling structured facts. Full command table, options, and output schema: [COMMANDS.md](COMMANDS.md) (Memory section).

`memory` is a single global instance (`~/.config/everyday/memory.db`, no `account` column, no `auth` module touch). It stores append-only `(subject, predicate, object)` triples with optional `--confidence` and `--source`. Re-adding the same triple creates a new version; `history` shows all versions including soft-deleted rows.

## Usage

```bash
# Record what the user prefers
everyday memory add user prefers rust --confidence 0.9 --source explicit --json

# Look up everything we know about the user
everyday memory get user --json
# → {"count":1,"facts":[{"id":"m...","subject":"user","predicate":"prefers","object":"rust","confidence":0.9,"source":"explicit","created_at":"..."}]}

# Filter by (subject, predicate)
everyday memory relation user prefers --json

# Multi-hop traversal — what does the user depend on, transitively?
everyday memory graph user --depth 2
# → user
#      +-- prefers --> rust
#      `-- works_on --> everyday

# Soft-delete a current fact (history keeps it)
everyday memory delete user prefers rust --json
everyday memory history user prefers rust --json    # includes deleted_at
everyday memory add user prefers go --json          # resurrection = a new version row

# Memory participates in cross-module search automatically
everyday search query "rust" --module memory --json

# Take everything out before the module is removed (lossless; no --json needed)
everyday memory export > mem-backup.json
```

## Subject naming convention

The program does not enforce a subject schema (no `[a-z][a-z0-9-]+` regex check, no vocabulary file). Conventions live here so multiple agents agree on the same vocabulary:

```
user                       # bare subject for the human user
project-everyday           # a project entity
tech:rust                  # domain-prefixed: a piece of technology knowledge
team:backend:alice         # hierarchical: team > sub-team > person
agent:self                 # agent's own self-description (rare)
```

Hierarchy is colon-delimited; agents that produce triples are expected to pick the right granularity. Cross-agent fact sharing works by default — two agents writing `(user, prefers, rust)` land in the same version history. Use `tech:rust` vs `tech:python` to avoid collisions on shared nouns.

## What memory is and isn't

- **Yes**: stable, structured, mostly-timeless facts ("user prefers rust", "project-everyday uses tokio").
- **No**: timestamped events (use `everyday timeline ...`).
- **No**: long prose (use `everyday note create` + `append`).
- **No**: free-form journal entries.

Decision rule: if there is a clear "moment T" at which a fact became true, it belongs in `timeline`. If it is a stable assertion that survives many days, it belongs in `memory`.

## Memory v2 deferred — cancelled

These were out of scope for v1 and will **never ship**: `undelete-by-id`, `search` (embedding-based), `merge`, `expire (TTL)`, `cleanup` (physical GC of soft-deleted rows), `stats`. The module is frozen (K005), so the deferral is now a cancellation, not a roadmap. Use `history` + `--include-deleted` for forensics, and `export` to take the data out.

## Exporting (before removal)

`everyday memory export` dumps the entire `memory` table — every version, including soft-deleted rows — as one JSON document:

```json
{ "export_version": 1, "exported_at": "…", "count": 1427, "facts": [ … ] }
```

It is the **only** action that surfaces superseded versions and soft-deleted rows in one shot; `get` / `list` see current state only, and `history` requires you to already know the exact `(S, P, O)`. It always emits JSON and ignores the global `--json` flag, so `> file.json` captures a valid document either way.

**Ordering is by `id`, which is NOT chronological** — date-sequence ids don't sort lexicographically (the hex PID segment is unpadded, and pre-R021 ids form a separate block). The order is stable, so repeated exports return identical `facts` (only `exported_at` differs) and the file is safe to diff or archive. To read facts in time order, sort on `created_at` yourself or use `history`.

There is **no `import`**. Keep the file; recovering from it is a manual exercise.
