# Development

Project structure, tech stack, build, architecture, and implementation status.
This is the complete version of the README "Project Structure", "Development"
and "Implementation Status" sections.

- [English](development.md) · [中文](development_zh.md)

---

## Project Structure

```
everyday/
├── src/
│   ├── main.rs          # Entry: parse → dispatch → render
│   ├── cli.rs           # clap command definitions
│   ├── config.rs        # Config loading & multi-account management
│   ├── error.rs         # Unified error type AgentError
│   ├── output.rs        # Output (Text/Json/Records rendering)
│   └── modules/
│       ├── mod.rs       # Executor trait + ModuleRegistry
│       ├── email.rs     # Email (IMAP/SMTP)
│       ├── calendar.rs  # Calendar (CalDAV)
│       ├── rss.rs       # RSS/Atom
│       ├── note.rs      # Notes & knowledge base (local SQLite)
│       ├── todo.rs      # To-do tasks (local SQLite)
│       ├── bookmark.rs  # Bookmarks (local SQLite)
├── skills/
│   ├── README.md              # Concise project intro for Agent users
│   └── everyday-cli/
│       ├── SKILL.md           # Agent Skill entry (follows the agentskills.io spec)
│       └── references/
│           ├── COMMANDS.md    # Full command reference (loaded on demand)
│           ├── TASKS.md       # Common task recipes (loaded on demand)
│           └── MEMORY.md      # Memory notebook semantics & naming conventions
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
    async fn execute(&self, action: &str, args: &[String], ctx: &RequestContext) -> Result<Output>;
}
```

Adding a module only takes: create a file + implement the trait + register one line. See [`agents.md`](../agents.md).

## Implementation Status

| Module | Status | Description |
|------|------|------|
| `config` | ✅ Fully available | path / list / get / set / init |
| `mail` | ✅ Fully available | IMAP receiving + SMTP sending + keyring credentials |
| `cal` | ✅ Fully available | CalDAV calendars / list / add / delete |
| `rss` | ✅ Fully available | follow / list / unfollow / digest / fetch |
| `note` | ✅ Fully available | search / list / create / read / append / update (local SQLite) |
| `todo` | ✅ Fully available | list / add / start / complete (local SQLite) |
| `bookmark` | ✅ Fully available | list / add (local SQLite) |
| `auth` | ✅ Fully available (v0.8.0) | login / logout / verify / list — consolidated credential lifecycle for all modules |
| `timeline` | ✅ Fully available | unified event log: today / yesterday / week / month / sync |
| `search` | ✅ Fully available (NEW in v0.7.0) | cross-module unified search: query all modules in one shot |
| `memory` | ✅ Fully available (NEW in v0.10.0) | append-only `(subject, predicate, object)` triple notebook with confidence/source + graph + Searchable |
| `health` | ✅ Fully available (NEW in v0.11.0) | root-level ops command: every module's local-only health check, exit 0/1 |
| `sync` | ✅ Fully available (NEW in v0.13.0) | bidirectional WebDAV file sync: 4 user DBs + config.toml, LWW conflicts with dual copies, `--push-only` / `--pull-only` / `--force`, opt-in auto_sync |
