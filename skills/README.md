# everyday — Getting Started for Agent Users

`everyday` is a local CLI toolkit written in Rust that acts as the "digital hands" of an AI Agent. It offers a unified command structure covering external-integration scenarios: email (IMAP/SMTP), calendar (CalDAV), RSS feeds, notes (local SQLite / optional Notion), to-dos (local SQLite / optional Notion), and configuration.

```
everyday <module> <action> [options] [--json] [--account NAME]
```

## Guidance for Agents

- **To run everyday commands**, load the **`everyday-cli`** skill (`everyday-cli/SKILL.md`). It contains trigger scenarios, must-follow rules, and common task examples.
- **The full command table, options, and output schema** live in `everyday-cli/references/COMMANDS.md`; read it on demand.
- **Always add `--json`** for interaction and process the structured data — an AI should not parse human-readable tables.
- **Credentials go through the system keyring** (`everyday/<module>/<account>`); passwords are never stored in the config file nor passed as command-line arguments.

## Module Status

| Module | Status |
|------|------|
| `config` · `mail` · `cal` · `rss` · `note` · `todo` | ✅ Available |

> This file is a concise intro for Agent users. The full human-readable documentation is in the repository root `README.md` (`README_ZH.md` for Chinese), and collaboration guidelines are in `agents.md`.

## Installing everyday

- **Prebuilt binaries** (Linux / macOS / Windows x86_64): [GitHub Releases](https://github.com/duyixian1234/everyday/releases), published automatically on every `v*` tag — download, extract, and add `everyday` to your `PATH`.
- **From source**: `cargo install --git https://github.com/duyixian1234/everyday.git`, or `git clone` then `cargo build --release`.
- Verify: `everyday --version`. Full install steps are in the repository root `README.md`.
