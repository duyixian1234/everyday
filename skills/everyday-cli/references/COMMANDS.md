# everyday CLI — Full Command Reference

Loaded on demand by the `everyday-cli` skill. Every command below supports the global flags `--json` (machine-readable output) and `--account <NAME>` (override the module's default account).

## Install

Prebuilt binaries (Linux / macOS / Windows, x86_64) are on [GitHub Releases](https://github.com/duyixian1234/everyday/releases) for every `v*` tag. Or install from source:

```bash
cargo install --git https://github.com/duyixian1234/everyday.git
```

Verify with `everyday --version`. Per-platform extraction steps are in the repo root [README.md](../../../README.md).

## Implementation status

| Module | Status | Notes |
|--------|--------|-------|
| `config` | ✅ Complete | path / list / get / set / init |
| `mail` | ✅ Complete (v0.6.0) | IMAP receive + SMTP send + keyring credentials + local envelope cache (`mail list` reads from `~/.config/everyday/mail_cache.db`, auto-syncs if stale > 15min, `--sync` to force) |
| `cal` | ✅ Complete | CalDAV login / calendars / list / add / delete |
| `rss` | ✅ Complete | follow / list / unfollow / digest / fetch |
| `note` | ✅ Complete | Notion: login / search / list / create / read / append / update |
| `todo` | ✅ Complete | Notion/local tasks (shared `notion-client` SDK for notion): login / init-db / list / add / start / complete / **delete** |
| `bookmark` | ✅ Complete | Notion/local bookmarks (shared `notion-client` SDK for notion): login / init-db / list / add |
| `timeline` | ✅ Complete (v0.5.0) | Unified event log aggregating mail / cal / rss + ops-log AOP trace. Preset windows (`today` / `yesterday` / `week` / `month`) plus `--from` / `--to` absolute windows and `--since` sliding-window start (date or `30m` / `2h` / `1d` / `7d`) |

---

## config — configuration management ✅

Config file: `~/.config/everyday/config.toml` (resolved cross-platform via `dirs`). Passwords never stored here.

| Command | Description | Example |
|---------|-------------|---------|
| `config path` | Show config file path | `everyday config path` |
| `config list` | List all config (TOML in text mode) | `everyday config list --json` |
| `config get <dotted.path>` | Read a config value (supports array index `mail.accounts.0.name`) | `everyday config get mail.accounts.0.username` |
| `config set <dotted.path> <value>` | Set a config value (auto-infers bool/int/float/string) | `everyday config set default_account.mail work` |
| `config init` | Create an example config file (no-op if exists) | `everyday config init` |

---

## mail — email management (IMAP/SMTP) ✅

Credentials: config holds account metadata → `everyday mail login` stores the password in the OS keyring → other commands read it automatically. Passwords never touch disk.

| Command | Description | Example |
|---------|-------------|---------|
| `mail login` | Interactively enter password into the OS keyring | `everyday mail login --account work` |
| `mail folders` | List all mailbox folders | `everyday mail folders --json` |
| `mail list` | List message summaries from local cache (auto-sync if stale; recurses all folders by default, sorted by date desc) | `everyday mail list --unread --limit 10 --json` |
| `mail read <uid>` | Read a single message in full (searches all folders by default) | `everyday mail read 12345 --json` |
| `mail search` | Full-text search (recurses all folders by default) | `everyday mail search --query "invoice" --json` |
| `mail send` | Send a message (SMTP STARTTLS) | `everyday mail send --to a@b.com --subject "Hi" --body "内容"` |

### mail options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
| `--unread` | `list` | Unread only |
| `--limit N` | `list` / `search` | Max rows, default 20 |
| `--folder NAME` | `list` / `read` / `search` | Specific folder (Chinese names supported; default recurses all) |
| `--no-recursive` | `list` / `read` / `search` | INBOX only (no recursion) |
| `--sync` | `list` | Force IMAP sync before listing (ignore staleness) |
| `--to ADDR` | `send` | Recipient (required) |
| `--subject S` | `send` | Subject (required) |
| `--body TEXT` | `send` | Body (required) |
| `--cc ADDR` | `send` | CC (optional) |

### mail list / search — JSON output (array of objects)

```json
[{"uid":"12345","folder":"INBOX","date":"Wed, 8 Jul 2026 08:29:31 +0000","from":"sender@example.com","subject":"邮件主题"}]
```

### mail read — JSON output (array of field/value pairs)

```json
[{"field":"subject","value":"..."},{"field":"from","value":"..."},{"field":"date","value":"..."},{"field":"folder","value":"Junk"},{"field":"body","value":"..."}]
```

---

## cal — calendar management (CalDAV) ✅

Credentials: config holds account metadata (`caldav_url`, `username`) → `everyday cal login` stores password in OS keyring → other commands read it automatically. Verified against QQ CalDAV (`dav.qq.com`).

**Ignoring calendars:** add `ignore_calendars = ["好友生日", "Tasks"]` under a `[[calendar.accounts]]` entry in `config.toml`. Matched by displayname (case-insensitive); ignored calendars are hidden from `cal calendars` / `cal list` / `cal add` for that account.

| Command | Description | Example |
|---------|-------------|---------|
| `cal login` | Interactively enter password into the OS keyring | `everyday cal login --account personal` |
| `cal calendars` | List calendar collections (中文列名: 路径/名称/颜色) | `everyday cal calendars --json` |
| `cal list` | List events (default: today & future; `--all` for all, `--today`/`--date` to filter) | `everyday cal list --json` |
| `cal add` | Add an event (icalendar VEVENT, PUT) | `everyday cal add --title T --start 2026-07-09T15:00:00Z --end 2026-07-09T16:00:00Z` |
| `cal delete` | Delete an event by href | `everyday cal delete --id "/calendar/.../ev.ics"` |

### cal options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
| `--today` | `list` | Filter to today's events |
| `--date YYYY-MM-DD` | `list` | Events on a specific date |
| `--all` | `list` | Include past events too (no date filter) |
| `--limit N` | `list` | Max rows, default 50 |
| `--title T` | `add` | Event title (required) |
| `--start ISO` | `add` | Start time, RFC3339 or `YYYY-MM-DDTHH:MM:SS` (required) |
| `--end ISO` | `add` | End time (required) |
| `--location L` | `add` | Location (optional) |
| `--description D` | `add` | Description (optional) |
| `--calendar HREF` | `add` | Target calendar href/name (default: first calendar) |
| `--id HREF` | `delete` | Event href from `cal list` (required) |

### cal list — JSON output (array of objects)

```json
[{"路径":"/calendar/.../ev.ics","开始":"2026-07-09 15:00","结束":"2026-07-09 16:00","主题":"meeting","地点":"Room A"}]
```

### cal calendars — JSON output

```json
[{"href":"/calendar/.../","name":"duyixian1234's QQMail Calendars","colour":""}]
```

---

## rss — RSS/Atom subscriptions ✅

| Command | Description | Example |
|---------|-------------|---------|
| `rss follow` | Add a feed to config | `everyday rss follow --name N --url URL [--category C]` |
| `rss list` | List followed feeds | `everyday rss list` |
| `rss unfollow` | Remove a feed | `everyday rss unfollow --name N` |
| `rss digest` | Aggregate recent items across feeds (sorted by date) | `everyday rss digest [--limit N] [--name FEED] [--category C]` |
| `rss fetch` | Fetch one feed and list its entries | `everyday rss fetch --name N [--limit N]` |

---

## note — Notion notes & knowledge base ✅

Credentials: config holds account metadata (`provider`, `default_database_id`, `default_page_id`) → `everyday note login` stores the Notion Integration Token (`ntn_...`) in the OS keyring → other commands read it automatically. The token never touches disk. Design goal: hide Notion's nested Block model behind plain-text/Markdown append and simplified property ops.

**Setup:** create a Notion integration to get the `ntn_...` token, run `everyday note login`, set `[[note.accounts]]` in config, then **share the target page/database with the integration** in Notion.

| Command | Description | Example |
|---------|-------------|---------|
| `note login` | Interactively enter Notion token into the OS keyring | `everyday note login --account personal` |
| `note search` | Search pages/databases by title | `everyday note search --query "工作" --limit 10 --json` |
| `note list` | List pages in a database (`--db` or `default_database_id`) | `everyday note list --db "db_abc" --limit 20 --json` |
| `note create` | Create a page (record) in a database, with properties | `everyday note create --title T --db ID --prop "状态:未读" --json` |
| `note read` | Read a page; render its content as aggregated Markdown | `everyday note read <page_id> --json` |
| `note append` | Append text/markdown blocks to a page (or pipe via stdin) | `everyday note append <page_id> --text "内容"` |
| `note update` | Update a page's properties (metadata) | `everyday note update <page_id> --prop "状态:已读" --json` |

## todo — Notion task database ✅

Built on the shared `notion-client` SDK (handles HTTP, token injection, 429 rate-limit retry). Maps a clean `TodoItem` (id / title / status / due / priority) to/from Notion page properties. Credentials: `everyday todo login` stores the Notion Integration Token (`ntn_...`) in the OS keyring (service `everyday/todo/<account>`); the token never touches disk.

**Setup:** create a Notion integration → `everyday todo login` → add `[[todo.accounts]]` with `parent_page_id` → `everyday todo init-db` (creates the Task/Status/Due/Priority database and writes `database_id` back to config; the integration must be granted access to the parent page).

| Command | Description | Example |
|---------|-------------|---------|
| `todo login` | Interactively enter Notion token into the OS keyring | `everyday todo login --account personal` |
| `todo init-db` | Create the todo database in Notion (needs `parent_page_id`); writes `database_id` back to config | `everyday todo init-db --parent "page_xyz"` |
| `todo list` | List incomplete todos, sorted by due (`--all` includes Done) | `everyday todo list --db "db_abc" --json` |
| `todo add` | Add a todo (`--title` required; `--due` / `--priority` optional) | `everyday todo add --title "写周报" --due 2026-07-15 --priority P1 --json` |
| `todo start` | Mark a todo as In Progress | `everyday todo start <page_id>` |
| `todo complete` | Mark a todo as Done | `everyday todo complete <page_id>` |

### todo options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
| `--parent PAGE_ID` | `init-db` | Parent page for the new database; falls back to config `parent_page_id` |
| `--db ID` | `list` / `add` | Target database; falls back to config `default_database_id` (written by `init-db`) |
| `--all` | `list` | Include completed (Done) todos |
| `--title T` | `add` | Task title (required) |
| `--due DATE` | `add` | Due date (ISO 8601, e.g. `2026-07-15`) |
| `--priority P` | `add` | Priority select: `P0` / `P1` / `P2` |

### todo list — JSON output (array of TodoItem)

```json
[{"id":"page_abc","title":"写周报","status":"Todo","due":"2026-07-15","priority":"P1"}]
```

### todo add / start / complete / init-db — JSON output (object)

```json
{"id":"page_abc","url":"https://www.notion.so/...","title":"写周报","database_id":"db_xyz"}
```

### note options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
| `--query Q` | `search` | Keyword matched against page/database titles (required) |
| `--db ID` | `create` / `list` | Target database id; defaults to `default_database_id` when omitted |
| `--prop K:V` | `create` / `update` | Property setter, repeatable. Encoded by db schema (title/rich_text/number/checkbox/select…); value may contain `:`. |
| `--text TEXT` | `append` | Text/markdown to append. If omitted, reads from `stdin` (non-TTY only) |
| `--limit N` | `search` / `list` | Max rows (`search` default 10, `list` default 50, cap 100; `0` = unlimited) |

### note search — JSON output (array of objects)

```json
[{"id":"abc123_x","type":"page","title":"2026年工作计划","last_edited":"2026-07-09 18:00","url":"https://www.notion.so/..."}]
```

### note list — JSON output (array of objects, properties simplified to strings)

```json
[{"id":"...","title":"Quick Note","url":"https://www.notion.so/...","last_edited":"2026-07-10T07:01:00.000Z","properties":{"名称":"Quick Note"}}]
```

### note create — JSON output (object)

```json
{"id":"...","url":"https://www.notion.so/...","title":"Rust 异步运行时深入浅出","database_id":"db_abc123"}
```

### note read — JSON output (object with aggregated Markdown)

```json
{"id":"abc123_x","title":"2026年工作计划","url":"https://www.notion.so/...","properties":{"Status":"In Progress"},"content":"# 2026年工作计划\n\n## 核心目标\n- 完成 everyday CLI 稳定版发布。"}
```

### note append — JSON output (object)

```json
{"id":"...","url":"https://www.notion.so/...","appended":3}
```

### note update — JSON output (object)

```json
{"id":"...","url":"https://www.notion.so/...","updated":1}
```

---

## bookmark — bookmarks (local SQLite by default / optional Notion)

Built on the shared `notion-client` SDK (handles HTTP, token injection, 429 rate-limit retry). Maps a clean `BookmarkItem` (id / url / title / tags) to/from Notion page properties (Title / URL / Tags). Credentials: `everyday bookmark login` stores the Notion Integration Token (`ntn_...`) in the OS keyring (service `everyday/bookmark/<account>`); the token never touches disk. The **local SQLite provider is the default** (`provider = "local"`, alias `sqlite`): no credentials, no network, bookmarks stored at `~/.config/everyday/bookmark-<account>.db`. Command usage is identical across both providers.

**Setup (Notion only):** create a Notion integration → `everyday bookmark login` → add `[[bookmark.accounts]]` with `parent_page_id` → `everyday bookmark init-db` (creates the Title/URL/Tags database and writes `database_id` back to config; the integration must be granted access to the parent page).

| Command | Description | Example |
|---------|-------------|---------|
| `bookmark login` | Interactively enter Notion token into the OS keyring | `everyday bookmark login --account personal` |
| `bookmark init-db` | Create the bookmark database (Notion needs `parent_page_id`); writes `database_id` back to config | `everyday bookmark init-db --parent "page_xyz"` |
| `bookmark list` | List bookmarks (`--tag` filters by a single tag) | `everyday bookmark list --tag rust --json` |
| `bookmark add` | Add a bookmark (`--url` and `--title` required; `--tags` optional, comma-separated) | `everyday bookmark add --url "https://..." --title "Rust" --tags "rust,cli" --json` |

### bookmark options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
| `--parent PAGE_ID` | `init-db` | Parent page for the new database; falls back to config `parent_page_id` |
| `--db ID` | `list` / `add` | Target database; falls back to config `default_database_id` (written by `init-db`, Notion only) |
| `--tag TAG` | `list` | Filter by a single tag (exact match); omit to list all |
| `--url U` | `add` | Bookmark URL (required) |
| `--title T` | `add` | Bookmark title (required) |
| `--tags a,b` | `add` | Comma-separated tags (optional, e.g. `rust,cli`); trimmed, empty entries dropped |

### bookmark list — JSON output (array of BookmarkItem)

```json
[{"id":"b18c0f92234d6a12c","url":"https://www.rust-lang.org","title":"The Rust Programming Language","tags":["rust","lang"]}]
```

### bookmark add — JSON output (object)

```json
{"id":"b18c0f92234d6a12c","url":"https://www.rust-lang.org","title":"The Rust Programming Language","tags":["rust","lang"]}
```

---

## timeline — unified event log ✅ (v0.5.0)

Append-only event log aggregating `mail` / `cal` / `rss` + the `ops-log` AOP trace of Notion-backed `note` / `todo` / `bookmark` writes. Storage is a separate SQLite at `~/.config/everyday/timeline.db` (does not touch provider DBs).

### timeline actions

| Action | Description | Usage |
|--------|-------------|-------|
| `today` | Local-time today's window | `everyday timeline today [--source S] [--account A] [--limit N] [--sync] [--since ...]` |
| `yesterday` | Local-time yesterday | `everyday timeline yesterday [...]` |
| `week` | Monday–Sunday of the current ISO week | `everyday timeline week [...]` |
| `month` | Calendar month so far | `everyday timeline month [...]` |
| `sync` | Pull from all (or `--source`-filtered) providers; idempotent, watermark-based | `everyday timeline sync [--source mail,cal,todo] [--since 2026-01-01]` |

### timeline options

| Option | Description |
|--------|-------------|
| `--json` | Switch to JSON output (recommended for agents) |
| `--source S[,S2]` | Comma-separated source filter; accepted values are `mail`, `cal`, `rss`, `note_local`, `todo_local`, `bookmark_local`, `note`, `todo`, `bookmark` |
| `--account A` | Filter to one account name |
| `--limit N` | Cap event count (default 100) |
| `--since DUR_OR_DATE` | Sliding-window start. `30m` / `2h` / `1d` / `7d` are relative to now; `YYYY-MM-DD` is start-of-day local. `to` is `now()`. |
| `--from F`, `--to T` | Absolute window, both `YYYY-MM-DD`. Overrides preset; takes precedence over `--since`. |
| `--sync` | Run `sync` first, then query (atomic, single CLI call) |

### timeline sync — JSON output

```json
{ "synced": 6, "total_events": 83, "providers": [
  { "source": "mail", "account": "personal", "events": 60, "status": "Ok" },
  { "source": "cal", "account": "personal", "events": 9, "status": "Ok" },
  { "source": "rss", "account": null, "events": 7, "status": "Ok" },
  { "source": "todo", "account": null, "events": 7, "status": "Ok" },
  { "source": "note", "account": null, "events": 0, "status": "Ok" },
  { "source": "bookmark", "account": null, "events": 0, "status": "Ok" }
] }
```

### timeline today / yesterday / week / month — JSON output (array of TimelineEvent)

```json
[
  {
    "id": "ev18c12dc5be4ae670-0",
    "source": "todo",
    "account": "personal",
    "event_type": "add",
    "timestamp": "2026-07-11T08:01:34+00:00",
    "title": "B2-test-text-mode-add",
    "summary": "",
    "ref_id": "39a961d0-46a4-81e2-acc8-f37de2d1158c",
    "metadata": { "status": null, "action": "add" }
  },
  {
    "id": "ev...",
    "source": "mail",
    "account": "personal",
    "event_type": "received",
    "timestamp": "2026-07-11T07:04:13+00:00",
    "title": "Your workspace is waiting",
    "summary": "From: ...\nFolder: INBOX",
    "ref_id": "personal:12345",
    "metadata": { "from": "...", "folder": "INBOX" }
  }
]
```

`source` values:
- `mail` / `cal` / `rss` — pulled from the network providers during `sync`.
- `todo` / `note` / `bookmark` — projected from `~/.config/everyday/ops-log.db` via `OpsLogProvider` (the result of AOP records of CLI writes).
- `*_local` suffix is **not** produced; local providers are projected under their module name (`todo`, `note`, `bookmark`).

`timestamp` is RFC3339 UTC. Display formatting is the consumer's job (the CLI's Text renderer formats it in the user's local timezone).

### Design constraints (do not expect otherwise)

- **Append-only.** Re-running `sync` does not duplicate rows — natural key `(source, account, ref_id, event_type, timestamp)` is upserted with `INSERT OR IGNORE`.
- **Cal is the only window-refresh provider.** Each `sync` rewrites the cal window `[last_sync, now+7d]`, so cancelled events disappear. Other providers are purely append.
- **No `--from` / `--to` and no `--since` together.** `--from` / `--to` win; `--since` wins over preset; preset is the fallback. The combinations `today + --since 2026-07-09` widen `from` while keeping `to` at `now()` (useful for "today's window expanded to start earlier").
- **Notion writes never hit the Notion API during sync.** They are inferred from `~/.config/everyday/ops-log.db`. Add `--sync` to ensure a recent write has been AOP-recorded (writes are recorded synchronously by the CLI, so this is rarely needed, but it helps when scripting).

---

## Config file format

```toml
[default_account]
mail = "work"
calendar = "personal"
note = "personal"
todo = "personal"
bookmark = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 587
username = "me@example.com"
tls = true
# password is NOT stored here; it lives in keyring service="everyday/mail/work"

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/me"
username = "me"

[[rss.feeds]]
name = "hackernews"
url = "https://hnrss.org/frontpage"
category = "tech"

[[note.accounts]]
name = "personal"
provider = "notion"
default_database_id = "db_abc123..."
default_page_id = "page_xyz789..."
# Notion Integration Token (ntn_...) is NOT stored here; it lives in keyring service="everyday/note/personal"

[[todo.accounts]]
name = "personal"
provider = "notion"
parent_page_id = "page_parent_..."     # init-db needs this
# default_database_id is written back here automatically by `everyday todo init-db`
# Notion Integration Token (ntn_...) is NOT stored here; it lives in keyring service="everyday/todo/personal"

[[bookmark.accounts]]
name = "personal"
provider = "notion"
parent_page_id = "page_parent_..."     # init-db needs this
# default_database_id is written back here automatically by `everyday bookmark init-db`
# Notion Integration Token (ntn_...) is NOT stored here; it lives in keyring service="everyday/bookmark/personal"
```

**Keyring service-name convention:** `everyday/<module>/<account>` (e.g. `everyday/mail/work`, `everyday/note/personal`, `everyday/todo/personal`, `everyday/bookmark/personal`).

---

## Error types (JSON mode)

Exit code `0` on success, `1` on failure. Error envelope:

```json
{"error": "ErrorType", "message": "Details..."}
```

`ErrorType` values (PascalCase): `ConfigError` · `AccountNotFound` · `AuthError` · `NetworkError` · `IoError` · `ModuleNotFound` · `UnknownAction` · `InvalidArgument` · `PermissionDenied` · `NotImplemented` · `Other`
