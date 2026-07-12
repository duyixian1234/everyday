# ADR R007: Macro for `Config::X_account()` lookups (module-scope, not inside `impl`)

**Status:** Accepted
**Date:** 2026-07-11

## Context

`Config` had five near-identical methods:

```rust
impl Config {
    pub fn mail_account(&self, name: &str) -> Result<&MailAccount, AgentError> { ... }
    pub fn cal_account(&self, name: &str) -> Result<&CalAccount, AgentError> { ... }
    pub fn note_account(&self, name: &str) -> Result<&NoteAccount, AgentError> { ... }
    pub fn todo_account(&self, name: &str) -> Result<&TodoAccount, AgentError> { ... }
    pub fn bookmark_account(&self, name: &str) -> Result<&BookmarkAccount, AgentError> { ... }
}
```

Each method did the same thing with a different field name and type:

```rust
let acct = self.mail.iter().find(|a| a.name == name)
    .ok_or(AgentError::AccountNotFound { ... })?;
Ok(acct)
```

Five copies of identical control flow with five different types. Adding a new module meant adding another copy.

## Decision

**Replace the five methods with a single macro at module scope.**

```rust
macro_rules! impl_account_lookup {
    ($config:ident, $method_name:ident, $field:ident, $account_ty:ty) => {
        impl $config {
            pub fn $method_name(&self, name: &str) -> Result<&$account_ty, AgentError> {
                self.$field.iter()
                    .find(|a| a.name == name)
                    .ok_or_else(|| AgentError::AccountNotFound {
                        module: stringify!($field).into(),
                        account: name.into(),
                    })
            }
        }
    };
}

impl_account_lookup!(Config, mail_account, mail, MailAccount);
impl_account_lookup!(Config, cal_account, cal, CalAccount);
impl_account_lookup!(Config, note_account, note, NoteAccount);
impl_account_lookup!(Config, todo_account, todo, TodoAccount);
impl_account_lookup!(Config, bookmark_account, bookmark, BookmarkAccount);
```

The macro **must be at module scope**, not inside `impl Config`. Rust's `macro_rules!` cannot be defined inside an impl block (stable Rust restriction). The macro then invokes a fresh `impl Config { ... }` block.

The fix is one of the "abstraction"-category items from the 2026-07-11/12 review — see commit `67a9b76`.

## Alternatives considered

### Generic helper method

```rust
fn lookup_account<T>(&self, list: &[T], name: &str) -> Result<&T, AgentError>;
```

- Needs `T: AccountLike` trait with a `fn name(&self) -> &str`.
- Possible; the trait boilerplate may exceed the macro's.
- Rejected: macros win on readability when the call sites are uniform.

### Inline the lookup at each call site

- Status quo before the macro.
- Five call sites × five modules = 25+ duplications.
- Rejected.

### Procedural macro (proc-macro crate)

- More powerful; supports hygiene.
- Build-time complexity for a five-line use case.
- Rejected.

## Consequences

- Five methods reduce to one macro invocation per module + one trait-free helper macro.
- Adding a new module: one line (`impl_account_lookup!(Config, x_account, x, XAccount);`).
- The macro is a stable-Rust pattern; no nightly features.
- The error message now interpolates the field name as a string, which is uniform and helpful.

## Cross-references

- The broader notion shared abstractions: [R009](R009-notion-common-local-module.md).
- The dual-provider build pattern that uses the same field shape: [R011](R011-add-dual-providers-macro.md).
- The merged account types: [R010](R010-notion-local-account.md).