# ADR R003: PoolGuard::Drop returns sessions synchronously

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

**`PoolGuard::Drop` synchronously locks the idle-session queue, returns the
session, releases the lock, and then notifies one waiter. It does not spawn an
async return task.**

This preserves panic-free runtime shutdown while ensuring that a waiter is
never awakened before the corresponding session is visible.

## Alternatives considered

### Asynchronously return via `tokio::spawn`

- Releasing a separate semaphore permit before the spawned task acquires the
  queue lock lets a waiter observe a false capacity signal.
- Rejected because mailboxes with more folders than sessions reliably enter
  this race.

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
- Session return and capacity publication are ordered: enqueue first, notify
  second.
- No session is leaked merely because the Tokio runtime is shutting down.
- A concurrent test checks that six waiters complete through a four-session
  queue without an exhaustion error.

## Cross-references

- The pool this protects: [M002](M002-imap-connection-pool.md).
- The "no `unwrap` / no panic" project rule: `agents.md` §"错误处理".
- The companion fix for DST-boundary parsing (also a panic-on-unwrap pattern): [R004](R004-dst-boundary-dates.md).