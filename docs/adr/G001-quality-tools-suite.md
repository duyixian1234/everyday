# ADR: Quality Tool Suite (G001)

**Status**: Accepted

**Date**: 2026-08-09

**Deciders**: Yixian (project owner)

---

## Context

Everyday hit three breaking releases in a row (v0.8 removed per-module `login`;
v0.12 changed `Executor` signatures; v0.13 removed the Notion provider). Each
was caught only by **manual ADR review** — there is no automated line of defense
for CLI/behavior-level breaking changes. Meanwhile the project grows: 357 tests,
389 crates, 71 ADRs, CI across 3 platforms / 4 targets, and an upcoming GOAI
competition entry where license compliance is a hard requirement.

Seven tools were evaluated for the engineering stack. This ADR records what was
adopted, what was rejected and why, and the trade-offs accepted.

## Decision

Adopt, in two batches:

### Batch 1 (this change set)

| Tool | Role | Gate |
| --- | --- | --- |
| `cargo-nextest` | Test runner on **CI only** — `--junit` reports (expandable in Actions UI) + failure retries | CI `Test` step |
| `typos` | Spelling gate across docs/code | CI job + `just typos` |
| `git-cliff` | Changelog from conventional commits (ADR IDs ride through verbatim) | Release runbook step (`git cliff -o CHANGELOG.md`) |
| `cargo-deny` | License + RUSTSEC audit | CI job + `just deny` |
| CLI contract tests (`tests/cli_contract.rs`) | **Replaces cargo-semver-checks** — see below | `cargo test` (in CI via nextest) |

### Batch 2 (deferred, separate scheduling)

- `cargo-dist` — modern release pipeline (automatic installers, checksums, SBOM,
  GitHub attestation). Will take over `release.yml`; attestation enabled, no
  Homebrew tap.
- `sccache` — CI cross-job compile cache. Deferred: `Swatinem/rust-cache` already
  covers registry+target; marginal CI benefit does not yet justify architecture
  complexity on Windows/MSVC.
- `cargo-cache` — **not a speed tool**: local disk hygiene (registry/target
  cleanup). Not wired into CI.

## Why cargo-semver-checks was rejected

`cargo-semver-checks` detects breaking changes in a **library crate's public
Rust API**. Everyday is a **pure `[[bin]]`** with no `lib.rs` and no public API.
All three historical breaking releases were **CLI/behavior-level** breaks
(subcommand removal, signature changes, provider removal) — invisible to
semver-checks. The fix is the CLI contract test layer instead:

- top-level `--help` subcommand set (adding/removing a module breaks CI),
- per-module action sets (removing an action breaks CI),
- `config.example.toml` shape (removing a config section breaks CI).

Deliberately **not** full `--help` golden snapshots — help copy churn must not
fail CI. Only names (the contract) are locked. If a `lib` target is ever split
out, re-evaluate semver-checks for the library half.

## cargo-deny baseline scan (2026-08-09)

389 crates, first full scan:

- **Licenses**: 22 flagged — all from 3 permissive families, **zero copyleft,
  zero unknown/missing**: `Unicode-3.0` (icu_* via idna_adapter chain),
  `0BSD` (mailparse, quoted_printable), `CDLA-Permissive-2.0` (webpki-roots).
  All added to `deny.toml` allow list.
- **Advisories**: zero security-class (CVE) findings. Four `unmaintained`
  advisories accepted and explicitly ignored (below). One yanked (`spin 0.9.8`)
  cleared via `cargo update -p spin`.

### Accepted unmaintained dependencies

`unmaintained` is "discontinued but not vulnerable". cargo-deny v0.20's
`unmaintained`/`yanked` keys take a *scope* (`all`/`workspace`/`transitive`/`none`),
not a severity. Policy: `unmaintained = "all"` + explicit `ignore` entries, so
**new** unmaintained deps fail CI until reviewed (with an ADR note).

| RUSTSEC | Crate | Chain | Accepted because |
| --- | --- | --- | --- |
| RUSTSEC-2025-0052 | async-std | async-imap (mail module) | async-imap's only async runtime; no drop-in replacement in its ecosystem |
| RUSTSEC-2024-0388 | derivative | keyring → zbus → secret-service (Linux credentials) | transitive; zbus is the standard secret-service client |
| RUSTSEC-2024-0384 | instant | zbus → futures-lite chain | transitive; maintained fork (`instant` → `web-time`) not yet adopted upstream |
| RUSTSEC-2024-0370 | proc-macro-error | tabled_derive → tabled (table output) | transitive; tabled upgrade (batch 2) may clear it |

## git-cliff scope boundary

git-cliff **renders the changelog only**. Its `--bump` (automatic semver from
breaking commits) is **not** adopted — version judgment stays single-sourced:
manual bump in `Cargo.toml` + `v*` tag, governed by ADRs and the CLI contract
layer. Two independent version-prediction systems would fight each other.

## nextest scope boundary

nextest replaces `cargo test` on **CI only**. Local suite is 357 tests / ~4s
measured — `cargo test` is already parallel (per-CPU `--test-threads`), so the
"3-10× faster" claim does not apply at this scale; the local tool switch would
be pure mental overhead. CI keeps nextest for junit reporting and future
test-set growth.

## Consequences

### Positive

1. **Automated breaking-change defense** for the exact failure mode that hit
   v0.8/v0.12/v0.13 — CLI contract tests catch it in CI.
2. **Compliance-ready dependency audit**: license allow-list + advisory policy
   is machine-enforced, not a release-time manual scan; GOAI entry can point to
   `deny.toml` + this ADR.
3. **Changelog discipline without extra ceremony**: commits already carry ADR
   IDs; git-cliff renders them verbatim.
4. **Spelling gate** stops doc rot across the large README/ADR/skills corpus.

### Negative (Tradeoffs)

1. **5 config files / CI jobs to maintain** (`typos.toml`, `deny.toml`,
   `git-cliff.toml`, nextest CI step, contract tests) — the standard cost of a
   quality-tool stack on a single-maintainer project; each is intentionally
   small and one-directional.
2. **CLI contract tests are redundant with clap's own parsing tests** — accepted
   by design: the point is to catch *drift from the shipped contract*, which
   clap's internal tests cannot.
3. **unmaintained deps are permanent** until upstream moves — recorded, not
   hidden; `cargo update` at release time may clear some over time.

## Related decisions

- [F006](F006-ci-release-github-only.md) — GitHub-only CI/release flow that this
  tool stack plugs into.
- [F007](F007-clap-subcommand-tree.md) — the clap command tree that
  `tests/cli_contract.rs` locks.
- [F013](F013-request-context-explicit-parameter.md) — a v0.12 breaking change
  the contract layer would have caught.
- [R019](R019-remove-notion-provider.md) — a v0.13 breaking change the contract
  layer would have caught.
