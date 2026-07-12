# ADR F004: Shared Notion client SDK with 429 backoff retry

**Status:** Accepted
**Date:** 2026-07-10

## Context

Three modules (`note`, `todo`, `bookmark`) all talk to the same Notion REST API. Each initially wrote its own HTTP request layer (`build_client` / `notion_request` / `api_get` / `api_post` / `api_patch`) and duplicated constants (`NOTION_API`, `NOTION_VERSION`, headers).

The duplication produced three problems:

1. **Bug surface × 3.** Any fix to authentication headers, base URL, or rate-limit handling had to be repeated three times.
2. **Inconsistent error mapping.** Different modules mapped 401/403/429 differently, surprising users.
3. **Drift.** One module's notion path was once quietly older than another's; tests caught it late.

## Decision

**All Notion traffic goes through a single shared client in `src/notion_client.rs`.**

The client exposes:

```rust
impl NotionClient {
    pub async fn request<B: Serialize, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<R, AgentError>;

    pub async fn get<R>(&self, path: &str) -> Result<R, AgentError>;
    pub async fn post<B, R>(&self, path: &str, body: &B) -> Result<R, AgentError>;
    pub async fn patch<B, R>(&self, path: &str, body: &B) -> Result<R, AgentError>;
}
```

Internals:

- Base URL: `https://api.notion.com/v1`.
- Header: `Notion-Version: 2022-06-28`, `Authorization: Bearer <token>`, `Content-Type: application/json`.
- Status mapping: `401`/`403` → `AgentError::Auth`; other non-2xx → `AgentError::Network`.
- **429 backoff retry**: at most one retry, reading `Retry-After` (default 1 s if absent).
- Pagination helpers for endpoints that return `next_cursor` are added per-module as needed (no global pagination loop, since each endpoint's shape differs).

Token storage continues to live in keyring (`service = everyday/<module>/<account>`, account = `token`) — see [F002](F002-multi-account-keyring.md). The client itself is stateless; it takes the token via constructor.

## Alternatives considered

### Keep per-module HTTP code

- Status quo at the time. Errors mapped inconsistently, three test suites to maintain.
- Rejected.

### One client per module with shared helper

- A `notion_http::request` helper that each module wraps in a thin adapter.
- Considered: keeps modules able to specialize headers per request.
- Rejected: in practice the specialization was never used; one shared client was simpler and the bug surface went from 3× to 1×.

### Use an existing third-party Notion SDK crate

- Several exist (`notion-rs`, `notion-client`).
- Rejected: most are thin wrappers over the same endpoints, none offered anything beyond what the local `NotionClient` does, and they added supply-chain surface.

## Consequences

- A bug fix or rate-limit improvement in the client benefits all three modules at once.
- All Notion modules share consistent error semantics (`Auth` vs `Network`), so the Agent can branch on error type instead of message.
- The 429 retry policy is uniform — one retry, then propagate. This is a deliberate trade: a stricter backoff loop would mask bugs and prolong tail latency.
- Migration: `note` was the last to migrate (commit history shows it kept its private helpers the longest); after migration, the `note` module's request layer was deleted and its tests re-pointed at the shared client.

## Cross-references

- Module-level integration details: [N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md), [B001](B001-bookmark-dual-provider.md).
- Shared Notion abstractions downstream of this SDK: [R009](R009-notion-common-local-module.md).
- Local provider design (the alternative to Notion): [F005](F005-default-provider-local.md).