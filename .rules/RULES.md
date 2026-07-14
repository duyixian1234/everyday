# .rules/ — Project Conventions Index

> Agents and humans: this directory holds **non-decisional conventions** — the project's
> workflow, coding style, testing requirements, security red lines, commit rules,
> Justfile commands, and known dependency pitfalls.
>
> **Design decisions live in `../docs/adr/`**, not here. Every ADR has the canonical
> shape and is the source of truth for "why". When the two conflict, the ADR wins.
>
> Cross-references use relative paths: `[R001](../docs/adr/README.md#refactoring-patterns-r-series)`.

## Files

| File | Topic | Read when… |
| --- | --- | --- |
| [01-workflow.md](01-workflow.md) | Dev workflow + **ADR extraction step** | Starting a task, finishing a task, or running a release runbook |
| [02-coding-style.md](02-coding-style.md) | rustfmt + clippy + derives + naming + async | Writing or reviewing Rust code in this repo |
| [03-testing.md](03-testing.md) | Unit / integration / mock requirements | Adding tests or evaluating `cargo test` coverage |
| [04-security.md](04-security.md) | Red lines + keyring discipline | Touching credentials, network calls, or config files |
| [05-commit.md](05-commit.md) | Conventional Commits + pre-commit checklist | Making a commit, especially a release commit |
| [06-justfile.md](06-justfile.md) | `just` recipes + cross-platform shells | Running dev commands or adding a new recipe |
| [07-dependency-pitfalls.md](07-dependency-pitfalls.md) | Rust edition 2024 + crate-level gotchas | Adding a dependency or debugging a build error |
| [08-comments.md](08-comments.md) | Comment language + ADR-link depth rules | Translating comments or adding an ADR link from source code |

## How to use this directory

1. Before a coding change, read [agents.md](../agents.md) for project framing.
2. Skim the relevant `.rules/*.md` file(s) for the topic at hand (don't read them all).
3. If the work introduces or changes a **decision** (architecture, contract, long-lived
   constraint), open [docs/adr/README.md](../docs/adr/README.md) and add an ADR.
4. After finishing a task, follow [01-workflow.md §"Finishing a task"](01-workflow.md) —
   it includes the **ADR extraction step** that keeps `progress.md` and
   `task_plan.md` as thin index files (no execution traces; see [governance.md](../governance.md) §4).

## Link integrity

A `just check-links` (cross-platform shell + PowerShell) command verifies that every
markdown link across this repo resolves to an existing file or anchor. Run it before
pushing — see [06-justfile.md](06-justfile.md) for details.
