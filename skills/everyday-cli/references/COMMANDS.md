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
| `mail` | ✅ Complete (v0.6.1) | IMAP receive + SMTP send + keyring credentials + local envelope cache (`mail list` reads from `~/.config/everyday/mail_cache.db`, auto-syncs if stale > 15min, `--sync` to force) |
| `cal` | ✅ Complete | CalDAV calendars / list / add / delete |
| `rss` | ✅ Complete | follow / list / unfollow / digest / fetch |
| `note` | ✅ Complete | Local SQLite: search / list / create / read / append / update |
| `todo` | ✅ Complete | Local SQLite tasks: list / add / start / complete / **delete** |
| `bookmark` | ✅ Complete | Local SQLite bookmarks: add / list |
| `auth` | ✅ Complete (v0.8.0) | login / logout / verify / list — consolidated credential lifecycle for all modules |
| `timeline` | ✅ Complete (v0.5.0) | Unified event log aggregating mail / cal / rss + local note / todo / bookmark activity. Preset windows (`today` / `yesterday` / `week` / `month`) plus `--from` / `--to` absolute windows and `--since` sliding-window start (date or `30m` / `2h` / `1d` / `7d`). v0.6.1 修复 `--from` 单独给定被静默回退 preset 的问题 |
| `memory` | ✅ Complete (v0.10.0) | Single-instance append-only `(subject, predicate, object)` triple notebook with `--confidence` / `--source` metadata; soft delete + full version history; forward-only BFS graph (depth 1..=5); participates in `everyday search`. No `account` column, no `auth` module touch. Storage at `~/.config/everyday/memory.db` |
| `search` | ✅ Complete (v0.7.0; v0.9.0 +mail, v0.10.0 +memory) | Cross-module unified search fan-out: `everyday search query "<q>" [--module a,b,c] [--since 7d] [--limit N]`. Modules: `note` / `todo` / `bookmark` / `rss` / `cal` / `mail` (local envelope cache) / `memory` (current-state view) |
| `health` | ✅ Complete (v0.11.0) | Root-level ops command (not a module): runs every module's local-only `health_check`, one row per module. Exit 0 when all healthy, 1 when any degraded. JSON = array of `{module, healthy, detail}` |
| `sync` | ✅ Complete (v0.13.0) | Cross-device **file-level** sync to a WebDAV directory (default Jianguoyun): 4 user DBs (bookmark/note/todo/memory) + config.toml. Snapshot + SHA-256 change detection, LWW conflicts with dual `.conflict-<UTC ts>` copies, first-sync direction auto-detection, `--push-only` / `--pull-only` / `--force`, opt-in `auto_sync` after write commands (D003). Auth via `everyday auth login --module webdav --account <name>` (application password → keyring). Design: D001–D003 |
| `mcp` | ✅ Complete (v0.15.0) | everyday as an **MCP server** over stdio (rmcp 3.x): every `(module, action)` is projected into a tool `<module>_<action>` (schemas from `module_arg_spec`, single source of truth). `serve` blocks until stdin EOF; `tools` prints the projected list + JSON Schemas. MCP clients connect via `{"mcpServers":{"everyday":{"command":"everyday","args":["mcp","serve"]}}}`. Design: F014 |
| `task` | ✅ Complete (F017) | Named no-shell commands: add / run / list / remove / history; process-tree timeout; 64 KiB-per-stream capture; SQLite audit history; five-field cron via daemon's independent scheduler loop |

---

## task — named no-shell commands ✅ (F017)

Tasks live under `[tasks.<name>]`; every run is recorded in
`~/.config/everyday/task.db`.

| Command | Example |
|---------|---------|
| `task add` | `everyday task add build --command cargo --args "check --all-targets" --capture-output true --json` |
| `task run` | `everyday task run build --json` |
| `task run` with extra argv | `everyday task run deploy -- --env staging` |
| `task list` | `everyday task list --json` |
| `task remove` | `everyday task remove build --json` |
| `task history` | `everyday task history build --limit 20 --json` |

