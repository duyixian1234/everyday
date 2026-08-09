# ADR R020: Environment-variable credential fallback (opt-in)

**Status:** Accepted
**Date:** 2026-08-09

> **Revises** [R015](R015-auth-credential-io.md), which rejected reading secrets from the environment. This ADR opens a **controlled, opt-in exception**: when the OS keyring backend is unavailable, credentials may be read from environment variables. R015's default remains authoritative — everyday never reads secrets from the environment unless the user explicitly enables the fallback.

## Context

Everyday is the AI agent's "hands": it runs on headless servers, CI runners, and sandboxes where **no OS keyring backend exists** (Linux without DBus/Secret Service, Windows sandboxes without Credential Manager, minimal containers).

Today, when the keyring backend is unavailable:

- `auth login --password <pwd>` fails with `KeyringUnavailable` — the only non-interactive injection path (R015) is dead.
- The interactive `rpassword` prompt (F002) blocks automation: the agent cannot inject keystrokes into a pseudo-terminal.
- There is **no third path**: secrets cannot be supplied in a way that survives across process invocations.

R015 rejected environment variables on security grounds: *"env would propagate the secret to every child process the agent later spawns."* That concern is real, but the current situation makes headless deployments *impossible*, not merely less secure. The resolution is an **explicit opt-in** that restores R015's default (no env) while giving headless operators a sanctioned escape hatch.

## Decision

### Trigger — explicit opt-in, dual-channel (never automatic)

Environment-variable credentials are **disabled by default**. The fallback activates only when **either** of these is set:

- Config: `[auth] env_credentials = true` in `~/.config/everyday/config.toml`, or
- Environment: `EVERYDAY_ENV_CREDENTIALS=1`.

The dual channel exists because business modules read credentials via `auth::get_credential_with_user(module, account, user)` which — by design (P2b, [F012](F012-architecture-deepening-phase.md)) — does **not** hold the global `Config`. Those call sites consult the environment switch; call sites with `Config` consult the config field **or** the environment switch. A headless agent enables everything with one export, no config edit needed.

There is **no auto-detection**: a silently falling-back default would bypass R015's red line without the user's knowledge.

**Switch scope (important).** The config field is only visible at call sites that hold the full `Config` (`auth list` / `auth verify` / a few module paths). The business-module **hot paths** — `email::imap_connect`, the calendar connect path, `sync` — read credentials via `auth::get_credential_with_user(module, account, user)`, which by design (P2b) has **no `Config`** and therefore consults the **environment switch only**. In practice: a headless agent that only sets `[auth] env_credentials = true` gets the fallback for `auth`-level commands but **not** for `mail`/`cal`/`sync`. Exporting `EVERYDAY_ENV_CREDENTIALS=1` activates the fallback everywhere. The read error message on no-`Config` paths tells the user exactly this.

### Naming

`EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD`, where:

- `<MODULE>` = the auth-internal module key: `MAIL` / `CAL` / `WEBDAV`.
- `<ACCOUNT>` = the account name, uppercased with every non-`[A-Z0-9]` character replaced by `_` (e.g. `work` → `WORK`, `my-work` → `MY_WORK`, `m1` → `M1`).

Collision: distinct account names may normalize to the same variable (e.g. `my-account` vs `my_account` → `MY_ACCOUNT`). **Accepted** — such names are an anti-pattern; documented, not encoded.

Example: `export EVERYDAY_MAIL_WORK_PASSWORD=hunter2`.

### Read precedence

`keyring → env → error`.

- The OS keyring, when it yields a credential, **always wins** — it remains the strongest security boundary.
- When the keyring entry is missing **or** the backend is unavailable, and the fallback is enabled, the env variable is consulted.
- If neither yields a credential, the existing auth error (with the `auth login` hint) is returned. No silent fallbacks.

### Action semantics

| Action | Behavior |
|---|---|
| `auth login` | Always writes the **keyring**. Keyring unavailable → explicit error naming the env variable to export (`export EVERYDAY_MAIL_WORK_PASSWORD=...`). Never pretends success. |
| `auth logout` | Deletes the keyring entry. Credential actually sourced from env (no keyring entry) → explicit error telling the user to `unset` the variable. |
| `auth verify` | Uses the same read chain (`keyring → env`) — an env-sourced credential can be verified against IMAP/CalDAV/WebDAV for real. |
| `auth list` | Fourth state `env`: keyring hit = `stored`; no keyring entry but env hit = `env`; neither = `missing`; no credential needed = `not_required`. Agents can branch on `status == "env"` (fix the export, don't run login). |

**Dual-source boundary (logout).** When the keyring entry **and** an env variable both exist, `logout` deletes only the keyring entry and appends a note that the env variable is still set (reads continue to succeed through the fallback). The keyring is the only thing `logout` can delete; env variables are the user's to unset. This is documented rather than made an error — the user may legitimately want the env var to keep serving other machines.

### Security mitigations

1. **Explicit opt-in only** — the config field defaults to `false`; the env switch must be set deliberately.
2. **Documented risk** — README, config example, and this ADR state that env-sourced secrets are visible to every child process the agent spawns.
3. **Visible source** — `auth list` marks env-sourced credentials as `env`, so the source is never silent.

No process-isolation mechanisms are attempted: that is the operating system's job and cannot be reliably done at the CLI layer.

## Alternatives considered

### Auto-detect keyring unavailability and fall back silently
- Bypasses R015 without user consent; mis-detection would silently switch security models.
- Rejected.

### Config-only switch
- `get_credential_with_user` has no `Config` (P2b). Would force a breaking signature change on 6 call sites and drag business modules back to a global `Config` dependency — an architecture regression.
- Rejected.

### Env-only switch
- Simplest, but removes the human-facing config affordance; inconsistent with the config-first convention.
- Rejected.

### Env precedence over keyring
- Makes "temporary override" easy but weakens the keyring as the source of truth; a stale env var could silently shadow a fresh keyring login.
- Rejected.

### Encoding collisions (`-` → `_`, `_` → `__`)
- Robust but produces non-intuitive variable names; the collision case is rare and documented.
- Rejected.

## Consequences

- Headless deployments now have a working credential path: `export EVERYDAY_ENV_CREDENTIALS=1` + per-account `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD`.
- Default behavior is unchanged: with no opt-in, everyday still never reads secrets from the environment (R015 holds).
- `auth list` grows a fourth status value (`env`) — consumers of the JSON output must be aware.
- `auth verify` consumes the same read chain; its env path is covered by the `get_credential` read-chain tests (the network verification itself depends on a live service and is not unit-testable offline).
- The security red line in `agents.md` / [F002](F002-multi-account-keyring.md) ("credentials never in config or logs") is untouched: env-sourced secrets still never appear in `config.toml` or logs.

## Cross-references

- Non-interactive input contract being revised: [R015](R015-auth-credential-io.md)
- Keyring service format + failure semantics: [F002](F002-multi-account-keyring.md)
- Credential lifecycle ownership: [R013](R013-auth-module-consolidation.md)
- Business modules hold config subsets, not the global `Config`: [F012](F012-architecture-deepening-phase.md)
