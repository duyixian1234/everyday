# ADR R021: Date-sequence IDs — replace the nanosecond-hex ID format

**Status:** Accepted
**Date:** 2026-08-14

## Context

`util::id::gen_id` produces ids of the form `{prefix}{nanos:x}-{pid:x}-{seq:x}` — a 16-hex-character nanosecond timestamp, the process PID, and a process-local counter (e.g. `n17abc...-1a2b3-1`). All local storage prefixes share it: note (`n`), todo (`t`), bookmark (`b`), memory (`m`), timeline events (`ev`), email cache (`mc`), rss items (`ri`).

The format is machine-perfect but human-hostile: an id cannot be read aloud, retyped, or compared at a glance, and the hex timestamp encodes no date without decoding. The owner's daily workflow (CLI + agent, pasting ids between commands) wants ids that are short, human-readable, and self-explanatory.

## Decision

Replace the format with a **date-sequence id**: `{prefix}{YYYYMMDD}-{pid:x}-{seq}`, where:

- `{prefix}` — the unchanged module discriminator (`n`/`t`/`b`/`m`/`ev`/`mc`/`ri`)
- `{YYYYMMDD}` — the **local-timezone calendar date** of creation (`chrono::Local`), e.g. `20260814`
- `{pid:x}` — the creating process's PID in hex (e.g. `1a2b`), the **cross-process uniqueness component** (see Revision below)
- `{seq}` — zero-padded 3-digit ordinal (≥ 001; extends to 4+ digits if a prefix ever exceeds 999 ids in one day per process)

Example: `n20260814-1a2b-001` reads as "the 1st note created by process 1a2b on 2026-08-14".

### Scope decisions

- **All prefixes, one change** — edit `util/id.rs` once; every module adopts the format together (no mixed formats across modules).
- **Per-prefix sequence** — each prefix counts its own ordinals from 001 each day; modules never share a sequence.
- **Daily reset** — counters reset on the local calendar day boundary.
- **In-process memory counters + PID segment** — `gen_id` stays a stateless pure function (a static day + per-prefix counter behind a mutex, plus the process PID). Uniqueness is guaranteed within a process by the counter and across processes by the PID segment. The SQLite `PRIMARY KEY` constraint remains the final safety net, but no longer as the *first* line of defense (see Revision).
- **No migration** — existing rows keep their old-format ids. Both formats coexist as valid ids (schema is `TEXT PRIMARY KEY` with no format constraint). References by exact string match (`note read`, `todo complete`, timeline `ref_id`, search `Hit.id`) keep working unchanged.
- **Version** — shipped as v0.17.1 (patch). Ids are opaque identifiers; reference semantics are unchanged, no consumer breaks.

## Alternatives considered

### Keep nanosecond-hex, add a human-readable alias column
- Would preserve strict uniqueness while giving display ids.
- Rejected: doubles the identity surface (the alias must be unique, resolvable, and synced), and the underlying id would still leak into `--json` output and `ref_id`.

### Query DB `MAX(seq)+1` for strict cross-process uniqueness
- Eliminates collision risk entirely.
- Rejected: `gen_id` would lose its pure-function shape and every module would need a "max today for this prefix" query — a 1-line change becomes 7 call sites; the collision it guards against is theoretical (see Decision).

### Keep the PID segment: `{prefix}{date}-{pid:x}-{seq}`
- Retains hard cross-process uniqueness.
- Initially rejected: defeats the readability goal — the id grows back toward the old length. **Adopted in the 2026-08-14 revision** after the collision-free premise failed (see below); at 19 chars it stays ≈25% shorter than the old 26-char format and the first 9 characters remain human-meaningful.

## Consequences

- `--json` `id` fields, timeline `ref_id`, and search `Hit.id` now carry date-sequence ids for new rows; old rows keep legacy ids. Field names are unchanged ([R001](R001-thread-local-json-mode.md) contract intact).
- `gen_id_embeds_pid` test is rewritten for the new format (date + PID segments); `gen_id_uses_prefix` and `gen_id_unique_within_loop` survive unchanged (the date segment keeps ids unique across the midnight boundary).
- WebDAV file-level sync ([D001](D001-webdav-file-sync.md)) is unaffected — it hashes whole DB files, not row ids.
- CONTEXT.md "短 UUID" phrasing is corrected to the canonical term 日期序号 ID.
- CLI help/error text for `todo`/`note` positional arguments still says `<page_id>` (Notion-era naming); user-facing text is cleaned to `<id>`. The `default_page_id` config field is **untouched** — it is a live config API ([R019](R019-remove-notion-provider.md) kept it).

## Revision (2026-08-14, during v0.17.1 quality gate)

**The "collision surface is ≈0" premise was wrong.** The memory module's end-to-end tests write to the real global `~/.config/everyday/memory.db`; under the original `{prefix}{YYYYMMDD}-{seq}` format they failed with `UNIQUE constraint failed: memory.id` even in isolation, because:

- The CLI is **one-shot per command** — every `everyday memory add` / `todo add` / `note create` is a fresh process whose ordinal restarts at `001`.
- The day-prefixed ordinal is therefore NOT unique across processes: the second CLI write of the day to the same prefix collides with the first (`t20260814-001` twice), and the agent's own automation writes (memory/todo) collide with the user's interactive writes.
- nextest runs each test in its own process, so test temp DBs named after `gen_id()` alone collided too.

This is not the "two processes writing simultaneously" edge case the original decision assumed — it is the primary daily workflow. The DB `PRIMARY KEY` would surface every collision as a hard error, i.e. a broken write command.

**Revised decision:** restore the PID segment — `{prefix}{YYYYMMDD}-{pid:x}-{seq}`. `gen_id` keeps its pure-function shape, all write paths are untouched, and cross-process uniqueness is restored to the old format's level. The DB `PRIMARY KEY` remains only as a final safety net, not the first line of defense.

## Cross-references

- [R019](R019-remove-notion-provider.md): removed the Notion provider whose page-id naming motivated `<page_id>`.
- [R001](R001-thread-local-json-mode.md): `--json` contract — id field values change format, field names unchanged.
- [D001](D001-webdav-file-sync.md): file-level sync unaffected by row-id format.
