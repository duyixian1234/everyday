# ADR R013: Consolidate all credential/login logic into a top-level `auth` module

**Status:** Accepted
**Date:** 2026-07-12

## Context

Every module that needs a secret currently owns its own `login` subcommand **and** its own keyring read/write:

- `everyday mail login` / `everyday cal login` — each carries a `get_password` + `<x>_login`; keyring user = `account.username`.
- `everyday note login` / `everyday todo login` / `everyday bookmark login` — share `crate::modules::local::login_notion` (see [R009](R009-notion-common-local-module.md)); keyring user = `KEYRING_USER = "token"`.
- local providers (`note_local` / `todo_local` / `bookmark_local`) carry a no-op `login` ("no login required").
- RSS has no credential at all.

Three concrete problems:

1. **Duplicated credential logic.** The keyring service format (`everyday/<module>/<account>`, see [F002](F002-multi-account-keyring.md)) is shared, but the *read/write code* is copy-pasted across five modules with two divergent keyring-user conventions (`username` vs `"token"`).
2. **No unified surface.** There is no way to list what is stored, delete a credential, or re-verify it without re-running a module's interactive `login`.
3. **`login` never authenticated.** Every existing `login` only *stored* the secret in the keyring; it never connected to the server. "Store" and "authenticate" were silently conflated — and authentication did not happen at all.

The project rule ([F001](F001-cli-shape.md)) favours one owner per cross-cutting concern and no special cases in `main.rs`. Credential handling is a cross-cutting concern that currently has *five* owners.

## Decision

**Create `crate::modules::auth` (an `AuthModule` implementing `Executor`) and register it as the top-level `auth` command. It owns the entire credential lifecycle.**

- **Actions:** `login`, `logout`, `list`, `verify`.
- **Credential ownership:** `auth` provides `store_credential`, `get_credential`, `delete_credential` (interactive prompt + keyring read/write). Modules call `auth::get_credential(module, account)` instead of their own `get_password` / `login_notion`.
- **Per-module `login` removed.** The `login` subcommands of `mail` / `cal` / `note` / `todo` / `bookmark` (and the local-provider no-op `login`) are deleted. This is a **breaking change**; scripts migrate to `everyday auth login --module <mod>`.
- **Command shape:** `everyday auth <action> --module <mod> [--account <name>] [--verify]`. `--account` reuses the existing global flag ([F007](F007-clap-subcommand-tree.md)) and defaults to the module's configured default account.
- **Strategy resolver — `resolve_strategy(module, account) -> AuthStrategy`, derived purely from `Config`** (no per-module declaration):
  - `Password` — `mail`, `cal`: keyring user = `account.username`; verify via IMAP / CalDAV connect.
  - `Token` — `note` / `todo` / `bookmark` with `provider = "notion"`: keyring user = `"token"`; verify via `notion_client`.
  - `None` — `note` / `todo` / `bookmark` with `provider = "local"`/`"sqlite"`, and `rss`: no credential; `verify` / `login` short-circuit to `not_required`.
- **Keyring layout frozen.** The service string `everyday/<module>/<account>` ([F002](F002-multi-account-keyring.md)) is **unchanged**. Only the keyring-user *selection* (username vs `"token"`) is centralized in `auth`. Existing stored credentials keep working.
- **`verify` reuses existing connection primitives** (`email::imap_connect`, the calendar connect path, `notion_client`) — no re-implementation of IMAP/CalDAV/Notion transport.

## Alternatives considered

### Keep `login` per module, add `auth` only as a thin alias
- Dual entry points; contradicts the consolidation goal; the duplication stays.
- Rejected.

### Two-phase removal (keep module `login` this release, delete next)
- Drags the migration across two releases; half-consolidated state in between.
- Rejected.

### `auth` fully self-contained, re-implements IMAP / CalDAV / Notion connect
- Duplicates transport code already owned by the modules; DRY violation.
- Rejected.

### Each module registers a `Verifier` into a registry that `auth` calls
- Cleaner abstraction, but over-engineered for five providers with stable connection entry points already in place.
- Rejected in favour of `auth` calling the existing `pub(crate)` connection functions directly.

## Consequences

- Single source of truth for credential store / get / delete / verify.
- **Breaking change:** `everyday mail login` / `cal login` / `note login` / `todo login` / `bookmark login` no longer exist.
- `auth` ↔ module is a **bidirectional reference within one crate** (`auth` calls `email::imap_connect`; `email` calls `auth::get_credential`). Single-crate Rust has no module topological constraint, so this compiles cleanly.
- `auth list` enumerates config accounts and probes keyring presence; each row carries `status ∈ {stored, missing, not_required}`.
- Legacy ADRs ([F002](F002-multi-account-keyring.md), [R009](R009-notion-common-local-module.md), [M001](M001-imap-stack.md), [C001](C001-caldav-stack.md), [N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md), [B001](B001-bookmark-dual-provider.md)) are updated to point login/credential logic at this ADR.
- Credential I/O contract (non-interactive input, secrets-not-from-env) is specified separately in [R015](R015-auth-credential-io.md); the explicit opt-in `verify` semantics in [R014](R014-auth-verify-opt-in.md).

## Cross-references

- CLI shape + Executor uniformity: [F001](F001-cli-shape.md)
- Frozen keyring service format: [F002](F002-multi-account-keyring.md)
- Data-driven clap tree reused for `auth`: [F007](F007-clap-subcommand-tree.md)
- Notion shared `login_notion` being absorbed: [R009](R009-notion-common-local-module.md) (partially superseded — the function moves into `auth`)
- Verify is an explicit opt-in: [R014](R014-auth-verify-opt-in.md)
- Credential input contract: [R015](R015-auth-credential-io.md)
