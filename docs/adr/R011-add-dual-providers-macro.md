# ADR R011: `add_dual_providers!` macro for `build_providers` (todo/note/bookmark)

**Status:** Accepted
**Date:** 2026-07-11

## Context

`note`, `todo`, and `bookmark` each build their `TimelineProviderRegistry` entries via a function that loops over the configured accounts and constructs the right `TimelineProvider` for each:

```rust
fn build_providers(config: &Config) -> TimelineProviderRegistry {
    let mut r = TimelineProviderRegistry::default();

    for acct in &config.note {
        let notion_ops = if acct.provider == "notion" {
            Some(Arc::new(OpsLogProvider::new("note", &acct.name)))
        } else { None };
        let local = if acct.provider != "notion" {
            Some(Arc::new(NoteLocalProvider::new(&acct.name)))
        } else { None };
        // ... and the same for `todo` and `bookmark`
    }

    r
}
```

Three modules × this pattern = three near-identical 30-line functions.

## Decision

**Factor the dual-provider construction into a `add_dual_providers!` macro.**

```rust
macro_rules! add_dual_providers {
    ($registry:expr, $config:expr, $module:literal, $local_ty:ty) => {
        for acct in &$config.$module {
            if acct.provider == "notion" {
                $registry.add(Arc::new(OpsLogProvider::new($module, &acct.name)));
            } else {
                $registry.add(Arc::new(<$local_ty>::new(&acct.name)));
            }
        }
    };
}

fn build_providers(config: &Config) -> TimelineProviderRegistry {
    let mut r = TimelineProviderRegistry::default();
    add_dual_providers!(r, config, "note", NoteLocalProvider);
    add_dual_providers!(r, config, "todo", TodoLocalProvider);
    add_dual_providers!(r, config, "bookmark", BookmarkLocalProvider);
    r
}
```

The macro is at module scope (Rust limitation; see [R007](R007-config-account-macro.md) for the same constraint). It generates the per-account `if provider == "notion"` branch — both the ops-log provider for notion accounts and the local SQLite provider for local accounts.

The fix is one of the "abstraction"-category items from the 2026-07-11/12 review — see commit `62d81f7`.

## Alternatives considered

### Generic helper function with a closure parameter

- The provider constructors have different types (`NoteLocalProvider::new(&str) -> NoteLocalProvider`, etc.) — unifying them requires a trait.
- Trait boilerplate exceeds the macro's.
- Rejected.

### Builder pattern on `TimelineProviderRegistry`

- `registry.add_for_module("note", &config.note, NoteLocalProvider::new, OpsLogProvider::new_note);`
- Cleaner API; would also need a constructor per provider.
- Considered; the macro wins on readability for the three current modules. The builder pattern is a future refactor if more modules add providers.

### Per-module `build_providers`

- Status quo. Drift between the three modules is the bug we're fixing.
- Rejected.

## Consequences

- Three `build_providers` functions collapse into one shared function with three macro invocations.
- Adding a new module that has both a notion and a local provider: one new line.
- The macro takes the local provider type as a type parameter — Rust's macro syntax handles this via `$local_ty:ty` and `<$local_ty>::new(...)`.
- The `OpsLogProvider` is shared across modules (see [L010](L010-ops-log-provider.md)) so it doesn't need per-module construction.

## Cross-references

- The same macro-not-in-impl-block constraint: [R007](R007-config-account-macro.md).
- The provider that handles notion accounts: [L010](L010-ops-log-provider.md).
- The shared notion abstractions: [R009](R009-notion-common-local-module.md).
- The merged account types: [R010](R010-notion-local-account.md).
- The dual-provider design at the module level: [F005](F005-default-provider-local.md), [N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md), [B001](B001-bookmark-dual-provider.md).