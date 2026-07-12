# ADR R017: Backend directory layout + action-layer scope boundary

**Status:** Accepted
**Date:** 2026-07-12

## Context

With the `Backend` trait decided ([R016](R016-action-backend-di.md)), the open
question is **where the trait + two impls live** and **how far the refactor
reaches**. Current flat layout:

- `src/modules/note.rs` — `NoteModule` (Executor) + arg parsing + render + the
  leaked `NotionClient` calls.
- `src/modules/note_local.rs` — local SQLite impl, also implements `Searchable`
  for the read-side search path.
- `search.rs` and `timeline/providers.rs` reference `crate::modules::note_local::*`.

The leak is concentrated in the dual-provider trio. `mail` / `cal` / `rss`
hard-reference a single client but expose no swappable provider, so DI there
has no payoff today.

## Decision

### Layout — directory form (L-B)

Each of `note` / `todo` / `bookmark` becomes a directory:

```
src/modules/note/
├── mod.rs      # NoteModule (Executor) — arg parsing + render; was note.rs
├── backend.rs  # NoteBackend trait + for_account factory
├── notion.rs   # NotionNoteBackend
└── local.rs    # LocalNoteBackend; was note_local.rs
```

- `mod.rs` declares `pub mod backend; pub mod notion; pub mod local;`.
- The module path `crate::modules::note` is unchanged → external `use`
  (`crate::modules::note::NoteModule`) stays valid.
- `note_local.rs` is renamed/moved to `note/local.rs`.

### Scope — dual-provider trio only

- **In scope**: `note` / `todo` / `bookmark` action layers.
- **Out of scope**: `mail` / `cal` / `rss` (single hard-coded client, no
  alternative provider → no DI benefit; tracked as future separate work).

### Explicit exceptions (not refactored)

- **`auth login --verify`** (`auth.rs:351`) instantiates `NotionClient` to
  validate a token. This is the auth module's legitimate credential-verification
  responsibility (see [R014](R014-auth-verify-opt-in.md)), not an action-layer
  leak. Left as-is.
- **Read-side `search` / `timeline`** already use `Searchable` / `TimelineProvider`
  and never instantiate `NotionClient`. Not touched.

### Known regression cost

Moving `note_local.rs` → `note/local.rs` requires updating the two
`use crate::modules::note_local` references in `search.rs` and
`timeline/providers.rs` to `crate::modules::note::local`. Accepted for
structural cleanliness.

## Alternatives considered

### Flat layout (L-A): new `note_backend.rs` + keep `note_local.rs` in place

- Minimal file moves, but mixes top-level `*_local.rs` with new `*_backend.rs`;
  inconsistent. The user chose the directory form for cleanliness.
- **Rejected.**

### Single `backends.rs` for all three modules (L-C)

- Fewest files, but one very long file mixing note/todo/bookmark impls.
- **Rejected.**

## Consequences

- Per-module structure is co-located and self-contained: trait / factory / impls
  sit together.
- Exactly two `use`-path updates required (`search.rs`, `timeline/providers.rs`).
- `just check-links` + `cargo build` must pass after the move.
- New contributors see one consistent shape for all three dual-provider modules.

## Cross-references

- The trait + DI principle this layout serves: [R016](R016-action-backend-di.md)
- Domain types + mocks that go into `notion.rs` / `local.rs`: [R018](R018-backend-domain-mocks.md)
- Read-side abstraction left untouched: [S001](S001-search-architecture.md) (Searchable), [L004](L004-timeline-provider-pull-only.md) (TimelineProvider)
- The verify exception's basis: [R014](R014-auth-verify-opt-in.md)
