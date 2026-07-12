# 05-commit.md — Conventional Commits + Pre-commit Checklist

> Backed by [F006](../docs/adr/F006-ci-release-github-only.md) for the release flow.
> This file is the per-commit discipline.

## Commit message format

```
<type>(<scope>): <subject>

<body — wrap at 72 cols; explain what and why, not how>

<footer — references, breaking change notes, co-authors>
```

### Allowed `<type>` values

| Type | When to use |
| --- | --- |
| `feat` | New user-facing behavior (a command, an action, a flag) |
| `fix` | A bug whose absence is observable (panic, wrong output, contract violation) |
| `refactor` | Internal change that preserves behavior (extract macro, rename, dedupe) |
| `test` | Only-tests changes |
| `docs` | Only-docs changes |
| `chore` | Build / CI / dependency / release commit |
| `perf` | Rare — only for measurable perf wins, with numbers in the body |

### `<scope>`

The primary module or layer touched. Common values: `mail`, `cal`, `rss`, `note`,
`todo`, `bookmark`, `timeline`, `config`, `cli`, `output`, `util`, `release`.

`scope` is optional for repo-wide changes (e.g. `chore: release v0.7.0`).

## Atomic commits

- **One commit = one task.** Don't accumulate; don't mix feature with formatting.
- After every commit, the project must `cargo build` and `cargo test`.
- Split a feat from a chore when the feature is logically separate
  (recommended pattern: `feat(...)` first, then `chore: bump version` later).

## Body content — what to say

- **What** changed (one sentence).
- **Why** this approach (only if non-obvious).
- **Tradeoffs** and rejected alternatives, if the commit is a decision-class
  change. If so, link the ADR.

If the body grows past ~10 lines, you may have packed two changes in one commit.

## Pre-commit checklist

Run in order (most are handled by `just ci`):

- [ ] `cargo build` passes
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] `cargo fmt --check` is clean (run `cargo fmt` first if not)
- [ ] `cargo test` is green
- [ ] `just check-links` passes
- [ ] [ADR extraction step](01-workflow.md#adr-extraction-step) is complete for
      decision-class changes
- [ ] [progress.md](../progress.md) timeline index updated with new ADR id
- [ ] Commit message follows the format above
- [ ] Single task — no unrelated drive-by changes

## Release commits

A release is **always** a separate `chore` commit that:

1. Bumps `Cargo.toml` `version`.
2. Lets `cargo build` regenerate `Cargo.lock`.
3. Updates version references in docs (`README.md`, `README_ZH.md`,
   `skills/*`).
4. Adds the new version line to [progress.md](../progress.md).
5. Tags annotated: `git tag -a vX.Y.Z -m "vX.Y.Z: <highlights>"`.

See [06-justfile.md](06-justfile.md) and the release runbook summary in
[01-workflow.md §"Release (runbook summary)"](01-workflow.md#release-runbook-summary).
