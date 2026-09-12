# ADR M008: Canonical folder keys + Message-ID duplicate suppression in the envelope cache

**Status:** Accepted
**Date:** 2026-09-12

## Context

The envelope cache ([M003](M003-envelope-cache.md)) is keyed by `(account, folder, uid)`, so the
*spelling* of `folder` is a storage contract, not a display detail. A folder reached the cache in two
spellings:

- **raw** — the modified UTF-7 name IMAP `LIST` returns (`&UXZO1mWHTvZZOQ-/&W1hoYw-`), used by the
  default recursive sync (`list_all_folders()` → `sync_folders_concurrent` → `upsert_envelopes`);
- **decoded** — the display name surfaced by `mail folders` (`其他文件夹/存档`), which is what a user
  or a script passes to `--folder`.

`resolve_folders(session, Some(f), _)` returned the caller's string **verbatim**, and
`upsert_envelopes` / `folder_state` had no normalization. A sync driven by a display name therefore
wrote a second row set for the same physical folder, while `get_folder_state` / `clear_folder` looked
the folder up under whichever spelling the caller happened to use.

The mail module worked around a real problem — a subfolder's new mail was not reaching the cache — by
syncing each subfolder explicitly by display name (`daily_brief.py` iterates `mail folders` output and
runs `mail list --sync --folder 其他文件夹/…`). That workaround is what forked the namespace, in the
same run: the full sync wrote the raw key, the per-folder pass wrote the decoded key ~8 s later.

Measured on a real cache:

| Observation | Value |
|---|---|
| `envelopes` rows / distinct messages | 165 / 110 → **55 phantom duplicates** |
| Non-canonical folder keys | 21 (every subfolder) |
| One message (uid 286) | two rows, identical `Message-ID`, `fetched_at` 8 s apart |
| `folder_state` | 21 subfolders with **two watermarks** each, so each is fetched twice per cycle |
| `mail list --json` | emitted both rows with the *same* decoded folder label, so no caller could tell them apart |

Downstream effect: the morning briefing renders "N new mails" from that list, and reported one mail as
two — a phantom "duplicate delivery" that recurred for five consecutive days.

## Decision

**1. One canonical key: the raw modified-UTF-7 name.** Every cache function that accepts a folder name
— `get_folder_state`, `upsert_envelopes`, `clear_folder`, `get_folder_uids`, `query_envelopes`,
`search_envelopes`, `search_envelopes_scoped` — canonicalizes it before touching the database. The
canonical form is `encode(decode(name))`: a name that already round-trips (any raw name, any plain
ASCII name) is kept byte-for-byte, so a server-correct key is never rewritten; anything else is treated
as a display name and mapped to its raw form. `INBOX` is folded to upper case (RFC 3501 §5.1 declares
it case-insensitive), so `--folder inbox` cannot fork the key either.

Canonicalizing at the **storage boundary** — not at each call site — is deliberate: the
`(account, folder, uid)` primary key is a cache invariant, and callers today include `mail list`,
`mail search --cached`, `mail read`, `mail cache gc`, the daemon, and the daily-briefing script.
Enforcing it in one place makes new callers safe by construction.

**2. The UTF-7 codec moves to `modules/imap_utf7`.** The decoder was private to `email.rs` and the
encoder did not exist. Both now live in a standalone module with the canonicalizer and its tests,
because the cache layer needs them but must not depend on the mail module (which itself depends on the
cache). Decoding stays display-only; encoding is the storage contract.

**3. Duplicate suppression on read.** `query_envelopes`, `search_envelopes` and
`search_envelopes_scoped` drop rows that repeat an identity already returned — the RFC 5322
`Message-ID`, scoped to the account. A mail filed in several folders is one message for a reader.
Rows without a usable `Message-ID` are never merged: two distinct mails can both lack the header, so
they fall back to the `(account, folder, uid)` triple as their identity.

**4. Existing caches are migrated once, out of band.** The invariant stops new forks; caches written
before this ADR still hold both spellings. A one-off repair folds non-canonical rows into the canonical
key (envelopes: copy-then-delete, keeping the later `fetched_at`; folder_state: `max_uid` = max,
`last_sync_at` = latest, then delete the duplicate watermark), verifies that the distinct-message count
is unchanged, and backs the database up first.

## Alternatives considered

### Normalize inside `resolve_folders` only
The narrowest fix, but it covers only the paths that go through folder resolution. `mail cache gc
--folder <display name>` and `mail search --cached --folder <display name>` talk to the cache directly,
and any future caller could re-introduce the fork. Rejected: the invariant belongs to the cache, and
the storage layer is the only place that can guarantee it.

### Map display → raw with an extra IMAP `LIST` round-trip
Not needed. Modified UTF-7 encoding is deterministic, so `encode(decode(name))` is pure local string
math — no network cost, no failure mode, and it works for a folder the account cannot currently list.
(Verified against every raw name observed in a real `folder_state` table: each round-trips
byte-for-byte.)

### Deduplicate on read only, leave the keys forked
Cheap and would have fixed the briefing's count, but keeps two watermarks per folder (double IMAP
fetch every cycle), keeps the cache ~1.5× larger than necessary, and leaves every consumer that does
not read through `query_envelopes` blind to the fork. Rejected as a symptom fix; retained only as
defence-in-depth (part 3) for the legitimate multi-folder case.

### Delete the redundant per-subfolder sync instead
Attractive — a default recursive sync already covers subfolders (the raw watermarks for all 21 were
written by the full sync, ahead of the per-folder pass) — but that is a separate question about sync
scheduling, and removing it would have left the key fork latent for any `--folder <display name>`
call. Out of scope here; tracked separately.

## Consequences

- One physical folder has one key, one row set, and one watermark: the double IMAP fetch per cycle is
  gone and the phantom "duplicate delivery" cannot recur.
- `mail list --json` no longer emits two rows for one message. A message filed in several folders
  appears once; the surviving row is the first in date-descending order. Consumers that need the
  folder placements must query IMAP rather than the cache — a deliberate trade in favour of the
  reader's view (the briefing counts mails, not mailbox placements).
- The stored `folder` column is now always the raw name; the CLI output still decodes for display
  (`email.rs` maps with `decode_imap_utf7`), so no user-visible format change.
- `mail cache gc` sees one folder per physical folder, and its UID comparisons now line up with what
  incremental sync wrote.
- `--folder` accepts either spelling everywhere (as before), but they are now the same folder rather
  than two.
- Adds `modules/imap_utf7.rs` (codec + canonicalizer) with 16 unit tests, 5 cache-level tests in
  `email_cache`, and a one-off migration for caches that predate this ADR.

## Cross-references

- Cache design and the K1 append-only retention this sharpens: [M003](M003-envelope-cache.md).
- Watermark semantics that were being forked in two: [M004](M004-uid-watermark-sync.md).
- Staleness / auto-sync that triggers the writes: [M005](M005-staleness-auto-sync.md).
- Local search paths that must return the reader's view: [M006](M006-mail-search-cached.md), [S007](S007-mail-search-local-cache.md).
- Server-reconciled cleanup that reads folder lists: [M007](M007-mail-cache-gc.md).
- Origin of the UTF-7 decoder, now extracted with its encoder: [M001](M001-imap-stack.md).
