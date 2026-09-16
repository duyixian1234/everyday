# ADR K005: Deprecate the memory module — frozen, export-then-remove

**Status:** Accepted
**Date:** 2026-09-16

## Context

The `memory` module shipped in v0.10.0 ([K001](K001-memory-module.md)) as the agent's own structured notebook: an append-only `(subject, predicate, object)` triple store at `~/.config/everyday/memory.db`, single global instance, no account column ([K004](K004-memory-single-instance.md)), participating in unified search via a `Searchable` provider ([K003](K003-memory-searchable.md)) and offering forward BFS traversal ([K002](K002-memory-graph-query.md)).

It never found a real consumer. The module's premise was that an agent would persist stable facts about the user, projects, and the world, then recall and refine them. In practice the agent's own context window and the project's `note` module cover the same ground, and the triple abstraction added a vocabulary decision ("is this `requires` or `needs`?") in front of every write without a corresponding payoff at read time. The `confidence` and `source` fields — the F003 justification for the module ([K001](K001-memory-module.md) §Context) — were rarely populated by callers.

The database, however, holds real data: `history` preserves every superseded version, and `delete` is a soft delete, so the current 1,406-row file contains facts that are **not** visible through any current-state query. Deleting the module outright would destroy that history with no way to retrieve it.

**Decision (2026-09-16): freeze the `memory` module and schedule its removal for a later major version.** Ship one new action, `memory export`, as the data escape hatch. Do not add features, do not fix non-data-loss bugs, do not extend the schema.

This is a **scoping** decision, not a technical failure: the implementation is sound and tested. It is the same shape of call as [R019](R019-remove-notion-provider.md) — a capability the project owner no longer uses, whose maintenance surface is no longer justified.

## Decision

### What "frozen" means

| Aspect | State |
|---|---|
| The seven existing actions | **Keep working.** `add` / `get` / `relation` / `list` / `delete` / `graph` / `history` are unchanged |
| Registry / MCP / Search registration | **Unchanged.** The module stays registered, stays projected as MCP tools, keeps its `Searchable` provider |
| Runtime deprecation warning | **None.** No `_warning` line, no stderr notice |
| New features / schema changes | **None.** The v1 "deferred to v2" list ([K001](K001-memory-module.md) §Deferred to v2) is cancelled, not deferred |
| Bug fixes | **Data-loss only.** A fix that prevents silent data loss is in scope; anything else is not |
| Data migration out | `everyday memory export` (new) |

The decision *not* to emit a runtime deprecation warning is deliberate. `memory` is consumed by agents through `--json`, and per the logging contract a warning rides along in JSON mode as a `_warning` key (`src/util/logging.rs`). Injecting that into every response of a working, frozen module is noise that a strict parser must then handle — for a sunset that the docs already announce.

### Scope — the new `export` action

- **Semantics**: lossless dump of the entire `memory` table. It reads the base table, **not** `current_state_view`, so superseded versions and soft-deleted rows are included. This is the property that makes it an escape hatch rather than a query: a consumer can reconstruct the current-state view from the dump, but not the reverse.
- **Envelope**: `{"export_version": 1, "exported_at": "<RFC3339>", "count": N, "facts": [...]}`, each fact carrying all 8 columns (`deleted_at` emitted as `null` for live rows rather than omitted). `export_version` is the load-bearing field — without it a future reader cannot tell which shape an archived document has.
- **Output**: always JSON, **ignoring the global `--json` flag**. The action's contract *is* "produce a JSON document", so `everyday memory export > mem.json` works without remembering the flag. This is a deliberate, documented exception to the global-flag convention.
- **Ordering**: `ORDER BY id`. `id` is a date-sequence id and therefore a **total order**, so the output is byte-reproducible. `created_at` (used by `list` / `history`) has millisecond precision and can collide for rows inserted in the same millisecond, leaving relative order undefined — unacceptable for a document meant to be archived and diffed.
- **Empty database**: `count: 0` with an empty array, exit code 0. This follows K001's empty-state semantics (`get` / `relation` / `list` / `graph` return empty results rather than erroring) and matters because export's likely consumer is an automated "back up my notebook" step, which should not fail on a notebook that was never written to.
- **Registration**: because `module_arg_spec()` is the single source of truth for both the clap CLI and MCP projection, the action appears as MCP tool `memory_export` with no edits to `src/modules/mcp/tool_registry.rs`. It is correctly **not** marked `[WRITE]`: `sync::is_write_action` is a positive allowlist of mutating actions, and `export` is read-only. `skills/everyday-cli/references/COMMANDS.md` documents it (required by `scripts/check_skill_drift.py`).

