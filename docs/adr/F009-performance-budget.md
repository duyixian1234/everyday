# ADR F009: Performance budget — cold start, network IO, large output

**Status:** Accepted
**Date:** 2026-07-12

## Context

Everyday is invoked by an AI Agent as a sub-process. Agents typically run it
many times per minute across short-lived shell-outs (`everyday timeline today
--json && everyday todo list --json && ...`). The dominant cost on each call is
therefore **process startup**, not module work. Slow cold starts become a system
problem, not just a UX problem.

Beyond startup, the codebase makes network calls (IMAP, CalDAV, Notion, RSS) and
streams potentially large outputs (envelope cache, timeline events) to the Agent
in JSON. Each of these has its own trap if mishandled.

## Decision

### 1. Cold start budget: < 100 ms (median on a warm Linux box)

- No blocking IO before `main()` reaches the dispatch site.
- The config file is opened lazily — only once we know which module is being
  invoked (a `--help`-only call never loads config).
- TOML parsing, keyring backend init, and module struct construction all happen
  **after** argument parsing, not before.

### 2. Network calls must have a timeout and be cancellable

- `reqwest::Client::builder().timeout(...)` on every instance. Defaults: 30s for
  reads, 10s for sends.
- Sibling: any `tokio::net::TcpStream` use is wrapped in `tokio::time::timeout`.
- Notion 429 backs off via `Retry-After` (default 1s) once.
- A failed module does not block siblings running in parallel (`futures::join_all`
  per source group; see [L009](L009-best-effort-sync.md)).

### 3. Large output: avoid full in-memory buffer; stream when possible

- Mail envelope cache + timeline events can each grow to tens of thousands of
  rows. Defaults cap at the user's `--limit` (today: 100).
- `--limit` is enforced at the SQL layer (`LIMIT <n>`), not at the rendering
  layer. This keeps memory bounded even for accidental `--limit=1000000`.
- `Table` rendering iterates and formats row by row; no column-width pre-scan
  over the full table.
- `--json` serializes through `serde_json::to_writer` against stdout (line
  buffering), not by building a `String` first.

### 4. Best-effort parallel sync

- The timeline orchestrator groups providers by source and runs each source's
  providers in parallel via `futures::join_all`; within a source, providers
  run serially to avoid redundant work.
- A failing provider does not abort the run — its watermark stays put and
  the next sync retries. See [L009](L009-best-effort-sync.md).

## Alternatives considered

### Lazy-loading the executor registry

- Idea: only construct the `Executor` structs that the user actually invokes.
- Rejected: the registry is already trivial to construct (8 entries, all
  `Arc<...>`-cheap) and the bookkeeping for lazy loading costs more code than
  it saves.

### Streaming JSON output

- Idea: emit the JSON envelope as a one-line-at-a-time NDJSON stream for very
  large outputs.
- Deferred: the current SQL `LIMIT <n>` cap keeps things bounded without
  forcing every module to become NDJSON-aware. Revisit when a real workload
  hits the limit.

### Synchronous network calls in `main()`

- Rejected: the project is `tokio`-native. Synchronous IO would block the
  scheduler and slow down parallel module invocations.

## Consequences

- A `--help` call ends in < 30 ms (no config, no keyring, no registry
  construction).
- Worst-case startup is bounded by argument parsing + clap table layout + the
  registry's first use. If this exceeds 100 ms we revisit the registry.
- Defaults chosen for safety, not throughput. A user who wants faster sync can
  pass `--sync` or shrink `--limit`; no internal knob needed.

## Cross-references

- CI gate that ensures the budget is not silently regressed:
  [F006](F006-ci-release-github-only.md).
- The timeline provider's sync discipline:
  [L009](L009-best-effort-sync.md).
- Output rendering that respects the JSON contract even on serialisation failure:
  [R002](R002-output-json-failure.md).