`command` is started directly without a shell; configured `args` is
whitespace-split. Extra argv is rejected unless `allow_extra_args=true`.
Timeout defaults to 60s (`0` = none), kills the process tree, and exits 124.
Text mode streams child output and mirrors its exit code. JSON mode routes child
output to stderr and emits `{"_result": ...}` on stdout. Scheduled runs always
capture; daemon checks five-field local-time cron schedules every 30s, skips
missed windows, and includes a pass in `daemon run --once`.

---

## health — module health check ✅ (v0.11.0)

Runs every module's `health_check` and renders one row per module. Checks are **local-only by design** (cache / config DB openable, keyring credential present) — never network calls, so `health` is fast and works offline. Modules without an override (search / auth / config) report `ok` via the default. All rows render regardless of state.

| Command | Description | Example |
|---------|-------------|---------|
| `health` | Probe every module's local health | `everyday health` / `everyday health --json` |

Text output is a `module | status | detail` table; JSON output is an array of `{"module": ..., "healthy": bool, "detail": ...}`. **Exit code**: 0 when all modules healthy, 1 when any is degraded (scripts can gate on it). Implemented as part of the F012 P3 lifecycle hooks.

---

## sync — cross-device file sync via WebDAV ✅ (v0.13.0)

Bidirectional file-level sync of the five user data files (`bookmark-<acct>.db` / `note-<acct>.db` / `todo-<acct>.db` / `memory.db` / `config.toml`) against a WebDAV directory (default `https://dav.jianguoyun.com/dav/everyday`). Derived caches (mail_cache / rss-items / timeline) are never synced. DBs are uploaded as `VACUUM INTO` snapshots (WAL-safe); changes are detected by content hash, conflicts by Last-Write-Wins mtime arbitration — the loser is preserved as `<name>.conflict-<UTC ts>.<ext>` **locally and on the remote**. First sync auto-detects direction: empty remote → push all; fresh device (empty config template) → pull all (remote listing drives the pull set). Sync state lives in `sync-state.json` (not synced; delete it or `--force` to rebuild from a full re-upload).

| Command | Description | Example |
|---------|-------------|---------|
| `sync` | Bidirectional sync (pull-then-push) | `everyday sync` / `everyday sync --json` |
| `sync --push-only` | Upload local changes only (also used by auto_sync) | `everyday sync --push-only --json` |
| `sync --pull-only` | Download remote changes only | `everyday sync --pull-only --json` |
| `sync --force` | Ignore `sync-state.json`: re-upload all local files + pull remote-only ones | `everyday sync --force --json` |

**JSON output**: `{ "remote": "<dir>", "files": [{name, action, detail?}], "pushed": N, "pulled": N, "skipped": N, "conflicts": N }` (+ `first_sync: "push_all"|"pull_all"` on an unambiguous first sync). `action` ∈ `push|pull|skip|conflict`; conflict entries carry `winner` (`local`|`remote`) and `conflict_copy`.

**Setup**: `[[webdav.accounts]]` in config.toml (name / url / username), then:

```bash
everyday auth login --module webdav --account personal   # application password → keyring everyday/webdav/personal
```

**Auto-sync (opt-in, default off)**: `auto_sync = true` on an account → successful write commands (`bookmark add`, `note create/append/update`, `todo add/start/complete/delete`, `memory add/delete`, `cal add/delete`, `config set`) do a best-effort push of changed files before returning. Failures print a warning (text stderr / JSON `_warning` line) and never change the exit code; query paths never sync.

---

## mcp — MCP server over stdio ✅ (v0.15.0)

`everyday` acts as a **Model Context Protocol (MCP) server**: any MCP-capable
agent (Claude Code / CodeBuddy / Cursor / Claude Desktop) connects over stdio
and gets every `(module, action)` as a tool named `<module>_<action>` (e.g.
`mail_list`, `note_add`, `timeline_today`). Tool argument schemas are generated
from the same `module_arg_spec()` the CLI uses — the MCP surface never drifts
from the CLI. The `mcp` module itself is not projected. Design: F014.

