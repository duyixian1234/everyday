# 02-coding-style.md — Rust Coding Style

> Backed by:
> - [F001](../docs/adr/F001-cli-shape.md) for the public module interface.
> - [R-series](../docs/adr/README.md#refactoring-patterns-r-series) for reusable patterns
>   this style enforces.

## Style basics

- `cargo fmt` with default settings, gated by `cargo fmt --check` in CI
  ([F006](../docs/adr/F006-ci-release-github-only.md)).
- `cargo clippy --all-targets -- -D warnings` is the lint bar — zero warnings
  required. CI fails on any clippy finding.
- Every public type carries `#[derive(Debug, Clone)]`. Config structs additionally
  derive `Deserialize` and `Serialize`.
- Public APIs have `///` doc comments. Modules carry `//!` at the top of the file
  to describe purpose and key invariants.
- Async functions use `async fn`. Trait methods need `#[async_trait]`.

## Naming

- Module files: lower case (`email.rs` — the module name describes the domain,
  not the CLI verb). CLI command aliases use the short form: `mail` → 邮件,
  `cal` → 日历.
- Types and traits: `PascalCase`.
- Functions and variables: `snake_case`.
- Constants: `SCREAMING_SNAKE_CASE`.
- Allocator / collection-like types: follow standard Rust convention (`Vec<T>`,
  `HashMap<K, V>`).

## Macros

- Define at **module scope**, never inside an `impl` block. Rust's macro_rules
  hygiene rejects `impl` block entries.
  - See [R007](../docs/adr/R007-config-account-macro.md).
- Prefer a macro over a generic helper when the signature would need to return
  `&str` borrowed from a `&dyn Fn` closure — the lifetime gymnastics are not
  worth it. Example: `add_dual_providers!` ([R011](../docs/adr/R011-add-dual-providers-macro.md)).

## Async / runtime

- All code is `tokio`. The runtime is single-threaded `current_thread` unless the
  caller requests otherwise.
- `tokio::runtime::Handle::try_current()` must guard any `tokio::spawn` call
  inside a `Drop` impl — otherwise the session is leaked when the runtime is
  shut down. See [R003](../docs/adr/R003-pool-guard-drop.md).
- DST-boundary date parsing: use `.earliest()` or `.latest()`, never
  `.unwrap()`. Spring-forward gap → `None`; fall-back ambiguity → `Some(latest)`.
  See [R004](../docs/adr/R004-dst-boundary-dates.md).

## Dependencies

- Run a `cargo tree -i <crate>` before adding anything new — if `everyday` only
  needs it for one module, gate it behind a feature flag.
- Prefer `rustls-tls` over `native-tls` to avoid OpenSSL chains.
- Avoid `default-features = true` — list the features you need.
- Record the reason for every new dep in `findings.md` **only if** the choice is
  a decision (e.g. "shared Notion client SDK vs module-local HTTP"). Otherwise it
  lives in [07-dependency-pitfalls.md](07-dependency-pitfalls.md) as a footnote.
- Knowledge-cutoff-detected crate quirks live in
  [07-dependency-pitfalls.md](07-dependency-pitfalls.md).

## No `unwrap()` in non-test code

- Production code uses `?` + `map_err` + context. `unwrap()` / `expect()` are
  allowed only inside `#[cfg(test)]` modules.
- Constructor `Result` returns are preferred over `expect` (e.g. `NotionClient::new`
  returns `Result` rather than `expect`-ing).

## Parse / arg conventions

- `parse_simple_args`: single-dash tokens (`-X`, `-1`) are **values**; double-dash
  tokens (`--xxx`) are **flags**. Reject negative numbers as flags.
  See [R005](../docs/adr/R005-parse-simple-args.md).
- Explicit `--json` is detected once at startup and stored in a thread-local
  (`is_json()` / `set_json_mode()`), not by re-scanning `std::env::args()`. See
  [R001](../docs/adr/R001-thread-local-json-mode.md).
- SQL token-boundary matching uses `GLOB`, not `LIKE`. See
  [R008](../docs/adr/R008-sql-glob-not-like.md).
- Argument validation that can fail should produce
  `AgentError::InvalidArgument`, never a silent fallback to a default.
