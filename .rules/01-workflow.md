# 01-workflow.md — Everyday Development Workflow

> This is the project's development discipline. It governs how work enters, moves
> through, and exits a task — including the **ADR extraction step** that keeps the
> docs (`progress.md`, `findings.md`) as thin indexes.

## Before a change

1. Open [agents.md](../agents.md). Skim the "Scope and positioning" section and the
   `.rules/` index. Read only the topic-relevant rule files (don't read them all).
2. Open [task_plan.md](../task_plan.md). Confirm the current phase and find the
   task you are picking up.
3. Mark the task as `in_progress` in `task_plan.md`.
4. Read the relevant source files — don't guess from memory. Modules follow the
   pattern in [02-coding-style.md](02-coding-style.md); their contracts are listed
   in [docs/adr/README.md](../docs/adr/README.md).

## During the change

- After every 2 web fetches / searches, write a short note to
  [findings.md](../findings.md) **only if** the note is a **decision-class** fact
  (i.e. something that will become an ADR). Otherwise discard it.
- If you hit a non-obvious error, append a one-liner to "Errors Encountered" in
  [task_plan.md](../task_plan.md). Don't accumulate them in `findings.md`.
- Do not commit mid-task. One task = one commit (see [05-commit.md](05-commit.md)).

## Finishing a task

A "task" is one of:

- A complete feature (e.g. `mail login` works end-to-end).
- A whole module's skeleton or its core actions.
- One phase of [task_plan.md](../task_plan.md).
- A tightly coupled small bundle (a bug + its test).

After the code works, run this checklist **in order**:

1. **Quality gates.** Run `just ci` (or each of `just check`, `just test`,
   `just build`). All green.
2. **Doc discipline.** Run `just check-links` (see
   [06-justfile.md](06-justfile.md)). Fix any broken link.
3. **ADR extraction step** — this is what keeps `progress.md` / `findings.md` thin.
   See the [next section](#adr-extraction-step).
4. **Commit.** Conventional Commit (see [05-commit.md](05-commit.md)). Message format:
   `<type>(<scope>): <subject>` where `type` ∈ `feat` / `fix` / `refactor` / `test` /
   `docs` / `chore`.
5. **Update [progress.md](../progress.md)** with the new ADR id under "ADR timeline".

### ADR extraction step

This step runs **after every task**, not just at release time. It is the only thing
keeping `findings.md` from bloating into a second `agents.md`.

**Goal.** Decide what is a real decision worth promoting to an ADR, file it
(`docs/adr/XXX-...md`), register it in [`docs/adr/README.md`](../docs/adr/README.md),
and remove the prose from `findings.md` / `progress.md` — replacing it with an ADR
link.

**What counts as a decision (migrate to ADR)?** A choice that:

- Constrains future code organization, public interface, or data model.
- A future reader would be surprised by reverse-rolling it.
- Establishes a long-lived invariant, security boundary, or performance budget.
- Supersedes a prior decision (link both ADRs).

Examples that belong in an ADR: "calendar uses window-refresh, not append",
"provider default is local SQLite for note/todo/bookmark", "panic-free
`PoolGuard::Drop`", "config goes through the Executor trait".

**What does not count (do NOT make an ADR for this):**

- A typo, rename, or formatting tweak.
- A single bug fix whose fix is self-evident from the test.
- A one-crate API quirk (e.g. "this crate renamed its function in v0.x"). These go
  into [07-dependency-pitfalls.md](07-dependency-pitfalls.md) as a one-liner.
- A refactor that produces a reusable macro/pattern: file a **R-series** ADR.

**Procedure:**

1. Re-read the diff and your own scratch notes from this task.
2. If a real decision exists, decide its series and next number:
   - `F` cross-cutting, `M` mail, `C` calendar, `N` note, `T` todo, `B` bookmark,
     `L` timeline, `R` refactoring patterns.
3. Create `docs/adr/<ID>-<kebab-title>.md` with the canonical ADR shape: Context,
   Decision, Alternatives considered, Consequences. Cross-link existing relevant
   ADRs.
4. Add a row to the relevant table in
   [docs/adr/README.md](../docs/adr/README.md) under its series section.
5. Replace the prose in [findings.md](../findings.md) /
   [progress.md](../progress.md) with `[id](docs/adr/<id>-...).md` links. If both
   files mention the same decision, leave only the canonical reference in one and
   point to it from the other.
6. Re-run `just check-links` to confirm.

**Caveat — release-time only:** if the task is itself the **release commit**,
[findings.md](../findings.md) should be re-read once more end-to-end. Any leftover
prose that still smuggles a decision into the body of the document is migrated now.

## Release (runbook summary)

Full release runbook lives in the project long-term memory
(`everyday/.workbuddy/memory/MEMORY.md` under "Release Runbook"). Summary:

1. `chore: release vX.Y.Z` commit bumping `Cargo.toml` + document version refs.
2. Annotated tag: `git tag -a vX.Y.Z -m "vX.Y.Z: <highlights>"`.
3. Push: `git push origin master && git push origin vX.Y.Z`. **Never `cnb`** —
   see [F006](../docs/adr/F006-ci-release-github-only.md).
4. `gh run watch <run-id> --exit-status` until release workflow completes
   (~6 min).

## Doc discipline summary

| Doc | Owns |
| --- | --- |
| [agents.md](../agents.md) | Project framing, core stack, index into the rest |
| `.rules/*.md` | Conventions, runbooks, gotchas (non-decisional) |
| [docs/adr/](../docs/adr/README.md) | Every design decision (F/M/C/N/T/B/L/R series) |
| [CONTEXT.md](../CONTEXT.md) | Domain glossary for each module (terms only) |
| [task_plan.md](../task_plan.md) | Phases, errors encountered, design-decision summary table |
| [progress.md](../progress.md) | Current status + ADR timeline index |
| [findings.md](../findings.md) | Pure ADR index (by topic) — no prose |
| [README.md](../README.md) / [README_ZH.md](../README_ZH.md) | End-user docs |
| [skills/](../skills/) | Agent-facing skill files |
