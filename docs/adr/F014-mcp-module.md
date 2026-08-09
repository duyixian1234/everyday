# ADR F014: MCP Module — Protocol Projection of Everyday Capabilities

**Status**: Accepted

**Date**: 2026-08-10

**Deciders**: Everyday Architecture Review (grill-with-docs session)

---

## Context

`everyday` is a Rust CLI exposing 12 modules through a single `Executor` trait
(`execute(action, args, &RequestContext) -> Result<Output>`) with a data-driven
clap subcommand tree ([F007](F007-clap-subcommand-tree.md)). Its positioning is
"the hands of your AI Agent": an agent shells out to `everyday <module> <action>`
and parses JSON output.

[F012](F012-architecture-deepening-phase.md) explicitly lists "REPL, API layer,
batch tools" as future frontends enabled by the service-layer separation. The
Model Context Protocol (MCP) is today's de-facto standard for an agent to invoke
external capabilities: Claude Code, CodeBuddy, Cursor etc. can be pointed at any
MCP server over stdio with zero bespoke integration code.

The agent currently reaches everyday through shell invocation, which has three
costs: the agent must know the exact CLI syntax per action, must construct a
`Command` + parse stdout per call, and every call pays process cold-start
([F009](F009-performance-budget.md), < 100 ms). A persistent MCP server lets the
agent discover tools (`tools/list`) and call them with typed JSON args over one
long-lived process, reusing a single `ModuleRegistry`.

The official Rust SDK is `rmcp`. It ships **3.x** (3.0.1, targets the 2026-07-28
spec, MSRV 1.88, `cargo add rmcp` default) and **2.x** (2.2.0, targets 2025-11-25).

## Decision

Add an `mcp` module implementing `Executor`, exposing every other module's
`(module, action)` commands to MCP clients as tools.

### Scope

- **Direction**: everyday is an **MCP server** (stdio transport), not a client.
  `everyday mcp serve` runs the server; `everyday mcp tools` prints the projected
  tool list + JSON Schemas for debugging (same builder as serve).
- **Library**: `rmcp` **3.x** (current default; backward compatible with
  2025-11-25 clients). Add to `[dependencies]` with `server` + `transport-io`
  (stdio) features. The JSON Schema is **hand-built** from `module_arg_spec()`
  (a small data-driven mapper, not `schemars` derive — the spec is declarative
  data, so a derive macro buys nothing).
- **Exposure**: every action of every module is projected to a tool — no
  whitelist/blacklist config in v1. Write actions (`mail send`, `note delete`,
  `todo complete`, …) are exposed; their tool descriptions carry a `[WRITE]`
  marker (a superset of `sync::is_write_action`, which gates file auto-sync).
  Rationale: consistent with "hands" positioning; the full tool list (~50) is
  acceptable to MCP clients. Revisit with a config filter only if tool-list
  length becomes a practical problem.

### Tool projection (Protocol projection)

- **Tool name**: `<module>_<action>` (e.g. `mail_list`, `note_add`,
  `timeline_today`).
- **Input schema**: generated from the existing `module_arg_spec()` — the
  single source of truth shared with clap. `ArgKind::Value` → string property,
  `Bool` → boolean, `Multi` → array, positional slot → `args` property, global
  `--account` → optional `account` property. No hand-written schemas; CLI and
  MCP can never drift.
- **Output**: call `execute()` then render `RenderMode::Json`; the JSON string
  is returned as MCP `content[0].text`. Identical data to
  `everyday <mod> <action> --json`.
- **Errors**: `AgentError` → MCP `isError = true`, `content[0].text` = error
  message.

### Integration

- `mcp` is an `Executor` module registered in `ModuleRegistry`, constructed with
  an injected `Arc<ModuleRegistry>` (cross-module orchestrator; same precedent as
  timeline/search/auth holding `Arc<Config>`). It needs the full registry to
  project every module.
- `serve` blocks until stdin EOF; it never returns an `Output` to `main.rs`, so
  the stdout contract is safe (JSON-RPC only). `--json` flag and
  `println!`-based final rendering in `main.rs` are bypassed in this path.
- Logging stays on stderr (the `serve` action itself goes through the existing
  `LoggingMiddleware` stack; tool calls run inside the serve loop and write
  directly to stderr); no
  stdout prints anywhere in the server path — a stdio-server hard rule.
