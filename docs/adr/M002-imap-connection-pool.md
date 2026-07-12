# ADR M002: IMAP connection pool M=4 with semaphore

**Status:** Accepted
**Date:** 2026-07-11

## Context

The pre-pool `mail list` walked folders in a single IMAP session:

```
SELECT INBOX → UID SEARCH → UID FETCH → SELECT Sent → UID SEARCH → UID FETCH → ...
```

For a mailbox with N folders this is 3N IMAP round-trips per `list`. With N = 10–30 (Gmail labels, NetEase custom folders), a `list` takes seconds on a normal connection, and AI agents calling `mail list --json` every few minutes feel the latency.

IMAP is a **stateful protocol** — a session can `SELECT` only one mailbox at a time. There is no command pipelining (RFC 3501: every tagged command waits for its tagged response). The only parallelism knob is "more sessions."

User scenario:

- AI Agent polls `mail list --json` every few minutes.
- Mailboxes have 10+ folders (INBOX, Sent, Drafts, plus custom labels).
- One `list` should be fast (< 500 ms incremental, < 3 s first run).

## Decision

**Maintain a fixed pool of M = 4 IMAP sessions plus a `tokio::Semaphore` to distribute N folders across them.**

- Startup: open 4 IMAP sessions in parallel, sharing the same keyring password.
- Per `mail list` sync: for each folder, acquire the semaphore, do `SELECT → UID SEARCH → UID FETCH` on any idle session, release.
- M = 4 is hardcoded. Not exposed as a flag or config.
- Session failure: any IMAP command failure on a session → tear it down, reconnect, retry the operation once on the new session.
- Best-effort across folders: one folder's failure does not block the others.

## Alternatives considered

### A. Single-session async pipelining

- Same session, sequential `SELECT A → SEARCH A → SELECT B → SEARCH B → ...`.
- Saves TLS handshakes but not IMAP round-trips — still 3N, the optimization target unmet.
- Rejected.

### B. Command pipelining (IMAP multiline)

- IMAP does not support tagged-command pipelining.
- Rejected: the protocol forbids it.

### C. Unbounded concurrency (one session per folder)

- Servers typically cap concurrent connections at 5–15. Excess triggers bans.
- No backpressure.
- Rejected.

### D. Reuse Timeline's `sync_state` watermark to skip folders

- Cross-couples `mail list` to Timeline state. `mail list` should not depend on Timeline having been synced.
- Rejected: already excluded during the Timeline design.

### E. Per-account pool (4 sessions × N accounts)

- For N = 2 accounts that's 8 sessions; some servers cap at 5.
- Marginal benefit: agents typically poll one account at a time.
- Rejected: one pool per CLI invocation is simpler and within server limits.

## Consequences

- New module file `src/modules/email_pool.rs` holds `Vec<Mutex<ImapSession>>` + `Arc<Semaphore>`. `email.rs` becomes thinner.
- First `list` after startup pays 4× TLS handshake (~1–2 s). Subsequent incremental lists reuse the warm pool; cost is `LIST` (folder enumeration) + N × `SELECT`.
- M = 4 is empirical: Gmail, Outlook, NetEase, QQ Mail all tolerate it without rate-limit. If a real cap surfaces later, expose `mail.imap_pool_size` config — not now.
- Memory: 4 sessions + TCP/TLS buffers, tens of KB. Negligible.
- The `PoolGuard::session()` API returns `Result<PoolGuard, AgentError>` (it can fail to acquire an idle session; it never panics). See [R003](R003-pool-guard-drop.md) for the Drop-time `Handle::try_current()` guarantee.
- Sync coordination: the orchestrator uses `futures::join_all` across folders, gated by the semaphore. Same pattern as [L009](L009-best-effort-sync.md).

## Cross-references

- The IMAP stack the pool runs on: [M001](M001-imap-stack.md).
- The envelope cache the pool writes into: [M003](M003-envelope-cache.md).
- The sync flow that calls into the pool: [M004](M004-uid-watermark-sync.md), [M005](M005-staleness-auto-sync.md).
- The `PoolGuard::Drop` guarantee: [R003](R003-pool-guard-drop.md).