# ADR M007: `mail cache gc` — server-reconciled ghost-envelope cleanup

**Status:** Accepted
**Date:** 2026-08-21

## Context

The envelope cache ([M003](M003-envelope-cache.md)) is append-only with K1 retention: sync only
appends, never deletes. Messages deleted on the server, or moved across folders, leave **ghost
envelopes** behind in `mail_cache.db`. M003 documented this as acceptable — the DB grows but
`mail list --limit` is usually dominated by fresh mail. M005 explicitly deferred a manual cleanup:
*"mail cache gc (manual cleanup)"* was listed as a future extension surface, not exposed today.

There is no intrinsic "this row is stale" marker in the cache. Because sync only appends, an
envelope row's presence says nothing about whether the message still exists on the server. The only
way to find ghosts truthfully is to reconcile against the server.

## Decision

Add a manual `mail cache gc` command.

1. **Explicit, manual-only.** `mail cache gc` is a user-invoked command. The daemon
   ([F016](F016-daemon-sync-scheduler.md)) stays strictly pull-only — it never deletes cache rows.
   The "query/sync separation" and "daemon = pull-only" iron rules are unchanged.

2. **Server-reconciled ghost detection.** For each folder in scope, connect to IMAP and:
   - SELECT the folder to read its current `uid_validity` and message set.
   - If the folder's `uid_validity` differs from the locally stored `folder_state.uid_validity`,
     **skip deletion for that folder** and report it. A changed `UIDVALIDITY` means UIDs were
     recycled; deleting by UID there would destroy valid local data
     ([M004](M004-uid-watermark-sync.md)).
   - Compare the server's UID set (from the current watermark window) against local envelope rows
     and remove rows whose UIDs no longer exist on the server.

3. **Scoped granularity.** `--account` / `--folder` select a subset; default is the whole cache
   (every account / every known folder).

4. **Watermark advance after success.** On a folder that reconciled cleanly, advance its
   `folder_state.max_uid` to the server's current value. This prunes the watermark too, so later
   incremental syncs do not repeatedly re-issue search windows over already-deleted UIDs. Watermark
   is only advanced for folders that reconciled successfully; failures leave the folder untouched.

5. **Destructive, so transparent.** `mail cache gc` removes rows permanently. It is an explicit
   user command, not auto-triggered, so no dry-run flag is required; help text and docs call out the
   destructive nature. Output reports per-folder rows removed / skipped and the reason for any skip.

## Alternatives considered

### Heuristic cleanup (time / watermark thresholds, no network)
Zero-network, but cannot know what the server actually holds — a message deleted server-side stays
indistinguishable from one still there. Risks deleting valid envelopes or keeping ghosts. Rejected
in favor of truthful server reconciliation (gc is an explicit command, so a network round is fine).

### Auto-gc inside the daemon sync cycle
The daemon is defined as the pull-only role ([F016](F016-daemon-sync-scheduler.md)); giving it a
delete capability changes its contract and makes a destructive, network-dependent operation run
unattended. Deferred — manual-only keeps the daemon simple and the destructive action intentional.

### Garbage-collect only, never advance the watermark
Keeps `max_uid` so the next sync re-examines the whole range. Rejected: the watermark would then
re-request UIDs we just deleted, wasting a full re-fetch every sync. Advancing the watermark after a
clean reconcile is the "prune" the cache needs.

## Consequences

- The cache stops growing without bound once gc is run periodically; `mail list` stays fast.
- gc is truthful: it only removes what the server confirms is gone, and skips any folder whose
  `UIDVALIDITY` changed (protecting reused UIDs).
- The daemon's pull-only contract is preserved.
- New `mail cache gc` action requires updating the CLI contract test (`tests/cli_contract.rs` mail
  action set), CLI help, `docs/commands*`, and the skill reference. Additive action — non-breaking.
- A server-reconciled gc can be slow on a large cache (network + per-folder SELECT), so it stays a
  manual, explicit command rather than something in the hot path.

## Cross-references

- Append-only / K1 retention that produces ghosts: [M003](M003-envelope-cache.md).
- UIDVALIDITY change handling this ADR reuses: [M004](M004-uid-watermark-sync.md).
- Daemon pull-only role (why gc is not auto): [F016](F016-daemon-sync-scheduler.md).
- Staleness / sync model that the cache rides on: [M005](M005-staleness-auto-sync.md).
- The local search path that will now see fewer ghosts: [M006](M006-mail-search-cached.md).
