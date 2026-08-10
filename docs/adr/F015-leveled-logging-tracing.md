# ADR F015: Leveled Logging via tracing — Default Quiet, `-v` Opt-in

**Status**: Accepted

**Date**: 2026-08-10

**Deciders**: Everyday Architecture Review (grill-with-docs session)

---

## Context

Everyday's stderr output is flat: 14 bare `eprintln!` sites, no level concept,
no logging framework. The dominant noise source is `LoggingMiddleware`
([F012](F012-architecture-deepening-phase.md), [F013](F013-request-context-explicit-parameter.md)):
it prints two lines per command (start + ok/error + elapsed) unconditionally
to stderr, and the auto-sync success notice fires on every write command. An
agent (`--json`) and an interactive user both pay that noise on every
invocation, and there is no way to filter it.

The stdout contract (R001 — command result is the only stdout payload; stderr
carries diagnostics) and the structured JSON shapes (`{"_log": ...}`,
`{"_warning": ...}`) are documented and agent-facing; any change must keep
them byte-compatible.

## Decision

Introduce `tracing` + `tracing-subscriber` and a level ladder controlled by a
new global `-v, --verbose` flag (`ArgAction::Count`, `global(true)`, mirroring
the existing `--json` pattern in [F007](F007-clap-subcommand-tree.md)):

- **0 `-v`** → `WARN`: warnings/errors visible, progress silenced (default).
- **`-v`** → `INFO`: middleware progress logs + info notices restored.
- **`-vv`** → `DEBUG`: reserved; no debug sites exist yet.

### Subscriber & layer

- A custom `tracing_subscriber::Layer` writes to stderr. Text mode renders
  the pre-tracing compact format (`[req] module action ok in 12ms`); JSON mode
  renders the exact `{"_log": ...}` shapes (start/ok/error/error_detail with
  `request_id`/`caller`/`module`/`action`/`elapsed_ms`/`message` fields).
- Only `everyday`-targeted events render (exact `everyday` or `everyday::`
  sub-targets); dependency-crate diagnostics (rmcp, hyper, …) stay silent,
  matching the pre-tracing behavior where nothing was set up for them.
- `LoggingMiddleware` stays in the middleware stack unconditionally; the level
  filter silences it. Gating is the subscriber's job, not the middleware's.
- The pre-parse fatal path (registry build failure before clap parse) keeps
  `eprintln!` — it cannot route through a subscriber installed after parsing,
  and errors are always visible anyway.

### Migration

All `eprintln!` sites migrate to `tracing` macros with explicit levels:
middleware before/after/on_error → `info!`; init failures, auto-sync failure,
search-provider failure, timeline-sync failure → `warn!` (visible by default);
mcp serve errors → `warn!`/`error!`; auto-sync success notice → `info!`
(follows `-v`).

## Alternatives considered

- **Gate the middleware behind a boolean flag** (no framework): smallest diff,
  but no level ladder for future debug logging and no single logging path —
  rejected, the whole point is one leveled path.
- **`tracing-subscriber` `fmt::Layer` with the built-in JSON formatter**: less
  code, but its JSON shape (timestamp/level/target) breaks the documented
  `{"_log"}`/`{"_warning"}` contract — rejected.
- **Env-var control (`EVERYDAY_LOG` / `RUST_LOG`)**: useful, but out of scope —
  the request was explicitly `-v`; env integration is a later, additive step.

## Consequences

- Default stderr is quiet: agents' stderr capture is clean; warnings and
  errors remain visible.
- `-v` restores progress logs with the identical text/JSON shapes — no
  consumer-visible break for anyone who already relied on stderr lines.
- One logging path exists; future debug sites get level filtering for free.
- Contract tests (Layer unit tests + binary-level integration tests) lock the
  shapes so a future refactor cannot silently change them (F010).
- The `message` field of the error path must not collide with tracing's
  implicit event-message field — events are emitted field-only, no format
  strings (documented in `LoggingMiddleware`).
