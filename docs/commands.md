# Command Reference

Per-module command tables for every `everyday` module, plus the global options,
output modes (text / JSON / errors / logging), and CLI behavior. This is the
complete version of the README "Command Reference" and "Output Modes" sections.

- [English](commands.md) · [中文](commands_zh.md)

---

### Global options

| Option | Description |
|------|------|
| `--json` | Output clean JSON, ideal for programmatic parsing |
| `--account <NAME>` | Override the module's default account |
| `--version` | Show the version |
| `--help` | Show help |

### config — configuration management

Manages the `~/.config/everyday/config.toml` file.

| Command | Description | Usage |
|------|------|------|
| `path` | Show the config file path | `everyday config path` |
| `list` | List all configuration | `everyday config list [--json]` |
| `get` | Read a config item (supports dotted paths and array indices) | `everyday config get <dotted.path>` |
| `set` | Set a config item (type inferred automatically) | `everyday config set <dotted.path> <value>` |
| `init` | Create a sample config | `everyday config init` |

**Dotted-path examples**:
```bash
everyday config get mail.accounts.0.name        # → work
everyday config get default_account.mail         # → work
everyday config set mail.accounts.0.imap_port 993
everyday config set default_account.mail personal
```

### mail — email management

Based on IMAP (receiving) and SMTP (sending); credentials go through the system keyring.

| Command | Description | Usage |
|------|------|------|
| `folders` | List all mailbox folders | `everyday mail folders [--account NAME]` |
| `list` | List message summaries (from local cache; auto-sync if stale) | `everyday mail list [--unread] [--limit N] [--folder NAME] [--no-recursive] [--sync]` |
| `read` | Read a single message (recursive lookup by default) | `everyday mail read <uid> [--folder NAME] [--no-recursive]` |
| `search` | Search messages | `everyday mail search --query Q [--limit N] [--folder NAME]` |
| `send` | Send a message | `everyday mail send --to ADDR --subject S --body TEXT [--cc ADDR]` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--account NAME` | all | Specify the account |
| `--unread` | `list` | Unread only |
| `--limit N` | `list` / `search` | Limit the count, default 20 |
| `--folder NAME` | `list` / `read` / `search` | Specify a folder (non-ASCII names supported); recurses all folders by default |
| `--no-recursive` | `list` / `read` / `search` | INBOX only |
| `--sync` | `list` | Force an IMAP sync before listing (ignore staleness) |
| `--to ADDR` | `send` | Recipient (required) |
| `--subject S` | `send` | Subject (required) |
| `--body TEXT` | `send` | Body (required) |
| `--cc ADDR` | `send` | Carbon copy |

**Recursive search**: `list` / `search` / `read` traverse all folders by default. `list` / `search` merge results across folders sorted by message date descending; `read` returns the first message whose UID matches (IMAP UIDs are unique only within a folder, not across folders, hence the recursive lookup).

### cal — calendar management (CalDAV)

| Command | Description | Status | Usage |
|------|------|------|------|
| `list` | List events | ✅ Available | `everyday cal list [--today\|--date YYYY-MM-DD]` |
| `add` | Add an event | ✅ Available | `everyday cal add --title T --start ISO --end ISO` |
| `delete` | Delete an event | ✅ Available | `everyday cal delete --id ID` |

### rss — RSS/Atom feeds

| Command | Description | Status | Usage |
|------|------|------|------|
| `follow` | Add a feed | ✅ Available | `everyday rss follow --name N --url URL [--category C]` |
| `list` | List feeds | ✅ Available | `everyday rss list` |
| `digest` | Aggregate recent items | ✅ Available | `everyday rss digest [--limit N]` |

### note — notes & knowledge base (local SQLite)

**Uses the local SQLite provider (`provider = "local"`, alias `sqlite`)**: no credentials, no network, data stored at `~/.config/everyday/note-<account>.db`, works out of the box.

| Command | Description | Usage |
|------|------|------|
| `search` | Search pages / databases by title | `everyday note search --query Q [--limit N]` |
| `list` | List pages in a database | `everyday note list [--limit N]` |
| `create` | Create a new page (record) in a database | `everyday note create --title T [--prop K:V ...]` |
| `read` | Read a page body, aggregated into Markdown | `everyday note read <id>` |
| `append` | Append a text block to the end of a page | `everyday note append [id] --text TEXT` |
| `update` | Modify page properties (meta) | `everyday note update <id> --prop K:V ...` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--account NAME` | all | Specify the account |
| `--query Q` | `search` | Keyword search (page / database title) |
| `--prop K:V` | `create` / `update` | Simplified property setting, repeatable; encoded precisely against the database schema (title / text / number / checkbox / select, etc.), values may contain colons |
| `--text TEXT` | `append` | Text to append; when omitted, read from piped `stdin` (non-terminal mode only) |
| `--limit N` | `search` / `list` | Limit the count (`search` default 10, `list` default 50, cap 100; `--limit 0` means unlimited) |