| Command | Description | Example |
|---------|-------------|---------|
| `serve` | Run the MCP stdio server (blocks until stdin EOF, then exits 0) | `everyday mcp serve` |
| `tools` | Print the projected tool list + JSON Schemas (debugging) | `everyday mcp tools` |

**Connect a client** (`claude_desktop_config.json` / Claude Code / CodeBuddy):

```json
{
  "mcpServers": {
    "everyday": {
      "command": "everyday",
      "args": ["mcp", "serve"]
    }
  }
}
```

**Tool call semantics**: results are the `--json`-rendered output (same data as
`everyday <mod> <action> --json`); failures come back as MCP `isError` with the
error message; the optional `account` argument mirrors the global `--account`
flag. The server is long-lived — config / DB changes apply on the **next
session start**. stdout carries JSON-RPC only; logs go to stderr.

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

## auth — credential lifecycle ✅

Consolidated credential management for all modules. Modules read stored credentials internally via `auth::get_credential`; you only use these commands to manage credentials in the OS keyring (default: store only; `--verify` also verifies). Password strategy (mail/cal/webdav) uses `--password`. If the flag is omitted, it falls back to an interactive prompt. Passwords never touch disk.

**Env fallback (opt-in, R020):** when the OS keyring backend is unavailable (headless server / CI / sandbox), credentials may be read from environment variables instead — enable via `[auth] env_credentials = true` in `config.toml` **or** `EVERYDAY_ENV_CREDENTIALS=1`. Variable name: `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` (e.g. `EVERYDAY_MAIL_WORK_PASSWORD`, account name uppercased, non-`[A-Z0-9]` → `_`). Read chain: keyring → env → error. `login` always writes the keyring; `logout` cannot delete an env-sourced credential (it tells you to `unset` the variable).

| Command | Description | Example |
|---------|-------------|---------|
| `auth login` | Store a credential in the OS keyring (optionally verify). `--module` required; `--account` defaults to the module's default account | `everyday auth login --module mail --account work --password PWD` |
| `auth logout` | Delete the stored credential from the keyring; errors with an `unset` hint if the credential comes from the environment | `everyday auth logout --module mail --account work` |
| `auth verify` | Read the stored credential (keyring → env) and verify it against the server (no re-prompt); reports `not_required` for local/sqlite or rss | `everyday auth verify --module note` |
| `auth list` | List configured accounts and their credential state: stored / env / missing / not_required | `everyday auth list --module todo` |

### auth options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--module <MOD>` | all | Target module (`mail` / `cal` / `note` / `todo` / `bookmark`) |
| `--account NAME` | all | Specify account (override default) |
| `--password PWD` | `login` | Password (mail/cal); falls back to interactive prompt |
| `--verify` | `login` | Also verify the credential against the server after storing |

---

## mail — email management (IMAP/SMTP) ✅

Credentials: config holds account metadata → credentials are stored via `everyday auth login --module mail [--account NAME]` (password stored in the OS keyring) → other commands read it automatically via `auth::get_credential`. Passwords never touch disk.

| Command | Description | Example |
|---------|-------------|---------|
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

`mail list` rows are typed records (F012 P6): `uid` is a JSON number and
`unread` a JSON boolean; `mail search` rows are plain strings.

```json
[{"uid":12345,"unread":true,"folder":"INBOX","date":"Wed, 8 Jul 2026 08:29:31 +0000","from":"sender@example.com","subject":"邮件主题"}]
```

### mail read — JSON output (array of field/value pairs)

```json
[{"field":"subject","value":"..."},{"field":"from","value":"..."},{"field":"date","value":"..."},{"field":"folder","value":"Junk"},{"field":"body","value":"..."}]
```

---

## cal — calendar management (CalDAV) ✅

