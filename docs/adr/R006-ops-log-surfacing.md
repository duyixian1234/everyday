# ADR R006: Surface ops-log write failures to the user

**Status:** Accepted
**Date:** 2026-07-11

## Context

The AOP ops-log hook ([L007](L007-notion-ops-log.md), [L011](L011-aop-handles-output-text.md)) runs **after** the user's command has succeeded:

```rust
let output = module.execute(action, args)?;
ops_log::maybe_log_op(module, action, account, &output)?;  // ← was `let _ = ...`
```

The original code used `let _ = ops_log::maybe_log_op(...)` — failures were silently swallowed. The user's command succeeded; the timeline just didn't get the event. They had no way to know.

Symptoms:

- A notion `todo add` writes successfully.
- The timeline shows no event.
- The user / agent assumes the timeline is up to date.
- Debugging requires running the CLI under a debugger and watching the ops-log.

## Decision

**Ops-log write failures must be surfaced to stderr (or the error envelope in `--json` mode) without aborting the user's command.**

Implementation:

```rust
match ops_log::maybe_log_op(module, action, account, &output) {
    Ok(()) => {}
    Err(e) => {
        if is_json() {
            eprintln!("{}", json!({"_warning": "OpsLog", "message": e.to_string()}));
        } else {
            eprintln!("warning: ops-log write failed: {}", e);
        }
    }
}
```

- The user's command output is unchanged.
- A warning appears on stderr.
- In `--json` mode, the warning is a structured `_warning` JSON line that agents can recognize and act on (e.g. trigger an immediate `timeline sync` or alert the user).
- Exit code stays 0 — the user's command succeeded; the audit log didn't.

The fix is one of the "risk"-category items from the 2026-07-11/12 review — see commit `79739cd`.

## Alternatives considered

### Promote ops-log failures to `AgentError`

- Fails the user's command even though the data write succeeded.
- Conceptually wrong: the user asked "add a todo", not "add a todo and audit-log it".
- Rejected.

### Silent swallow (status quo)

- Status quo.
- Breaks the principle of least surprise: the user has no signal that something went wrong.
- Rejected.

### Write ops-log in a background thread that retries

- More robust against transient SQLite locks.
- Considered; deferred to a future "ops-log daemon" if write failures become common.
- For now: surface + manual investigation.

### Block the user command until ops-log succeeds

- Strictest consistency: user command and audit log are atomic.
- Cost: a transient SQLite lock aborts the user's todo add, even though Notion already accepted it.
- Rejected.

## Consequences

- A failure to write ops-log no longer goes unnoticed.
- The Agent can choose to retry the sync (and thus re-derive the missing event from the ops-log replay).
- The `--json` `_warning` envelope is a small new contract; consumers that ignore unknown fields are unaffected.
- This is the canonical pattern for "secondary side-effect failures" in this codebase: surface, don't block.

## Cross-references

- The AOP hook this protects: [L007](L007-notion-ops-log.md).
- The hook's text-mode parsing: [L011](L011-aop-handles-output-text.md).
- The JSON mode that controls the warning's format: [R001](R001-thread-local-json-mode.md).
- The same pattern applied to `timeline` sync / insert failures: [L009](L009-best-effort-sync.md).