> **Local provider (default)**: no setup needed — just run `everyday note create` / `append`; the database file is created automatically.

### todo — to-do tasks (local SQLite)

**Uses the local SQLite provider (`provider = "local"`, alias `sqlite`)**: no credentials, no network, tasks stored at `~/.config/everyday/todo-<account>.db`, tables auto-created per command, works out of the box.

| Command | Description | Usage |
|------|------|------|
| `list` | List unfinished tasks (by Due ascending) | `everyday todo list [--all]` |
| `add` | Add a task | `everyday todo add --title T [--due DATE] [--priority P]` |
| `start` | Mark a task as In Progress | `everyday todo start <id>` |
| `complete` | Mark a task as Done | `everyday todo complete <id>` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--account NAME` | all | Specify the account |
| `--all` | `list` | List all tasks (including Done) |
| `--title T` | `add` | Task title (required) |
| `--due DATE` | `add` | Due date (ISO 8601, e.g. `2026-07-15`) |
| `--priority P` | `add` | Priority (select: P0 / P1 / P2) |

> **Local provider (default)**: no setup needed — just run `everyday todo add` / `list`; the database file and tables are created automatically.

### bookmark — bookmarks (local SQLite)

**Uses the local SQLite provider (`provider = "local"`, alias `sqlite`)**: no credentials, no network, bookmarks stored at `~/.config/everyday/bookmark-<account>.db` (a `bookmarks` table plus a `bookmark_tags` relation table enabling precise per-tag filtering), tables auto-created per command, works out of the box.

| Command | Description | Usage |
|------|------|------|
| `list` | List bookmarks (`--tag` filters by a single tag) | `everyday bookmark list [--tag TAG]` |
| `add` | Add a bookmark | `everyday bookmark add --url U --title T [--tags a,b]` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--account NAME` | all | Specify the account |
| `--tag TAG` | `list` | Filter by a single tag (exact match); omit to list all |
| `--url U` | `add` | Bookmark URL (required) |
| `--title T` | `add` | Bookmark title (required) |
| `--tags a,b` | `add` | Comma-separated tags (optional; e.g. `rust,cli`) |

**Tag parsing**: `--tags "rust, cli , web"` is split on commas, trimmed, and empty entries dropped → `["rust", "cli", "web"]`.

> **Local provider (default)**: no setup needed — just run `everyday bookmark add` / `list`; the database file and tables are created automatically.

### auth — credential lifecycle (NEW in v0.8.0)

Consolidated credential management for all modules. Modules read stored credentials internally via `auth::get_credential`; you only use these commands to manage credentials in the OS keyring. Password strategy (mail/cal/webdav) uses `--password`. If the flag is omitted, it falls back to an interactive prompt. Passwords never touch disk.

**Env fallback (opt-in, R020):** when no OS keyring backend exists (headless server / CI / sandbox), enable `[auth] env_credentials = true` (or `EVERYDAY_ENV_CREDENTIALS=1`) and export `EVERYDAY_<MODULE>_<ACCOUNT>_PASSWORD` (e.g. `EVERYDAY_MAIL_WORK_PASSWORD`). Read chain: keyring → env → error. Both switches activate the fallback for all modules, including `mail list` / `cal` / `sync` hot paths (the config field is mirrored process-wide at startup). Env-sourced secrets are visible to every child process — use only when the keyring is truly unavailable.

