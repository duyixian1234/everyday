# ADR M003: Envelope cache — dual-table SQLite design with K1 append-only retention

**Status:** Accepted
**Date:** 2026-07-11

## Context

`mail list` reads should be fast and ideally network-free. Two cache shapes were considered:

- **Watermark-only cache.** Store per-folder `max_uid` only; envelopes still come from the server on every `list`. Small DB, no benefit to latency.
- **Full envelope cache.** One row per email with envelope fields. `list` reads SQLite locally (sub-100 ms), sync writes incrementally. DB may grow to 10–100 MB over time.

User feedback chose the latter: *"通过读取本地 sqlite 中的旧邮件时间戳，只获取更新的邮件"* — i.e. cache the envelopes, sync only the deltas, keep `list` zero-network.

The cache must live somewhere independent from Timeline's `timeline.db` — different concerns, different access patterns.

## Decision

**`mail_cache.db` at `~/.config/everyday/mail_cache.db`, two tables, append-only retention.**

### Table 1: `envelopes`

| Column | Type | Notes |
|--------|------|-------|
| `account` | TEXT NOT NULL | |
| `folder` | TEXT NOT NULL | |
| `uid` | INTEGER NOT NULL | IMAP UID, folder-scoped |
| `date` | TEXT NOT NULL | RFC3339 UTC |
| `from_addr` | TEXT NOT NULL | `mailbox@host` |
| `subject` | TEXT NOT NULL | MIME-decoded |
| `flags` | TEXT NOT NULL | space-separated IMAP flags |
| `message_id` | TEXT NULL | RFC 5322 `Message-ID` header |
| `size` | INTEGER NULL | `RFC822.SIZE` in bytes |
| `to_addr` | TEXT NULL | first To recipient |
| `fetched_at` | TEXT NOT NULL | RFC3339 UTC |

- Primary key: `(account, folder, uid)`. UID is folder-scoped — the same email has different UIDs in different folders. The composite key expresses this exactly.
- Index: `(account, date DESC)` to serve `mail list`'s default sort-by-date query without a full table scan.
- The body / attachments are **not** cached. `mail read` continues to go to IMAP for `BODY[]`.

### Table 2: `folder_state`

| Column | Type | Notes |
|--------|------|-------|
| `account` | TEXT NOT NULL | |
| `folder` | TEXT NOT NULL | |
| `uid_validity` | INTEGER NOT NULL | for UIDVALIDITY-change detection |
| `max_uid` | INTEGER NOT NULL DEFAULT 0 | sync watermark |
| `last_sync_at` | TEXT NOT NULL | RFC3339 UTC, drives staleness |

- Primary key: `(account, folder)`.

### Retention: K1 — append-only, never physically delete

- Server-side deletions / cross-folder moves leave "ghost envelopes" locally.
- Default `mail list --limit 20` orders by date desc; ghost envelopes are usually pushed out of the limit window.
- No reconcile, no TTL, no `DELETE` in the sync flow.
- Trade-off: DB grows unboundedly. ~300 bytes per row, 100k envelopes ≈ 30 MB, 1M ≈ 300 MB. Acceptable today.
- Escape hatch: `mail cache gc` is a future, optional command for users whose caches grow large.

## Alternatives considered

### Watermark-only cache

- `mail list` still hits the server on every call.
- Rejected: explicitly contradicts the user requirement.

### Primary key `(account, message_id)`

- One global row per email across folders.
- Sync couldn't use UID-only incremental — would need to fetch envelopes first to learn `message_id`.
- Cross-domain complexity (UID/folder) for negligible gain.
- Rejected.

### Soft-delete with reconcile (`active=0` after each sync)

- Adds a final `UIDSEARCH UID 1:*` to enumerate all UIDs, marking missing ones inactive.
- Better correctness but breaks the "incremental over the watermark" optimization.
- K1 + `limit 20` already hide ghosts in practice.
- Rejected: complexity not justified.

### TTL cleanup (`fetched_at > 365 days` → physical delete)

- Brutal, risks deleting envelopes the user wanted to keep.
- Conflicts with K1's simplicity rule.
- Rejected.

## Consequences

- `mail list` default query: `SELECT uid, folder, date, from, subject FROM envelopes WHERE account = ? ORDER BY date DESC LIMIT 20` — millisecond response.
- Cross-folder `list` may show the same `message_id` twice (once per folder). Acceptable; future option to add `GROUP BY message_id` if it becomes annoying.
- `folder_state` updates and `envelopes` writes happen in **the same transaction** (see [M004](M004-uid-watermark-sync.md)) so a crash never leaves the watermark ahead of the rows.
- Account deletion needs cascade: `DELETE FROM envelopes WHERE account = ?` then `DELETE FROM folder_state WHERE account = ?`.
- The same envelope data also flows into Timeline via a separate path (`fetch_for_timeline`) — `mail_cache.db` and `timeline.db` are intentionally not joined. See `CONTEXT.md` §"Mail Cache" for the boundary rationale.

## Cross-references

- The sync process that fills this cache: [M004](M004-uid-watermark-sync.md).
- The staleness check that decides when to refill it: [M005](M005-staleness-auto-sync.md).
- The IMAP pool that powers concurrent folder syncs: [M002](M002-imap-connection-pool.md).
- `flags` snapshot semantics (the field that drifts between web-client edits and local cache): [M005](M005-staleness-auto-sync.md).