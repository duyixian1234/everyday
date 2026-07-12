# ADR F003: Module scope — external integration interface only (no fs/net/sys)

**Status:** Accepted
**Date:** 2026-07-10

## Context

The original PRD positioned Everyday as a "deep into the operating system" runtime toolbox, with modules covering file search (`fs`), web scraping / generic HTTP (`net`), system monitoring (`sys`), clipboard, and similar utilities. A review surfaced a structural question: **what should this CLI actually be?**

Two coherent identities were possible:

1. **External integration interface.** The CLI fronts systems the Agent cannot easily reach on its own: protocols with state (IMAP, CalDAV), systems with credentials (Notion, SMTP), systems with bounded access (RSS feeds).
2. **General-purpose local toolbox.** Wrap `find`, `curl`, `rg`, `sysinfo`, clipboard — anything an Agent might call.

## Decision

**Everyday is the external integration interface. The CLI exposes only what an Agent cannot trivially do with shell tools.**

- Every shipped module must encapsulate one or more of:
  - A non-trivial **protocol** (IMAP, SMTP, CalDAV, RSS, Notion REST).
  - **State** that the CLI must own and resume (caches, watermarks, sync windows).
  - **Credentials** that should not live in shell history or environment variables.
- Modules that are "shell-equivalent" (file search, generic HTTP fetch, system metrics, clipboard) are **out of scope**.

This rule was implemented by removing the originally planned `fs` / `net` / `sys` / clipboard modules and dropping their only-dependencies (`scraper`, `ignore`, `walkdir`, `arboard`, `sysinfo`, `notify`). The `reqwest` dependency stays because `rss` / `note` / `todo` reuse it for legitimate reasons.

The scope rule is now the authoritative section in `agents.md` ("范围与定位") and the original PRD was deleted in commit `fc14584`.

## Alternatives considered

### Keep `fs` / `net` / `sys`

- Pro: a single binary covers many Agent needs.
- Con: every capability is duplicated by shell tools the Agent already invokes (`fd`, `rg`, `curl`, `top`, etc.).
- Con: the CLI grows an unbounded surface area, each new module needs its own docs, tests, error model — none of it differentiated.
- Con: contradicts the "every command must justify its existence" project discipline.
- Rejected.

### Make the CLI a generic wrapper around `find`/`curl`/etc.

- Pro: trivially easy to implement.
- Con: no Agent actually wants a slower, more opaque `find`.
- Rejected.

### Keep `net` only, for authenticated HTTP that bypasses curl's auth pain

- Considered briefly: most authenticated HTTP the Agent needs is better served by a one-shot script.
- Rejected on consistency grounds: if `net` exists, every other generic capability comes back.

## Consequences

- New module proposals must answer "what protocol / state / credential does this encapsulate that shell cannot?" before being accepted.
- The CLI stays small (seven modules today) and each module's value is high.
- Cross-cutting infrastructure (Executor trait, Output, AgentError) gets amortized across a small, focused module set.
- The project can refuse feature requests that smell like shell wrappers.
- Removes the temptation to grow a `quick-script-runner` module later.

## Cross-references

- The cross-cutting abstractions that make this discipline feasible: [F001](F001-cli-shape.md), [F002](F002-multi-account-keyring.md).
- A later, related decision that reinforces this scope: [R012](R012-config-executor-trait.md) folds `config` into the standard Executor dispatch.