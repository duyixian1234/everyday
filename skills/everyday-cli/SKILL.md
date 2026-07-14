---
name: everyday-cli
description: Operates the everyday local Rust CLI for agent automation — IMAP/SMTP email (list, read, search, send), CalDAV calendar (calendars, list, add, delete events), RSS feeds (follow, list, digest), bookmarks (local SQLite by default / optional Notion, add, list, tag-filter), Notion note/knowledge-base and todo tasks (search, list, create, read, append, update, init-db, delete), unified event timeline (today, yesterday, week, month, sync), cross-module unified search (everyday search query "<q>" --module a,b,c --since 7d --limit N), credential lifecycle via the consolidated `auth` module (login / logout / verify / list), structured agent memory notebook (memory add / get / relation / list / delete / graph / history), and config management. Use when the user asks to check/read/send email, manage calendar events, read RSS digests, save bookmarks, capture notes/todos to Notion, persist structured facts to the agent's own memory, query an aggregated timeline of recent activity, search across all integrations in one shot, manage credentials, or run everyday commands. Always pass --json for machine-readable output.
license: MIT
---

# everyday CLI

`everyday` is a Rust CLI installed on the local machine. It gives an agent hands-on access to the user's machine: email, calendar, RSS, and config. The binary is `everyday` (on PATH, or `target/release/everyday` after `cargo build --release`).

## Install

