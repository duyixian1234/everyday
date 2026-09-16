# ADR F019: GitHub Issues as the plan and progress tracker

**Status:** Accepted
**Date:** 2026-09-16

## Context

The project kept two hand-maintained planning documents at the repo root:

| File | Owned | Failure mode observed |
| --- | --- | --- |
| `task_plan.md` | Phase list, `Errors Encountered` table, key-design-decision summary, multi-account config draft | The "Current status" line became a single ~2000-character paragraph restating every release since v0.17.2; the file had grown to 193 lines of which the phase list alone dominated |
| `progress.md` | One-liner-per-release status, ADR chronological index, release log | Grew to 137 lines; the `当前状态` bullets were near-identical in shape to the release log below them, so every release was written **twice** — once as prose, once as a table row |

Both files were under a standing "keep it thin" rule ([01-workflow.md](../../.rules/01-workflow.md)
§"ADR extraction step"): after every task, decision-class prose must be pushed into an
ADR so the two files stay indexes. In practice the rule leaked in three ways:

1. **Duplication.** A release had to be described in `progress.md` §当前状态, again in
   `progress.md` §发版流水, and again in `task_plan.md` §当前状态. The three copies
   drifted (e.g. `task_plan.md` could only summarize releases back to v0.17.2 before
   truncating).
2. **No claimable unit of work.** "Mark the task as `in_progress` in `task_plan.md`"
   was the only stated mechanism for signalling who was working on what, and it was a
   prose edit in a file every agent edits — a merge-conflict hot spot with no
   assignee, no open/closed state, and no way to query "what is ready".
