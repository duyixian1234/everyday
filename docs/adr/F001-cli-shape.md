# ADR F001: CLI command shape, Executor trait, Output, AgentError

**Status:** Accepted
**Date:** 2026-07-08

## Context

Everyday is a Rust CLI that fronts many external integrations for an AI Agent. Four cross-cutting questions had to be settled before any module could be written:

1. **Command grammar.** What does every command look like? How do global flags apply?
2. **Module dispatch.** How does the main binary discover and invoke modules without embedding module-specific code in `main.rs`?
3. **Output rendering.** Modules produce data in different shapes (a list, a key/value map, a free-form message). The Agent consumes primarily JSON; humans read primarily text. Both must be first-class.
4. **Error model.** Errors must round-trip through both text and JSON without losing structure, and exit codes must be predictable.

## Decision

### 1. Command grammar

Every command follows `everyday <module> <action> [options]`.

- `<module>` is a flat enum: `mail` / `cal` / `rss` / `note` / `todo` / `bookmark` / `timeline` / `config`.
- `<action>` is module-specific (e.g. `mail list`, `mail send`, `todo add`).
- Global flags: `--json` (switch to JSON output), `--account <name>` (override the default account).

### 2. `Executor` trait + `ModuleRegistry`

```rust
#[async_trait]
pub trait Executor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    async fn execute(&self, action: &str, args: &ActionArgs) -> Result<Output, AgentError>;
    fn actions(&self) -> Vec<ActionDoc>;
}
```

- `main.rs` only knows about `Box<dyn Executor>` and the registry.
- New module = one file + one registration line. No special-casing in `main.rs` (the historical config exception was later removed — see [R012](R012-config-executor-trait.md)).
- Since [F007](F007-clap-subcommand-tree.md) the CLI surface is declared as data via `module_arg_spec()`; help is delegated to clap's native `--help`.

### 3. `Output` enum

```rust
pub enum Output {
    Text(String),
    Json(serde_json::Value),
    Table(tabled::Table),
}
```

- Text mode: `Text` is passed through verbatim; `Json` is pretty-printed; `Table` is rendered to the terminal.
- JSON mode (`--json`): `Text` is passed through verbatim (used by AOP hooks — see [L011](L011-aop-handles-output-text.md)); `Json` is serialized compact; `Table` is serialized as a JSON array.
- See [R002](R002-output-json-failure.md) for the JSON serialization failure contract.

### 4. `AgentError` + JSON error envelope

```json
{"error": "ErrorType", "message": "Details..."}
```

- `AgentError` implements `serde::Serialize` so the envelope is always consistent.
- Exit codes: success `0`, failure non-zero. The exact non-zero code is allowed to grow as new variants are introduced.
- No `unwrap()` / `expect()` in non-test code; errors propagate with `?` and contextual `map_err`.

## Alternatives considered

### Free-form CLI grammar

- Without a fixed shape, agent prompts would have to enumerate options per module.
- Rejected: the Agent is the primary user; uniformity is the product.

### Module-specific dispatch branches in `main.rs`

- Leads to `match module { Mail => ..., Cal => ..., ... }` in main. Adding a module means editing main.
- Rejected: violates "new module = one registration line".

### Free-form error strings

- The Agent would have to string-match to decide retry behavior.
- Rejected: typed error envelope is the only way to make errors actionable.

### One unified output type for everything

- Forcing every result to be `Vec<Row>` or `Map<String, Value>` loses the natural shape of free-form messages ("`mail login` saved credentials").
- Rejected: `Output` keeps each shape distinct; rendering unifies them.

## Consequences

- Adding a module is bounded to two surfaces: implement `Executor`, register in `ModuleRegistry`.
- The Agent can blindly call `everyday <anything> --json` and parse the response: success → expected shape; failure → `{"error":..., "message":...}` envelope.
- `--json` is detected once at startup (see [R001](R001-thread-local-json-mode.md)) and propagates through a thread-local rather than repeated `std::env::args()` scans.
- Configuration defaults are picked by the module via `Config` (see [F002](F002-multi-account-keyring.md)).