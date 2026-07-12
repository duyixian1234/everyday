# ADR L011: AOP ops-log hook must parse Output::Text variant

**Status:** Accepted
**Date:** 2026-07-11

## Context

The AOP ops-log hook ([L007](L007-notion-ops-log.md)) extracts `ref_id` and `title` from the module's `Output` to record an event. The first implementation only handled `Output::Json(serde_json::Value)`.

`Output` has three variants (see [F001](F001-cli-shape.md)): `Text(String)`, `Json(Value)`, `Table(tabled::Table)`. The early hook was tested against `--json` only. In default **text mode**, modules return `Output::Text` — and the hook silently no-oped. Every notion write done without `--json` was invisible to the timeline.

This was caught end-to-end during v0.5.0 testing: `timeline list` showed zero events despite `todo add` succeeding.

## Decision

**The AOP hook must parse both `Output::Text` and `Output::Json`.**

- New helper `parse_text_ref_id_and_title(text: &str) -> (Option<String>, Option<String>)`.
- Pattern: most module outputs in text mode look like `created todo '<title>' (id=<ref>)`. The helper uses a regex (or hand-rolled scanner) to pull `(id, title)` out.
- For `Output::Json`, the existing path stays.
- For `Output::Table`, the hook skips logging (table output doesn't carry a single `ref_id`/`title`).

A test (`textmode-test-after-fix`) covers the text-mode path explicitly.

## Alternatives considered

### Force every module to return `Output::Json` even in text mode

- Cleanest data path.
- Rejected: changes the user-visible text output and loses the carefully formatted text rendering modules produce.

### Bypass the hook in text mode and add a manual `timeline opslog backfill` command

- Pragmatic, but pushes the burden onto the user.
- Rejected: the whole point of the AOP hook is "the user doesn't have to remember to log."

### Store ops-log from within each module's `execute`

- Each module calls `ops_log::record(...)` on every write.
- Rejected: this is exactly the push model [L004](L004-timeline-provider-pull-only.md) rejected. The AOP hook achieves the same outcome without per-module coupling.

## Consequences

- The AOP hook now records notion events regardless of `--json`.
- Adding a new module to the hook is a one-line registration (already true); the parsing helper now supports the common text-mode output shapes.
- A unit test asserts the text-mode path; the existing JSON-mode tests remain.
- The hook continues to be best-effort — failures don't block the user's command but are surfaced to stderr ([R006](R006-ops-log-surfacing.md)).

## Cross-references

- The AOP hook architecture: [L007](L007-notion-ops-log.md).
- The `Output` enum this must understand: [F001](F001-cli-shape.md).
- The surfacing of hook failures: [R006](R006-ops-log-surfacing.md).
- The provider that reads these ops-log rows: [L010](L010-ops-log-provider.md).