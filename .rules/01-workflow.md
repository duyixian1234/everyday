# 01-workflow.md — Everyday Development Workflow

> This is the project's development discipline. It governs how work enters, moves
> through, and exits a task — including the **ADR extraction step** that keeps
> decisions in `docs/adr/` and conventions in `.rules/`.
>
> **Planning and progress live in GitHub Issues**, not in the repository. See
> [F019](../docs/adr/F019-github-issues-as-plan-tracker.md) for why `progress.md` and
> `task_plan.md` were removed, and [docs/agents/issue-tracker.md](../docs/agents/issue-tracker.md)
> for the `gh` commands.

## Before a change

1. Open [agents.md](../agents.md). Skim the "Scope and positioning" section and the
   `.rules/` index. Read only the topic-relevant rule files (don't read them all).
2. **Find the work in the tracker.** List what is claimable:
   `gh issue list --state open --label ready-for-agent`. Read the one you are picking
   up with `gh issue view <n> --comments`. A multi-part feature has a **spec issue**
   whose children are numbered `t1:`…`tN:` tickets — work the ticket, not the spec.
3. **Claim it** — `gh issue edit <n> --add-assignee @me`. This is the replacement for
   the old "mark the task `in_progress` in `task_plan.md`" step.
4. Read the relevant source files — don't guess from memory. Modules follow the
   pattern in [02-coding-style.md](02-coding-style.md); their contracts are listed
   in [docs/adr/README.md](../docs/adr/README.md).

If the work is not in the tracker yet, create the issue first
(`gh issue create --label needs-triage`) rather than starting unplanned work.

## During the change

- After every 2 web fetches / searches, write a short note to your scratch
  buffer **only if** the note is a **decision-class** fact (i.e. something that
  will become an ADR). Otherwise discard it. Decision-class notes migrate to
  an ADR per §"ADR extraction step" below — there is no separate findings file.
- If you hit a non-obvious error, record it as a one-liner in
  [07-dependency-pitfalls.md](07-dependency-pitfalls.md) under the relevant crate or
  the "Platform and toolchain traps" section. If it is a *decision* rather than a
  mechanical fix, it becomes an ADR instead. Don't accumulate either in an issue
  comment or a scratch file.
- Do not commit mid-task. One task = one commit (see [05-commit.md](05-commit.md)).

## Finishing a task

A "task" is one of:

- A complete feature (e.g. `mail login` works end-to-end).
- A whole module's skeleton or its core actions.
- One tracker issue (typically a `tN:` ticket under a spec).
- A tightly coupled small bundle (a bug + its test).

After the code works, run this checklist **in order**:

1. **Quality gates.** Run `just ci` (or each of `just check`, `just test`,
   `just build`). All green.
2. **Doc discipline.** Run `just check-links` (see
   [06-justfile.md](06-justfile.md)). Fix any broken link.
3. **ADR extraction step** — this is what keeps decision-class content out of the
   tracker and in the ADR catalog. See the [next section](#adr-extraction-step).
4. **Commit.** Conventional Commit (see [05-commit.md](05-commit.md)). Message format:
   `<type>(<scope>): <subject>` where `type` ∈ `feat` / `fix` / `refactor` / `test` /
   `docs` / `chore`.
5. **Close the issue** — `gh issue close <n> --comment "..."`, naming the ADR id(s) if
   the task produced any. If the change ships in a release, add the release row to
   [docs/releases.md](../docs/releases.md) as part of the release commit (step below).

### ADR extraction step

This step runs **after every task**, not just at release time. It is the only thing
keeping decision-class content in the ADR catalog instead of drifting into issue
bodies, `.rules/`, or a new planning file.

**Goal.** Decide what is a real decision worth promoting to an ADR, file it
(`docs/adr/XXX-...md`), register it in [`docs/adr/README.md`](../docs/adr/README.md),
and make sure nothing else restates it in prose.

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
4. Add **two** rows to [docs/adr/README.md](../docs/adr/README.md): one in the
   relevant series table, and one in the **ADR timeline** table at the bottom.
5. Delete any prose elsewhere that restates the decision — an issue body, a
   `.rules/*.md` paragraph, or a duplicate summary table — and leave an
   `[id](docs/adr/<id>-...).md` link instead. The ADR is the only place the reasoning
   lives; everything else points at it.
6. Re-run `just check-links` to confirm.

**Caveat — release-time only:** if the task is itself the **release commit**, re-read
[docs/releases.md](../docs/releases.md) once more end-to-end. Any leftover prose that
still smuggles a decision into a release row is migrated now — a release row gets a
summary and an ADR link, nothing more.

## Tracker discipline

Planning state is off-repo, so these rules keep it usable:

- **One issue = one unit of work.** Don't stack unrelated changes onto an open issue;
  open a second one.
- **Close with context.** `gh issue close <n> --comment "<what shipped> (<ADR id, if
  any>)"` — the closing comment is the only progress trace, so make it name the
  artifact.
- **Issues are not permanent.** If a fact would still be true after the issue closes
  (an invariant, a contract, a decision), it belongs in an ADR or `.rules/` — write it
  there and link it from the closing comment.
- **Never add a root planning file.** `progress.md` / `task_plan.md` are deliberately
  gone ([F019](../docs/adr/F019-github-issues-as-plan-tracker.md)). Long-form plan
  content goes in the issue or the ADR, never in a new Markdown file at the repo root.

## Release (runbook summary)

Full release runbook lives in the project long-term memory
(`everyday/.workbuddy/memory/MEMORY.md` under "Release Runbook"). Summary:

The release pipeline is managed by **cargo-dist** (see
[G001](../docs/adr/G001-quality-tools-suite.md)): `.github/workflows/release.yml`
is machine-generated by `dist generate` from `dist-workspace.toml` — **never
hand-edit it**. To change targets/installers/attestations, edit
`dist-workspace.toml` and run `dist generate`.

1. `chore: release vX.Y.Z` commit bumping `Cargo.toml` + document version refs.
2. Append this release's changelog: `git cliff --unreleased --tag vX.Y.Z --prepend
   CHANGELOG.md`, and include it in the release commit (git-cliff renders only;
   version bumps stay manual + ADR-governed). **Do not regenerate wholesale with
   `git cliff -o CHANGELOG.md`** — older sections carry hand-curated edits (e.g.
   v0.18.1's `*(mail)* … (#32, fixes #31)` does not match the raw commit subject),
   and a full render silently drops them.
3. Prepend the release row to [docs/releases.md](../docs/releases.md) — version, tag,
   one-paragraph summary, primary ADR link(s).
4. Annotated tag: `git tag -a vX.Y.Z -m "vX.Y.Z: <highlights>"`.
5. Push: `git push origin master && git push origin vX.Y.Z`. **Never `cnb`** —
   see [F006](../docs/adr/F006-ci-release-github-only.md).
6. `gh run watch <run-id> --exit-status` until the dist workflow completes
   (plan → build-local → build-global → host; artifacts get Sigstore
   attestations automatically).

## Doc discipline summary

| Doc | Owns |
| --- | --- |
| [agents.md](../agents.md) | Project framing, core stack, index into the rest |
| `.rules/*.md` | Conventions, runbooks, gotchas (non-decisional) |
| [docs/adr/](../docs/adr/README.md) | Every design decision (F/M/C/N/T/B/L/R/S/K/D/G series) + the chronological ADR timeline |
| [CONTEXT.md](../CONTEXT.md) | Domain glossary for each module (terms only) |
| [docs/releases.md](../docs/releases.md) | Release log — one row per release + primary ADR |
| [docs/agents/](../docs/agents/issue-tracker.md) | Tracker conventions (issue tracker, triage labels, domain docs) |
| **GitHub Issues** | Current plan, in-flight work, phase milestones ([F019](../docs/adr/F019-github-issues-as-plan-tracker.md)) |
| [README.md](../README.md) / [README_ZH.md](../README_ZH.md) | End-user docs |
| [skills/](../skills/) | Agent-facing skill files |
