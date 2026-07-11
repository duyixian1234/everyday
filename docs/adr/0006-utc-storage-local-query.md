# ADR 0006: UTC timestamp storage with local-timezone query

**Status:** Accepted
**Date:** 2026-07-11

## Context

Timeline events have a `timestamp` field stored as RFC3339 strings in SQLite. Queries like `timeline today` / `timeline week` express time ranges in the user's local timezone (e.g., "today" in UTC+8 means 00:00 to 23:59:59 local, which is 16:00 previous-day-UTC to 15:59:59 current-day-UTC).

SQLite has no native datetime type — RFC3339 strings are compared lexicographically. This works correctly **only if all timestamps use the same timezone offset**, because RFC3339 strings with different offsets have inconsistent dictionary ordering relative to chronological ordering.

Example: `2026-07-11T16:00:00+08:00` and `2026-07-11T08:00:00Z` represent the same instant, but `+08:00` > `Z` lexicographically, so `BETWEEN` queries would order them incorrectly.

## Decision

**Store all timestamps as UTC (RFC3339 with `Z` suffix). Convert to local timezone at query boundary and output.**

- Storage: `chrono::Utc::now().to_rfc3339()` → `2026-07-11T06:30:00+00:00` (or equivalent `Z` form).
- Query: `timeline today` computes local date bounds `[00:00, 23:59:59]` via `chrono::Local`, converts to UTC, then `WHERE timestamp BETWEEN <utc_from> AND <utc_to>`.
- Output: text mode displays local time (`MM-DD HH:MM`); JSON mode outputs UTC RFC3339 for machine consumption.

## Alternatives considered

### Store local time (RFC3339 with offset)

- SQLite `BETWEEN` string comparison breaks when offsets differ (`+08:00` vs `Z` vs `+09:00`).
- Would require parsing all timestamps to compare, negating the index benefit of lexicographic string comparison.

### Store UTC, query in UTC

- `timeline today` = UTC's today. For UTC+8 users, midnight boundary is off by 8 hours — queries at 22:00 local would show "tomorrow's" events as "today" and miss early-morning events.
- Unacceptable for non-UTC users.

### Store as Unix epoch integer

- Correct ordering, but loses readability (debugging raw SQLite requires conversion).
- Breaks consistency with existing modules (todo/note/bookmark already store RFC3339 strings for `created_at` / `updated_at`).

## Consequences

- All timestamps are UTC; timezone conversion happens only at the query and display boundary.
- `chrono::Local` is used for local date bounds (no `chrono-tz` dependency needed — system timezone is sufficient).
- JSON consumers must convert UTC to local for display. This is standard practice for machine-readable timestamps.
