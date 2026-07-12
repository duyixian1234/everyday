# ADR F002: Multi-account configuration + OS keyring credentials

**Status:** Accepted
**Date:** 2026-07-08

> **Update (2026-07-12):** The keyring *service format* below is unchanged and remains authoritative. Ownership of credential store / get / delete / verify moved to the top-level `auth` module — see [R013](R013-auth-module-consolidation.md) (and [R014](R014-auth-verify-opt-in.md) / [R015](R015-auth-credential-io.md)).

## Context

Users typically have more than one of each integration: a work mail account and a personal one, two calendars, several RSS feeds grouped by topic. The CLI must:

- Name each account so a single command can pick one (`mail list --account work`).
- Persist non-secret configuration in a file the user can inspect and edit.
- Persist secrets (IMAP/SMTP passwords, Notion tokens) somewhere that survives reinstalls but never ends up in plaintext on disk or in logs.

## Decision

### Configuration file

- Path: `~/.config/everyday/config.toml` (resolved via `dirs::config_dir()` for cross-platform parity).
- Top-level: `[default_account]` mapping `module → account_name`.
- Each module: `[[<module>.accounts]]` array. An account has at least a `name` and module-specific fields (host, port, username).
- Username / host / port are stored in plaintext. Secrets never are.

### OS keyring for secrets

- Crate: `keyring`.
- Service name: `everyday/<module>/<account>`.
- Account name: the upstream username (e.g. the IMAP login, the Notion token's owning account).
- Prompts for missing passwords via `rpassword::prompt_password` (run inside `tokio::task::spawn_blocking`).

### Account resolution

- If `--account` is given on the command line, it overrides the default.
- Otherwise, `[default_account.<module>]` is consulted.
- If neither yields an account, the executor returns `AgentError::AccountNotFound` — never silently picks the first account or panics.

### Keyring failure semantics

- Empty password or keyring entry missing → prompt the user; if still empty after prompt → `Auth` error.
- Keyring backend unavailable (headless server, sandbox without DBus/Secret Service) → `AgentError::KeyringUnavailable` with the configured fallback (typically interactive prompt).

## Alternatives considered

### One global default account across modules

- `everyday --account personal mail list` would always be the work account because "personal" is the global default.
- Rejected: account naming is per-domain.

### Secrets in config file with `chmod 600`

- Single file easier to back up, but operators routinely check config into dotfiles repos.
- Any logging or error path that prints config leaks the secret.
- Rejected: explicit project rule "credentials never in config or logs".

### OS-specific secret store per platform

- Keychain on macOS, Credential Manager on Windows, Secret Service on Linux.
- The `keyring` crate already abstracts these and degrades gracefully on missing backends.
- Rejected: would only add code, not capabilities.

### Storing secrets in SQLite

- Rejected: defeats the OS sandbox model, complicates backup, and a leaked DB exposes everything.

## Consequences

- A user editing `config.toml` never accidentally pastes a token there — the password prompt is the only path.
- All modules share the same account-resolution convention (see [M001](M001-imap-stack.md) for the mail-specific Keyring username convention).
- Adding a new secret to a module only requires picking the keyring service/account format and adding the entry to the docs.
- Cross-module tools (e.g. [L007](L007-notion-ops-log.md)'s ops-log hook) can rely on `account` being a stable string identifier.

## Cross-references

- Implementation contract for thread-local JSON mode: [R001](R001-thread-local-json-mode.md).
- CalDAV-specific account handling: [C001](C001-caldav-stack.md).
- Notion keyring convention: [N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md).