# ADR R022: Unified comment-preserving config editor (ConfigEditor)

**Status**: Accepted
**Date**: 2026-08-18

**Deciders**: Everyday Architecture Review (improve-codebase-architecture + grill-with-docs session)

---

## Context

Everyday mutates `config.toml` from two places with **different lossiness guarantees**:

- `config set <dotted.path> <value>` — reads the file into a `toml::Value`, upserts a dotted path, then re-serialises the **whole** file with `toml::to_string_pretty`, dropping hand-written comments.
- `task add` / `task remove` — introduced in ADR [F017](F017-task-module.md) with `toml_edit` (comment-preserving), because users hand-write comments in `config.toml` and expect them to survive.

F017 §"Config write lossiness" explicitly flagged this divergence and called it out as a candidate for a future unification: *"a future change may upgrade `config set` to `toml_edit` as well."*

The cost of leaving it split is a confusing user-visible invariant: a user who keeps a commented `config.toml`, runs `task add` (comments survive), then later runs `config set` (comments die) sees silent data loss. The `config` module is also no longer the single writer of `config.toml` — the task module reaches into `Config::config_path()` and implements its own `toml_edit` traversal, bypassing the module whose job it is.

Three design tensions drive this ADR:

- **One write seam vs two**: the `config` module should own every mutation of `config.toml`, so "what a config write preserves" is one fact rather than two drifting behaviours.
- **`config set` generality vs task's typed shape**: `config set` sets arbitrary dotted paths (deep nesting, auto-extended arrays); `task add` inserts a strongly-typed `[tasks.<name>]` table. A shared seam must serve both without forcing either through the other's shape.
- **Write-time validation vs incremental setup**: `config set` currently writes any path with no semantic validation (deferring errors to the next `Config::load`); `task add` validates upfront (name regex, non-empty command, 5-field cron). Validating `config set` is desirable, but whole-document validation would reject legitimate multi-step incremental setup.

## Decision

### The `config` module is the single writer, via a `ConfigEditor`

Introduce a `ConfigEditor` in the config module (`src/modules/config/editor.rs`, or equivalent) that is the **only** code that writes `config.toml`. It wraps the file as a `toml_edit::DocumentMut` and exposes **method-level operations**; callers never touch `toml_edit` directly:

- `set_dotted(path, raw_value)` — the dotted-path upsert (deep table creation, array-index auto-extension, string→bool/int/float/string coercion) that `config set` uses today, rewritten against `toml_edit`.
- `insert_task(name, task)` / `remove_task(name)` — the typed `[tasks.<name>]` operations that `task add` / `task remove` use today.
- A shared internal `load → mutate → atomic write` pipe underneath all of them.

`config set` routes through `set_dotted`; `task add`/`remove` route through `insert_task`/`remove_task`. `src/modules/task/config_edit.rs` is removed; its logic folds into the editor.

### Comment preservation is the invariant for both paths

Because both writers share the `DocumentMut` round-trip, every config write preserves hand-written comments. The divergence in F017 is closed.

### Atomic writes (temp + rename)

Every editor mutation writes via temp-file + rename, matching the existing pattern in `daemon/state.rs` and `sync/state.rs` (`write temp → rename`). This closes the torn-write window that the daemon scheduler (which re-reads `config.toml` every 30 s) could otherwise observe mid-write.

### Per-path write-time validation

`config set` validates at write time only for paths with a **registered validator**; unknown paths pass through unchanged (preserving today's lenient behaviour for module configs and enabling incremental multi-step setup):

- `tasks.<name>` → `validate_task_config` (name regex, non-empty command, 5-field cron).
- `daemon.interval_seconds` → `>= 1`.
- The validator registry lives alongside the editor; adding a rule for a new path is a one-line registration.

### Read side surfaces the raw document (text mode)

`config list` in **text mode** renders the raw comment-preserving document instead of a `toml::to_string_pretty(&cfg)` re-serialisation. `config get <dotted.path>` walks the raw `DocumentMut`. The **JSON mode of `config list` keeps returning the parsed `Config` struct** — it is the structured data contract consumed by AI agents / the MCP projection and must not become raw TOML.

## Alternatives considered

- **Only extract a shared low-level pipe; leave `config set` lossy.** Minimal change, but the confusing invariant (task preserves, config set doesn't) survives; rejected — the whole point is one guarantee.
- **Whole-document `Config::validate()` on every `config set`.** Stronger, but rejects legitimate incremental setup (setting one account field at a time); rejected in favour of per-path validators.
- **`edit_document(f)` closure primitive instead of a `ConfigEditor` struct.** More general, but leaks `toml_edit` to every caller and gives up the chance to hide validation + atomicity behind method-level operations; rejected for a deeper interface.
- **`toml_edit` for `config set`, keep task's separate editor.** Leaves two `toml_edit` writers; rejected — the seam belongs in one place (the config module).
- **Non-atomic writes.** Simpler, but reopens the torn-write window the daemon can observe; rejected in favour of the repo's existing temp+rename pattern.

## Consequences

- `config set` and `task add`/`remove` now share one comment-preserving, atomic, validated write seam; the `config` module is again the sole writer of `config.toml`.
- `src/modules/task/config_edit.rs` (plus its tests) is deleted; the task module becomes a thin caller of the editor.
- `config set`'s dotted-upsert and coercion logic moves into the editor, rewritten against `toml_edit`; behaviour for unvalidated paths is unchanged.
- `config list` (text) / `config get` show the true file; `config list --json` remains the structured `Config` contract (no agent-visible change).
- The daemon scheduler's "tolerate a malformed mid-edit file" fallback becomes effectively dead (atomic writes never emit malformed files) but is harmless to keep.

## Cross-references

- [F017](F017-task-module.md): introduced `config_edit.rs` and the comment-preserving divergence this ADR closes.
- [R012](R012-config-executor-trait.md): `ConfigModule` goes through the `Executor` trait — the config module as the home of config-write logic.
- [R021](R021-date-sequence-id.md): adjacent task-module work shipped in v0.17.x.
- `daemon/state.rs` / `sync/state.rs`: the temp+rename atomic-write pattern reused here.
