# 04-security.md — Security Red Lines

> Backed by [F002](../docs/adr/F002-multi-account-keyring.md) for credential storage
> and [F009](../docs/adr/F009-performance-budget.md) for network timeouts. This file
> is the operational checklist; the ADRs are the rationale.

## Credentials

- ❌ **Never** store passwords or tokens in `config.toml`, env vars, command line,
  or logs.
- ✅ Secrets only via [`keyring`](https://crates.io/crates/keyring) under service
  name `everyday/<module>/<account>` (see [F002](../docs/adr/F002-multi-account-keyring.md)).
- ✅ Empty-password outcome: `Auth` error — never a retry loop or a panic.
- ✅ On a headless box with no keyring backend: surface
  `AgentError::KeyringUnavailable` and offer the interactive prompt as
  fallback (already implemented in modules).

## Network calls

- ✅ Every `reqwest::Client` is built with `.timeout(...)`. Default: 30s for
  reads, 10s for sends (see [F009](../docs/adr/F009-performance-budget.md)).
- ❌ No use of `tokio::net::TcpStream` without a `tokio::time::timeout` wrapper.
- ❌ No retry loop on `Auth` / `InvalidArgument` — these are terminal.
- ✅ On 429 (Notion): back off using the `Retry-After` header (default 1s), once
  only. See the [Notion shared client decision](../docs/adr/F004-shared-notion-client.md).

## Local file operations

- ✅ Reading `config.toml` handles `PermissionDenied` → `AgentError::PermissionDenied`.
- ✅ Writing the SQLite caches (`mail_cache.db`, `timeline.db`, `ops-log.db`)
  creates the parent dir; never assumes it exists.
- ❌ Do not write user-controlled paths without normalization (no `..`).

## Output and logging

- ❌ `println!` / `eprintln!` of a full IMAP envelope risks leaking
  `Message-ID` correlators. Prefer the rendered `Text` form (subject + counterparty
  + date) only.
- ✅ JSON output never embeds the credential — even on `Auth` errors, the
  message is "missing credential for `<account>`", not the credential value.
- ✅ `--json` is the contract boundary: errors with no `serde::Serialize`
  impl must fall back to a generic envelope — see
  [R002](../docs/adr/R002-output-json-failure.md).

## Concurrency hazards

- A `Drop` impl that calls `tokio::spawn` **must** first try `Handle::try_current()`.
  Without it, dropping a `PoolGuard` after the runtime exits panics. See
  [R003](../docs/adr/R003-pool-guard-drop.md).
- `Local.from_local_datetime(&ndt).unwrap()` panics on DST spring-forward; use
  `.earliest()` or `.latest()`. See [R004](../docs/adr/R004-dst-boundary-dates.md).

## Reporting

Found a vulnerability or a red-line breach in this codebase? Open an issue
**without** a public reproducer — security over visibility.
