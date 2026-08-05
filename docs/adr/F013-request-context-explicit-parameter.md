# ADR: Explicit-Parameter Request Context (v0.12 breaking)

**Status**: Accepted

**Date**: 2026-08-05

**Deciders**: Everyday Architecture Review (codebase-design skill)

---

## Context

[F012](F012-architecture-deepening-phase.md) item P4 introduced a
`RequestContext { request_id, deadline, caller }` to enable tracing, deadline
enforcement, and permissions checking across the dispatch stack. It landed in
v0.11 **non-breakingly**: the context was installed into a thread-local once
per command by `main.rs` (`set_request_context` / `clear_request_context`),
and middleware/modules read it via `current()` / `request_id()` — the same
thread-local pattern as JSON mode ([R001](R001-thread-local-json-mode.md)).

The explicit-parameter form was explicitly deferred to v0.12 as a **breaking
change** (F012 P4 row; "explicit-parameter form (breaking) deferred to v0.12").

**Why the thread-local form is insufficient for v0.12+:**

1. **Concurrent front-ends.** A thread-local holds exactly one context per OS
   thread. A REPL, API layer, or batch tool that interleaves requests on one
   thread (or dispatches child tasks) cannot give each request its own context.
2. **Hidden dependency.** Consumers read global state instead of receiving the
   request; the trait signature hides what a dispatch actually depends on.
3. **Lifetime ambiguity.** `set`/`clear` bookkeeping in the dispatcher is easy
   to get wrong (leak into the next request, or lose context on early returns).

## Decision

Thread `&RequestContext` explicitly through the dispatch stack. Concretely:

- `Executor::execute(&self, action, args, ctx: &RequestContext)` — `ctx` is the
  final parameter. Modules that don't consume it name it `_ctx`.
- `Middleware::before/after/on_error` each take `ctx: &RequestContext` first.
- `run_with_middleware(middleware, ctx, module_name, module, action, args)`
  forwards `ctx` to every hook and to `module.execute`.
- `main.rs` builds one context per invocation
  (`RequestContext::cli(generate_request_id())`) and passes it into
  `run_with_middleware`.
- The thread-local mechanism is **removed** (`set_request_context`,
  `clear_request_context`, `current`, `request_id`, and the `CURRENT`
  thread-local). `generate_request_id()` and the `RequestContext` struct
  (with `cli()`) remain.
- `LoggingMiddleware` reads `request_id` and `caller` straight off the passed
  context (and now logs `caller` in JSON `_log` lines); `deadline` stays
  data-only — enforcement is the caller's job (a future deadline middleware
  may consume it).

This is the v0.12 breaking half of F012 P4. The v0.11 thread-local form is
superseded; `RequestContext` is now a parameter, never ambient state.

## Alternatives considered

### Alt 1: Keep the thread-local, add an explicit `execute_with_ctx` second method
Rejected. Two dispatch paths means modules must decide which to implement;
the REPL/API benefit requires the explicit path anyway, so the thread-local
becomes dead weight and a trap (silently wrong context in concurrent use).

### Alt 2: Owned `RequestContext` instead of `&`
Rejected. Every module would clone (or move and re-borrow) the context;
readers only need read access. `&` keeps the signature zero-cost and makes
"same context for the whole dispatch" obvious.

### Alt 3: Put `ctx` first in `execute`
Rejected on churn grounds. Appending as the final parameter makes the
signature diff for existing modules a single added argument; the argument
position carries no semantic weight.

## Consequences

### Positive

1. **REPL / API / batch front-ends become possible** — each dispatch carries
   its own context; no ambient state to corrupt.
2. **Honest interface.** `execute`'s dependencies are now visible in its
   signature; readers no longer need to know about a global.
3. **Middleware is self-contained.** Hooks receive the context they log, so
   `LoggingMiddleware` no longer reads a thread-local.
4. **Cleanup.** The `caller` field's `#[allow(dead_code)]` is removed (consumed
   by logging); `request_context.rs` loses the thread-local plumbing.

### Negative (Tradeoffs)

1. **Breaking change** for any custom `Executor` implementor: `execute` grows a
   `ctx: &RequestContext` parameter. Internal modules updated in the same
   commit; see the migration guide below.
2. **Mechanical churn** across all modules (one added parameter per impl).

### Migration guide (v0.12)

For a custom module implementing `Executor`:

```rust
// Before (v0.11)
async fn execute(&self, action: &str, args: &[String]) -> Result<Output> { ... }

// After (v0.12)
async fn execute(
    &self,
    action: &str,
    args: &[String],
    ctx: &RequestContext,
) -> Result<Output> { ... }
```

- Add `use everyday::shared::request_context::RequestContext;` (or path-qualify
  the type).
- If you don't need the context, name the parameter `_ctx`.
- Read context off the reference when you do need it: `ctx.request_id`,
  `ctx.deadline`, `ctx.caller`.
- The v0.11 helpers `request_context::set_request_context`,
  `clear_request_context`, `current`, and `request_id` were **removed** — there
  is no ambient context anymore. Middleware hooks likewise take `ctx` as their
  first argument.

## Related decisions

- [F012](F012-architecture-deepening-phase.md) — P4 design and the original
  v0.11 non-breaking thread-local form; this ADR implements the deferred
  explicit-parameter half.
- [R001](R001-thread-local-json-mode.md) — the thread-local JSON-mode pattern
  (kept; `json_mode` remains thread-local — it is process-wide, not
  per-request).
- [F001](F001-cli-shape.md) — the `Executor` trait this ADR modifies.