Credentials: config holds account metadata (`caldav_url`, `username`) → credentials are stored via `everyday auth login --module cal [--account NAME]` (password in OS keyring) → other commands read it automatically via `auth::get_credential`. Verified against QQ CalDAV (`dav.qq.com`).

**Ignoring calendars:** add `ignore_calendars = ["好友生日", "Tasks"]` under a `[[calendar.accounts]]` entry in `config.toml`. Matched by displayname (case-insensitive); ignored calendars are hidden from `cal calendars` / `cal list` / `cal add` for that account.

| Command | Description | Example |
|---------|-------------|---------|
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
| `rss digest` | Aggregate reading view: summary column, cache-first (use `--fresh` to force live), `--since` window | `everyday rss digest [--limit N] [--name FEED] [--category C] [--since 30m\|7d\|YYYY-MM-DD] [--fresh]` |
| `rss fetch` | Fetch one feed: `--name N` subscription (writes cache) or `<url>` stateless debug (no cache write) | `everyday rss fetch (--name N \| <url>) [--limit N]` |

---

## note — notes & knowledge base (local SQLite) ✅

Notes are stored in a local SQLite database per account (`~/.config/everyday/note-<account>.db`); no credentials or network needed — data lives entirely on the local machine. Plain-text/Markdown append and simplified property ops.

| Command | Description | Example |
|---------|-------------|---------|
| `note search` | Search notes by title | `everyday note search --query "工作" --limit 10 --json` |
| `note list` | List notes | `everyday note list --limit 20 --json` |
| `note create` | Create a note with properties | `everyday note create --title T --prop "状态:未读" --json` |
| `note read` | Read a note; render its content as aggregated Markdown | `everyday note read <note_id> --json` |
| `note append` | Append text/markdown to a note (or pipe via stdin) | `everyday note append <note_id> --text "内容"` |
| `note update` | Update a note's properties (metadata) | `everyday note update <note_id> --prop "状态:已读" --json` |

## todo — task management (local SQLite) ✅

Todos are stored in a local SQLite database per account (`~/.config/everyday/todo-<account>.db`); no credentials or network needed. A clean `TodoItem` (id / title / status / due / priority) is mapped to/from local rows.

| Command | Description | Example |
|---------|-------------|---------|
| `todo list` | List incomplete todos, sorted by due (`--all` includes Done) | `everyday todo list --json` |
| `todo add` | Add a todo (`--title` required; `--due` / `--priority` optional) | `everyday todo add --title "写周报" --due 2026-07-15 --priority P1 --json` |
| `todo start` | Mark a todo as In Progress | `everyday todo start <todo_id>` |
| `todo complete` | Mark a todo as Done | `everyday todo complete <todo_id>` |
| `todo delete` | Delete a todo | `everyday todo delete <todo_id> --json` |

### todo options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
| `--all` | `list` | Include completed (Done) todos |
| `--title T` | `add` | Task title (required) |
| `--due DATE` | `add` | Due date (ISO 8601, e.g. `2026-07-15`) |
| `--priority P` | `add` | Priority select: `P0` / `P1` / `P2` |

### todo list — JSON output (array of TodoItem)

```json
[{"id":"page_abc","title":"写周报","status":"Todo","due":"2026-07-15","priority":"P1"}]
```

### todo add — JSON output (object)

```json
{"id":"todo_abc","url":"","title":"写周报"}
```

### todo start / complete — JSON output (object)

```json
{"id":"todo_abc","status":"In Progress","url":""}
```

### note options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
| `--query Q` | `search` | Keyword matched against note titles (required) |
| `--prop K:V` | `create` / `update` | Property setter, repeatable. Value may contain `:`. |
| `--text TEXT` | `append` | Text/markdown to append. If omitted, reads from `stdin` (non-TTY only) |
| `--limit N` | `search` / `list` | Max rows (`search` default 10, `list` default 50, cap 100; `0` = unlimited) |

### note search — JSON output (array of objects)

```json
[{"id":"abc123_x","title":"2026年工作计划","last_edited":"2026-07-09 18:00","url":""}]
```

