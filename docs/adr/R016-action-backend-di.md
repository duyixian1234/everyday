# ADR R016: Action-layer Backend trait + Dependency Inversion for note/todo/bookmark

**Status:** Accepted
**Date:** 2026-07-12

## Context

The dual-provider trio (`note` / `todo` / `bookmark`) currently branches on
`account.provider` inside its module `execute()`, and for the notion path each
action helper instantiates `NotionClient` **directly** and calls its methods:

- `note.rs`: 6 `NotionClient::new` sites (search / list / create / read / append / update)
- `todo.rs`: 5 sites (list / add / set_status / delete + verify-adjacent)
- `bookmark.rs`: 3 sites (add / list + dispatch)

This violates two SOLID principles:

- **Dependency Inversion (DIP)**: the high-level action logic (`note.rs`)
  depends on the low-level `notion_client` crate. A change to the Notion SDK
  ripples into the module's command-handling code.
- **Single Responsibility (SRP)**: the module file both *selects* the provider
  and *speaks the provider-specific protocol*. It knows `is_local_provider`,
  fetches the keyring token, and drives `NotionClient`.

mail / cal / rss each hard-reference a single client (`async_imap` /
`CalDavClient` / `reqwest`), but they have **no alternative provider today**,
so DI yields zero benefit there — explicitly out of scope (see
[R017](R017-backend-layout-scope.md)).

The read-side is already clean: `search.rs` (`Searchable`) and
`timeline/providers.rs` (`TimelineProvider`) abstract over sources and never
instantiate `NotionClient`. They are orthogonal to this action-layer refactor.

A naming trap: `timeline/providers.rs` already defines `NoteProvider` /
`NoteLocalProvider` implementing `TimelineProvider` (read-side). Reusing
`Provider` for the action layer would clash in the crate and the glossary.

## Decision

Introduce an **action-layer `Backend` trait** per module, with Dependency
Inversion via a factory:

1. **Trait per module** — `NoteBackend` / `TodoBackend` / `BookmarkBackend`.
   Methods are **per-action** (Interface Segregation):
   `NoteBackend::{ search, list, create, read, append, update }`,
   `TodoBackend::{ list, add, set_status, delete }`,
   `BookmarkBackend::{ add, list }`. Each returns a typed **domain struct**
   (see [R018](R018-backend-domain-mocks.md)), never `Output`.

2. **Two implementations** per module:
   - `NotionNoteBackend` — wraps `NotionClient`; converts notion_client errors
     to `AgentError` at its boundary.
   - `LocalNoteBackend` — wraps the existing local SQLite impl
     (`note_local.rs` → `note/local.rs`, see [R017](R017-backend-layout-scope.md));
     returns the **same** domain type.

3. **Factory centralizes construction** —
   `NoteBackend::for_account(&Config, &Account) -> Result<Box<dyn NoteBackend>>`.
   The factory branches on `account.provider` (via `is_local_provider`),
   reads the notion token through `auth::get_credential`
   ([R013](R013-auth-module-consolidation.md) / [F002](F002-multi-account-keyring.md)),
   constructs the concrete backend, and returns `Box<dyn NoteBackend>`.
   The module's action code (`note.rs`) **never** names `NotionClient`,
   **never** branches on provider, **never** touches the keyring.

4. **`#[async_trait]`** for the trait (matches the rest of the crate; native
   `async fn in trait` is not object-safe for `Box<dyn>` without `async_trait`
   or `BoxFuture`). Methods return the existing `Result<T>` = `AgentError`.

5. **Naming `Backend`, not `Provider`** — deliberate avoidance of the
   read-side `NoteProvider` / `TimelineProvider` collision. See the
   disambiguation in `CONTEXT.md` §"Action Backend".

## Alternatives considered

### Reuse `Provider` naming

- Collides with `timeline/providers.rs::NoteProvider` (implements
  `TimelineProvider`). Would cause crate-internal ambiguity and glossary
  confusion (two meanings of "Provider").
- **Rejected.**

### Unified `execute(action, args)` dispatch on the trait

- One method, an `NoteAction` enum, args bundled. New action = new enum
  variant, no trait-signature change.
- Loses ISP (a test double must match every action); weak type safety; the
  module still has to decode args.
- **Rejected** (this was grill option 2).

### Factory placed inside the module file

- `note.rs` owns `fn build_backend(account) -> Box<dyn NoteBackend>` and
  `use`s the concrete backend types to construct them.
- Module still depends on low-level construction → violates DIP. The user's
  literal "don't call notion_client in note.rs" is met, but the seam is weak.
- **Rejected** in favour of centralizing the factory in the backend submodule
  (grill option Z), so the module imports only `NoteBackend`.

## Consequences

- `note.rs` / `todo.rs` / `bookmark.rs` drop their `use notion_client`; the
  `NotionClient::new` count in module files goes to **zero**.
- Provider selection + token fetch live in exactly one place per module (the
  factory), testable in isolation.
- The DI seam enables `MockNoteBackend` ([R018](R018-backend-domain-mocks.md))
  → action-layer unit tests with no network / keyring.
- `auth login --verify` for notion remains a **deliberate exception**
  ([R017](R017-backend-layout-scope.md)): validating a token inherently calls
  `NotionClient` and is the auth module's job, not action-layer leak.

## Cross-references

- Executor / Output / AgentError the backends stay compatible with: [F001](F001-cli-shape.md)
- Keyring credential contract the factory relies on: [F002](F002-multi-account-keyring.md)
- Shared Notion SDK wrapped by `NotionNoteBackend`: [F004](F004-shared-notion-client.md)
- Default local SQLite provider: [F005](F005-default-provider-local.md)
- `auth::get_credential` the factory calls: [R013](R013-auth-module-consolidation.md), [R014](R014-auth-verify-opt-in.md), [R015](R015-auth-credential-io.md)
- Directory layout + scope boundary: [R017](R017-backend-layout-scope.md)
- Domain types + in-memory mocks: [R018](R018-backend-domain-mocks.md)
- Glossary disambiguation (Backend vs Provider): `CONTEXT.md` §"Action Backend"

## Implementation checklist

1. Create `src/modules/note/{mod.rs, backend.rs, notion.rs, local.rs}` (see R017).
   Move current `note.rs` → `note/mod.rs`; `note_local.rs` → `note/local.rs`.
2. In `note/backend.rs`: define `trait NoteBackend` (`#[async_trait]`, per-action
   methods returning domain types) + `NoteBackend::for_account(&Config, &Account)`.
3. In `note/notion.rs`: `NotionNoteBackend` — move the 6 notion-path helper
   bodies here, `NotionClient` constructed inside `for_account` (not per action).
4. In `note/local.rs`: `impl NoteBackend for LocalNoteBackend` — wrap existing
   SQLite logic, return the same domain types.
5. In `note/mod.rs`: `execute()` resolves account → `let backend =
   NoteBackend::for_account(&self.config, &account)?` → call backend method →
   render domain struct to `Output`. Delete `is_local_provider` branch + keyring reads.
6. Repeat steps 1–5 for `todo` and `bookmark`.
7. Update `use crate::modules::note_local` → `crate::modules::note::local` in
   `search.rs` and `timeline/providers.rs`.
8. Add `MockNoteBackend` / `MockTodoBackend` / `MockBookmarkBackend` + action-layer
   unit tests (see R018).
9. Gate: `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
   `cargo test`, `just check-links` all green.