### What stays untouched

- **No removal now.** The module is frozen, not deleted. Its actions remain in `tests/cli_contract.rs`'s `MODULE_ACTIONS`, and `memory` stays in `TOP_LEVEL_COMMANDS`.
- **K001–K004 are revised, not superseded.** All four still **describe live code** — the schema, the graph semantics, the Searchable adapter, the single-instance storage are all exactly as documented. Marking them `Superseded by` would be factually false today; it says "this is no longer how it works", which is not true while the module runs. They carry a revised status and an `> **Update (…):**` banner pointing here.
- **`skills/everyday-cli/references/MEMORY.md` is kept**, with a deprecation banner. Its subject-naming convention is the *only* mechanism providing multi-agent isolation ([K004](K004-memory-single-instance.md) §Decision); deleting it while the module is live would remove that guidance. It is deleted when the module is.

### Removal (future, separate ADR)

Removal is a **breaking change** — `tests/cli_contract.rs` states outright that removing a module requires a major bump, an ADR, and a contract-list update. It lands in a later major version via its own ADR; this one records the direction and the escape hatch, and deliberately commits to **no version number**.

When removal happens, these are the known tripwires (identified during this change, so the removal ADR does not have to rediscover them):

| Site | Why it breaks on removal |
|---|---|
| `src/modules/mod.rs` — `health_check_all` sort key | Special-cases `"memory" => (2, *name)` to force it last; the arm and its doc comment die with the module |
| `src/modules/mod.rs` — `health_check_all_orders_config_first_memory_last` | Asserts `rows.last()` is `memory`; must be rewritten or deleted |
| `src/modules/search.rs` — `build_registry_empty_config` | Asserts the registry is non-empty *because* memory is the only unconditional provider; removing it empties the registry under an empty config |
| `src/modules/mcp/tool_registry.rs` | Test asserts `memory_add` carries `[WRITE]` |
| `src/cli.rs` — `long_about` | Module list literal contains `memory` |
| `src/modules/sync/` | `memory.db` is a first-class member of the sync namespace (`manifest`, `resolve_remote_target`) and `is_write_action` names `memory add`/`delete`; `memory.db` would need a documented fate (orphaned file, or migrate-then-drop) |
| `skills/everyday-cli/references/MEMORY.md` | Deleted; it is linked from `SKILL.md` and `COMMANDS.md`, so those links must go too or `just check-links` fails |
| `CONTEXT.md` §Memory | ~34 mentions across 95 lines; the glossary entry is rewritten per the R019 precedent (describe current state, append `（… 已于 vX.Y.Z 移除，见 [K005](…)。）`) |

## Alternatives considered

### Remove the module now
- Smallest long-term surface; no frozen code to reason about.
- Rejected: destroys superseded versions and soft-deleted rows, which no current-state query can surface. The whole reason `export` exists is that this data is currently unobtainable by any other means.

### Keep the module and maintain it
- No migration burden; the code already works.
- Rejected: the module has no consumer. Maintaining a vocabulary-bearing abstraction that nobody writes to is cost without benefit, and it is the same judgment [R019](R019-remove-notion-provider.md) made about the Notion provider.

### Deprecate without providing `export`
- Smaller diff; no new action to document or keep working.
- Rejected: the freeze would be a trap. A user (or agent) reading "deprecated" has no way to get their data out before removal, and the removal ADR would have to invent one anyway — under time pressure, after users have already lost the ability to plan.

