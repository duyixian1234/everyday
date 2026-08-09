# ADR R019: Remove the Notion provider — note/todo/bookmark become local-only

**Status:** Accepted
**Date:** 2026-08-09

## Context

`note` / `todo` / `bookmark` have shipped a dual-provider design since v0.3.0: a local SQLite backend (default, [F005](F005-default-provider-local.md)) and a remote Notion API backend ([N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md), [B001](B001-bookmark-dual-provider.md)) sharing one HTTP client ([F004](F004-shared-notion-client.md)) and an ops-log audit path for timeline projection ([L007](L007-notion-ops-log.md), [L010](L010-ops-log-provider.md)).

The Notion path is ~2,700 lines of code (three `Notion*Backend` providers, `NotionClient`, keyring token flow in `auth`, `NotionLocalAccount` config fields, the ops-log AOP hook) plus ~160 doc references, and it serves a capability the project owner no longer uses: cross-device sync via a remote service. The local provider covers every CLI action with millisecond latency and no credentials. The Notion path adds network failure modes, rate limits, token lifecycle, and a separate audit database whose only consumer is the Notion timeline projection.

**Decision (2026-08-09): remove Notion provider support entirely.** `note` / `todo` / `bookmark` become local-only. This is a deliberate scoping decision, not a technical failure of the Notion implementation — the dual-provider abstraction ([R016](R016-action-backend-di.md), [R018](R018-backend-domain-mocks.md)) was sound and is retained in simplified form.

## Decision

### Scope — what is removed

- **Code**: `src/shared/notion_client.rs`, `src/modules/{note,todo,bookmark}/notion.rs`, `src/ops_log.rs` (entirely Notion-serving), `src/shared/keyring_user.rs`. `OpsLogProvider` and the `add_dual_providers!` macro in `src/modules/timeline/providers.rs` are removed (the dual-provider build collapses to a plain local-provider loop). Notion branches in `auth` (`Token` auth strategy, `--token` flag, `NotionClient` verify) and `config` (`valid_notion_id` validation, provider triple) are removed. `set_module_database_id` (notion `init-db` writeback) is removed from `local`.
- **Config**: account records keep `name` + `provider` (`local`/`sqlite` aliases only) + `db_path`; Notion-only fields `parent_page_id` / `default_database_id` are dropped. `NotionLocalAccount` is renamed to `LocalAccount` (type aliases `TodoAccount`/`BookmarkAccount`/`NoteAccount` kept for call-site compat).
- **Validation**: `provider = "notion"` in an existing config now fails at load time with a migration hint — no silent fallback (silent fallback would make a user believe their data is still in Notion while the CLI reads/writes an empty local DB).
- **Domain types**: Notion-only fields (`NoteCreated.database_id`, `resource`/`unit` discriminators that were constant "note"/"line" under local) are removed and render branches simplified; **JSON output keys stay stable** — the core keys (`id`/`title`/`url`/`properties`/`content`, todo's `status`/`due`/`priority`, bookmark's `url`/`tags`) are preserved verbatim so existing `--json` consumers keep working. `url` is emitted as an empty string for local notes/todos (no notion URL exists); Notion-only keys (`database_id`, `type` in note search, `archived` in todo delete) are dropped.
- **CLI**: `init-db` action and `--db` flag (notion database id) are removed from `note`/`todo`/`bookmark`. `todo init-db` semantics (local DB is created lazily on first use, [R009](R009-notion-common-local-module.md)) are unchanged.
- **Docs**: CONTEXT.md glossary, README/README_ZH, progress.md, task_plan.md, config.example.toml, agents.md, and the repo `skills/everyday-cli` references are cleaned. Superseded ADRs are marked, not deleted (project convention — see F012/F013).
- **Version**: shipped as v0.13.0 (breaking change).

### Superseded ADRs

| ADR | Fate |
|-----|------|
| [F004](F004-shared-notion-client.md) | Superseded by R019 (client removed) |
| [N001](N001-notion-note-module.md) | Superseded by R019 |
| [T001](T001-notion-todo-module.md) | Superseded by R019 |
| [B001](B001-bookmark-dual-provider.md) | Superseded by R019 (no dual provider) |
| [L007](L007-notion-ops-log.md) | Superseded by R019 (ops-log removed) |
| [L010](L010-ops-log-provider.md) | Superseded by R019 (provider removed) |
| [R010](R010-notion-local-account.md) | Superseded by R019 (`NotionLocalAccount` renamed) |
| [R011](R011-add-dual-providers-macro.md) | Superseded by R019 (macro removed) |
| [F005](F005-default-provider-local.md) | **Revised, stays Accepted** — conclusion (local is the default) survives; only the "notion remains available" clause dies |
| [R009](R009-notion-common-local-module.md) | **Revised, stays Accepted** — `parse_tags` survives in `local`; `login_flow` already moved to `auth` ([R013](R013-auth-module-consolidation.md)); `set_module_database_id` dies |

## Alternatives considered

### Keep Notion but hide it behind a feature flag
- Keeps cross-device sync for hypothetical users.
- Rejected: nobody in the project's real usage relies on it; the flag would still ship ~2,700 lines of dead-ish code and its whole doc surface.

### Silent fallback: notion accounts read/write local DBs
- Zero-config upgrade.
- Rejected: data-loss-adjacent — user's Notion data appears to "disappear" and new writes go to an empty local store. Load-time error with a migration hint is the honest path.

### Keep `NotionLocalAccount` fields as inert config
- Smaller diff.
- Rejected: `parent_page_id` / `default_database_id` are pure Notion semantics; keeping them as always-None fields would confuse future readers (and `serde` ignores unknown fields anyway, so old configs load fine without them).

## Consequences

- `note` / `todo` / `bookmark` each become single-provider modules: one `Local*Backend`, one timeline provider, one search provider. The `Backend` trait and `for_account` factory stay ([R016](R016-action-backend-di.md)) — they now have exactly one concrete implementation, which keeps the DI seam testable via `Mock*Backend` ([R018](R018-backend-domain-mocks.md)).
- `ops-log.db` is orphaned: existing files on user machines are simply never read again (no migration, no deletion — harmless leftover).
- `auth` supports only `Password` (`mail`/`cal`) and `None` (local/sqlite, rss) strategies; `--token` and Notion `verify` disappear.
- Timeline `note`/`todo`/`bookmark` events come solely from the local SQLite providers (current-state projection, [L008](L008-local-provider-degraded-granularity.md)).
- Config validation error for `provider = "notion"`: actionable message pointing to local migration.
- Old configs containing `parent_page_id` / `default_database_id` load cleanly (fields ignored by serde).

## Cross-references

- Supersedes: [F004](F004-shared-notion-client.md), [N001](N001-notion-note-module.md), [T001](T001-notion-todo-module.md), [B001](B001-bookmark-dual-provider.md), [L007](L007-notion-ops-log.md), [L010](L010-ops-log-provider.md), [R010](R010-notion-local-account.md), [R011](R011-add-dual-providers-macro.md).
- Revised: [F005](F005-default-provider-local.md), [R009](R009-notion-common-local-module.md).
- Retained scaffolding: [R016](R016-action-backend-di.md), [R017](R017-backend-layout-scope.md), [R018](R018-backend-domain-mocks.md), [L008](L008-local-provider-degraded-granularity.md).
