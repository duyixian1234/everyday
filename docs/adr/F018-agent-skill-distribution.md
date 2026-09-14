# ADR F018: Agent Skill Distribution — repo as the live source via link

**Status**: Accepted
**Date**: 2026-09-12

**Deciders**: Everyday Architecture Review (grill-with-docs session)

---

## Context

`skills/` holds the agent-facing documentation: `everyday-cli/SKILL.md` plus
`references/{COMMANDS,MEMORY,TASKS}.md`, and a `skills/README.md` entry point.
Agent hosts load these files directly, so they are a **runtime contract**, not
merely docs — an agent that reads a stale `COMMANDS.md` will issue commands that
no longer exist, or miss ones that do.

Three facts made the status quo untenable:

- **Unnoticed drift is the default.** `skills/README.md` carried a hand-maintained
  module table that had fallen **five releases behind** (it stopped at `sync`,
  v0.13.0, while the CLI had 15 modules). `COMMANDS.md` was missing its `daemon`
  row entirely. Nothing detected either.
- **No tooling guards the directory.** A survey confirmed zero `skill` references
  in the `Justfile`, both `.github/workflows/` files, `dist-workspace.toml`, and
  `scripts/check_doc_links.py`. `just check-links` validates relative links inside
  these files but asserts nothing about their *content*.
- **The installed copy was a hand-made copy, not a link.** On the reference
  machine, `~/.agents/skills/everyday-cli` was a real directory (a copy), with six
  further agent homes junctioning to *it*. So editing the repo changed nothing an
  agent actually reads: propagation required a manual re-copy that no tool
  performed and no doc described. The copy happened to match HEAD's *content*
  (differing only by CRLF), which is exactly why the failure was invisible — it
  was correct by luck, not by construction.

Scope was settled by evidence: `skills/` is **repo-only documentation**, shipped
in no release artifact (`dist-workspace.toml` produces binary archives only, and
`Cargo.toml` declares no `include`), and `skills/README.md` was never copied into
any skill directory — so restructuring it breaks no external consumer.

## Decision

Fix the content, remove the duplication that caused it, then make recurrence
structurally impossible.

### 1. `COMMANDS.md` is the single owner of the module inventory

`skills/README.md` no longer keeps a module table or any version numbers. It
points at `COMMANDS.md` for status and at the binary for the live surface
(`everyday --help`, `everyday <module> --help`, `everyday mcp tools --json`).

Duplicating one inventory across two skill files *is* what caused the five-release
drift; a second copy would re-rot on the next module.

### 2. The binary is the machine-readable source of truth

`everyday mcp tools --json` already projects every `(module, action)` pair with a
full JSON Schema, generated from `module_arg_spec` ([F014](F014-mcp-module.md)).
It is therefore a real contract diff target — no clap scraping needed.

### 3. Drift is checked in CI

`scripts/check_skill_drift.py` (via `just check-skill-drift`, wired into
`just ci`) asserts every projected `(module, action)` appears in `COMMANDS.md`,
failing with an explicit list of missing pairs.

`mcp` and `health` are excluded: `mcp` cannot project itself and `health` is a
root-level command, not a module. **All 14 other modules project tools** — the
exclusion list is exactly `{mcp, health}`, not a heuristic.

### 4. The repo is the live source: link, don't copy

`just skill-link` creates `<agent-skills-dir>/everyday-cli` as a junction
(Windows) or symlink (Unix) to `skills/everyday-cli` in this repo.

Because it is a link, edits take effect immediately in every agent session that
loads the skill, and **drift becomes structurally impossible**: there is only one
copy, and it is the one under version control.

## Alternatives considered

- **Refresh the module table in place (keep the duplication).** Rejected: it
  schedules the next occurrence of the same bug — the table was accurate when
  written and rotted anyway.
- **Copy-based install (`just skill-install`).** A one-command copy is safer than
  a link in one respect — it provides a **staging gate**, so a half-finished edit
  is not instantly live in every agent session. Rejected as the default because it
  keeps a second physical copy that can silently diverge; this is precisely the
  failure mode observed. The tradeoff is recorded here so the choice can be
  revisited, and `just skill-link` is idempotent and refuses to clobber a
  non-empty plain directory that is not already a link.
- **Leave the install undocumented local state.** Rejected: an undiscoverable
  machine-local junction means a fresh clone silently lacks the guarantee, while
  the docs claim it exists.
- **Generate the reference docs from clap definitions.** Strictly stronger — the
  tables could not drift at all. Rejected for now as a design project in its own
  right (authored prose vs generated output); `check_skill_drift.py` captures most
  of the value at a fraction of the cost.

## Consequences

- **`skills/` now participates in CI.** `just ci` gains a gate that requires a
  built `everyday` binary to enumerate the projected tools.
- **A stale doc is now a build failure**, not a silent agent misbehavior.
- **The link removes the staging gate.** After `just skill-link`, editing
  `skills/` is live in all agent sessions immediately, including a running one,
  and `git checkout` / branch switches mutate the live skill. This is the accepted
  cost of eliminating drift; the copy alternative above is the escape hatch.
- **The link is machine-local setup, not repo state.** F018 plus `just skill-link`
  make it reproducible and discoverable; the junction itself is untracked, so a
  fresh clone must run `just skill-link` once.
- `COMMANDS.md` remains the human-authored reference; `daemon` (F016) is now
  documented there alongside the other modules.
