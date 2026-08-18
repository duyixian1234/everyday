# ADR F017: Task Module — user-defined command execution & cron scheduling

**Status**: Accepted
**Date**: 2026-08-18

**Deciders**: Everyday Architecture Review (grill-with-docs session)

---

## Context

everyday has no capability to execute user-defined commands: the codebase has no process-spawn precedent (the only process interaction is the daemon's PID liveness probe). The owner's workflow needs named commands — configured once in `config.toml`, run on demand or on a schedule, with a durable execution history.

Three design tensions drive this ADR:

- **R001 output contract**: stdout is the exclusive structured-result channel. A `task run` that executes a subprocess must decide where the child's stdout goes without breaking the contract for data-returning commands.
- **Scheduling precision vs the daemon cycle**: the owner chose **cron expressions** (Q14=b). Cron has minute granularity; the daemon's sync cycle sleeps `interval_seconds` (default 900) after completion ([F016]) — piggybacking task dispatch on that cycle cannot meet cron precision. The owner authorized daemon mechanism changes as needed.
- **Config write lossiness**: existing `config set` rewrites `config.toml` via `toml::to_string_pretty`, dropping hand-written comments. The owner chose `toml_edit` (Q9) so `task add` preserves comments.

## Decision

### Task configuration `[tasks.<name>]`

```toml
[tasks.deploy]
command = "deploy.sh"        # required: executable file or path, never a shell string
args = "--env prod"          # optional; whitespace-split into argv
allow_extra_args = false
timeout_secs = 60            # 0 = no timeout
capture_output = false       # manual runs only; scheduled runs always capture
schedule = ""                # optional cron; empty = manual only
```

- `Config` gains `pub tasks: HashMap<String, TaskConfig>` (`#[serde(default)]`) — `[tasks.<name>]` maps name → `TaskConfig`. Field naming follows the `[daemon] interval_seconds` convention.
- Name validation `^[A-Za-z0-9][A-Za-z0-9_-]*$` at `task add` and config validation.
- `task add` writes via `toml_edit` (comment-preserving) — a deliberate divergence from `config set`'s lossy rewrite; a future change may upgrade `config set` to `toml_edit` as well.

### CLI surface

- `everyday task add <name> --command <cmd> [--args <s>] [--allow-extra-args <bool>] [--timeout <secs>] [--capture-output <bool>] [--schedule <cron>]` — errors on duplicate name; validates name / command / cron upfront.
- `everyday task run <name> [-- extra...]` — dual-mode output (below); always records to SQLite.
- `everyday task list [--json]` / `everyday task remove <name>` (config only; execution history retained) / `everyday task history <name> [--json] [--limit N]`.
- Module registered per [F007] / [R012] (3 touch points: `modules/mod.rs` declaration, `ModuleRegistry::build`, `cli.rs` long_about); `tests/cli_contract.rs` TOP_LEVEL_COMMANDS + MODULE_ACTIONS updated.

### Execution semantics (no shell)

- `command` is spawned directly as the executable; `args` (configured and extra) whitespace-split into argv. **No shell, no injection surface.** Known v1 limitation: an argument containing spaces cannot be expressed.
- Extra args `-- extra...` append to the configured args iff `allow_extra_args=true`; otherwise the run is refused.
- Timeout default 60 s (`--timeout 0` = none). On timeout: kill the whole process tree (Windows `taskkill /T /F`, Unix process group), record `status=timeout`, everyday exits 124.
- `capture_output=true`: tee — stream live to the terminal AND persist stdout/stderr as separate columns (64 KiB truncation per stream with a truncation marker). `false`: stream only, not persisted.
- `task run` output contract: default **passthrough** — the child inherits stdio, everyday's exit code mirrors the child's (124 on timeout). This is the explicit R001 exception: an executor is not a data-returning command. With `--json`: child output is **captured only** (no live echo), and stdout emits a single `{"_result": {...}}` envelope — stdout is the exclusive structured-result channel. The run is recorded in both modes. Captured `stdout`/`stderr` fields decode child bytes as UTF-8 with a GBK fallback so Windows commands (`ipconfig`) don't produce U+FFFD mojibake in the JSON.

### SQLite storage

- Fixed `~/.config/everyday/task.db` (sqlx, single connection, `create_if_missing`, lazy `CREATE TABLE IF NOT EXISTS` — the bookmark pattern).
- `task_runs`: `id` TEXT PK (R021, prefix `tk`), `task_name`, `command`, `args` / `extra_args` / `resolved_args` (JSON arrays), `allow_extra_args`, `timeout_secs`, `capture_output`, `cwd`, `status` (`success` / `failed` / `timeout`), `exit_code` (NULL when timed out), `timed_out`, `stdout`, `stderr`, `started_at`, `duration_ms`; index `(task_name, started_at DESC)`. No auto-prune in v1.
- `task_schedule_state(task_name TEXT PK, next_due_at TEXT)` — next-due persistence, single source of truth, survives daemon restarts.

### Cron scheduling in the daemon

- Schedule syntax: standard 5-field cron (`min hour dom mon dow`), local time, parsed and validated at `task add` and config load via the **croner** crate (new dependency; chrono-native, actively maintained).
- **Scheduler loop** (the daemon mechanism change the owner authorized): a dedicated tokio task spawned by `daemon run`, independent of the sync cycle ([F016] loop untouched). Every 30 s it checks due tasks: for each task with `next_due <= now`, run at most once, then `next_due = cron.after(now)` — **no backlog** (windows missed while the daemon was down are skipped, never backfilled). Results go to `task.db` + `daemon.log`; the loop selects on the shared CancellationToken; graceful shutdown waits for an in-flight run up to its own timeout.
- Scheduled runs **always capture** output regardless of `capture_output` — there is no terminal to watch; output is the only observability beyond the exit code.
- Failure reporting: `status=failed` / `timeout` record + daemon log only. No notification channel exists in the daemon; mail/WeChat notification is a separate topic, out of scope.
- The scheduler runs tasks sequentially; it runs concurrently with the sync cycle. sqlx pool `max_connections(1)` serializes DB access between the two loops (no transactions held across await → no deadlock).
- `--once`: one scheduler pass executes before the cycle summary.
- Manual `task run` vs the scheduler: no lock in v1 — the same task may run twice concurrently; accepted and documented.

### Threat model

`config.toml` is now a code-execution surface. Mitigations: (1) no shell — the config expresses a fixed argv with no interpolation; (2) triggers are explicit (manual `run`, or the opt-in `schedule` field); (3) every execution is recorded. A compromised config still implies arbitrary command execution — inherent to the feature, documented rather than mitigated.

## Alternatives considered

- **Shell execution** (`cmd /c`, `sh -c`): flexible, but platform-divergent and injectable; rejected.
- **Piggyback task dispatch on the sync cycle**: zero daemon changes, but the 900 s default tick cannot meet cron's minute precision; rejected (owner authorized the loop).
- **Interval DSL (`"30m"` / `"daily@09:30"`) instead of cron**: zero new dependency; rejected by the owner (Q14=b) — cron chosen.
- **`config set`-style lossy rewrite for `task add`**: consistent but drops hand-written comments; rejected (Q9) — `toml_edit`.
- **Catch-up backlog on daemon restart**: runs every missed window (disaster for frequent schedules); rejected (Q15) — at most one run per due.
- **Failure notification (email via the existing SMTP code)**: valuable but a separate engineering track (SMTP wiring, dedup, config); deferred (Q18=a).
- **`cron` crate vs `croner`**: croner chosen — maintained, chrono-native, parse-time validation; the `cron` crate is older and unmaintained.
- **Per-task locks / overlap protection**: v1 accepts concurrent manual + scheduled runs of the same task; locking adds daemon-state complexity without a demonstrated need.

## Consequences

- everyday gains its first subprocess-execution capability; `config.toml` becomes a code-execution surface (threat model above).
- The daemon gains its first concurrent loop alongside the sync cycle; DB access serializes via the single-connection pool.
- New dependencies: `toml_edit`, `croner`.
- R001 gains one explicit exception (passthrough `task run`); `--json` keeps the contract — child output is captured into `_result`, never echoed to stderr.
- `cli_contract.rs`, `config.example.toml`, README, `docs/`, and the everyday-cli skill reference updated in the same change.
- `[daemon]` and `[tasks]` remain independent config sections; no new `daemon` fields required (scheduler cadence fixed at 30 s in v1).

## Cross-references

- [F016](F016-daemon-sync-scheduler.md): the sync cycle this ADR extends with a scheduler loop (cycle untouched).
- [R021](R021-date-sequence-id.md): execution-record ids (`tk` prefix).
- [R001](R001-thread-local-json-mode.md): output contract; `task run` passthrough exception.
- [F007](F007-clap-subcommand-tree.md) / [R012](R012-config-executor-trait.md): module registration and the `Executor` trait.
- [R019](R019-remove-notion-provider.md): precedent for config-shape changes with `#[serde(default)]`.
