# Everyday CLI

> The Rust-powered hands for your AI Agent.

**语言 / Language:** **English** · [简体中文](README_ZH.md)

`everyday` is a high-performance, memory-safe local CLI toolkit written in Rust. It acts as the "digital hands" of an AI Agent, offering a unified command structure that covers external-integration scenarios — email, calendar, RSS feeds, notes (local SQLite by default / optional Notion), to-dos (local SQLite by default / optional Notion), bookmarks (local SQLite by default / optional Notion), and a structured agent memory notebook — with dual Text / JSON output modes.

## Features

- **Unified command structure**: `everyday <module> <action> [options]`, low learning curve
- **Dual output modes**: Text by default (human-readable tables); `--json` switches to clean JSON (the primary mode for AI interaction)
- **Multi-account support**: each module supports multiple named accounts, switchable via `--account`
- **Credential safety**: passwords go through the system keyring (macOS Keychain / Windows Credential Manager / Linux Secret Service) and are never written to disk
- **Cross-platform**: Windows / macOS / Linux
- **High performance**: cold start < 100ms, async runtime (tokio), memory safe

## Installation

### Download a prebuilt binary (recommended)

Download the archive for your platform from [GitHub Releases](https://github.com/duyixian1234/everyday/releases), extract it, and add `everyday` to your `PATH`. Every release ships assets for each platform (including macOS x86_64 and Apple Silicon / aarch64):

| Platform | Asset | Extract / Install |
|------|----------|-------------|
| Linux (x86_64) | `everyday-x86_64-unknown-linux-gnu.tar.gz` | `tar xzf <file> && sudo mv everyday /usr/local/bin/` |
| macOS (x86_64) | `everyday-x86_64-apple-darwin.tar.gz` | `tar xzf <file> && sudo mv everyday /usr/local/bin/` |
| macOS (Apple Silicon / aarch64) | `everyday-aarch64-apple-darwin.tar.gz` | `tar xzf <file> && sudo mv everyday /usr/local/bin/` |
| Windows (x86_64) | `everyday-x86_64-pc-windows-msvc.zip` | Extract and put `everyday.exe` into a `PATH` directory |

One-line install for macOS / Linux (always fetches latest):

```bash
# Linux
curl -L https://github.com/duyixian1234/everyday/releases/latest/download/everyday-x86_64-unknown-linux-gnu.tar.gz | tar xz && sudo mv everyday /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/duyixian1234/everyday/releases/latest/download/everyday-x86_64-apple-darwin.tar.gz | tar xz && sudo mv everyday /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/duyixian1234/everyday/releases/latest/download/everyday-aarch64-apple-darwin.tar.gz | tar xz && sudo mv everyday /usr/local/bin/
```

> Binaries are built and published automatically by CI on every `v*` tag (see `.github/workflows/release.yml`), covering Linux / macOS (x86_64 and aarch64) / Windows — three platforms, four architectures.

### Build from source

```bash
git clone https://github.com/duyixian1234/everyday.git
cd everyday
cargo build --release
```

The compiled binary is at `target/release/everyday`; add it to your `PATH`.

### Install via cargo

```bash
cargo install --git https://github.com/duyixian1234/everyday.git
```

### Verify the installation

```bash
everyday --version
everyday config path
```

## Quick Start

### 1. Initialize the config

```bash
# Generate a sample config file
everyday config init

# Show the config path
everyday config path
# → ~/.config/everyday/config.toml
```

### 2. Configure a mail account

Edit `~/.config/everyday/config.toml`:

```toml
[default_account]
mail = "work"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 587
username = "me@example.com"
tls = true
```

Or set each field via the command line:

```bash
everyday config set default_account.mail work
everyday config set mail.accounts.0.name work
everyday config set mail.accounts.0.imap_host imap.example.com
everyday config set mail.accounts.0.smtp_host smtp.example.com
everyday config set mail.accounts.0.username me@example.com
```

### 3. Store the password

```bash
everyday auth login --module mail --account work
# Prompts for the password and stores it in the system keyring (never on disk)
```

### 4. Start using it

```bash
# List unread mail
everyday mail list --unread

# JSON mode (AI-friendly)
everyday mail list --unread --limit 10 --json
```

## Command Reference

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

### note — notes & knowledge base (local SQLite by default / optional Notion)

**Uses the local SQLite provider by default (`provider = "local"`, alias `sqlite`)**: no credentials, no network, data stored at `~/.config/everyday/note-<account>.db`, works out of the box. You can also set `provider = "notion"` to use the Notion API, which hides the tedious block nesting and exposes two high-level capabilities to the Agent — **plain-text / Markdown append** and **simplified property operations** (the Notion integration token lives only in the system keyring, never on disk). Command usage is identical across both providers.

| Command | Description | Usage |
|------|------|------|
| `search` | Search pages / databases by title | `everyday note search --query Q [--limit N]` |
| `list` | List pages in a database | `everyday note list [--db ID] [--limit N]` |
| `create` | Create a new page (record) in a database | `everyday note create --title T [--db ID] [--prop K:V ...]` |
| `read` | Read a page body, aggregated into Markdown | `everyday note read <page_id>` |
| `append` | Append a text block to the end of a page | `everyday note append [page_id] --text TEXT` |
| `update` | Modify page properties (meta) | `everyday note update <page_id> --prop K:V ...` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--account NAME` | all | Specify the account |
| `--query Q` | `search` | Keyword search (page / database title) |
| `--db ID` | `create` / `list` | Target database ID; falls back to config `default_database_id` |
| `--prop K:V` | `create` / `update` | Simplified property setting, repeatable; encoded precisely against the database schema (title / text / number / checkbox / select, etc.), values may contain colons |
| `--text TEXT` | `append` | Text to append; when omitted, read from piped `stdin` (non-terminal mode only) |
| `--limit N` | `search` / `list` | Limit the count (`search` default 10, `list` default 50, cap 100; `--limit 0` means unlimited) |

> **Local provider (default)**: no setup needed — just run `everyday note create` / `append`; the database file is created automatically.
> **Notion provider**: create an integration in Notion to get an `ntn_...` token → store it via `everyday auth login --module note` in the keyring → set that account to `provider = "notion"` in the config and fill in `default_database_id` / `default_page_id` → **share** the target page / database with the integration in Notion.

### todo — to-do tasks (local SQLite by default / optional Notion)

**Uses the local SQLite provider by default (`provider = "local"`, alias `sqlite`)**: no credentials, no network, tasks stored at `~/.config/everyday/todo-<account>.db`, tables auto-created per command, works out of the box. You can also set `provider = "notion"` to use a Notion database: low-level HTTP / token injection / 429 rate-limit retries are handled uniformly by the shared `notion-client`, while this module maps the clean domain model `TodoItem` (id / title / status / due / priority) to Notion's raw properties with strong typing (the token lives only in the system keyring `everyday/todo/<account>`; non-secret metadata such as `database_id` may be stored in the config). Command usage is identical across both providers.

| Command | Description | Usage |
|------|------|------|
| `init-db` | Init tables: local provider creates the SQLite table; Notion provider creates the task database (requires `parent_page_id`) and back-fills `database_id` | `everyday todo init-db [--account NAME] [--parent PAGE_ID]` |
| `list` | List unfinished tasks (by Due ascending) | `everyday todo list [--db ID] [--all]` |
| `add` | Add a task | `everyday todo add --title T [--due DATE] [--priority P] [--db ID]` |
| `start` | Mark a task as In Progress | `everyday todo start <page_id>` |
| `complete` | Mark a task as Done | `everyday todo complete <page_id>` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--account NAME` | all | Specify the account |
| `--parent PAGE_ID` | `init-db` | Parent page when creating the database; falls back to config `parent_page_id` |
| `--db ID` | `list` / `add` | Target database ID; falls back to config `default_database_id` (auto-filled after `init-db`) |
| `--all` | `list` | List all tasks (including Done) |
| `--title T` | `add` | Task title (required) |
| `--due DATE` | `add` | Due date (ISO 8601, e.g. `2026-07-15`) |
| `--priority P` | `add` | Priority (select: P0 / P1 / P2) |

> **Local provider (default)**: no setup needed — just run `everyday todo add` / `list`; the database file and tables are created automatically.
> **Notion provider**: create an integration in Notion to get an `ntn_...` token → store it via `everyday auth login --module todo` in the keyring → set that account to `provider = "notion"` in the config and fill in `parent_page_id` → `everyday todo init-db` to create the task database and authorize the integration to access the parent page. Then `list` / `add` / `start` / `complete` are ready to use.

### bookmark — bookmarks (local SQLite by default / optional Notion)

**Uses the local SQLite provider by default (`provider = "local"`, alias `sqlite`)**: no credentials, no network, bookmarks stored at `~/.config/everyday/bookmark-<account>.db` (a `bookmarks` table plus a `bookmark_tags` relation table enabling precise per-tag filtering), tables auto-created per command, works out of the box. You can also set `provider = "notion"` to use a Notion database: low-level HTTP / token injection / 429 rate-limit retries are handled uniformly by the shared `notion-client`, while this module maps the clean domain model `BookmarkItem` (id / url / title / tags) to Notion's raw properties (Title / URL / Tags) with strong typing (the token lives only in the system keyring `everyday/bookmark/<account>`; non-secret metadata such as `database_id` may be stored in the config). Command usage is identical across both providers.

| Command | Description | Usage |
|------|------|------|
| `init-db` | Init storage: local provider creates the SQLite tables; Notion provider creates the bookmark database (requires `parent_page_id`) and back-fills `database_id` | `everyday bookmark init-db [--account NAME] [--parent PAGE_ID]` |
| `list` | List bookmarks (`--tag` filters by a single tag) | `everyday bookmark list [--tag TAG] [--db ID]` |
| `add` | Add a bookmark | `everyday bookmark add --url U --title T [--tags a,b] [--db ID]` |

**Option details**:

| Option | Applies to | Description |
|------|----------|------|
| `--account NAME` | all | Specify the account |
| `--parent PAGE_ID` | `init-db` | Parent page when creating the database; falls back to config `parent_page_id` |
| `--db ID` | `list` / `add` | Target database ID (Notion only); falls back to config `default_database_id` (auto-filled after `init-db`) |
| `--tag TAG` | `list` | Filter by a single tag (exact match); omit to list all |
| `--url U` | `add` | Bookmark URL (required) |
| `--title T` | `add` | Bookmark title (required) |
| `--tags a,b` | `add` | Comma-separated tags (optional; e.g. `rust,cli`) |

**Tag parsing**: `--tags "rust, cli , web"` is split on commas, trimmed, and empty entries dropped → `["rust", "cli", "web"]`.

> **Local provider (default)**: no setup needed — just run `everyday bookmark add` / `list`; the database file and tables are created automatically.
> **Notion provider**: create an integration in Notion to get an `ntn_...` token → store it via `everyday auth login --module bookmark` in the keyring → set that account to `provider = "notion"` in the config and fill in `parent_page_id` → `everyday bookmark init-db` to create the bookmark database and authorize the integration to access the parent page. Then `list` / `add` are ready to use.

### auth — credential lifecycle (NEW in v0.8.0)

Consolidated credential management for all modules. Modules read stored credentials internally via `auth::get_credential`; you only use these commands to manage credentials in the OS keyring. Password strategy (mail/cal) uses `--password`; Notion token strategy (note/todo/bookmark when `provider=notion`) uses `--token`. If the flag is omitted, it falls back to an interactive prompt. Passwords/tokens never touch disk.

| Command | Description | Usage |
|------|------|------|
| `login` | Store a credential in the OS keyring (optionally verify with `--verify`). `--module` required; `--account` defaults to the module's default account | `everyday auth login --module mail --account work --password PWD` |
| `logout` | Delete the stored credential from the keyring | `everyday auth logout --module mail --account work` |
| `verify` | Read the stored credential and verify it against the server (no re-prompt); reports `not_required` for local/sqlite or rss | `everyday auth verify --module note` |
| `list` | List configured accounts and their keyring state (stored / missing / not_required) | `everyday auth list --module todo` |

### timeline — unified event timeline (NEW in v0.5.0)

A single, append-only event log that aggregates events from **mail · cal · rss** plus the `ops-log` audit trail of Notion-backed `note` / `todo` / `bookmark` writes. Each source has a `TimelineProvider` adapter; sync is parallel across sources but serial within a source (rate-limit friendly). Storage is SQLite at `~/.config/everyday/timeline.db` (separate from the provider DBs).

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

# Notion todo / note / bookmark writes are visible via the ops-log provider,
# so deltas show up automatically after each `add` / `update` / `delete`.
everyday timeline today --source todo --json
```

### search — cross-module unified search (NEW in v0.7.0)

One query, all modules. A single `everyday search` call fans out concurrently to every registered `Searchable` provider (note / todo / bookmark / rss / cal / mail / memory), merges the hits into one time-ordered list, and renders them as Text or JSON. Empty results exit 0; per-module failures are surfaced as `SearchWarning` on stderr (text mode) or as a structured `{"_warning": ...}` line (`--json` mode) without aborting the whole query.

| Command | Description | Usage |
|------|------|------|
| `query` | Run a free-text query across every searchable module | `everyday search query "<q>" [--module a,b,c] [--since 7d] [--limit N] [--json]` |

**Module scope**: `note` / `todo` / `bookmark` (local SQLite, GLOB over title + content/url/tag), `rss` (a local item cache table at `~/.config/everyday/rss-items.db` populated by `rss digest` / `rss fetch`), `cal` (full-pull + in-memory GLOB over summary / location / start), `mail` (local envelope cache via [S007], since v0.9.0), `memory` (current-state view GLOB over subject/predicate/object, since v0.10.0). Notion-backed accounts are skipped in v1 (live-fetch-on-search was rejected for being slow / rate-limit prone).

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
- **Cal is window-refresh**: unlike the append-only mail / rss / ops-log providers, `cal` rewrites its window (`[last_sync, now+7d]`) so cancelled events actually disappear.
- **Notion via ops-log, not via Notion API**: respect the user privacy posture in `CONTEXT.md`; the agent never programmatically browses the Notion workspace — only AOP-recorded writes show up. Local providers, when used, still go through their own `TimelineProvider`.

See `docs/CONTEXT.md` + `docs/adr/0001`–`0009` for the full design rationale.

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

**Subject naming convention** (program does not enforce; documented in `skills/everyday-cli/SKILL.md`):

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

## Output Modes

### Text mode (default)

Great for direct terminal viewing; tables align automatically:

```
$ everyday mail list --unread --limit 3
uid    folder  date                          from              subject
-----------------------------------------------------------------------------
12345  INBOX   Wed, 8 Jul 2026 08:29 +0000  sender@x.com      Hello
12344  INBOX   Wed, 8 Jul 2026 07:15 +0000  boss@x.com        Weekly Report
12343  Drafts  Wed, 8 Jul 2026 06:00 +0000  me@x.com          Draft
```

### JSON mode (`--json`)

Outputs clean JSON with no extra whitespace, ideal for programmatic parsing:

```bash
$ everyday mail list --unread --limit 2 --json
[{"uid":"12345","folder":"INBOX","date":"Wed, 8 Jul 2026 08:29:31 +0000","from":"sender@x.com","subject":"Hello"},{"uid":"12344","folder":"INBOX","date":"Wed, 8 Jul 2026 07:15:00 +0000","from":"boss@x.com","subject":"Weekly Report"}]
```

### Error output

Error format in JSON mode:

```json
{"error": "AccountNotFound", "message": "mail account 'work'"}
```

Exit codes: `0` on success, `1` on failure.

## Configuration

Config file path: `~/.config/everyday/config.toml`

```toml
[default_account]
mail = "work"
calendar = "personal"
note = "personal"
bookmark = "personal"

[[mail.accounts]]
name = "work"
imap_host = "imap.example.com"
imap_port = 993          # default 993
smtp_host = "smtp.example.com"
smtp_port = 587          # default 587
username = "me@example.com"
tls = true               # default true

[[mail.accounts]]
name = "personal"
imap_host = "imap.gmail.com"
imap_port = 993
smtp_host = "smtp.gmail.com"
smtp_port = 587
username = "me@gmail.com"
tls = true

[[calendar.accounts]]
name = "personal"
caldav_url = "https://caldav.example.com/me"
username = "me"

[[rss.feeds]]
name = "hackernews"
url = "https://hnrss.org/frontpage"
category = "tech"

# Notes / to-dos default to the local SQLite provider — works out of the box, no credentials
[[note.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/notes.db"   # optional, defaults to ~/.config/everyday/note-personal.db

[[todo.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/todos.db"   # optional, defaults to ~/.config/everyday/todo-personal.db

[[bookmark.accounts]]
name = "personal"
provider = "local"
# db_path = "/absolute/path/to/bookmarks.db"   # optional, defaults to ~/.config/everyday/bookmark-personal.db

# For Notion: switch the account to provider = "notion" and configure it per each module's "prerequisites"
# [[note.accounts]]
# name = "notion"
# provider = "notion"
# default_database_id = "db_abc123..."   # use your real Notion ID
# default_page_id = "page_xyz789..."
# The Notion integration token (ntn_...) is NOT written here; store it via `everyday auth login --module note`
```

### Credential safety

Passwords are **never** stored in the config file; they are managed through the system keyring:

- **keyring service naming**: `everyday/<module>/<account>` (e.g. `everyday/mail/work`)
- **Store a credential**: `everyday auth login --module mail --account work` (interactive input; password stored in the keyring)
- **Read a credential**: the module reads it from the keyring automatically via `auth::get_credential` — no manual step needed

### Multiple accounts

Each module supports multiple named accounts:

- Defined via arrays such as `[[mail.accounts]]` in the config file
- `[default_account]` specifies the default account name per module
- `--account NAME` overrides the default

## Usage Examples

### Mail

```bash
# List all folders
everyday mail folders

# View the 10 most recent unread messages (JSON)
everyday mail list --unread --limit 10 --json

# Search messages in a specific folder
everyday mail search --query "invoice" --folder INBOX --json

# Read a message
everyday mail read 12345 --json

# Send a message
everyday mail send \
  --to recipient@example.com \
  --subject "Weekly report" \
  --body "Summary of this week's work..." \
  --cc manager@example.com

# Switch account
everyday mail list --account personal --json
```

### Config

```bash
# Initialize
everyday config init

# Show config
everyday config list

# Read an item
everyday config get mail.accounts.0.username

# Modify an item
everyday config set mail.accounts.0.smtp_port 465

# Verify
everyday config get mail.accounts.0.smtp_port
```

### Notes (local SQLite by default)

```bash
# The local provider needs no login; only provider = "notion" requires interactively storing a token (keyring only, never on disk)
everyday auth login --module note

# Search pages / databases (JSON)
everyday note search --query "work" --json

# List pages in a database (falls back to config default_database_id)
everyday note list --json
everyday note list --db "db_abc123" --limit 20

# Create a record in a database with multiple properties
everyday note create \
  --title "A Deep Dive into Rust Async Runtimes" \
  --prop "Type:Article" \
  --prop "Status:Unread" \
  --prop "URL:https://..."

# Read a page body (aggregated into Markdown)
everyday note read <page_id> --json

# Append a quick note to the default scratch page (page_id optional — auto-resolves default_page_id)
everyday note append --text "### Auto-captured by AI
Found a competitor link in message 12345: https://..."

# Append via pipe
echo "Batch-captured content" | everyday note append <page_id>

# Update page properties
everyday note update <page_id> --prop "Status:Read"
```

### To-dos (local SQLite by default)

```bash
# The local provider needs no login — just add / list (tables auto-created);
# only provider = "notion" requires this one-time setup: store the token, create the task database
everyday auth login --module todo
everyday todo init-db --parent "<page_id>"     # authorize the integration to access the parent page in Notion

# List unfinished tasks (by Due ascending)
everyday todo list --json

# All tasks (including completed)
everyday todo list --all --json

# Add a task
everyday todo add --title "Write weekly report" --due 2026-07-15 --priority P1

# Status transitions (returns the page id and url)
everyday todo start <page_id>
everyday todo complete <page_id>
```

### Bookmarks (local SQLite by default)

```bash
# The local provider needs no login — just add / list (tables auto-created);
# only provider = "notion" requires this one-time setup: store the token, create the bookmark database
everyday auth login --module bookmark
everyday bookmark init-db --parent "<page_id>"   # Notion only: authorize the integration to access the parent page

# Add a bookmark with tags
everyday bookmark add \
  --url "https://www.rust-lang.org" \
  --title "The Rust Programming Language" \
  --tags "rust,lang"

# List all bookmarks (JSON)
everyday bookmark list --json

# Filter by a single tag
everyday bookmark list --tag rust
```

## Project Structure

```
everyday/
├── src/
│   ├── main.rs          # Entry: parse → dispatch → render
│   ├── cli.rs           # clap command definitions
│   ├── config.rs        # Config loading & multi-account management
│   ├── error.rs         # Unified error type AgentError
│   ├── output.rs        # Output (Text/Json/Records rendering)
│   ├── notion_client.rs # Shared low-level Notion API client (HTTP/rate-limit/deserialization)
│   └── modules/
│       ├── mod.rs       # Executor trait + ModuleRegistry
│       ├── email.rs     # Email (IMAP/SMTP)
│       ├── calendar.rs  # Calendar (CalDAV)
│       ├── rss.rs       # RSS/Atom
│       ├── note.rs      # Notes & knowledge base (Notion API)
│       ├── todo.rs      # To-do tasks (Notion, based on notion_client)
│       ├── bookmark.rs  # Bookmarks (Notion, based on notion_client)
├── skills/
│   ├── README.md              # Concise project intro for Agent users
│   └── everyday-cli/
│       ├── SKILL.md           # Agent Skill entry (follows the agentskills.io spec)
│       └── references/
│           └── COMMANDS.md    # Full command reference (loaded on demand)
├── Cargo.toml
├── config.example.toml
└── agents.md            # AI Agent collaboration guidelines
```

## Development

### Tech stack

- **Language**: Rust (edition 2024)
- **Async runtime**: tokio
- **CLI parsing**: clap (derive)
- **Serialization**: serde + serde_json + toml
- **Email**: async-imap (IMAP) + lettre (SMTP) + mailparse
- **Credentials**: keyring (system keyring)
- **TLS**: rustls + webpki-roots

### Build

```bash
cargo build
cargo clippy -- -D warnings
cargo test
```

### Architecture

The core design is built around the `Executor` trait; the main program dispatches via trait objects, keeping modules decoupled:

```rust
#[async_trait]
pub trait Executor: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn actions(&self) -> Vec<ActionDoc>;
    async fn execute(&self, action: &str, args: &[String]) -> Result<Output>;
}
```

Adding a module only takes: create a file + implement the trait + register one line. See [`agents.md`](agents.md).

## Implementation Status

| Module | Status | Description |
|------|------|------|
| `config` | ✅ Fully available | path / list / get / set / init |
| `mail` | ✅ Fully available | IMAP receiving + SMTP sending + keyring credentials |
| `cal` | ✅ Fully available | CalDAV calendars / list / add / delete |
| `rss` | ✅ Fully available | follow / list / unfollow / digest / fetch |
| `note` | ✅ Fully available | search / list / create / read / append / update (local SQLite by default, optional Notion API) |
| `todo` | ✅ Fully available | init-db / list / add / start / complete (local SQLite by default, optional Notion API) |
| `bookmark` | ✅ Fully available | init-db / list / add (local SQLite by default, optional Notion API) |
| `auth` | ✅ Fully available (v0.8.0) | login / logout / verify / list — consolidated credential lifecycle for all modules |
| `timeline` | ✅ Fully available | unified event log: today / yesterday / week / month / sync |
| `search` | ✅ Fully available (NEW in v0.7.0) | cross-module unified search: query all modules in one shot |
| `memory` | ✅ Fully available (NEW in v0.10.0) | append-only `(subject, predicate, object)` triple notebook with confidence/source + graph + Searchable |

## License

MIT
