# ADR R002: Output JSON serialization failure must not break the `--json` contract

**Status:** Accepted
**Date:** 2026-07-11

## Context

In `--json` mode, modules return `Output::Json(serde_json::Value)` or `Output::Table(tabled::Table)`. The renderer is responsible for serializing these to stdout. If the renderer panics or returns an error and the program crashes, the user sees a Rust backtrace — useless to the Agent and worse than the failure the `--json` envelope was designed to prevent.

A specific instance: a custom `Serialize` impl on one of the data types had a bug; the JSON serialization of `Output::Table` panicked mid-write, producing a partial JSON stream followed by a panic. Agents downstream of that stream got `unexpected end of JSON input`.

## Decision

**The `--json` rendering path must never panic. Any serialization error must produce a structured `AgentError::Json` response and exit code 1.**

Implementation contract:

- Wrap every `serde_json::to_writer*` / `tabled::Table::to_json` call in `?` (or equivalent).
- Errors map to `AgentError::Json { message }` (or the existing `Serialization` variant).
- The CLI prints the error envelope as `{"error": "Json", "message": "..."}` — same shape as every other error.
- The process exits with code 1.
- The renderer must not partially write to stdout before the error; use a `Vec<u8>` buffer and write atomically.

The fix is one of the "contract"-category items from the 2026-07-11/12 review — see commit `0d1b954`.

## Alternatives considered

### Allow panic; surface backtrace in `--json` mode

- Backtrace is useless to the Agent.
- Process crashes with non-zero code, which is fine, but the *output* is garbage.
- Rejected.

### Print partial JSON followed by an error marker

- Easier to implement (no buffering).
- Downstream parsers see partial JSON and may or may not detect the marker.
- Rejected: explicit contracts are better than mid-stream sentinel values.

### Always return `Output::Json` even in text mode (so the renderer is uniform)

- Avoids the issue by removing the `Table` variant in JSON mode.
- Considered: but `Table` renders nicely for `--json` consumers that want array-of-objects.
- Rejected.

## Consequences

- `--json` consumers can rely on "either a complete JSON payload or an error envelope, never partial output."
- Adding a new `Serialize` impl must keep the "no panics" discipline (covered by the existing rule: no `unwrap()` in non-test code — see `agents.md` §"错误处理").
- The renderer's buffering adds one allocation per render. Negligible.
- A unit test asserts that a deliberately broken serializer produces the error envelope, not a panic.

## Cross-references

- The `Output` enum this contract serves: [F001](F001-cli-shape.md).
- The JSON mode thread-local this depends on: [R001](R001-thread-local-json-mode.md).
- The error envelope contract: [F001](F001-cli-shape.md) §4.