3. **Duplicated tracker.** The repo already ran a real issue tracker for exactly this
   content — specs with numbered `t1:`…`t6:` child tickets, triage labels, and closed
   history (#12–#31). Plan state for the same releases lived in GitHub *and* in the
   two Markdown files, and the two disagreed.

The forcing question: **what do the two files do that the issue tracker cannot, and
what do they do that it does better?**

## Decision

**Delete `progress.md` and `task_plan.md`. GitHub Issues (via `gh`) is the single
tracker for plan and progress.** Each piece of content they held moves to the
artifact that actually owns it:

| Was in | Now lives in | Why |
| --- | --- | --- |
| Phase list (`Phase 1`…`Phase 25`) | **GitHub milestones**, one per phase, plus closed issues per phase | A phase is a scheduling container with a completion state — a milestone is that object, and it can be queried (`gh issue list --milestone`) |
| Task claiming (`in_progress`) | **Issue assignee + open/closed state** | Native, conflict-free, and visible to every agent; a prose `in_progress` edit is neither |
| `Errors Encountered` table | [`.rules/07-dependency-pitfalls.md`](../../.rules/07-dependency-pitfalls.md) | A non-obvious crate/toolchain error whose fix is mechanical is a *pitfall*, which that file already owns; decision-class errors become ADRs |
| Key-design-decision summary table | The ADR it summarizes | The table restated ADR content; ADRs are the source of truth |
| Multi-account `config.toml` draft | Superseded by `config.example.toml` + [R022](./R022-config-editor.md) | The draft described a shape that shipped; keeping a stale sketch invited copy-paste of a wrong schema |
| Release log (`发版流水`) | [`docs/releases.md`](../releases.md) | The release runbook reads it, and `gh release` cannot carry the "primary ADR" column (GitHub Milestones could, but the runbook wants a file it can open in the diff) |
| ADR chronological index | [docs/adr/README.md](./README.md) §"ADR timeline" | The index is an ADR-catalog concern; it now sits next to the by-series index it complements |

### Scope boundaries — what goes where

This is the part future contributors actually need. **A new planning document at the
repo root is not allowed**; route the intent instead:

| Intent | Home | Mechanism |
| --- | --- | --- |
| "We should eventually do X" / a feature request | GitHub issue | `gh issue create`, label `needs-triage` |
| A multi-part feature awaiting implementation | **Spec issue** — one issue holding the design, with child `tN:` ticket issues | `gh issue create`; children linked per [issue-tracker.md](../../docs/agents/issue-tracker.md) |
| A unit of work an agent can claim | **Ticket issue**, label `ready-for-agent` | Claim with `gh issue edit <n> --add-assignee @me`; finish with `gh issue close` |
| A phase / release train | **GitHub milestone** | `gh api` milestone create; issues attach via `--milestone` |
| A design decision (constrains future code/interface/data) | **ADR** in `docs/adr/` | See [01-workflow.md](../../.rules/01-workflow.md) §"ADR extraction step" |
| A non-decisional convention, runbook, or crate quirk | **`.rules/*.md`** | Insert into the topic file; never a new root file |
| What a future reader needs to know *why* | ADR — **never** an issue body | Issues close and are archived; ADRs are permanent |

Rule of thumb: **issues are ephemeral and queryable (state, assignment, milestone);
ADRs and `.rules` are permanent and reviewed (why, invariants).** If content would
still be true after the issue closes, it does not belong in the issue.

### Workflow changes

- [`.rules/01-workflow.md`](../../.rules/01-workflow.md) §"Before a change": instead of
  "open `task_plan.md`, find the phase, mark it `in_progress`", the flow is now
  `gh issue list --state open --label ready-for-agent` → claim via assignee.
- §"During the change": a non-obvious error goes to `.rules/07-dependency-pitfalls.md`
  (mechanical) or a new ADR (decisional) — not to a plan file.
- §"Finishing a task": the checklist loses step 5 ("update `progress.md`") and gains
  closing the issue with the ADR id in the closing comment.
- The **ADR extraction step is unchanged in substance** — it still runs after every
  task. Only its output target changed: `progress.md`/`task_plan.md` prose is gone, so
  the step now verifies that decision-class content reached an ADR at all.
- [`.rules/05-commit.md`](../../.rules/05-commit.md) pre-commit checklist: the
  "`progress.md` timeline index updated" item becomes "ADR registered in
  `docs/adr/README.md`, and its timeline row added"; the release-commit item pointing
  at `progress.md` becomes `docs/releases.md`.

## Alternatives considered

- **Keep both files but enforce the thin-index rule harder.** The rule already existed
  and was already being violated in three directions. Adding a fourth enforcement
  mechanism addresses the symptom (bloat) and not the cause (duplicated state — a
  tracker that cannot be queried, assigned, or closed).
- **Delete the files and put *everything* in GitHub, including the release log.**
  Rejected: the release runbook ([01-workflow.md](../../.rules/01-workflow.md)
  §"Release (runbook summary)") reads and appends to the release log as part of the
  release commit, where a file in the diff is reviewed alongside `Cargo.toml`. GitHub
  Releases carry the artifacts, not the ADR linkage. `docs/releases.md` stays a file.
- **Backfill 34 closed issues, one per past release.** Rejected as noise: the releases
  are already represented by annotated tags, `CHANGELOG.md` sections, and GitHub
  Releases. `docs/releases.md` adds the one column GitHub lacks (primary ADR), with no
  34 extra notifications.
- **Model phases as labels rather than milestones.** Labels are flat and cannot carry a
  completion state or a due date; "which phase is open" would need a query convention
  instead of a native object. Milestones are the built-in fit.
- **Keep `task_plan.md`'s `Errors Encountered` table where it was.** It is precisely a
  pitfall list, and it duplicated entries that already existed in
  `.rules/07-dependency-pitfalls.md` under crate headings (the `lettre` feature-set
  item, for one). Two homes for the same one-liners is the same duplication problem.
- **Replace the ADR chronological index with a `gh` query over ADR-registering commits.**
  Rejected: the index is what a reader consults when they know roughly *when* something
  landed but not its series prefix; a git query cannot supply the one-line summary. It
  moved next to the by-series table in [docs/adr/README.md](./README.md) instead.

## Consequences

- **One tracker.** Plan state is queryable (`gh issue list --state open --label
  ready-for-agent`, `--milestone`), assignable, and has a real lifecycle. Two agents
  no longer race to edit the same `in_progress` line.
- **No triple-writing a release.** A release is described once in `docs/releases.md`
  (runbook) and once as release notes (`gh release`); the ADR timeline row in
  [docs/adr/README.md](./README.md) is the only in-repo summary.
- **Issues are off-repo state.** A clone without network access cannot see the plan.
  This is accepted: the *decisions* (ADRs) and the *release history* (`docs/releases.md`,
  `CHANGELOG.md`) are all in-tree, so a clone can still be fully understood without the
  tracker. Only "what is in flight right now" requires `gh`.
- **`just check-links` coverage shrinks slightly but correctly** — the removed files
  were large link hubs, so link rot now surfaces in fewer places; conversely, more
  cross-reference burden lands on [docs/adr/README.md](./README.md) and
  [`agents.md`](../../agents.md), which stay under the same gate.
- **Phase history is preserved in the milestone list, not in a file.** The 25 phases
  documented in the deleted `task_plan.md` are re-created as milestones with their
  completion state; the design decisions each phase produced are already ADRs.
- **Agents must be told.** Without an explicit pointer, an agent trained on this repo's
  older layout will re-create `task_plan.md`. [`agents.md`](../../agents.md) documents
  the tracker as the planning entry point, and this ADR is the citation for why the
  files are gone.
