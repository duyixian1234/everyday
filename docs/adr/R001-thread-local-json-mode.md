# ADR R001: Thread-local `is_json()` instead of `std::env::args()` scan

**Status:** Accepted
**Date:** 2026-07-11

## Context

The original `is_json()` helper walked `std::env::args()` on every call to decide whether the user passed `--json`. Three problems with that:

1. **Allocates per call.** Each invocation constructs a fresh `Args` iterator and scans it. Cheap individually, but `is_json()` is called from module hot paths (output rendering, AOP hook — see [L007](L007-notion-ops-log.md)).
2. **Subprocess pollution risk.** Some test harnesses or wrappers mutate `argv` after process start. `std::env::args()` reflects whatever the OS sees; a future middleware could surprise us.
3. **Subtle behavior under clap.** When `--json` is consumed by clap, `std::env::args()` still shows it; modules reading args post-parse would see both forms. The decision should be made **once**, at the dispatch boundary, and frozen for the rest of the request.

## Decision

**Compute `is_json()` once at `main.rs` dispatch, store it in a thread-local, and read it via `is_json()` everywhere else.**

```rust
thread_local! {
    static JSON_MODE: RefCell<bool> = const { RefCell::new(false) };
}

pub fn is_json() -> bool { JSON_MODE.with(|b| *b.borrow()) }
pub fn set_json_mode(on: bool) { JSON_MODE.with(|b| *b.set(on)); }
```

- `main.rs` is the single caller of `set_json_mode(...)` — set after parsing the global flag, before dispatching to the module.
- Modules call `is_json()` from rendering code; no module re-scans `argv`.
- The thread-local choice matches the dispatch model (one request, one thread, sometimes a `tokio::task::spawn_blocking` for blocking calls — both see the same value).
- The function pair lives in `src/json_mode.rs` (or equivalent) and is imported wherever needed.

## Alternatives considered

### Pass `RenderMode` explicitly through every call

- Most explicit; no hidden state.
- Verbose; every function that might render takes an extra parameter.
- Breaks the existing `Output` API which assumes the renderer knows the mode.
- Rejected.

### Read `std::env::args()` once at startup, store globally

- Effectively the same as thread-local.
- Thread-local is slightly more flexible (multiple threads in tests can opt in/out independently).
- Considered equivalent; thread-local chosen for the test ergonomics.

### Use a config file or env var

- Adds an env var (`EVERYDAY_JSON=1`).
- Two ways to flip JSON mode is worse than one.
- Rejected.

## Consequences

- `is_json()` is now an O(1) thread-local read.
- Modules can be tested in isolation: a test sets `set_json_mode(true)` once, runs any number of `is_json()` checks.
- The fix is one of the "risk"-category items from the 2026-07-11/12 review — see commit `f80c5f4`.
- Any future feature that needs JSON-mode awareness reads the same function; no scattered `std::env::args()` calls.

## Cross-references

- The `Output` enum this affects: [F001](F001-cli-shape.md).
- The dispatch path that calls `set_json_mode`: [F007](F007-clap-subcommand-tree.md).
- The AOP hook that depends on JSON mode being stable per request: [L011](L011-aop-handles-output-text.md).
- The JSON serialization failure contract: [R002](R002-output-json-failure.md).