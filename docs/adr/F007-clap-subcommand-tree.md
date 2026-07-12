# ADR F007: Data-driven clap subcommand tree via module_arg_spec

**Status:** Accepted
**Date:** 2020-07-12 (implemented 2026-07-12, commit `0c41559`)

## Context

The first implementation of help/registry had a problem:

- `clap`'s built-in `--help` intercepts at the top level before `main.rs` can dispatch. The previous code therefore **pre-scanned raw arguments** (`module_help`, `action_help`, `detect_subcommand_help`) and re-built a `ModuleRegistry` just to render help.
- The pre-scan was a parallel argument parser that could disagree with the real parser. It also duplicated the registry construction logic.
- Adding a new module meant teaching three places about it: the module itself, the registry, the help pre-scanner.
- `Executor::name()` and `actions()` were vestigial — clap was already authoritative for the help surface; the methods only existed to feed the pre-scanner.

A second concern was argument coercion: clap's typed `ArgMatches` downcast can panic if a module declares the wrong kind, and modules were reading positional values without checking whether they were declared.

## Decision

**Each module declares its CLI surface as data; `cli.rs` builds the clap tree from that data; help is delegated to clap natively.**

### The data shape

```rust
pub trait Executor {
    // ...
    fn module_arg_spec(&self) -> ModuleArgSpec;
}

pub struct ModuleArgSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub actions: &'static [ActionArgSpec],
    pub global_flags: &'static [ArgSpec],
}

pub struct ActionArgSpec {
    pub name: &'static str,
    pub flags: &'static [ArgSpec],
    pub positionals: &'static [Positional],
}
```

All fields are `&'static`. The cost is zero at runtime — clap is built from immutable static data.

### ArgKind typing

```rust
pub enum ArgKind { Bool, Value, Multi }
```

`matches_to_args()` reads arguments strictly per the declared `ArgKind` and `Positional`. Modules no longer need to downcast clap matches; they call `parse_simple_args` (see [R005](R005-parse-simple-args.md)) on the resulting `Vec<String>` as before.

### What was deleted

- `module_help` / `action_help` / `detect_subcommand_help` and the `ModuleRegistry` reconstruction branch in `main.rs`.
- `Executor::name()`, `Executor::actions()`, and the `ActionDoc` struct (the methods were only consumed by the deleted help path).
- Help is now rendered by clap itself: `everyday --help`, `everyday <module> --help`, `everyday <module> <action> --help` all work natively, including when `config.toml` is corrupt.

## Alternatives considered

### Keep the pre-scan and rebuild registry for help

- Status quo. Doesn't fix the parallel-parser drift; module authors have to update three places.
- Rejected.

### Use `clap_derive` with `#[derive(Parser)]` per module

- Pro: idiomatic, less boilerplate.
- Con: each module would own its own `clap` derive — the dispatch in `main.rs` would still need to know which `Parser` to construct.
- Con: derivation cost paid at runtime (parser construction), whereas static data is zero-cost.
- Con: harder to share global flags uniformly.
- Rejected.

### Keep `--help` as a special module action

- Treat help as just another `Executor::execute("help", ...)` call.
- Pro: no clap tree at all.
- Con: re-implementing what clap already does; would have to maintain formatting.
- Rejected.

## Consequences

- Adding a module still means "one file + one registration line", but the registration now also covers the help surface automatically.
- `parse_simple_args` keeps its job inside modules (see [R005](R005-parse-simple-args.md)); the data-driven clap tree only feeds modules `Vec<String>` they already knew how to parse.
- Quality gates caught a regression: `--source bogus`, `--limit -1`, and a single bad `--since` date now fail at the clap boundary or at the explicit `resolve_query_range` step ([L013](L013-from-explicit-error.md)), so silent fallbacks can't return.
- Module authors must now declare `ArgKind` correctly; an incorrectly declared `Value` flag produces a clear clap error rather than a panic on downcast.
- The dead-code attributes that were hiding the help infrastructure were also removed, restoring normal `#[deny(dead_code)]` discipline.

## Cross-references

- The CLI shape this serves: [F001](F001-cli-shape.md).
- The argument parser it feeds: [R005](R005-parse-simple-args.md).
- The `--from` solo error it forced to surface: [L013](L013-from-explicit-error.md).
- Refactoring that folded the config module into the same Executor dispatch: [R012](R012-config-executor-trait.md).