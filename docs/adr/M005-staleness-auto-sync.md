# ADR M005: Staleness-based auto-sync + flags snapshot + search bypass

**Status:** Accepted
**Date:** 2026-07-11

## Context

Three sub-decisions had to be settled on top of the envelope cache ([M003](M003-envelope-cache.md)) and watermark sync ([M004](M004-uid-watermark-sync.md)):

1. **When does `mail list` trigger a sync?** Never / always / staleness-threshold?
2. **How stale is `flags`?** Users marking messages read in the web UI — how soon does the local `--unread` filter reflect it?
3. **Does `mail search` use the cache?** Search needs broader query semantics than the envelope cache can serve.

These questions interact: the cache is great for `list`, but `search` and `flags` are different concerns.

## Decision

### 1. Staleness-based auto-sync (15 minutes)

- Default `mail list`:
  - Read all target folders' `folder_state.last_sync_at`.
  - If any folder is older than **15 minutes** (hardcoded), trigger one sync round before listing.
  - Otherwise serve the query from local SQLite.
- `--sync` flag: force a sync regardless of staleness.
- No `--no-cache` / `--full` flag (KISS — most users want the cached path).
- 15 minutes is hardcoded. Not exposed as a config.

### 2. `flags` are a sync-time snapshot (F1)

- `envelopes.flags` reflects the server's state at the most recent sync.
- `mail list --unread` is accurate as of that sync — up to 15 minutes behind reality.
- Users marking mail read in another client may see `--unread` still showing it as unread for up to 15 minutes.
- This trade-off is **explicit and documented**. No per-`list` `UID FETCH FLAGS` re-pull (would break the "zero-network list" promise).
- See [M003](M003-envelope-cache.md) for the related K1 retention rule.

### 3. `mail search` bypasses the cache

- `mail search --query Q` runs the existing IMAP `SEARCH TEXT "Q"` over each folder directly.
- Local `LIKE` over `subject`/`from` cannot match `BODY`, header values, or the many IMAP search keys users expect.
- Asymmetric with `mail list`: `list` is fast (cache), `search` is slow (server).
- Future escape hatch: `mail search --cached` for a local-only `LIKE` path — out of scope for this ADR.

## Alternatives considered

### Sync every `list`

- Violates [L005](L005-no-auto-sync.md): every query hits the network, breaks the 100 ms cold-start budget.
- Wasteful: the same `list` repeated seconds apart triggers two full syncs.
- Rejected.

### No auto-sync (`--sync` only)

- Agent must remember to sync.
- Friendly for humans, unfriendly for agents.
- Rejected.

### Staleness threshold configurable (`[mail] staleness_minutes`)

- Adds a config surface for a value most users don't need to tune.
- 15 minutes matches typical agent polling cadence.
- Rejected: future exposure if feedback warrants.

### Re-fetch `flags` before every `list`

- Cheap `UID FETCH FLAGS`, but breaks zero-network `list`.
- 15 minutes lag is acceptable for the `--unread` use case.
- Rejected.

### `mail search` over local envelope `LIKE`

- Only `subject` / `from` fields searchable.
- IMAP `SEARCH TEXT` covers `subject`, `body`, and many headers.
- Rejected: silent semantic loss.

### `mail search` with local-first fallback to IMAP

- Two code paths to maintain, two semantics to document.
- Users would have to know which backend served a given query.
- Rejected.

## Consequences

- `mail list` is predictable for agents: local query < 100 ms after warmup; first call may pay 1–3 s for an auto-sync.
- Text output explains what happened: `synced N folders (M new envelopes), listed K from cache`. JSON output carries the same fields in the structured payload.
- `--unread` lag is a documented edge — agents that need live state must call `--sync` explicitly.
- `mail search` / `mail list` asymmetry must be explained in `--help` and the README — already done.
- Future extension surface kept small: `--full` (skip cache), `--cached` (local `LIKE`), `mail cache gc` (manual cleanup). None are exposed today.

## Cross-references

- The cache being consulted: [M003](M003-envelope-cache.md).
- The sync flow invoked when staleness fires: [M004](M004-uid-watermark-sync.md).
- The pool that runs the sync concurrently: [M002](M002-imap-connection-pool.md).
- The query/sync separation principle that justifies the staleness threshold: [L005](L005-no-auto-sync.md).
- Timeline's separate "no auto-sync" rule (same principle, different layer): [L005](L005-no-auto-sync.md).