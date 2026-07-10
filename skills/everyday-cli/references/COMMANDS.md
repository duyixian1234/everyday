# everyday CLI — Full Command Reference

Loaded on demand by the `everyday-cli` skill. Every command below supports the global flags `--json` (machine-readable output) and `--account <NAME>` (override the module's default account).

## Implementation status

| Module | Status | Notes |
|--------|--------|-------|
| `config` | ✅ Complete | path / list / get / set / init |
| `mail` | ✅ Complete | IMAP receive + SMTP send + keyring credentials |
| `cal` | ✅ Complete | CalDAV login / calendars / list / add / delete |
| `rss` | ✅ Complete | follow / list / unfollow / digest / fetch |
| `note` | ✅ Complete | Notion: login / search / list / create / read / append / update |

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
| `mail list` | List message summaries (recurses all folders by default, sorted by date desc) | `everyday mail list --unread --limit 10 --json` |
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

## rss — RSS/Atom subscriptions ⚠️ (skeleton)

| Command | Description | Status | Example |
|---------|-------------|--------|---------|
| `rss follow` | Add a feed | ⚠️ | `everyday rss follow --name N --url URL` |
| `rss list` | List feeds | ⚠️ | `everyday rss list` |
| `rss digest` | Aggregate recent items | ⚠️ | `everyday rss digest --limit 20` |

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

## Config file format

```toml
[default_account]
mail = "work"
calendar = "personal"
note = "personal"

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
```

**Keyring service-name convention:** `everyday/<module>/<account>` (e.g. `everyday/mail/work`, `everyday/note/personal`).

---

## Error types (JSON mode)

Exit code `0` on success, `1` on failure. Error envelope:

```json
{"error": "ErrorType", "message": "Details..."}
```

`ErrorType` values (PascalCase): `ConfigError` · `AccountNotFound` · `AuthError` · `NetworkError` · `IoError` · `ModuleNotFound` · `UnknownAction` · `InvalidArgument` · `PermissionDenied` · `NotImplemented` · `Other`
