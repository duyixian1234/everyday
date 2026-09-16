# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues. Use the `gh` CLI for all operations.

**This tracker is the project's plan and progress record** — the repo deliberately has
no `progress.md` / `task_plan.md`. See
[F019](../adr/F019-github-issues-as-plan-tracker.md) for the reasoning and the
content-boundary table.

## What lives where

| Intent | Home |
| --- | --- |
| Work to be done, in flight, or done | Issue (assignee + open/closed) |
| A phase / release train | **Milestone** |
| A design decision (constrains code, interface, or data model) | **ADR** — never the issue body |
| A non-decisional convention or runbook | `.rules/*.md` |
| A crate or toolchain gotcha | `.rules/07-dependency-pitfalls.md` |
| Release history | [`docs/releases.md`](../releases.md) |

If a fact would still be true after the issue closes, it belongs in an ADR or
`.rules/`, not in the issue. Link it from the closing comment instead.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments by `jq` and also fetching labels.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

Infer the repo from `git remote -v` — `gh` does this automatically when run inside a clone.

## Finding and claiming work

```bash
gh issue list --state open --label ready-for-agent     # claimable
gh issue view <n> --comments                           # read before starting
gh issue edit <n> --add-assignee @me                   # claim
gh issue close <n> --comment "<what shipped> (<ADR id>)"  # finish
```

**Spec / ticket pattern.** A multi-part feature gets one **spec issue** holding the
design and intent, plus child issues numbered `t1:`…`tN:` as the actual units of work.
Work the ticket, not the spec, and close the spec only when every child is closed.
Recent examples: #28 `Mail 补遗 (spec)` with children #29/#30, and #17 `task 模块
(spec)` with children #18–#21.

## Phases as milestones

Phases used to be a list in `task_plan.md`. They are now **GitHub milestones** — one
milestone per phase, carrying its completion state and its issues:

```bash
gh api repos/{owner}/{repo}/milestones --jq '.[] | {number, title, state, open_issues, closed_issues}'
gh issue list --milestone "<phase>" --state all
gh issue create --title "..." --milestone "<phase>"
```

Milestone titles mirror the historical phase names (`Phase 22: Daemon 自动同步`) so
the pre-F019 history stays traceable.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --comments` and `gh pr diff <number>` for the diff.
- **List external PRs for triage**: `gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments` then keep only `authorAssociation` of `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR`, or `NONE` (drop `OWNER`/`MEMBER`/`COLLABORATOR`).
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either — resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (`gh api` on the sub-issues endpoint). Where sub-issues aren't enabled, add the child to a task list in the map body and put `Part of #<map>` at the top of the child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's **native issue dependencies** — the canonical, UI-visible representation. Add an edge with `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/<owner>/<repo>/issues/<n> --jq .id`, _not_ the `#number` or `node_id`). GitHub reports `issue_dependencies_summary.blocked_by` (open blockers only — the live gate). Where dependencies aren't available, fall back to a `Blocked by: #<n>, #<n>` line at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children (`gh issue list --state open`, scoped to the map's sub-issues / task list), drop any with an open blocker (`issue_dependencies_summary.blocked_by > 0`, or an open issue in the `Blocked by` line) or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --add-assignee @me` — the session's first write.
- **Resolve**: `gh issue comment <n> --body "<answer>"`, then `gh issue close <n>`, then append a context pointer (gist + link) to the map's Decisions-so-far.