### Emit a runtime deprecation warning on every `memory` invocation
- Makes the sunset impossible to miss.
- Rejected: `memory` is called by agents in `--json` mode, where warnings surface as a `_warning` key in the response body. That pollutes every call of a module that still works correctly, for information the docs and `export`'s own help text already carry. Deprecation is announced where a reader looks for it, not where a parser trips over it.

### Make `export` respect the global `--json` flag
- Consistent with every other action.
- Rejected: it makes the primary use — capture the document to a file — silently produce a text-mode rendering nothing can parse. The action has exactly one output format, so the flag has nothing to select between.

### Order the dump by `created_at DESC` for consistency with `list` / `history`
- Consistent with the other read actions.
- Rejected: consistency buys nothing in an archive, and costs reproducibility. `created_at` collisions (same-millisecond inserts) leave the relative order of those rows undefined, so two exports of an unchanged database could differ. `ORDER BY id` is a total order.

### Version the module's removal as v1.0.0
- Gives users a concrete deadline to plan around.
- Rejected: this project versions at 0.x and has sat at 0.18.x for many releases; a 1.0.0 trigger may be arbitrarily far away, which makes the number meaningless while still being a commitment that a future decision must honor or visibly break. The direction is recorded; the version is not.

### Introduce a new `Deprecated` ADR status
- Expresses the state precisely.
- Rejected: the repo's status vocabulary is exactly `Accepted` / `Accepted (revised <date> by [X](…) — <clause>)` / `Accepted (amended)` / `Superseded by [X](…) (<date>, <reason>)`, documented in [docs/adr/README.md](README.md). `Accepted` is accurate here — this decision has been accepted and is being executed (the freeze is in effect, `export` ships with it); "removal later" is a consequence, not a different status. Adding a status for one module-level decision would expand global vocabulary out of proportion.

## Consequences

- `memory` has 8 actions instead of 7. The CLI contract test locks the new one; the other 7 are unchanged.
- Deprecation is **documented, not enforced**. Nothing in the program prevents continued use, and an agent that never reads the docs will not learn the module is frozen. This is the accepted cost of not emitting runtime warnings.
- Archived export documents are self-describing via `export_version`, but there is **no `import`**. Recovering from a dump is a manual exercise (or a future tool); the ADR records this so the gap is not a surprise during removal.
- `memory.db` continues to grow; the freeze means the monotonically-growing-file note in [K001](K001-memory-module.md) §Consequences still stands, and the deferred `cleanup` action stays cancelled.
- The four K-series ADRs read as live design documents with a banner explaining that the module they describe is frozen. A reader who follows only K001 will find the banner; one who reads only `CONTEXT.md` will find the same note there.
- Removal will be a breaking change requiring a major bump, and the tripwire table above makes that ADR a checklist rather than an investigation.
- The `sync` namespace still carries `memory.db`. If a user removes their WebDAV sync account before the module is removed, that file is simply never read again (no migration, no deletion) — the same harmless-orphan outcome R019 left for `ops-log.db`.

## Cross-references

- Revised: [K001](K001-memory-module.md), [K002](K002-memory-graph-query.md), [K003](K003-memory-searchable.md), [K004](K004-memory-single-instance.md) — all four stay Accepted; they describe live code, and now carry a banner pointing here.
- Precedent: [R019](R019-remove-notion-provider.md) — the repo's other removal ADR, and the model for this one's shape. R019 removed a provider outright; K005 freezes a module and defers its removal.
- Related: [F003](F003-module-scope-external-integration.md) — the module-scope gate K001 originally passed. The "confidence / source are first-class, which shell SQL cannot express" justification did not survive contact with real usage; that is an input to this decision.
- Related: [F001](F001-cli-shape.md) — `Executor` trait, `AgentError`, `Output`; the global `--json` convention that `export` deliberately excepts itself from.
- Related: [F014](F014-mcp-module.md) — the MCP projection that derives `memory_export` automatically from `module_arg_spec()`.
- Related: [D001](D001-webdav-file-sync.md) — `memory.db` as a sync-namespace member; relevant to the removal tripwire table.
