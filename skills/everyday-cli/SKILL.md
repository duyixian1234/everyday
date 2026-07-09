---
name: everyday-cli
description: Operates the everyday local Rust CLI for agent automation — IMAP/SMTP email (list, read, search, send), CalDAV calendar (calendars, list, add, delete events), system status (CPU/memory/disk), and config management. Use when the user asks to check/read/send email, manage calendar events, monitor system resources, or run everyday commands. Always pass --json for machine-readable output.
license: MIT
---

# everyday CLI

`everyday` is a Rust CLI installed on the local machine. It gives an agent hands-on access to the user's machine: email, system status, and config. The binary is `everyday` (on PATH, or `target/release/everyday` after `cargo build --release`).

## Command structure

```
everyday <module> <action> [options] [--json] [--account NAME]
```

Modules: `mail` · `sys` · `config` · `fs` · `net` · `cal` · `rss`

## Rules (follow exactly)

1. **Always pass `--json`.** The agent parses structured output, never human tables. This is the primary mode for agent interaction.
   ```bash
   everyday mail list --unread --limit 10 --json
   ```
2. **Never put secrets in commands.** Passwords live in the OS keyring; never pass them as arguments or print them.
3. **Credentials live in the keyring, not the config file.** Config holds only account metadata. Keyring service name is `everyday/<module>/<account>` (e.g. `everyday/mail/work`).
4. **Skeleton modules return `NotImplemented`.** `fs`, `net`, and `rss` are not yet implemented — do not promise them. `mail`, `cal`, `sys status`, and `config` work today.

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

**System status:**
```bash
everyday sys status --json
# → [{"resource":"cpu","used":"12.3%","total":"100.0%","pct":"12.3%"}, ...]
```

**List today's calendar events:**
```bash
everyday cal list --json
# → [{"href":"/cal/ev.ics","start":"2026-07-09 15:00","end":"2026-07-09 16:00","summary":"meeting","location":""}]
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

## Error format

JSON mode errors:
```json
{"error": "AccountNotFound", "message": "mail account 'work'"}
```
Exit code is `1` on failure. Handle `NotImplemented` by telling the user the feature is pending; suggest an alternative if one exists.

## Full command reference

For the complete command tables, all options, and output schemas, read [references/COMMANDS.md](references/COMMANDS.md).
