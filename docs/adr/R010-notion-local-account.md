# ADR R010: `NotionLocalAccount` merge + type alias (TodoAccount / BookmarkAccount)

**Status:** Accepted
**Date:** 2026-07-11

## Context

The local-account record for `todo` and `bookmark` was **identical**:

```rust
struct TodoAccount {
    name: String,
    provider: String,
    // ... no other fields
}

struct BookmarkAccount {
    name: String,
    provider: String,
    // ... no other fields
}
```

`note`'s local account had a `default_database_id` field but otherwise matched. The three structs existed because each module wanted its own concrete type for `Config::todo_account(name) -> &TodoAccount`. Now that the lookup is macro-generated ([R007](R007-config-account-macro.md)), the concrete return type doesn't matter — the macro instantiates it.

## Decision

**Define a single `NotionLocalAccount` struct and use it (via type alias where helpful) for `note` / `todo` / `bookmark` local accounts.**

```rust
pub struct NotionLocalAccount {
    pub name: String,
    pub provider: String,
    pub default_database_id: Option<String>,
}

pub type TodoAccount = NotionLocalAccount;
pub type BookmarkAccount = NotionLocalAccount;
pub type NoteAccount = NotionLocalAccount;
```

The type aliases are kept so that call sites that say `&TodoAccount` continue to compile and read correctly. New code should prefer `&NotionLocalAccount` directly.

The fix is one of the "abstraction"-category items from the 2026-07-11/12 review — see commit `9524f12`.

## Alternatives considered

### Keep three structs, de-duplicate via a common trait

- `trait LocalAccount { fn name(&self) -> &str; fn provider(&self) -> &str; fn default_database_id(&self) -> Option<&str>; }`
- Trait adds a `dyn`-dispatch surface where none was needed.
- Rejected: aliases are simpler and zero-cost.

### Use `serde(tag = "module")` enum

- One struct with a `module: Module` field.
- Doesn't really save lines and complicates the config schema.
- Rejected.

### Drop the `default_database_id` field

- It's used by `init-db` to write back the created Notion database id.
- Removing it means writing back via a different path; more code, no win.
- Rejected.

## Consequences

- One struct definition, three type aliases. Less duplication, same type signatures.
- New code prefers `&NotionLocalAccount`. Existing code with `&TodoAccount` / `&BookmarkAccount` / `&NoteAccount` compiles unchanged.
- The macro from [R007](R007-config-account-macro.md) instantiates the concrete type per module — the alias keeps it readable.

## Cross-references

- The macro that looks up accounts: [R007](R007-config-account-macro.md).
- The shared notion abstractions: [R009](R009-notion-common-local-module.md).
- The dual-provider build: [R011](R011-add-dual-providers-macro.md).
- The original account types in the modules: [N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md), [B001](B001-bookmark-dual-provider.md).