### note list — JSON output (array of objects, properties simplified to strings)

```json
[{"id":"...","title":"Quick Note","url":"","last_edited":"2026-07-10T07:01:00.000Z","properties":{"名称":"Quick Note"}}]
```

### note create — JSON output (object)

```json
{"id":"...","title":"Rust 异步运行时深入浅出","properties":{"状态":"未读"}}
```

### note read — JSON output (object with aggregated Markdown)

```json
{"id":"abc123_x","title":"2026年工作计划","url":"","properties":{"Status":"In Progress"},"content":"# 2026年工作计划\n\n## 核心目标\n- 完成 everyday CLI 稳定版发布。"}
```

### note append — JSON output (object)

```json
{"id":"...","url":"","appended":3}
```

### note update — JSON output (object)

```json
{"id":"...","url":"","updated":1}
```

---

## bookmark — bookmarks (local SQLite) ✅

Bookmarks are stored in a local SQLite database per account (`~/.config/everyday/bookmark-<account>.db`); no credentials, no network. A clean `BookmarkItem` (id / url / title / tags) is mapped to/from local rows.

| Command | Description | Example |
|---------|-------------|---------|
| `bookmark list` | List bookmarks (`--tag` filters by a single tag) | `everyday bookmark list --tag rust --json` |
| `bookmark add` | Add a bookmark (`--url` and `--title` required; `--tags` optional, comma-separated) | `everyday bookmark add --url "https://..." --title "Rust" --tags "rust,cli" --json` |

### bookmark options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--account NAME` | all | Specify account (override default) |
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

Append-only event log aggregating `mail` / `cal` / `rss` + local `note` / `todo` / `bookmark` activity. Events come from the local providers (`mail` / `cal` / `rss` pulled during `sync`; `note` / `todo` / `bookmark` projected from their local SQLite tables). Storage is a separate SQLite at `~/.config/everyday/timeline.db` (does not touch provider DBs).

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
- `todo` / `note` / `bookmark` — projected from their local SQLite tables (current-state projection).
- `*_local` suffix is **not** produced; local providers are projected under their module name (`todo`, `note`, `bookmark`).

`timestamp` is RFC3339 UTC. Display formatting is the consumer's job (the CLI's Text renderer formats it in the user's local timezone).

### Design constraints (do not expect otherwise)

- **Append-only.** Re-running `sync` does not duplicate rows — natural key `(source, account, ref_id, event_type, timestamp)` is upserted with `INSERT OR IGNORE`.
- **Cal is the only window-refresh provider.** Each `sync` rewrites the cal window `[last_sync, now+7d]`, so cancelled events disappear. Other providers are purely append.
- **No `--from` / `--to` and no `--since` together.** `--from` / `--to` win; `--since` wins over preset; preset is the fallback. The combinations `today + --since 2026-07-09` widen `from` while keeping `to` at `now()` (useful for "today's window expanded to start earlier").
- **Local note/todo/bookmark events are projected from their SQLite tables during `sync`.** Each `sync` re-projects current state, upserted by the natural key above.

---

## memory — structured agent notebook ✅ (v0.10.0)

A persistent, append-only notebook for the agent itself. Triples `(subject, predicate, object)` with optional `--confidence` (default `1.0`) and `--source`. Re-adding the same triple creates a new version row; `delete` soft-deletes the current-state row; `history` returns every version including deleted ones. Storage is a single global SQLite file at `~/.config/everyday/memory.db` — no `account` column, no `auth` module touch (K004).