| Command | Description | Usage |
|------|------|------|
| `login` | Store a credential in the OS keyring (optionally verify with `--verify`). `--module` required; `--account` defaults to the module's default account | `everyday auth login --module mail --account work --password PWD` |
| `logout` | Delete the stored credential from the keyring; errors with an `unset` hint if the credential comes from the environment | `everyday auth logout --module mail --account work` |
| `verify` | Read the stored credential (keyring → env) and verify it against the server (no re-prompt); reports `not_required` for local/sqlite or rss | `everyday auth verify --module note` |
| `list` | List configured accounts and their credential state: stored / env / missing / not_required | `everyday auth list --module todo` |

For WebDAV device sync, store the **application password** (not the login password): `everyday auth login --module webdav --account personal` (keyring `everyday/webdav/personal`).

### timeline — unified event timeline (NEW in v0.5.0)

A single, append-only event log that aggregates local events from **mail · cal · rss · note · todo · bookmark**. Each source has a `TimelineProvider` adapter; sync is parallel across sources but serial within a source (rate-limit friendly). Storage is SQLite at `~/.config/everyday/timeline.db` (separate from the provider DBs).

**Why**: instead of polling each module separately, the agent issues one query and gets a unified, time-ordered feed across all integrations.

| Command | Description | Usage |
|------|------|------|
| `today` / `yesterday` / `week` / `month` | Query a preset window (Mon–Sun for week, calendar month for month) | `everyday timeline today [--source S] [--account A] [--limit N] [--since DURATION_OR_DATE]` |
| `sync` | Pull from all configured providers (or a `--source`-filtered subset) into `timeline.db`; idempotent, watermark-based | `everyday timeline sync [--source mail,cal,todo] [--since 2026-01-01]` |

**Common flags**:

| Flag | Applies to | Description |
|------|------|------|
| `--json` | all | Switch to JSON output (recommended for agents) |
| `--source S[,S2]` | all | Comma-separated filter, e.g. `mail,cal` or `todo` |
| `--account A` | all | Filter to one account name (e.g. `personal`) |
| `--limit N` | query | Cap event count, default 100 |
| `--since DUR_OR_DATE` | all | Sliding window start. `30m` / `2h` / `1d` / `7d` relative to now, or `YYYY-MM-DD` for start-of-day. `to` is `now()`. (Implicit `--from`/`--to` is also accepted for absolute windows.) |
| `--sync` | query | Run `sync` first, then query (atomic) |

**Example**:

```bash
# Today's events across all sources, JSON output
everyday timeline today --json | jq '.[].title'

# Sync only mail and cal, then show this week
everyday timeline sync --source mail,cal
everyday timeline week --json

# Anything since 30 minutes ago (sub-day precision)
everyday timeline today --since 30m --json

everyday timeline today --source todo --json
```

### search — cross-module unified search (NEW in v0.7.0)

One query, all modules. A single `everyday search` call fans out concurrently to every registered `Searchable` provider (note / todo / bookmark / rss / cal / mail / memory), merges the hits into one time-ordered list, and renders them as Text or JSON. Empty results exit 0; per-module failures are surfaced as `SearchWarning` on stderr (text mode) or as a structured `{"_warning": ...}` line (`--json` mode) without aborting the whole query.

| Command | Description | Usage |
|------|------|------|
| `query` | Run a free-text query across every searchable module | `everyday search query "<q>" [--module a,b,c] [--since 7d] [--limit N] [--json]` |

**Module scope**: `note` / `todo` / `bookmark` (local SQLite, GLOB over title + content/url/tag), `rss` (a local item cache table at `~/.config/everyday/rss-items.db` populated by `rss digest` / `rss fetch`), `cal` (full-pull + in-memory GLOB over summary / location / start), `mail` (local envelope cache via [S007], since v0.9.0), `memory` (current-state view GLOB over subject/predicate/object, since v0.10.0).

