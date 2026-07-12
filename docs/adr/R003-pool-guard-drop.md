# ADR R003: PoolGuard::Drop must guard `tokio::spawn` with `Handle::try_current()`

**Status:** Accepted
**Date:** 2026-07-11

## Context

The IMAP connection pool ([M002](M002-imap-connection-pool.md)) uses a `PoolGuard` RAII type that returns its session to the pool when dropped. Returning the session means enqueueing it on a `tokio::sync::Mutex` held inside an async task — i.e. `tokio::spawn`.

The naive implementation:

```rust
impl Drop for PoolGuard<'_> {
    fn drop(&mut self) {
        tokio::spawn(async move {
            self.pool.lock().await.push_back(self.session);
        });
    }
}
```

This panics when the runtime is already shutting down (e.g. the user hits Ctrl-C, or the only task completes and `Runtime::drop` starts tearing down worker threads). `tokio::spawn` with no live runtime panics with `there is no reactor running, must be called from the context of a Tokio 1.x runtime`.

That panic fires on the *drop glue*, not on the user's command — and it can happen after the user's `Output` has been written. The exit code becomes non-zero, the agent sees a clean response followed by a process crash, and the cause is invisible.

## Decision

**`PoolGuard::Drop` must check for a live runtime before calling `tokio::spawn`.**

```rust
impl Drop for PoolGuard<'_> {
    fn drop(&mut self) {
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                self.pool.lock().await.push_back(self.session);
            });
        } else {
            // Runtime is down — leak the session. The pool's existing
            // M-allocated limit holds; next list creates fresh sessions.
            // (Alternative: synchronously push back via try_lock, but
            // try_lock can fail under contention; leak is acceptable.)
        }
    }
}
```

The fix is one of the "panic"-category items from the 2026-07-11/12 review — see commit `7a30cd5`.

## Alternatives considered

### Synchronously push back via `try_lock`

- No spawn needed; works during shutdown.
- `try_lock` can fail under contention; the session is then dropped, which closes the IMAP connection. Acceptable.
- Considered; the leak path was kept as the simpler choice since shutdown is rare and the pool re-establishes on next list.

### Wrap the entire CLI in a long-lived runtime that lives longer than user code

- Forces a single-runtime model.
- Incompatible with future use of the pool inside a library that may be called from a different runtime.
- Rejected.

### Use `tokio::task::block_in_place` to push back synchronously

- Requires the multi-thread runtime feature and `Send` bounds.
- Heavier than needed.
- Rejected.

### Move pool return into `Drop` for `&mut self` only; never on `&self`

- Doesn't help — `Drop` always takes `&mut self`.
- Rejected: misread the problem.

## Consequences

- `PoolGuard::Drop` is panic-free under all shutdown paths.
- During normal operation the behavior is unchanged: the session returns to the pool asynchronously.
- During shutdown the session leaks; next `mail list` rebuilds the pool from scratch.
- A test asserts that calling `drop` on a `PoolGuard` after the runtime has exited does not panic.

## Cross-references

- The pool this protects: [M002](M002-imap-connection-pool.md).
- The "no `unwrap` / no panic" project rule: `agents.md` §"错误处理".
- The companion fix for DST-boundary parsing (also a panic-on-unwrap pattern): [R004](R004-dst-boundary-dates.md).