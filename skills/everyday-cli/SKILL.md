---
name: everyday-cli
description: Operates the everyday local Rust CLI for agent automation — IMAP/SMTP email (list, read, search, send), CalDAV calendar (calendars, list, add, delete events), RSS feeds (follow, list, digest), bookmarks (local SQLite by default / optional Notion, add, list, tag-filter), Notion note/knowledge-base and todo tasks (search, list, create, read, append, update, init-db, delete), unified event timeline (today, yesterday, week, month, sync), cross-module unified search (everyday search query "<q>" --module a,b,c --since 7d --limit N), credential lifecycle via the consolidated `auth` module (login / logout / verify / list), structured agent memory notebook (memory add / get / relation / list / delete / graph / history), and config management. Use when the user asks to check/read/send email, manage calendar events, read RSS digests, save bookmarks, capture notes/todos to Notion, persist structured facts to the agent's own memory, query an aggregated timeline of recent activity, search across all integrations in one shot, manage credentials, or run everyday commands. Always pass --json for machine-readable output.
license: MIT
---

# everyday CLI

`everyday` is a Rust CLI on the local machine that gives an agent hands-on access to the user's email (IMAP/SMTP), calendar (CalDAV), RSS feeds, bookmarks, Notion notes/todos, an aggregated activity timeline, and a structured agent-memory notebook. Binary: `everyday` (on PATH).

## Install

Prebuilt binaries for Linux / macOS / Windows on [GitHub Releases](https://github.com/duyixian1234/everyday/releases) (published for every `v*` tag); or from source:

```bash
cargo install --git https://github.com/duyixian1234/everyday.git
```

Verify with `everyday --version`. Full per-platform steps: repo root [README.md](../../../README.md).

## Command structure

```
everyday <module> <action> [options] [--json] [--account NAME]
```

Modules: `mail` · `cal` · `rss` · `bookmark` · `note` · `todo` · `timeline` · `memory` · `search` · `config` (+ root-level `health`)

## Rules (follow exactly)

1. **Always pass `--json`.** The agent parses structured output, never human tables.
2. **Never put secrets in commands.** Passwords live in the OS keyring; never pass them as arguments or print them.
3. **Credentials live in the keyring, not the config file.** Config holds only account metadata. Keyring service name is `everyday/<module>/<account>` (e.g. `everyday/mail/work`).
4. **Verify per action before assuming.** Modules are implemented but feature sets differ — the exact actions, options, and output schemas are in [references/COMMANDS.md](references/COMMANDS.md).
5. **`everyday health --json`** runs a local-only health probe of every module (cache DB openable, keyring credential present — never network). Exit code 1 + `"healthy": false` rows identify degraded modules. Use it to diagnose before deeper debugging. Dispatch logs (`_log` lines, request ids) go to stderr and are safe to ignore unless debugging.
6. **`timeline today --json` is the aggregated activity snapshot.** Prefer it over per-module polling unless the user explicitly asks for a specific module.
7. **`memory` is the agent's own structured notebook.** Use `everyday memory add` to persist stable facts about the user, projects, or the world; use `memory get <SUBJECT>` to recall them. Subject naming is a convention, not enforced — see [references/MEMORY.md](references/MEMORY.md). Memory facts automatically participate in `everyday search`.

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

After this, `mail` commands work without re-entering credentials. Notion-backed modules (`note` / `todo` / `bookmark`) have their own setup — see [references/TASKS.md](references/TASKS.md).

## Common tasks

Everyday recipes — read/send mail, calendar events, notes, todos, timeline queries — are in [references/TASKS.md](references/TASKS.md).

## Memory notebook

`memory` semantics, subject naming convention, and what belongs in memory vs timeline: [references/MEMORY.md](references/MEMORY.md).

## Error format

JSON mode errors:

```json
{ "error": "AccountNotFound", "message": "mail account 'work'" }
```

Exit code is `1` on failure. Handle `NotImplemented` by telling the user the feature is pending; suggest an alternative if one exists.

## Full command reference

For the complete command tables, all options, and output schemas, read [references/COMMANDS.md](references/COMMANDS.md).
