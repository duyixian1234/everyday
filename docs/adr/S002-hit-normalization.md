# ADR S002: Hit normalization & SearchQuery contract

**Status:** Accepted
**Date:** 2026-07-12

## Context
Participating modules return structurally different data (IMAP envelopes, CalDAV VEVENTs, RSS items, SQLite rows). The aggregator must merge them without per-module branching, and an AI agent consuming `--json` needs a single stable shape.

## Decision
Define two structs in `src/search.rs`:

```rust
pub struct SearchQuery {
    pub raw: String,                 // user query string
    pub since: Option<DateTime<Utc>>, // optional lower time bound (UTC)
    pub limit: Option<usize>,        // global result cap override
}

pub struct Hit {
    pub module: &'static str,  // e.g. "mail", "cal", "rss", "note", "todo", "bookmark"
    pub account: Option<String>,
    pub id: String,
    pub title: String,
    pub snippet: String,       // short contextual excerpt
    pub url: Option<String>,   // deep-link / source URL when available
    pub ts: Option<DateTime<Utc>>, // module's primary time, UTC ([L006](L006-utc-storage-local-query.md))
    pub kind: &'static str,    // entity type within module when ambiguous
}
```

- `ts` is stored UTC; local-timezone rendering is the consumer's concern ([L006](L006-utc-storage-local-query.md)).
- `snippet` is a short, bounded excerpt (truncation strategy per [S004](S004-execution-model.md)).
- `url` may be empty for purely-local entities; agents use `module`+`id` to act via the respective module's actions.

## Alternatives considered
- **Untyped `serde_json::Value` passthrough (rejected):** preserves module specifics but forces the agent to branch per module and breaks stable schema contracts.
- **Per-module result enum (rejected):** requires the aggregator to match on module type, defeating the purpose of a unified list.

## Consequences
- All modules map their native rows/items into `Hit`; the aggregator merges `Vec<Hit>` with no module-specific code.
- Schema is stable for agents; any field addition is backward-compatible.
