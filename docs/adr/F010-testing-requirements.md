# ADR F010: Testing requirements — coverage, mocks, and CI behaviour

**Status:** Accepted
**Date:** 2026-07-12

## Context

Everyday is an AI-Agent-facing CLI. A regression that breaks a JSON output shape
silently degrades the Agent's downstream behaviour, with no stack trace. A panic
during `--json` execution silently destroys the Agent's parseable response.
Both classes of regression are high-blast-radius.

Two prior collapses confirmed the need for explicit testing rules:

1. The original `src/output.rs` could panic with `Result::unwrap` during
   `Table` rendering — silent regression only caught by ad-hoc usage.
2. The timeline implementation's `query_events` had a parameter-binding bug
   that returned half the expected rows in tests but looked correct in
   production logs.

This ADR codifies the non-negotiable testing discipline so that "no tests"
becomes a merge blocker rather than an oversight.

## Decision

### 1. Mandatory tests

- **Configuration loading** must be covered: happy-path, missing-file path,
  malformed TOML path, multi-account resolution.
- **`Output` rendering** must be covered for both modes:
  - `Text` produces literal text.
  - `--json` produces parseable JSON for `Text` (via pass-through), `Json`
    (compact), and `Table` (JSON array).
  - JSON-envelope failure path: see [R002](R002-output-json-failure.md).
- **`AgentError` serialization** must produce the `{"error", "message"}`
  envelope for each variant that can plausibly surface in user-facing JSON.
- **Every `Executor` implementation** must have at least one happy-path
  integration test that exercises `--json` and asserts on the parsed output.

### 2. Network-touching code is mocked

- IMAP / SMTP / CalDAV / Notion / RSS fetchers run behind a trait that tests
  inject with a fake.
- An integration test that touches the network is gated behind `#[ignore]` or
  a feature flag; CI does not enable it.
- Time-dependent code takes a `Clock` trait or uses `tokio::time::pause()`.

### 3. Test layout

- Unit tests: `#[cfg(test)] mod tests` at the bottom of the file they cover.
- Integration tests (cross-module or binary entry-point): `tests/` directory.
- Shared test utilities (fake `Clock`, `gen_id`, mock IMAP session): in
  `tests/common/` or a `#[cfg(test)]` module in a `test_util` crate if a
  single binary crate does not suffice.

### 4. CI behaviour

- `cargo test` must pass on `ubuntu-latest`, `macos-latest`,
  `windows-latest` for every PR (see [F006](F006-ci-release-github-only.md)).
- `cargo clippy --all-targets -- -D warnings` is non-negotiable and runs
  together with `cargo test` on each CI target.
- A failing test is a blocker, not a discussion item.

### 5. Bug-fix discipline

- Every bug fix lands with **at least one new test** that previously failed
  (or proves the failure mode on the fixed branch).
- Refactors that touch observable behaviour must keep existing tests green
  and may add new tests for previously untested paths.

## Alternatives considered

### Coverage percentage gate

- Idea: enforce e.g. ≥ 80% line coverage in CI.
- Rejected: coverage tracks quantity, not the contracts that matter. The
  mandatory-test list above gives stronger guarantees on the failure modes
  that have actually bitten us.

### Property-based testing

- For date / time / envelope parsing, this would catch more.
- Deferred: the IMAP / CalDAV / Notion stacks have plenty of unmodelled
  quirks; adding proptest before we know the model would just produce
  rejected seeds.

### Network-in-CI by default

- Rejected: real-network tests are flake-prone and depend on per-test
  account provisioning. The current discipline (mock for unit, ignored for
  integration) keeps signal high and flake low.

## Consequences

- A regression that breaks a JSON contract is caught locally before push.
- A new module that skips tests is a merge blocker (CI fails).
- The cost: occasional "this test is fake" friction when wiring up a mock,
  paid back many times over by `cargo test` being trustworthy.

## Cross-references

- Where the rules apply operationally: [`.rules/03-testing.md`](../../.rules/03-testing.md).
- The CLI surface those tests verify: [F001](F001-cli-shape.md).
- Refactoring patterns documented from fix tests:
  [R-series](README.md#refactoring-patterns-r-series).
