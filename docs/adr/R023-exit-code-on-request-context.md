# ADR R023: Exit code is a host concern on `RequestContext`, not an `Output` variant

**Status:** Accepted
**Date:** 2026-08-18

## Context

`task run` (ADR [F017](F017-task-module.md)) executes a subprocess and must make the CLI process exit with the child's status — `mirrored_exit_code` maps `timed_out → 124`, else `exit_code.unwrap_or(1)`. The first implementation carried this through a new `Output::ExitCode { output, code }` variant wrapping the renderable value.

That shape leaks a process concern into the wrong layer. `Output` is the value every module returns to the host, rendered identically whether the front-end is the CLI or the MCP server ([F014](F014-mcp-module.md)). But only the CLI has a process exit. `finalize` was the sole consumer of the exit code, and the MCP call path renders `Output` directly (`tool_registry.rs`) and silently dropped the wrapped code. `Output::ExitCode` was a **shallow variant** — one constructor (`task/mod.rs`) and one consumer (`finalize`) — that made the renderable value type know about process-exit semantics.

## Decision

Remove `Output::ExitCode` and `with_exit_code()`. The exit code is announced by the module on the **`RequestContext`** and read at the CLI boundary:

- `RequestContext` gains an optional exit-code slot backed by `std::sync::OnceLock<i32>` — interior mutability so the context keeps flowing as `&` through the middleware chain and `Executor::execute` while staying `Send + Sync` (required by the boxed `+ Send` future).
  - `set_exit_code(&self, code)` announces an explicit code; first-wins.
  - `exit_code(&self) -> Option<i32>` / `effective_exit_code(&self) -> i32` (`unwrap_or(0)`) read it.
- `task run` calls `ctx.set_exit_code(mirrored_exit_code(&record))` instead of wrapping its `Output`.
- `finalize(result, ctx, mode)` reads `ctx.effective_exit_code()` for the `Ok` case; `main.rs` passes `&ctx`.
- MCP is unaffected: it renders `Output` directly and never reads the slot, so an agent calling `task_run` continues to read `status` / `exit_code` from the `_result` envelope.

`Output` now carries only the value; process-exit policy is the host's concern.

## Alternatives considered

- **Keep `Output::ExitCode { output, code }`.** Rejected: makes the renderable value type know about process exit; the code is silently dropped on the MCP path; one constructor + one consumer is a shallow variant (interface nearly as complex as the implementation).
- **Optional `exit_code: Option<i32>` field on `Output`.** Rejected: still pushes a process concern onto the value type, and requires `Option` bookkeeping on every `Output` construction rather than an opt-in channel.
- **Widen `Executor::execute` to return `Result<(Output, Option<i32>)>`.** Rejected: touches all 17 executor impls and the middleware signature — maximum blast radius for a one-caller need.
- **`RequestContext` as `&mut`.** Rejected: interior mutability via `OnceLock` keeps the signature `&` across all executors with zero signature changes; `Cell` was rejected because it is `!Sync` and would break the `+ Send` boxed future.

## Consequences

- `Output` returns to being purely a value type; the exit-code concern lives at the CLI boundary where a process actually exits.
- MCP's `task_run` behavior is unchanged and no longer silently "loses" a wrapped code — there is nothing to lose.
- `RequestContext` gains its first mutable policy channel, documented as such; other front-ends (REPL/API, future) read it if they have a process exit, ignore it otherwise.
- The `output.rs` test that asserted the old variant becomes a `RequestContext`-based test (`announced_exit_code_is_used_and_output_preserved`); the `runner` tests for `mirrored_exit_code` are unchanged.

## Cross-references

- [F001](F001-cli-shape.md): `Output` as the module result contract.
- [F017](F017-task-module.md): `task run` passthrough + exit-code mirroring (the original motivation).
- [F014](F014-mcp-module.md): MCP renders `Output` directly; no process exit.
- [F013](F013-request-context-explicit-parameter.md): `RequestContext` threaded through `Executor::execute` + middleware.
- [R001](R001-thread-local-json-mode.md): `--json` output contract.