**Query semantics**: whitespace-tokenized, OR over tokens, case-insensitive GLOB substring (`lower(col) GLOB '*token*'`). Per-module hard cap = 50; global cap = 20 (default). `ts desc` ordering; each module's primary time is its `ts` (note: updated_at; todo: updated_at; bookmark: created_at; rss: published; cal: event start).

**Example**:

```bash
# Find anything mentioning "rust" across all modules, JSON output
everyday search query "rust" --json

# Restrict to note + todo, with a 7-day lower bound
everyday search query "rust timeline" --module note,todo --since 7d

# Cap the merged result to 5 hits
everyday search query "release" --limit 5
```

**Design notes**:

- **Append-only**: events have a natural unique key `(source, account, ref_id, event_type, timestamp)` (`INSERT OR IGNORE`), so re-running `sync` is safe.
- **UTC storage, local display**: timestamps are stored in UTC and rendered in the local timezone.
- **Cal is window-refresh**: unlike the append-only mail / rss providers, `cal` rewrites its window (`[last_sync, now+7d]`) so cancelled events actually disappear.

See `CONTEXT.md` + `adr/0001`–`0009` for the full design rationale.

### memory — structured agent notebook (NEW in v0.10.0)

A persistent, append-only notebook for the agent itself — store stable facts as `(subject, predicate, object)` triples with optional `confidence` and `source`. Triples are versioned: re-adding the same `(S, P, O)` creates a new row (the previous one is preserved in history). Soft delete hides rows from current-state queries but keeps them in `history`. Storage is a single global SQLite file at `~/.config/everyday/memory.db` (no `account`, no `auth` module touch). Memory participates in `everyday search` as a `Searchable` provider over the current-state view.

| Command | Description | Usage |
|------|------|------|
| `add` | Append a triple (creates a new version if `(S,P,O)` already exists) | `everyday memory add <S> <P> <O> [--confidence N] [--source LABEL]` |
| `get` | List current-state triples for a subject | `everyday memory get <SUBJECT>` |
| `relation` | List current-state triples matching `(subject, predicate)` | `everyday memory relation <SUBJECT> <PREDICATE>` |
| `list` | List all current-state triples (capped at 100) | `everyday memory list [--limit N]` |
| `delete` | Soft-delete the current-state row of a triple | `everyday memory delete <S> <P> <O>` |
| `graph` | Forward BFS from a subject (depth 1..=5, default 2) | `everyday memory graph <SUBJECT> [--depth N] [--include-deleted]` |
| `history` | Show all versions of a triple (including deleted rows) | `everyday memory history <S> <P> <O>` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--confidence N` | `add` | Confidence in `[0.0, 1.0]` (default `1.0`) |
| `--source LABEL` | `add` | Free-text provenance label (e.g. `explicit`, `inferred`) |
| `--limit N` | `list` | Cap row count, default 100 |
| `--depth N` | `graph` | Recursion depth in `1..=5`, default 2 |
| `--include-deleted` | `graph` | Include soft-deleted edges in the traversal |

**Subject naming convention** (program does not enforce; documented in `../skills/everyday-cli/references/MEMORY.md`):

```
user                       # bare subject for the human user
project-everyday           # project entity
tech:rust                  # domain-prefixed: technology knowledge
team:backend:alice         # hierarchical: team > sub-team > person
```

**Example**:
```bash
# Record what the user prefers
everyday memory add user prefers rust --confidence 0.9 --source explicit --json

# Look up everything we know about the user
everyday memory get user --json

# Multi-hop traversal
everyday memory graph user --depth 2