- One `ModuleRegistry` is built once per `serve` session and reused for every
  tool call; module lifecycle (`initialize_all`/`shutdown_all`) runs at
  session start/end.
- **Concurrency**: tool calls are serialized behind a `Mutex`. Modules assume a
  single-invocation lifecycle; a single LLM client sends serial requests anyway,
  so serialization costs nothing and eliminates concurrency risk.
- **Account**: optional `account` parameter on every tool, forwarded as
  `--account <x>` into the args — identical semantics to the CLI global flag.
- **No config surface**: no `[mcp]` config section in v1 (stdio server needs no
  port/auth/timeout knobs). Introduce config when a real knob appears.

## Alternatives considered

- **MCP client direction** (everyday consumes external MCP servers): rejected —
  contradicts "expose our capabilities", a different feature.
- **Streamable-HTTP / SSE transport**: rejected for v1 — everyday is a local
  tool for the user's own agent; stdio is zero-config and the agent ecosystem's
  default. HTTP adds port + auth surface with no v1 consumer.
- **rmcp 2.x**: viable (more tutorials), but 3.x is the maintained default,
  satisfies MSRV, and is backward compatible with 2025-11-25 clients.
- **Hand-written tool schemas per action**: rejected — duplicates
  `module_arg_spec()` and will drift.
- **`structuredContent` for `Output::Json`**: rejected for v1 — dual rendering
  paths; a JSON string in `content[0].text` is uniformly parseable by agents.
- **Hard-code `mcp` in `main.rs` like `health`**: rejected — violates "add an
  mcp module" and skips the uniform `Executor` path.
- **Read-only exposure**: rejected — would amputate the "hands" positioning;
  mitigated via write-marking descriptions instead.
- **Tool whitelist/blacklist config**: deferred — YAGNI until tool-list length
  becomes a measured problem.

## Consequences

### Positive

1. Any MCP-capable agent connects to everyday's full capability set with a
   one-line `mcpServers` entry and typed tool calls — no per-action shell glue.
2. One long-lived process amortizes cold-start across tool calls (F009 win).
3. Single source of truth for tool schemas (`module_arg_spec`) — no drift.
4. Zero config surface keeps v1 small.

### Negative

1. `mcp` module needs the full `Arc<ModuleRegistry>` — a "super-module" with a
   wider construction footprint than business modules (justified: it projects
   all of them).
2. Full tool list (~50) makes `tools/list` larger for the model; acceptable for
   v1, revisit with a filter if it measurably hurts.
3. Long-lived process changes operational profile: config/db changes won't be
   picked up until the session restarts (documented in README).

### Risks

| Risk | Mitigation |
|------|-----------|
| stdout pollution in server path | stdout is exclusively JSON-RPC; all logs to stderr; contract test asserts no stray stdout |
| rmcp 3.x API being new | implement against official docs/source; contract tests protect against behavioral drift |
| write actions triggered by agent | tool descriptions mark writes; user grants the agent the server only deliberately |

## Implementation plan

1. Add `rmcp` (3.x, features `server` + `transport-io`) to `Cargo.toml`.
2. `src/modules/mcp/` directory module: `tool_registry.rs` (single projection
   builder — pure logic, unit-testable), `mod.rs` (Executor, `serve`/`tools`
   actions + the rmcp `ServerHandler` adapter).
3. Wire `Arc<ModuleRegistry>` injection into `ModuleRegistry::build` (build
   returns `Arc<Self>`; `mcp` resolves the registry via a `OnceLock` cell).
4. Contract tests: `mcp serve`/`mcp tools` in the command tree; a stdio
   end-to-end test asserting initialize round-trip, tool call semantics, and
   that stdout carries JSON-RPC only (tests/mcp_stdio.rs).
5. Update README(/_ZH), skills/everyday-cli, ADR index, progress.md.

## Related decisions

- [F007](F007-clap-subcommand-tree.md): `module_arg_spec` as the schema source.
- [F012](F012-architecture-deepening-phase.md): new-frontend positioning.
- [F013](F013-request-context-explicit-parameter.md): `RequestContext` flows into
  `execute` unchanged.
- [F009](F009-performance-budget.md): cold-start amortization.
- Glossary: [`CONTEXT.md` §MCP](../../CONTEXT.md).
