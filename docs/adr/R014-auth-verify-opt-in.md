# ADR R014: `verify` is an explicit opt-in step, separate from credential storage

**Status:** Accepted
**Date:** 2026-07-12

## Context

Before this decision, `login` only *stored* a secret in the keyring and never contacted the server. Two distinct needs were therefore unmet:

1. **Store without network.** Saving a credential should be fast and work offline — no point connecting to IMAP just to remember a password.
2. **Re-verify a stored credential without re-entering it.** Credentials expire, tokens get revoked, passwords rotate. An agent (or a human) needs to ask "is what I have still valid?" without being prompted to type the secret again.

Confusing the two leads to either (a) `login` doing a network round-trip every time (slow, fails offline) or (b) no way to check validity at all (the historical state).

## Decision

**Credential storage is the default; authentication (`verify`) is an explicit, opt-in action.**

- `everyday auth login --module <mod> [--account <name>]` — **stores only** (credential management). No server contact.
- `everyday auth login --module <mod> [--account <name>] --verify` — stores, then connects to verify the stored credential (store + verify in one call).
- `everyday auth verify --module <mod> [--account <name>]` — reads the **already-stored** credential from the keyring and connects to verify it. **No interactive input.** Returns success/failure with a provider-specific error message.
- For the `None` strategy (local/sqlite provider, rss) `verify` short-circuits and returns `not_required` rather than an error.

`verify` success means the credential produced a successful authentication against the external service (IMAP/CalDAV `LOGIN`, or a Notion API call). Failure surfaces the underlying auth error (e.g. `imap login failed`, `notion 401`).

## Alternatives considered

### Flag-only (`--verify` on `login`, no standalone `verify`)
- Simplest, but to re-check a credential you must re-run `login` and re-type the secret — defeats the "check without re-prompting" goal, especially painful for AI automation.
- Rejected.

### Always verify on `login` (no separation)
- `login` becomes slow and fails offline; couples "remember my password" to "server reachable right now".
- Rejected as the default (still available on demand via `--verify`).

## Consequences

- Clear separation: `auth login` = "I have a secret, keep it"; `auth verify` = "is the secret I kept still good?".
- AI agents can re-verify credentials on a schedule or after an auth error **without** re-prompting for the password.
- The `None`-strategy short-circuit keeps `auth list` / `auth verify` uniform across all modules even where no credential exists.
- Implementation reuses the same connection primitives as normal module actions (see [R013](R013-auth-module-consolidation.md)).

## Cross-references

- Credential lifecycle ownership: [R013](R013-auth-module-consolidation.md)
- Non-interactive input for `login`: [R015](R015-auth-credential-io.md)
