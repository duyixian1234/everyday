# ADR M004: UID watermark + UIDVALIDITY incremental sync

**Status:** Accepted
**Date:** 2026-07-11

## Context

The envelope cache ([M003](M003-envelope-cache.md)) needs an incremental sync from IMAP. Three strategies were considered:

- **Date window:** `SEARCH SINCE (now - 30 days)`. Cheap but IMAP `SINCE` is date-precision only — late-arriving mail with an older `Date` header gets missed.
- **UID range:** `UIDSEARCH UID <max_uid+1>:*`. UID is monotonically increasing within a folder (RFC 3501 invariant, except on UIDVALIDITY change). RFC 3501 §2.3.1.1 mandates UIDVALIDITY detection.
- **Full SEARCH every time:** `UIDSEARCH ALL`. Defeats the purpose.

First run (no watermark) is unavoidable: `UIDSEARCH UID 1:*` is the all-UIDs form.

## Decision

**Single-folder sync flow:**

1. `SELECT <folder>` — read current `UIDVALIDITY` from the `Mailbox` struct returned by `async-imap`.
2. Read local `folder_state` row → `(uid_validity, max_uid, last_sync_at)`.
3. **UIDVALIDITY mismatch:** wipe the folder's envelopes + reset watermark to 0, fall through to full sync.
4. **Watermark == 0 (first sync):** `UIDSEARCH UID 1:*` (all).
5. **Normal incremental:** `UIDSEARCH UID <max_uid+1>:*` (new only).
6. `UID FETCH <uids> (UID ENVELOPE FLAGS RFC822.SIZE)` — batched envelope fetch.
7. Upsert rows into `envelopes`.
8. Update `folder_state`: `max_uid = max(new_uids)`, `last_sync_at = now()`. **Step 7 and 8 run in the same transaction.**

Failure semantics:

- Any step fails (SELECT / SEARCH / FETCH / DB) → abort this folder's sync, **do not update watermark**, surface to caller as `Failed`.
- Other folders continue (best-effort, see [L009](L009-best-effort-sync.md)).

## Alternatives considered

### Date window

- Misses late-arriving mail with an old `Date` header.
- Less precise than UID.
- Rejected.

### No UIDVALIDITY detection

- After server-side rebuild, old UIDs are reused for different emails. Local cache lies silently.
- RFC 3501 mandates detection.
- Rejected.

### Watermark update without a transaction

- Envelopes written first, watermark last.
- If the process dies between the two: watermark didn't advance → harmless re-pull (idempotent via the upsert).
- If envelopes partially written and watermark already advanced: next sync skips messages that never made it into `envelopes`. **Real data loss.**
- Single transaction eliminates the second case at trivial cost.

### SQLite WAL + async watermark

- Better write throughput, but envelope writes are infrequent (one per folder per sync).
- Rejected: complexity not justified.

## Consequences

- `folder_state.last_sync_at` is the staleness signal read by [M005](M005-staleness-auto-sync.md).
- Each folder pays one extra `SELECT` round-trip per sync (to read `UIDVALIDITY`). This is the cheapest IMAP command; the cost is negligible.
- `max_uid` always increases; server-side `EXPUNGE` doesn't reduce it. New mail beyond `max_uid` is caught by the `+1:*` range.
- Cross-folder IMAP moves: the source folder's UID is unchanged (IMAP `COPY + DELETE` keeps the source UID allocated until expunge); the destination folder gets a fresh UID, caught by its own incremental search. The source folder retains the ghost envelope per K1 (see [M003](M003-envelope-cache.md)).
- Per-folder failure semantics match Timeline's: watermark unchanged, next sync retries the same window.

## Cross-references

- The cache being filled: [M003](M003-envelope-cache.md).
- The staleness check that drives when to run this flow: [M005](M005-staleness-auto-sync.md).
- The pool that runs it concurrently across folders: [M002](M002-imap-connection-pool.md).
- The failure semantics it follows: [L009](L009-best-effort-sync.md).