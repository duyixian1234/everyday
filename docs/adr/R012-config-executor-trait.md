# ADR R012: `ConfigModule` goes through the `Executor` trait

**Status:** Accepted
**Date:** 2026-07-12

## Context

The `config` module exposes `everyday config get`, `everyday config set`, `everyday config edit`, etc. It was the **only** module not registered in `ModuleRegistry`. Instead, `main.rs` had three special-case branches:

```rust
match module.as_str() {
    "mail" => ...,
    "cal" => ...,
    "config" => {
        // inline implementation of `config get/set/edit` here, NOT dispatched via Executor
        // ...
    }
    _ => ...,
}
```

The original justification was "config is too special; its arguments are dotted paths (`config get mail.work.host`) and the dispatch is different". The cost:

- Three branches in `main.rs` that have to be kept in sync with the actual config actions.
- `everyday config --help` requires the special-case branch to know what help to print.
- `module_arg_spec()` (see [F007](F007-clap-subcommand-tree.md)) doesn't see config, so the clap tree is incomplete.
- A bug in the config branch bypasses the standard `RenderMode` / `is_json()` / error envelope flow.

The project's hard rule (see [F001](F001-cli-shape.md)) is: **new module = new file + one registration line**. Config was the lone violator.

## Decision

**`ConfigModule` implements `Executor` and is registered in `ModuleRegistry` like every other module.**

- `ConfigModule::module_arg_spec()` returns `ModuleArgSpec { name: "config", actions: &[get, set, edit, ...] }`.
- Each config action implements the same `execute` signature: takes `ActionArgs`, returns `Output`.
- The dotted-path argument is parsed by the module using the same `parse_simple_args` ([R005](R005-parse-simple-args.md)) other modules use, with a small helper for the `a.b.c` syntax.
- `main.rs` loses the three special branches.

The fix is the last "special-case" item from the 2026-07-11/12 review — see commit `fa31601`.

## Alternatives considered

### Keep the special case, document it

- Status quo.
- The project's rule explicitly forbids special cases.
- Rejected.

### Pass a closure through the registry

- `ModuleRegistry::register_with_dispatch(name, |args| { ... })`
- Two paths into a module — by `Executor` and by closure.
- Rejected: doubles the API surface for no win.

### Make config actions a sub-Executor inside ConfigModule

- `ConfigModule::execute("get", args)` dispatches to `GetExecutor`, `SetExecutor`, etc.
- More nesting than needed; one Executor per module is fine.

## Consequences

- `main.rs` is now uniform: every module goes through `ModuleRegistry`.
- `everyday config --help` works natively via clap (see [F007](F007-clap-subcommand-tree.md)).
- The dotted-path parsing is a module-local concern, not a `main.rs` concern.
- Adding a new config action (e.g. `config validate`) is the same shape as adding any other module's action.
- The test surface grows: `ConfigModule` gets the same test coverage as every other Executor.

## Cross-references

- The Executor trait this module now implements: [F001](F001-cli-shape.md).
- The data-driven clap subcommand tree that consumes `module_arg_spec()`: [F007](F007-clap-subcommand-tree.md).
- The argument parser the module uses internally: [R005](R005-parse-simple-args.md).
- The JSON mode it inherits: [R001](R001-thread-local-json-mode.md).
- The project rule against special cases in `main.rs`: `agents.md` §"Executor trait" + the "new module = one file + one line" rule.