# ADR R015: Non-interactive credential input via flags; secrets never read from environment

**Status:** Accepted
**Date:** 2026-07-12

## Context

Everyday is the AI agent's "hands" — it is invoked programmatically, frequently without a TTY. The legacy `login` used `rpassword`, an **interactive** prompt, which blocks automation (the agent would have to inject keystrokes into a pseudo-terminal — fragile and insecure).

We need a non-interactive path to supply a secret, with a clear security boundary: secrets must not leak into child-process environments.

## Decision

**`auth login` accepts the secret as a flag; when the flag is absent it falls back to the interactive prompt.**

- Password strategy: `everyday auth login --module mail --account work --password <pwd>`
- Token strategy: `everyday auth login --module note --account default --token <ntn_...>`
- Secrets are passed via **argv** (visible only to the process, short-lived, consistent with how `config set` already accepts values). They are **never** read from environment variables — env would propagate the secret to every child process the agent later spawns.
- In JSON mode the prompt and the secret are **not echoed**; output is the structured result only.
- Interactive prompt remains the fallback when neither flag is given, preserving human usability.

## Alternatives considered

### stdin pipe (`echo <tok> | everyday auth login --module note --account default --stdin`)
- More secure than argv (no process-list visibility), but forces the agent to shell-escape the secret and manage a pipe; more friction for marginal gain given everyday already accepts values via argv elsewhere.
- Considered, not chosen as the primary path.

### Read secret from environment variable
- Trivial for automation, but leaks the credential into the environment of every subsequent child process. Unacceptable.
- Rejected.

### Interactive only
- Blocks AI automation entirely.
- Rejected.

## Consequences

- `everyday auth login --module mail --account work --password <pwd> --verify` runs store + verify end-to-end with no TTY.
- argv exposure is accepted as bounded (process-table visibility for the command's lifetime), consistent with the project's existing `config set` precedent and the fact that everyday never persists plaintext to disk.
- Modules no longer prompt for secrets themselves; the prompt logic lives solely in `auth`.

## Cross-references

- Credential lifecycle ownership: [R013](R013-auth-module-consolidation.md)
- Verify semantics: [R014](R014-auth-verify-opt-in.md)
- Secrets live only in the OS keyring, never config: [F002](F002-multi-account-keyring.md)
