# ADR R009: Common `local` module for shared Notion abstractions (`login_flow`, `parse_tags`, `set_module_database_id`)

**Status:** Accepted
**Date:** 2026-07-11

> **Update (2026-07-12):** The `login_flow` helper described here moved into the top-level `auth` module; per-module `login` is removed. The `parse_tags` / `set_module_database_id` helpers remain in `local`. See [R013](R013-auth-module-consolidation.md).

## Context

`note`, `todo`, and `bookmark` are three Notion-backed modules. Each originally wrote its own:

- **`login_flow`**: prompt for a Notion token (if missing), verify against Notion (`users.me`), store in keyring.
- **`parse_tags`**: turn a comma-separated `--tags a,b,c` string into `Vec<String>`.
- **`set_module_database_id`**: write the database id returned by `init-db` back into `config.toml` under the right module's account.

Three modules × three helpers = nine near-identical copies. Drift was already visible:

- `note::login_flow` validated the token with `users.me`.
- `todo::login_flow` accepted any non-empty token (validation came later in `list`).
- `bookmark::login_flow` didn't exist; the user had to set the token manually.

## Decision

**Consolidate the three helpers into a single `crate::modules::local` module.**

```rust
// src/modules/local.rs
pub async fn login_flow(module: &str, account: &str) -> Result<(), AgentError> { ... }
pub fn parse_tags(input: &str) -> Vec<String> { ... }
pub fn set_module_database_id(module: &str, account: &str, db_id: &str) -> Result<(), AgentError> { ... }
```

Each Notion-backed module's `login` / `add` / `init-db` actions delegate to these helpers. The local SQLite versions of `note` / `todo` / `bookmark` also use the helpers (the helpers work for both providers — the difference is what keyring entry they look at).

The fix spans three commits in the "abstraction"-category from the 2026-07-11/12 review:

- `0d99aa7` — `parse_tags` consolidation.
- `3cd4397` — `set_module_database_id` consolidation.
- `a2cbf74` — `login_flow` consolidation.

## Alternatives considered

### Keep per-module helpers, add a `notion_common` crate

- Three small helpers don't justify a separate crate.
- Crate extraction is a one-way door (changes module paths for everyone).
- Rejected.

### One helper per file in a new `notion_common/` directory

- Same effect as `local.rs` but with more files.
- Rejected: the helpers are small enough to share a file.

### Trait-based polymorphism

- `trait NotionModule { fn login_flow() -> ...; fn parse_tags() -> ...; }`
- Overkill for three methods with identical bodies.
- Rejected.

### Make the helpers macros

- Macros are evaluated at the call site, not declared once.
- The helpers have actual logic (async, error mapping), not just boilerplate.
- Rejected.

## Consequences

- All three modules now use the same `login_flow`: prompt, validate against `users.me`, store in keyring.
- `parse_tags` handles whitespace and empty inputs uniformly.
- `set_module_database_id` uses the same config-edit path (`toml::Value` manipulation) for every module.
- Adding a new Notion-backed module is now: implement the module, call the three helpers, done.
- `local.rs` becomes a small but critical file — it has its own tests.

## Cross-references

- The shared Notion client these helpers sit on top of: [F004](F004-shared-notion-client.md).
- The macro for `Config::X_account()` lookups: [R007](R007-config-account-macro.md).
- The merged account types: [R010](R010-notion-local-account.md).
- The dual-provider build pattern: [R011](R011-add-dual-providers-macro.md).