# Memory facts join `everyday search` automatically
everyday search query "rust" --module memory --json
```

### health — module health check (NEW in v0.11.0)

Runs every module's `health_check` and renders one row per module. Checks are **local-only by design** (cache / config DB openable, keyring credential present) — never network calls, so `health` is fast and works offline. Modules that don't override the check (search / auth / config) report `ok` via the default. All rows render regardless of state; the **exit code is 0 when every module is healthy and 1 when any module is degraded** (so scripts can gate on it).

| Command | Description | Usage |
|------|------|------|
| `health` | Probe every module's local health | `everyday health [--json]` |

**Text output** (one row per module):

```
$ everyday health
module    status  detail
------------------------
config    ok      ok
auth      ok      ok
...
```

**JSON output** (`--json`):

```json
[{"detail":"ok","healthy":true,"module":"config"},{"detail":"ok","healthy":true,"module":"auth"},...]
```

Implemented as part of the lifecycle hooks in [F012](adr/F012-architecture-deepening-phase.md) (P3).

### sync — cross-device file sync via WebDAV (NEW in v0.13.0)

Bidirectional, **file-level** sync of your real user data to a WebDAV directory (default: Jianguoyun `dav.jianguoyun.com`): the four user DBs (`bookmark-<acct>.db` / `note-<acct>.db` / `todo-<acct>.db` / `memory.db`) plus `config.toml`. Derived caches (mail_cache / rss-items / timeline) are never synced. Changes are detected by content hash (DBs are snapshotted via `VACUUM INTO` first, so WAL data is included); conflicts are resolved by **Last-Write-Wins with a dual conflict copy** — the loser is kept as `<name>.conflict-<UTC ts>.<ext>` both locally and on the remote, so nothing is ever lost.

| Command | Description | Usage |
|------|------|------|
| `sync` | Bidirectional sync (pull-then-push). First sync auto-detects the direction: empty remote → push all; fresh device (empty config template) → pull all | `everyday sync` |
| `--push-only` | Upload local changes only | `everyday sync --push-only` |
| `--pull-only` | Download remote changes only | `everyday sync --pull-only` |
| `--force` | Ignore `sync-state.json` and re-upload everything local + pull remote-only files | `everyday sync --force` |

**Setup**: configure the account in `config.toml` (`[[webdav.accounts]]` — name / url / username), then store the **application password** (not the login password) in the keyring:

```
everyday auth login --module webdav --account personal
```

**Auto-sync (opt-in, default off)**: with `auto_sync = true` on an account, a successful write command (`bookmark add`, `note create`, `memory add`, ...) does a best-effort push of the changed files before returning. Failures only print a warning line and never change the command's exit code; query paths never trigger sync ([D003](adr/D003-auto-sync-cli-boundary.md)).

Sync state lives in `sync-state.json` next to the config (not synced itself); deleting it or passing `--force` rebuilds it from a full re-upload. Design: [D001](adr/D001-webdav-file-sync.md) / [D002](adr/D002-snapshot-hash-state.md) / [D003](adr/D003-auto-sync-cli-boundary.md).

### mcp — expose everyday as an MCP server (NEW in v0.15.0)

Turns `everyday` into a **Model Context Protocol (MCP) server** over stdio. Any
MCP-capable agent (Claude Code, CodeBuddy, Cursor, ...) connects with a one-line
`mcpServers` entry and gets every `(module, action)` as a **tool** named
`<module>_<action>` — argument schemas are generated from the same
`module_arg_spec()` the CLI uses, so the MCP surface can never drift from the
CLI. Design: [F014](adr/F014-mcp-module.md), glossary: `../CONTEXT.md` §MCP.

| Command | Description | Usage |
|------|------|------|
| `serve` | Run the MCP stdio server (blocks until stdin closes, then exits 0) | `everyday mcp serve` |
| `tools` | Print the projected tool list + JSON Schemas (debugging) | `everyday mcp tools` |

**Connect an MCP client** (e.g. Claude Desktop `claude_desktop_config.json`, or the
equivalent `mcpServers` block in Claude Code / CodeBuddy):

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

**Notes**

- One tool per `(module, action)`: `mail_list`, `note_add`, `timeline_today`, ...
  The `mcp` module itself is not projected (`mcp_*` tools do not exist).
- Tool results are the `--json`-rendered output; failures come back as MCP
  `isError`. The optional `account` argument mirrors the CLI `--account` flag.
- The server is a **long-lived process**: config / database changes are picked
  up on the **next session start**, not mid-session.
- stdout is reserved for JSON-RPC; all logging goes to stderr.

### daemon — resident auto-sync (NEW in v0.17.0)

Runs a background process that **pulls** mail / rss / timeline events on a
schedule, so `timeline`, `search` and `mail list` queries always see fresh data
without an explicit `--sync`. The daemon is the **only** role allowed to pull
periodically — query paths never auto-sync (L005), so behavior is identical
whether it runs or not. Design: [F016](adr/F016-daemon-sync-scheduler.md);
operations guide: [daemon.md](daemon.md).

| Command | Description | Usage |
|------|------|------|
| `run` | Resident sync loop: syncs once immediately, then one cycle every `interval_seconds` | `everyday daemon run` |
| `run --once` | Run a single sync cycle then exit (manual catch-up / debugging) | `everyday daemon run --once` |
| `run --sources mail,rss` | Override the `[daemon].sources` whitelist for this run | `everyday daemon run --once --sources mail,rss` |
| `status` | Running / stopped (pid-liveness corrected), last cycle, per-source results | `everyday daemon status [--json]` |

**Notes**

- Configure via `[daemon]` in `config.toml`: `enabled`, `interval_seconds`
  (default 900), `sources` (empty = all). `enabled = false` makes `run` refuse
  to start (exit 1).
- Each cycle (sequential, best-effort): timeline event pull + mail envelope
  cache sync (**all server folders**) + rss cache pull. A failing action is
  recorded, never fatal.
- `daemon run` is a **foreground resident process** — install it with the OS
  service manager (`nssm` / `launchd` / `systemd`); see
  [daemon.md](daemon.md). State and logs live in
  `~/.config/everyday/daemon-state.json` and `daemon.log`.

## Output Modes

### Text mode (default)

Great for direct terminal viewing; tables align automatically:

```
$ everyday mail list --unread --limit 3
uid    unread  folder  date                          from              subject
-----------------------------------------------------------------------------
12345  true    INBOX   Wed, 8 Jul 2026 08:29 +0000  sender@x.com      Hello
12344  true    INBOX   Wed, 8 Jul 2026 07:15 +0000  boss@x.com        Weekly Report
12343  false   Drafts  Wed, 8 Jul 2026 06:00 +0000  me@x.com          Draft
```

### JSON mode (`--json`)

Outputs clean JSON with no extra whitespace, ideal for programmatic parsing:

```bash
$ everyday mail list --unread --limit 2 --json
[{"uid":12345,"unread":true,"folder":"INBOX","date":"Wed, 8 Jul 2026 08:29:31 +0000","from":"sender@x.com","subject":"Hello"},{"uid":12344,"unread":true,"folder":"INBOX","date":"Wed, 8 Jul 2026 07:15:00 +0000","from":"boss@x.com","subject":"Weekly Report"}]
```

> `mail list` rows are **typed records**: `uid` is a JSON number and `unread` a
> JSON boolean (previously everything was stringified). See
> [F012](adr/F012-architecture-deepening-phase.md) P6.

### Error output

Error format in JSON mode:

```json
{"error": "AccountNotFound", "message": "mail account 'work'"}
```

Exit codes: `0` on success, `1` on failure.

### Logging & verbosity

All diagnostics go to stderr; stdout carries only the command result
([R001](adr/R001-thread-local-json-mode.md)). Default is **quiet**
(WARN): warnings and errors are visible, per-command progress logs are
silent.

| Flag | Level | Effect |
| --- | --- | --- |
| (none) | WARN | Default: warnings/errors visible, progress silent |
| `-v` | INFO | Restores middleware progress logs: `[req] module action ok in 12ms` (text) or `{"_log": ...}` lines (JSON mode) |
| `-vv` | DEBUG | Reserved (no debug output yet) |

- The auto-sync success notice (`warning: auto_sync_pushed: N file(s) pushed`) is info-level and appears with `-v`; failures (`auto_sync_failed`) are always visible.
- In JSON mode stderr lines are structured: `{"_log": ...}` (middleware progress), `{"_warning": ...}` (partial failures: init failure / auto_sync / search provider / timeline sync), `{"_error": ...}` (fatal).
- The stdout contract is unchanged: the command result (including `--json`) is the only thing on stdout.

