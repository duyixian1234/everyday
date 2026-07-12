# ADR N001: Note module — Notion API integration that shields block nesting

**Status:** Accepted
**Date:** 2026-07-10

## Context

The `note` module fronts Notion's page model. The raw Notion API exposes pages as nested block trees (`paragraph → text`, `bulleted_list_item → text`, `toggle → children → paragraph → text`, ...). An Agent reading or appending notes has no business walking that tree; it wants flat text or Markdown.

Three constraints shaped the design:

1. **Simplicity for the Agent.** Surface `read` / `append` / `create` / `update` with flat-text or simple-property semantics. Hide the block tree.
2. **Credentials.** Notion integration tokens must never live in `config.toml`. Use the standard keyring convention.
3. **Shared client.** Reuse the cross-module Notion SDK (see [F004](F004-shared-notion-client.md)) — don't write a private HTTP layer.

## Decision

### Actions

```
everyday note login                # interactive: prompt for token, save to keyring
everyday note init-db              # create a Notion database "Notes" with Title / Tags properties
everyday note list   [--tag T]     # list notes in the database
everyday note read   <id>          # fetch a page; aggregate blocks to Markdown
everyday note create  --prop K:V   # create a page with title/tags properties
everyday note update  <id> [--prop K:V]
everyday note append  <id> [text]  # append Markdown-lite text to a page
```

- `read` recursively walks the block tree, converting paragraphs, headings, lists, toggles, code blocks, quotes into Markdown. Output in text mode is the rendered Markdown; in `--json` mode it's `{id, title, url, properties, content}`.
- `append` accepts Markdown-lite (headings, lists, paragraphs, fenced code). Anything more exotic is a future enhancement.
- `init-db` is idempotent: if the database already exists in the configured parent page, it's reused; otherwise created.

### Authentication and keyring

- Token stored in keyring: `service = "everyday/note/<account>"`, `account = "token"`.
- `login` prompts for the token if no keyring entry exists; otherwise it verifies the existing token with a lightweight `users.me` call.
- 401/403 from Notion → `AgentError::Auth`. Other non-2xx → `AgentError::Network`.

### Output mode detection

- Text vs `--json` is detected via `is_json()` (thread-local, see [R001](R001-thread-local-json-mode.md)).
- This module predates the clap subcommand tree ([F007](F007-clap-subcommand-tree.md)); its `parse_simple_args` ([R005](R005-parse-simple-args.md)) continues to be the parser of record.

### Local provider

- Since [F005](F005-default-provider-local.md), the **default** provider is `local` (SQLite). The Notion provider described above is still first-class via `provider = "notion"`.

## Alternatives considered

### Expose the raw block tree

- Maximum fidelity.
- Agent has to know Notion's block schema; every change to Notion's API risks breaking the Agent.
- Rejected.

### Markdown-only with no notion properties

- Simpler still.
- Loses the ability to filter by tag, set due dates, etc.
- Rejected for the create/update paths; `append` keeps Markdown-lite.

### Use a third-party `notion` Rust crate

- Several exist. Most wrap the same endpoints without adding value over a 200-line `NotionClient` (see [F004](F004-shared-notion-client.md)).
- Rejected.

### Defer `init-db` and require the user to set `database_id` manually

- One less Notion API surface to write.
- Worse UX: the user has to copy/paste a Notion URL into config.
- Rejected.

## Consequences

- The Agent's mental model is "note = title + body", regardless of how Notion stores it underneath.
- The shared `NotionClient` ([F004](F004-shared-notion-client.md)) handles auth headers, 429 backoff, and error mapping uniformly across all three Notion-backed modules.
- Markdown-lite is a deliberate scope limit. Tables, equations, embeds — future work.
- The local provider shares the CLI shape but stores rows in SQLite; the Agent's mental model is identical.
- `note read` in `--json` mode is the data source Timeline uses when the account is `provider = "notion"` (via ops-log — see [L007](L007-notion-ops-log.md) and [L010](L010-ops-log-provider.md)).

## Cross-references

- The shared Notion client this builds on: [F004](F004-shared-notion-client.md).
- The default local provider for new accounts: [F005](F005-default-provider-local.md).
- The Timeline notion-source event projection: [L007](L007-notion-ops-log.md), [L010](L010-ops-log-provider.md).
- Cross-module notion abstractions (`login_flow`, `parse_tags`, `set_module_database_id`) consolidated into `local`: [R009](R009-notion-common-local-module.md).
- Output mode detection: [R001](R001-thread-local-json-mode.md).