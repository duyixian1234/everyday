# everyday CLI — Common Tasks

Everyday recipes for the most common agent workflows, grouped by module. Loaded on demand by the `everyday-cli` skill when a concrete task needs a runnable command. Full command tables, all options, and output schemas: [COMMANDS.md](COMMANDS.md).

## Quick reference

| Task | Command |
|------|---------|
| Unread mail | `everyday mail list --unread --limit 10 --json` |
| Read a message | `everyday mail read <uid> --json` |
| Search mail | `everyday mail search --query "invoice" --json` |
| Send mail | `everyday mail send --to a@b.com --subject "Hi" --body "内容"` |
| Calendar events | `everyday cal list --json` |
| Add calendar event | `everyday cal add --title "会议" --start "2026-07-09T15:00:00Z" --end "2026-07-09T16:00:00Z"` |
| Search notes | `everyday note search --query "工作" --json` |
| Create a note record | `everyday note create --title "..." --prop "状态:未读"` |
| List todos | `everyday todo list --json` |
| Add a todo | `everyday todo add --title "写周报" --due 2026-07-15 --priority P1` |
| Recent activity snapshot | `everyday timeline today --json` |

## Mail

**Read unread mail (JSON):**

```bash
everyday mail list --unread --limit 10 --json
# → [{"uid":12345,"unread":true,"folder":"INBOX","date":"...","from":"...","subject":"..."}]
# (uid is a JSON number, unread a JSON boolean — F012 P6 typed records)
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

## Calendar

**List calendar events** (today & future by default; `--all` for all):

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

## Notes (Notion)

**Search / list pages (JSON):**

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

**Read a page as Markdown** (JSON returns aggregated `{id,title,url,properties,content}`):

```bash
everyday note read <page_id> --json
```

**Append a flash note** (text arg, or pipe via stdin):

```bash
everyday note append --text "### AI 自动捕获
发现竞品链接：https://..."
echo "批量捕获内容" | everyday note append <page_id>
```

**Update page properties:**

```bash
everyday note update <page_id> --prop "状态:已读"
```

**First-time Notion setup:** store the `ntn_...` integration token via `everyday auth login --module note` (service `everyday/note/<account>`). The target page/database must be shared with the integration in Notion. `--db` / page id default to `default_database_id` / `default_page_id` from config when omitted.

## Todos (Notion task database)

Built on the shared `notion-client`. **First-time setup:** store the token via `everyday auth login --module todo` (service `everyday/todo/<account>`), add `[[todo.accounts]]` with `parent_page_id`, run `everyday todo init-db` (the integration must be granted access to the parent page). `--db` defaults to the `default_database_id` written by `init-db`.

```bash
everyday todo init-db --parent "<page_id>"     # create the task database; writes database_id back to config
everyday todo list --json                      # incomplete todos, sorted by due
everyday todo list --all --json                # include Done
everyday todo add --title "写周报" --due 2026-07-15 --priority P1
everyday todo start <page_id>                  # → In Progress
everyday todo complete <page_id>               # → Done
everyday todo delete <page_id>                 # archive (Notion) / physical delete (local)
```

## Timeline (unified activity log)

Aggregates mail + cal + rss + Notion writes (`note` / `todo` / `bookmark`).

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