Prebuilt binaries for Linux / macOS (x86_64 & Apple Silicon / aarch64) / Windows (x86_64) are published on [GitHub Releases](https://github.com/duyixian1234/everyday/releases) for every `v*` tag. Download the matching asset, extract, and put `everyday` (or `everyday.exe`) on PATH. Or build from source:

```bash
cargo install --git https://github.com/duyixian1234/everyday.git
# or
git clone https://github.com/duyixian1234/everyday.git && cd everyday && cargo build --release
```

Verify with `everyday --version`. Full install steps (per-platform extraction commands, one-liners) are in the repo root [README.md](../../../README.md).

## Command structure

```
everyday <module> <action> [options] [--json] [--account NAME]
```

Modules: `mail` · `cal` · `rss` · `bookmark` · `note` · `todo` · `timeline` · `memory` · `search` · `config`

## Rules (follow exactly)

1. **Always pass `--json`.** The agent parses structured output, never human tables. This is the primary mode for agent interaction.
   ```bash
   everyday mail list --unread --limit 10 --json
   ```
2. **Never put secrets in commands.** Passwords live in the OS keyring; never pass them as arguments or print them.
3. **Credentials live in the keyring, not the config file.** Config holds only account metadata. Keyring service name is `everyday/<module>/<account>` (e.g. `everyday/mail/work`).
4. **Modules.** `mail` (IMAP/SMTP), `cal` (CalDAV), `rss` (feeds), `bookmark` (local SQLite / Notion bookmarks), `note` (Notion), `todo` (Notion tasks + `delete`), `timeline` (unified event log: `today` / `yesterday` / `week` / `month` / `sync`), `memory` (single-instance append-only triple notebook: `add` / `get` / `relation` / `list` / `delete` / `graph` / `history` — no account, no auth touch), `search` (cross-module unified query: `everyday search query "<q>" [--module a,b,c] [--since 7d] [--limit N]`), and `config` are implemented — verify per action. Always pass `--json` for machine-readable output.
5. **`timeline today --json` is the aggregated activity snapshot.** It is one of the cheapest ways to answer "what's happened recently across all my integrations?". Always prefer it over per-module polling unless the user explicitly asks for a specific module.
6. **`memory` is the agent's own structured notebook.** Use `everyday memory add` to persist stable facts about the user, projects, or the world (subjects like `user`, `project-everyday`, `tech:rust`); use `memory get <SUBJECT>` to recall them. Subject naming is a convention enforced by the agent, not the program — see [Subject naming convention](#memory-subject-naming-convention) below. Memory facts automatically participate in `everyday search`.

## First-time setup (only if config is missing)

```bash
everyday config init
everyday config set mail.accounts.0.name work
everyday config set mail.accounts.0.imap_host imap.example.com
everyday config set mail.accounts.0.smtp_host smtp.example.com
everyday config set mail.accounts.0.username me@example.com
everyday config set default_account.mail work
everyday auth login --module mail --account work   # prompts for password, saved to keyring
```

After this, `mail` commands work without re-entering credentials.

## Common tasks

**Read unread mail (JSON):**

```bash
everyday mail list --unread --limit 10 --json
# → [{"uid":"12345","folder":"INBOX","date":"...","from":"...","subject":"..."}]
```

`mail list` reads from a local envelope cache (`~/.config/everyday/mail_cache.db`) — fast, no IMAP round-trip on warm cache. Auto-syncs if any target folder's `last_sync_at` is older than 15 minutes. Pass `--sync` to force an immediate sync (e.g. after returning from offline). `mail search` and `mail read` still go directly to IMAP.

**Read a single message:**

```bash
# read 默认递归所有文件夹查找该 UID（与 list 一致），无需手动指定 folder
everyday mail read 12345 --json
# 也可限定单文件夹 / 仅 INBOX：
everyday mail read 12345 --folder INBOX --json
```

**Search mail:**

```bash
everyday mail search --query "invoice" --json
```

**Send mail:**

```bash
everyday mail send --to a@b.com --subject "Hi" --body "内容"
```

**List calendar events (today & future by default; `--all` for all):**

```bash
everyday cal list --json
# → [{"路径":"/cal/ev.ics","开始":"2026-07-09 15:00","结束":"2026-07-09 16:00","主题":"meeting","地点":""}]
```

**Add a calendar event:**

```bash
everyday cal add --title "会议" --start "2026-07-09T15:00:00Z" --end "2026-07-09T16:00:00Z"
```

**List calendars / delete event:**

```bash
everyday cal calendars --json           # list calendar collections (get hrefs)
everyday cal delete --id "/cal/ev.ics"  # delete by href from `cal list`
```

**Search / list Notion pages (JSON):**

```bash
everyday note search --query "工作" --json
# → [{"id":"...","type":"page","title":"2026年工作计划","last_edited":"...","url":"..."}]
everyday note list --json                       # pages in default_database_id
everyday note list --db "db_abc123" --limit 20  # pages in a specific database
```

**Create a record in a Notion database (with properties):**

```bash
everyday note create \
  --title "Rust 异步运行时深入浅出" \
  --prop "类型:文章" --prop "状态:未读" --prop "URL:https://..."
```

**Read a page as Markdown (JSON returns aggregated `{id,title,url,properties,content}`):**

```bash
everyday note read <page_id> --json
```

**Append a flash note (text arg, or pipe via stdin):**

```bash
everyday note append --text "### AI 自动捕获
发现竞品链接：https://..."
echo "批量捕获内容" | everyday note append <page_id>
```

**Update page properties:**

```bash
everyday note update <page_id> --prop "状态:已读"
```

First-time Notion setup: store the `ntn_...` integration token via `everyday auth login --module note` (service `everyday/note/<account>`). The target page/database must be shared with the integration in Notion. `--db` / page id default to `default_database_id` / `default_page_id` from config when omitted.

**Manage todos (Notion task database, built on the shared `notion-client`):**

```bash
everyday auth login --module todo              # store Notion token (keyring service everyday/todo/<account>)
everyday todo init-db --parent "<page_id>"     # create the task database; writes database_id back to config
everyday todo list --json                      # incomplete todos, sorted by due
everyday todo list --all --json                # include Done
everyday todo add --title "写周报" --due 2026-07-15 --priority P1
everyday todo start <page_id>                  # → In Progress
everyday todo complete <page_id>               # → Done
everyday todo delete <page_id>                 # archive (Notion) / physical delete (local)
```

**Query the unified timeline (mail + cal + rss + notion writes):**

```bash
# All events in the last 24 hours, top sources
everyday timeline today --json

# Filter to one source / one account
everyday timeline today --source todo --account personal --json

# Sub-day sliding window (preserves minute precision)
everyday timeline today --since 30m --json         # 30 minutes ago
everyday timeline today --since 12h --json         # 12 hours ago

# Explicit absolute window
everyday timeline --from 2026-07-09 --to 2026-07-11 --json

# Sync first, then query (atomic). Without --sync, query hits the cached timeline.db.
everyday timeline today --sync --json

# Targeted sync (only refresh mail and rss)
everyday timeline sync --source mail,rss --json
```

First-time todo setup: store the token via `everyday auth login --module todo` (service `everyday/todo/<account>`), then add `[[todo.accounts]]` with `parent_page_id` and run `everyday todo init-db` (the integration must be granted access to the parent page). `--db` defaults to the `default_database_id` written by `init-db`.

## Memory (agent's own notebook)

`memory` is a single global instance (`~/.config/everyday/memory.db`, no `account` column, no `auth` module touch). It stores append-only `(subject, predicate, object)` triples with optional `--confidence` and `--source`. Re-adding the same triple creates a new version; `history` shows all versions including soft-deleted rows.

```bash
# Record what the user prefers
everyday memory add user prefers rust --confidence 0.9 --source explicit --json

# Look up everything we know about the user
everyday memory get user --json
# → {"count":1,"facts":[{"id":"m...","subject":"user","predicate":"prefers","object":"rust","confidence":0.9,"source":"explicit","created_at":"..."}]}

# Filter by (subject, predicate)
everyday memory relation user prefers --json

# Multi-hop traversal — what does the user depend on, transitively?
everyday memory graph user --depth 2
# → user
#      +-- prefers --> rust
#      `-- works_on --> everyday

# Soft-delete a current fact (history keeps it)
everyday memory delete user prefers rust --json
everyday memory history user prefers rust --json    # includes deleted_at
everyday memory add user prefers go --json          # resurrection = a new version row

# Memory participates in cross-module search automatically
everyday search query "rust" --module memory --json
```

### Memory subject naming convention

The program does not enforce a subject schema (no `[a-z][a-z0-9-]+` regex check, no vocabulary file). Conventions live here so multiple agents agree on the same vocabulary:

```
user                       # bare subject for the human user
project-everyday           # a project entity
tech:rust                  # domain-prefixed: a piece of technology knowledge
team:backend:alice         # hierarchical: team > sub-team > person
agent:self                 # agent's own self-description (rare)
```

Hierarchy is colon-delimited; agents that produce triples are expected to pick the right granularity. Cross-agent fact sharing works by default — two agents writing `(user, prefers, rust)` land in the same version history.

### What memory is and isn't

- **Yes**: stable, structured, mostly-timeless facts ("user prefers rust", "project-everyday uses tokio").
- **No**: timestamped events (use `everyday timeline ...`).
- **No**: long prose (use `everyday note create` + `append`).
- **No**: free-form journal entries.

Decision rule: if there is a clear "moment T" at which a fact became true, it belongs in `timeline`. If it is a stable assertion that survives many days, it belongs in `memory`.

### Memory v2 deferred

These are explicitly out of scope for v1 and should not be assumed by callers: `undelete-by-id`, `search` (embedding-based), `merge`, `expire (TTL)`, `cleanup` (physical GC of soft-deleted rows), `stats`. Use `history` + `--include-deleted` for forensics.

## Error format

JSON mode errors:

```json
{ "error": "AccountNotFound", "message": "mail account 'work'" }
```

Exit code is `1` on failure. Handle `NotImplemented` by telling the user the feature is pending; suggest an alternative if one exists.

## Full command reference

For the complete command tables, all options, and output schemas, read [references/COMMANDS.md](references/COMMANDS.md).
