# 03-testing.md — Testing Requirements

> Backed by: F-series for CLI surface guarantees, M/C/T/N/B/L-series for module
> contracts. This file defines how the testing discipline actually operates on
> the ground.

## Where tests live

- Unit tests: `#[cfg(test)] mod tests` at the **bottom** of the source file they
  test. Keeps the test next to its code without exporting anything.
- Integration tests (cross-module or binary entry-point): the `tests/` directory.
- Skip-net tests (mock IMAP, mock CalDAV, mock Notion): gated behind
  `#[ignore]` or behind a `#[cfg(feature = "integration-tests")]` flag. CI does
  not run them by default.

## What must be tested

The following have no exceptions — every module must cover them:

1. **Config loading + multi-account resolution.**
2. **Output rendering**: both `Text` and `--json` paths, including the JSON
   envelope on failure (see [R002](../docs/adr/R002-output-json-failure.md)).
3. **AgentError serialization** to the JSON envelope `{"error": "Type", "message": "..."}`.
4. **Each `Executor` implementation:** at least one happy-path integration test
   that uses `--json` and asserts on the parsed output.
5. **SQL layer (mail cache / timeline / ops-log):** upsert idempotency, watermark
   monotonicity, UIDVALIDITY reset, K1 append retention, GLOB token matching
   (vs LIKE).

## Mocking policy

- Network calls (IMAP, SMTP, CalDAV, Notion, RSS fetcher): wrapped behind a trait
  so tests inject a fake. Fakes live next to the trait definition.
- Time-dependent code: take a `Clock: Fn() -> DateTime<Utc>` (or use
  `tokio::time::pause()`).
- Random IDs: take a `gen_id: Fn() -> String` or an `AtomicU64` that tests can
  reset.

## Quality bar before pushing

- `cargo test` — every unit test green; **all** ignored tests justified
  (comment with what infrastructure they need).
- Coverage is **not** enforced as a percentage gate, but every module's core
  path must be hit by at least one test.

## When to add a test

- Every bug fix lands with **at least one new test** that reproduces the
  failure mode.
- Every refactor that touches behavior (not just formatting) must keep the
  existing tests green; add new ones if a previously untested path is exercised.

## Tooling

- `cargo test` runs everything unless told otherwise.
- `cargo test --lib <module>` for a focused loop.
- `cargo test -- --nocapture` when the failure message needs stdout to be visible.
- `cargo bench` is not used; performance budgets are enforced structurally
  (see [F009](../docs/adr/F009-performance-budget.md)).
