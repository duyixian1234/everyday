# ADR F016: Daemon Sync Scheduler — periodic pull as a resident process

**Status**: Accepted

**Date**: 2026-08-13

**Deciders**: Everyday Architecture Review (grill-with-docs session)

---

## Context

The user's real pain: sync is manual/command-triggered only (`mail list --sync`,
`timeline --sync`). Data does not stay fresh automatically; an AI agent querying
`timeline` / `search` / `mail list` may read stale local caches. The dominant
gap is Timeline: by [L005](L005-no-auto-sync.md) queries never auto-sync and
read SQLite only, so without an explicit `timeline sync` the event log reflects
the last manual sync. `mail list` has its own 15-minute staleness rule
([M005](M005-staleness-auto-sync.md)), but Timeline and cross-module `search`
have no automatic freshness mechanism at all.

Two hard constraints govern any solution:

- **[L005](L005-no-auto-sync.md)** — queries never trigger sync; query path is
  SQLite-only, millisecond-level. This rule must not be weakened.
- **[D003](D003-auto-sync-cli-boundary.md)** — everyday is a short-lived CLI
  process; `tokio::spawn` work is not guaranteed to finish when `main` returns;
  there is no true fire-and-forget. D003 explicitly reserved the upgrade path:
  *"未来若引入常驻进程（daemon / 系统托盘），可升级为真后台推送，不破坏本 ADR 的显式语义。"*

User decision Q12: do **not** ship SCM/launchd/systemd integration code.
Guide existing solutions in docs instead (Windows nssm / Task Scheduler,
macOS launchd plist, Linux systemd unit).

## Decision

Introduce `everyday daemon` — a resident process that is the **only role
allowed to pull periodically**. Query semantics are unchanged whether or not
the daemon runs ([L005] holds with no exception).

### CLI shape

- `everyday daemon run [--once] [--sources mail,rss]` — foreground resident
  (or one-cycle mode). No `start`/`stop`/daemonize: the OS service manager
  (nssm / launchd / systemd) supervises the foreground process.
- `everyday daemon status [--json]` — running? / enabled? / last cycle /
  per-source results, from the state file + live PID probe.
- Registered via `ModuleRegistry` (new module `src/modules/daemon/`);
  `tests/cli_contract.rs` TOP_LEVEL_COMMANDS + MODULE_ACTIONS updated.

### Configuration `[daemon]`

```toml
[daemon]
enabled = true          # false → `daemon run` errors out (exit 1)
interval_seconds = 900  # sleep after a cycle completes (no tick catch-up)
sources = []            # empty = all; whitelist e.g. ["mail","rss"]
```

All fields `#[serde(default)]` — backward compatible; `config.example.toml`
updated. `enabled` is the "should this be resident" switch; a service manager
restart loop must not spin an empty process, so `run` refuses when disabled.

### Sync Cycle (one periodic cycle, three actions, sequential)

| Action | What | Reuse |
|---|---|---|
| timeline | `orchestrator::run_sync` (all sources / sources-filtered; join_all parallel; best-effort per-provider watermarks, [L009]) | existing |
| mail cache | IMAP LIST → **all server folders** (no exclusion, incl. Sent/Trash/Junk/Drafts) incremental `UID max_uid+1:*` per folder; LIST each cycle for folder discovery | extend email_cache sync entry |
| rss | pull feeds into rss-items.db | reuse rss fetch internals |

- Local providers (todo/note/bookmark) cost ~0 and always run inside the
  timeline action.
- Cycle = run all three actions, then `sleep(interval_seconds)` — *sleep
  after completion* (no `tokio::time::interval` catch-up; IMAP rate-limit
  safety, see Alternatives).
- Failure policy: keep going (best-effort, [L009] spirit); failures recorded
  in state file + logs; retried next cycle. No backoff in v0.17.
- `sources` whitelist maps uniformly: a source in the list turns on its
  timeline provider **and** its cache action (mail → timeline mail provider +
  mail cache sync; rss → timeline rss provider + rss fetch).

### State file `~/.config/everyday/daemon-state.json`

```json
{
  "pid": 12345, "running": true, "enabled": true, "interval_seconds": 900,
  "started_at": "…", "last_cycle_at": "…", "cycles": 1, "last_cycle_ok": true,
  "exit_at": null, "exit_ok": null,
  "sources": {
    "timeline": { "ok": true, "events": 12, "error": null },
    "mail":     { "ok": true, "folders": 8, "envelopes": 34, "error": null },
    "rss":      { "ok": true, "items": 5, "error": null }
  }
}
```

