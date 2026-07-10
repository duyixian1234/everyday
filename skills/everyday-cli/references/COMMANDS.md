# everyday CLI — Full Command Reference

Loaded on demand by the `everyday-cli` skill. Every command below supports the global flags `--json` (machine-readable output) and `--account <NAME>` (override the module's default account).

## Implementation status

| Module | Status | Notes |
|--------|--------|-------|
| `config` | ✅ Complete | path / list / get / set / init |
| `mail` | ✅ Complete | IMAP receive + SMTP send + keyring credentials |
| `cal` | ✅ Complete | CalDAV login / calendars / list / add / delete |
| `rss` | ✅ Complete | follow / list / unfollow / digest / fetch |

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

## Config file format

```toml
[default_account]
mail = "work"
calendar = "personal"

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
```

**Keyring service-name convention:** `everyday/<module>/<account>` (e.g. `everyday/mail/work`).

---

## Error types (JSON mode)

Exit code `0` on success, `1` on failure. Error envelope:

```json
{"error": "ErrorType", "message": "Details..."}
```

`ErrorType` values (PascalCase): `ConfigError` · `AccountNotFound` · `AuthError` · `NetworkError` · `IoError` · `ModuleNotFound` · `UnknownAction` · `InvalidArgument` · `PermissionDenied` · `NotImplemented` · `Other`