| Command | Description | Example |
|---------|-------------|---------|
| `memory add <S> <P> <O>` | Append a triple; creates a new version if `(S, P, O)` already exists | `everyday memory add user prefers rust --confidence 0.9 --source explicit --json` |
| `memory get <SUBJECT>` | List current-state triples for a subject | `everyday memory get user --json` |
| `memory relation <SUBJECT> <PREDICATE>` | List current-state triples matching `(subject, predicate)` | `everyday memory relation user prefers --json` |
| `memory list` | List all current-state triples (default cap 100) | `everyday memory list --limit 50 --json` |
| `memory delete <S> <P> <O>` | Soft-delete the current-state row of a triple | `everyday memory delete user prefers rust --json` |
| `memory graph <SUBJECT>` | Forward BFS from a subject (depth default 2, max 5) | `everyday memory graph user --depth 2` |
| `memory history <S> <P> <O>` | Show all versions of a triple (incl. deleted) | `everyday memory history user prefers rust --json` |

### memory options

| Flag | Applies to | Description |
|------|-----------|-------------|
| `--confidence N` | `add` | Confidence in `[0.0, 1.0]` (default `1.0`); out-of-range or non-numeric → `InvalidArgument` |
| `--source LABEL` | `add` | Free-text provenance label (e.g. `explicit`, `inferred`) |
| `--limit N` | `list` | Cap row count (default 100) |
| `--depth N` | `graph` | Recursion depth in `1..=5` (default 2); out of range → `InvalidArgument` |
| `--include-deleted` | `graph` | Include soft-deleted edges in the traversal (default hidden) |

### memory JSON output

`add` returns the inserted fact; `get` / `relation` / `list` return `{"facts": [...], "count": N}`; `delete` returns `{id, subject, predicate, object, deleted_at}`; `history` returns `{"history": [...], "count": N}` (each row carries `deleted_at`); `graph` returns a nested object (text mode: indented markdown tree).

```json
{
  "facts": [
    {
      "id": "m18c2...",
      "subject": "user",
      "predicate": "prefers",
      "object": "rust",
      "confidence": 0.9,
      "source": "explicit",
      "created_at": "2026-07-14T15:03:38+00:00"
    }
  ],
  "count": 1
}
```

### memory behavior & semantics

- **Append-only**: `add` always inserts a new row. Re-adding the same `(S, P, O)` does **not** update the existing row; it appends a new version. `history` returns every version.
- **Soft delete**: `delete` sets `deleted_at = now()` on the **current-state** row (the row with `MAX(created_at) WHERE deleted_at IS NULL`). Subsequent `delete` calls on the same triple return `InvalidArgument("triple not found or already deleted")`.
- **Resurrection**: `add` after delete creates a new row (append-only). To recover a specific historical row by `id`, wait for v2 `undelete-by-id` (K001, v2 deferred).
- **No semantic validation**: the program does not enforce `prefers`/`knows`/etc.; triples are free-form. Conventions live in [MEMORY.md](MEMORY.md).
- **Graph**: forward-only BFS over current state; cycle detection via visited set keyed by `(subject, predicate, object)`. `--include-deleted` flips the source view to the underlying table for the traversal.
- **Searchable**: memory facts (current state) participate in `everyday search`. `Hit.id` is `"memory:<row_id>"` so agents can drill into `memory history` / `memory get` via the id.

### Subject naming convention

The recommended vocabulary (and rationale) lives in [MEMORY.md](MEMORY.md); it is a convention, not enforced in code.

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
provider = "local"   # alias "sqlite"; `provider = "notion"` is no longer supported (validation error)

[[todo.accounts]]
name = "personal"
provider = "local"

[[bookmark.accounts]]
name = "personal"
provider = "local"
```

**Keyring service-name convention:** `everyday/<module>/<account>` (e.g. `everyday/mail/work`, `everyday/note/personal`, `everyday/todo/personal`, `everyday/bookmark/personal`).

---

## Error types (JSON mode)

Exit code `0` on success, `1` on failure. Error envelope:

```json
{"error": "ErrorType", "message": "Details..."}
```

`ErrorType` values (PascalCase): `ConfigError` · `AccountNotFound` · `AuthError` · `NetworkError` · `IoError` · `ModuleNotFound` · `UnknownAction` · `InvalidArgument` · `PermissionDenied` · `NotImplemented` · `Other`