- Written: at startup / after each cycle / at exit (`running=false`,
  `exit_at`/`exit_ok` filled; last cycle's `sources` retained for
  "how fresh is my data" lookups).
- `status` "running" = PID liveness probe (Unix `kill(pid,0)`, Windows
  process probe); dead PID ⇒ stopped even if the file still says `running`.
- Re-entry guard: `run` errors out (exit 1) if a live PID is already present.

### Logging & output contract (R001)

- Resident `run`: stdout **fully silent** (a resident process has no
  "command result"; same as `mcp serve`). stderr follows the existing
  level ladder (default WARN quiet; `-v`/`-vv`).
- `--once`: stdout emits the sync summary (aligned with `timeline sync`
  output shape; JSON carries provider stats) — it is a finite command.
- File log `~/.config/everyday/daemon.log`: fixed **INFO** (independent of
  `-v`), append-only, no rotation (WARN-default volume is tiny); documented
  manual cleanup. Implemented by extending `Sink`/`EverydayLayer` in
  `src/util/logging.rs` with a file writer.
- State-file write failure on the exit path → `_error` + exit 1 (does not
  block sync itself).

### Graceful shutdown — single unified path

`graceful_shutdown()` is the one exit path: `--once` completion,
SIGINT/SIGTERM (Unix), Ctrl+C (Windows) all converge on it — write final
state → close file log → exit 0. Integration tests exercise the full path
via `--once`; the signal→path trigger hop is covered by unit tests
(injected signal future). No `EVERYDAY_DAEMON_MAX_CYCLES`-style test hooks
in the CLI surface (user decision, Q8=b).

## Alternatives considered

- **Extend per-module staleness auto-sync** (like M005) to all modules:
  doesn't cover Timeline (`search`/`timeline` freshness is the core pain);
  per-module timers drift apart and there is no unified status view.
- **OS scheduler (cron / Task Scheduler / launchd) invoking `timeline sync`**:
  non-portable per platform, no daemon status file, and each invocation pays
  full cold-start — acceptable as a fallback, but the resident process gives
  one status source and one log.
- **Staleness-triggered sync on query**: already rejected by
  [L005](L005-no-auto-sync.md) (unpredictable latency, rate-limit risk).
  Not revisited.
- **True daemonize (`fork` + detach)**: unreliable on Windows; the OS
  service manager already supervises foreground processes — rejected.
- **`tokio::time::interval` fixed ticks**: tick catch-up when a cycle runs
  longer than the interval risks back-to-back IMAP syncs (rate limits);
  sleep-after-completion gives a stable cadence — chosen.
- **`EVERYDAY_DAEMON_MAX_CYCLES` test hook**: user rejected; `--once` +
  injected signal/interval unit tests cover the same ground without polluting
  the CLI surface.

## Consequences

- Data freshness ≤ `max(interval_seconds, per-source cycle cost)` while the
  daemon runs; `timeline today` / `search` / `mail list` read fresh caches
  without manual `--sync`.
- `mail list` keeps its own 15-minute staleness rule; the daemon additionally
  refreshes all server folders (Sent/Trash/Junk/Drafts included) — a superset
  of `mail list --sync`'s target set.
- Periodic network traffic on every cycle (IMAP / CalDAV / RSS) at the
  configured cadence (default 15 min); users on metered links can raise
  `interval_seconds` or set `enabled = false`.
- L005/D003 untouched: query path never syncs; the CLI's explicit-sync
  semantics are unchanged. D003's reserved "resident process" upgrade path
  is now exercised for pull; a future push side (auto_sync as true
  background) can build on the same skeleton.
- `cli_contract.rs`, `config.example.toml`, README, and the everyday-cli
  skill reference must be updated in the same change.
- Docs: `docs/daemon.md` ships nssm / launchd plist / systemd unit examples
  (no SCM code in-repo, per Q12).

## Cross-references

- Query/sync separation preserved: [L005](L005-no-auto-sync.md).
- CLI process boundary & the reserved upgrade path: [D003](D003-auto-sync-cli-boundary.md).
- The orchestrator the timeline action reuses: [L009](L009-best-effort-sync.md).
- Mail staleness layer (unchanged): [M005](M005-staleness-auto-sync.md).
- Leveled logging reused for daemon stderr/file: [F015](F015-leveled-logging-tracing.md).
- CLI contract tests locked by the change: [G001](G001-quality-tools-suite.md).
