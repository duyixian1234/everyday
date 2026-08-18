# Everyday CLI

- [English](README.md) · [中文](README_ZH.md)

---


> The Rust-powered hands for your AI Agent.

**语言 / Language:** **English** · [简体中文](README_ZH.md)

`everyday` is a high-performance, memory-safe local CLI toolkit written in Rust. It acts as the "digital hands" of an AI Agent, offering a unified command structure that covers external-integration scenarios — email, calendar, RSS feeds, notes (local SQLite), to-dos (local SQLite), bookmarks (local SQLite), and a structured agent memory notebook — with dual Text / JSON output modes.

## What everyday is for

Everyday is a **personal information infrastructure for AI Agents**: a single, local-first command surface through which any agent (or human) can read, write and remember personal data across services, without handing credentials to any third party.

A complete agent task loop looks like this:

```bash
# 1. Agent aggregates the day across mail / calendar / RSS / todos
everyday timeline today --json

# 2. Agent answers questions across every module in one query
everyday search query "project roadmap" --json

# 3. Agent records long-term facts it learned about the user
everyday memory add user prefers rust --json

# 4. Agent turns decisions into tracked todos
everyday todo add --title "prepare demo" --priority P1 --json
```

Every command accepts `--json`, so an agent consumes structured output directly — this is an **agent-first CLI**, not a human-first CLI with a JSON escape hatch. Credentials live in the OS keyring (never on disk), data stays in local SQLite, and the whole binary cold-starts in under 100ms.

## Features

- **Unified command structure**: `everyday <module> <action> [options]`, low learning curve
- **Dual output modes**: Text by default (human-readable tables); `--json` switches to clean JSON (the primary mode for AI interaction)
- **Multi-account support**: each module supports multiple named accounts, switchable via `--account`
- **Credential safety**: passwords go through the system keyring (macOS Keychain / Windows Credential Manager / Linux Secret Service) and are never written to disk
- **Cross-platform**: Windows / macOS / Linux
- **High performance**: cold start < 100ms, async runtime (tokio), memory safe


## Installation

Install via the one-line script (fetches latest release), download a prebuilt
binary from [GitHub Releases](https://github.com/duyixian1234/everyday/releases),
build from source, or install via cargo:

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/duyixian1234/everyday/releases/latest/download/everyday-installer.sh | sh
```

```powershell
# Windows (PowerShell)
powershell -ExecutionPolicy Bypass -c "irm https://github.com/duyixian1234/everyday/releases/latest/download/everyday-installer.ps1 | iex"
```

```bash
cargo install --git https://github.com/duyixian1234/everyday.git

everyday --version   # verify
```

> Full per-platform steps, asset table and checksums:
> [docs/installation.md](docs/installation.md).

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


## Command Overview

| Module | Purpose | Entry |
|------|------|------|
| `config` | configuration management | `everyday config` |
| `mail` | email management | `everyday mail` |
| `cal` | calendar management (CalDAV) | `everyday cal` |
| `rss` | RSS / Atom feeds | `everyday rss` |
| `note` | notes & knowledge base | `everyday note` |
| `todo` | to-do tasks | `everyday todo` |
| `bookmark` | bookmarks | `everyday bookmark` |
| `auth` | credential lifecycle | `everyday auth` |
| `timeline` | unified event timeline | `everyday timeline` |
| `search` | cross-module unified search | `everyday search` |
| `memory` | structured agent notebook | `everyday memory` |
| `health` | module health check | `everyday health` |
| `sync` | WebDAV file sync | `everyday sync` |
| `mcp` | expose everyday as an MCP server | `everyday mcp` |
| `daemon` | resident auto-sync | `everyday daemon` |
| `task` | named no-shell commands, history and cron schedules | `everyday task` |

> Complete per-module tables, options and output modes:
> [docs/commands.md](docs/commands.md).

## Documentation

- [Installation](docs/installation.md) — install script, prebuilt binaries, source build
- [Command Reference](docs/commands.md) — every module's command tables + output modes
- [Configuration](docs/configuration.md) — config.toml, credential safety, multi-account
- [Usage Examples](docs/examples.md) — copy-paste recipes per module
- [Development](docs/development.md) — tech stack, build, architecture, implementation status
- [Daemon operations guide](docs/daemon.md) — install as a system service (nssm / launchd / systemd)
- [Design decisions](docs/adr/) — F/M/C/N/T/B/L/R series ADRs
- [Collaboration guide](agents.md) — contributor workflow

## License

MIT
