---
name: everyday-cli
description: Operates the everyday local Rust CLI for agent automation — IMAP/SMTP email (list, read, search, send), CalDAV calendar (calendars, list, add, delete events), RSS feeds (follow, list, digest), Notion note/knowledge-base (search, list, create, read, append, update, login), and config management. Use when the user asks to check/read/send email, manage calendar events, read RSS digests, capture notes to Notion, or run everyday commands. Always pass --json for machine-readable output.
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

Modules: `mail` · `cal` · `rss` · `note` · `config`

## Rules (follow exactly)

1. **Always pass `--json`.** The agent parses structured output, never human tables. This is the primary mode for agent interaction.
   ```bash
   everyday mail list --unread --limit 10 --json
   ```
2. **Never put secrets in commands.** Passwords live in the OS keyring; never pass them as arguments or print them.
3. **Credentials live in the keyring, not the config file.** Config holds only account metadata. Keyring service name is `everyday/<module>/<account>` (e.g. `everyday/mail/work`).
4. **Modules.** `mail` (IMAP/SMTP), `cal` (CalDAV), `rss` (feeds), `note` (Notion), and `config` are implemented — verify per action. Always pass `--json` for machine-readable output.

## First-time setup (only if config is missing)

```bash
everyday config init
everyday config set mail.accounts.0.name work
everyday config set mail.accounts.0.imap_host imap.example.com
everyday config set mail.accounts.0.smtp_host smtp.example.com
everyday config set mail.accounts.0.username me@example.com
everyday config set default_account.mail work
everyday mail login --account work   # prompts for password, saved to keyring
```

After this, `mail` commands work without re-entering credentials.

## Common tasks

**Read unread mail (JSON):**

```bash
everyday mail list --unread --limit 10 --json
# → [{"uid":"12345","folder":"INBOX","date":"...","from":"...","subject":"..."}]
```

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

First-time Notion setup: `everyday note login` (stores the `ntn_...` integration token in the OS keyring, service `everyday/note/<account>`). The target page/database must be shared with the integration in Notion. `--db` / page id default to `default_database_id` / `default_page_id` from config when omitted.

## Error format

JSON mode errors:

```json
{ "error": "AccountNotFound", "message": "mail account 'work'" }
```

Exit code is `1` on failure. Handle `NotImplemented` by telling the user the feature is pending; suggest an alternative if one exists.

## Full command reference

For the complete command tables, all options, and output schemas, read [references/COMMANDS.md](references/COMMANDS.md).
