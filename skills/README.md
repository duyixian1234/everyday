# everyday — Getting Started for Agent Users

`everyday` is a local CLI toolkit written in Rust that acts as the "digital hands" of an AI Agent. It offers a unified command structure covering external-integration scenarios: email (IMAP/SMTP), calendar (CalDAV), RSS feeds, notes / to-dos / bookmarks (local SQLite), an aggregated **timeline** event log that unifies them, cross-module **search**, an agent **memory** notebook, **auth** credential lifecycle, **sync** via WebDAV, named no-shell **task** execution with cron scheduling, a resident **daemon**, and an **mcp** stdio server.

```
everyday <module> <action> [options] [--json] [--account NAME]
```

## Guidance for Agents

- **To run everyday commands**, load the **`everyday-cli`** skill (`everyday-cli/SKILL.md`). It contains trigger scenarios and must-follow rules; per-module recipes live in `everyday-cli/references/TASKS.md`.
- **The full command table, options, and output schema** live in `everyday-cli/references/COMMANDS.md`; read it on demand.
- **Always add `--json`** for interaction and process the structured data — an AI should not parse human-readable tables.
- **Credentials go through the system keyring** (`everyday/<module>/<account>`); passwords are never stored in the config file nor passed as command-line arguments.
- **For "what happened recently across everything?"**: run `everyday timeline today --json` — it is one of the cheapest ways to get an aggregated activity snapshot across all integrations.

## Module Status

The authoritative module list, per-module status, actions, options, and output
schemas live in **[`everyday-cli/references/COMMANDS.md`](everyday-cli/references/COMMANDS.md)**.

For the live surface on a given machine, ask the binary — it is the source of
truth and cannot go stale:

```bash
everyday --help              # all modules
everyday <module> --help     # actions for one module
everyday mcp tools --json    # every (module, action) with JSON Schema
```

This file deliberately keeps **no** module inventory and **no** version numbers:
a hand-maintained copy of the module table previously drifted five releases
behind. See [F018](../docs/adr/F018-agent-skill-distribution.md).

## Installing the skill files (for agent hosts)

The skill files are repo-only documentation — they ship in no release artifact
and no packaging step installs them. To make the live skill track this working
tree, link the repo copy into your agent skills directory instead of copying it:

```bash
just skill-link
```

That creates `<agent-skills-dir>/everyday-cli` as a junction / symlink to
`skills/everyday-cli` in this repo. Because it is a link and not a copy, edits
here take effect immediately in every agent session and drift becomes
structurally impossible. A plain copy would work too, but it goes stale silently
— see [F018](../docs/adr/F018-agent-skill-distribution.md) for the tradeoff.

> This file is a concise intro for Agent users. The full human-readable documentation is in the repository root `README.md` (`README_ZH.md` for Chinese), and collaboration guidelines are in `agents.md`.

## Installing everyday

- **Prebuilt binaries** (Linux / macOS / Windows x86_64 + macOS aarch64): [GitHub Releases](https://github.com/duyixian1234/everyday/releases), published automatically on every `v*` tag — download, extract, and add `everyday` to your `PATH`.
- **From source**: `cargo install --git https://github.com/duyixian1234/everyday.git`, or `git clone` then `cargo build --release`.
- Verify: `everyday --version`. Full install steps are in the repository root `README.md`.
