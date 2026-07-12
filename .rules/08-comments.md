# 08-comments.md — Comment Policy

> The only authoritative rule for what stays, what gets translated, and what
> gets rewritten as an ADR link. Backed by the cross-reference discipline in
> [01-workflow.md](01-workflow.md) and the link checker in
> [06-justfile.md](06-justfile.md).

## TL;DR

Source-code comments are **English only**. Anything else is a typo or a TODO.

| Bucket | Treatment |
| --- | --- |
| `//` / `///` / `//!` comment containing Chinese | **Translate to English**; promote design-decision comments to an ADR link |
| `// ============ 模块 ============` section header | Translate the label to English |
| CLI help string literal in `description` / `usage` / `help` of `ActionArgSpec` / `ArgSpec` | **Keep in Chinese** — user-facing UI |
| JSON fixture string value in `#[cfg(test)]` (e.g. `"写文档"`, `"买咖啡"`) | **Keep** — legitimate test data |
| Example decode data inside a comment (base64 byte streams, UTF-16BE encoded mailbox names) | **Keep** — legitimate example content |
| URL string literal containing Chinese (`https://app.notion.com/p/写周报-...`) | **Keep** — legitimate business data |

The four "keep" rows are not comments; they are string literals. The rule for
comments is the first row.

## The four-bucket classification

A line containing Chinese is either a **comment** or a **string literal**. Decide
before touching it.

### Bucket 1 — translate (comment that contains Chinese)

Source-of-truth rule: every `//`, `///`, `//!` comment line that contains Chinese
must become English. If the comment is **design/architecture content** (a
contract, invariant, boundary, or rationale a future reader would be surprised
to reverse), the comment becomes a markdown link to the relevant ADR.

Quick test for "should this be an ADR link?":

- Does it explain a constraint that future code must respect? → ADR.
- Does it record why a particular shape was chosen? → ADR.
- Is it just restating what the code does ("this increments the counter")?
  → remove it, the code already says so.

### Bucket 2 — translate (section header banner)

Banners like `// ============ 模块 ============` are comments and must be
translated. Render the translation in the same `// ===== ` style. There is no
ADR link target — these are navigation aids inside the file.

### Bucket 3 — keep (user-facing string literal)

CLI help text is rendered by clap into `everyday <module> <action> --help`
output. The audience is a Chinese-speaking operator reading a terminal. Changing
it to English breaks the UX contract.

Concrete markers:

- `description: "保存 Notion 凭证到系统 keyring"` inside `ActionArgSpec { ... }`
- `usage: "everyday todo login [--account NAME]"` in the same struct
- `help: "数据库 ID"` inside `ArgSpec { ... }`

These are **string literals**, not comments. They stay in Chinese.

### Bucket 4 — keep (test / example / business data string literal)

Three sub-categories of string literals that look like Chinese but are not
comments:

- **JSON fixture in `#[cfg(test)]`** — the title in `TimelineEvent::new("todo",
  Some("personal"), "created", now, "买咖啡", ...)` is the test's notion of
  "user-typed Chinese content". Translating it loses test realism.
- **Example decode data** — `email.rs` documents `=?UTF-8?B?5L2g5aW9?` decoding
  to `"你好"`. The Chinese is the *expected decoded value* of the example, not a
  comment about the code.
- **URL string literals** — `https://app.notion.com/p/写周报-39a961...` is a
  real Notion page URL captured by the ops-log fixture. The Chinese is the page
  title baked into the URL.

When unsure, ask: "is this a Rust comment (`//` / `///` / `//!`)?". If no, leave
it alone.

## ADR link depth rule

Links to `docs/adr/*.md` are **relative**. The number of `../` segments depends
on the source file's depth:

| Source file lives in | ADR link prefix |
| --- | --- |
| `src/<file>.rs` (1 level) | `[id](../docs/adr/<id>-...md)` |
| `src/modules/<file>.rs` (2 levels) | `[id](../../docs/adr/<id>-...md)` |
| `src/modules/timeline/<file>.rs` (3 levels) | `[id](../../../docs/adr/<id>-...md)` |

`scripts/check-doc-links.sh` validates every link in `.md` and `.rs` files. A
wrong depth produces a broken link that fails `just check-links` — never guess.

## How to add a new ADR link

1. **Confirm the ADR file exists** — `ls docs/adr/<ID>-*.md` before writing.
2. **Pick the right depth** — table above.
3. **Place at the right comment level** — module-level decisions belong in the
   top-of-file `//!` doc-block; function-level decisions belong on the function's
   `///` doc; single-line explanations go on the `//` line directly above the
   code.
4. **Cross-link in both directions** — when a comment in `note.rs` links to
   `R009`, the `R009` ADR's "Cross-references" section should mention `note.rs`.
   See [01-workflow.md §ADR extraction step](01-workflow.md#adr-extraction-step).

## Verification commands

Run these before committing a comment-cleanup change:

```bash
# 1. No Chinese comments remain in source.
grep -rnP '(//|///).*\p{Han}' src/ --include='*.rs'
# Expected: no matches (or only Bucket-3/4 false positives — JSON fixtures,
# base64 decode examples, URL literals — which the grep cannot distinguish
# from comments; verify manually).

# 2. Every ADR link in source resolves to a real file.
grep -oE '\]\(\.\.+/docs/adr/[^)]+\.md\)' src/modules/timeline.rs | sort -u
# Each result must exist under docs/adr/. Manual cross-check with:
ls docs/adr/L*.md   # for L-series links, etc.

# 3. fmt clean.
cargo fmt --check

# 4. All quality gates green.
just ci              # check + check-links + test + build
```

`just check-links` may hang on Windows in the safe-delete cleanup step (path
mangling); if it does, fall back to steps 1 and 2 above.

## Commit discipline

Each cleaned-up `.rs` file is **one commit**. Do not bundle two modules' comment
translations into a single commit — the diff is hard to review and revert.

Conventional Commit subject for comment cleanup:

```
refactor(comments): clean up src/modules/<file>.rs
```

The commit body should call out:

- Which ADR links were added and why.
- Which "kept" bucket the Chinese strings belong to (CLI help / test fixture /
  example data / URL literal) — proves the keep was deliberate.

## Worked example

Before (a function-level comment that should be both translated and ADR-linked):

```rust
/// 时间增量拉取：返回 `created_at` 或 `updated_at` 落在窗口内的 todo。
///
/// 本地 provider 降级语义：从当前态快照拉取，非完整转移历史。
```

After:

```rust
/// Timeline incremental pull: return todos whose `created_at` or `updated_at`
/// falls within the window.
///
/// Local provider degraded semantics: pulled from the current-state snapshot,
/// not the full transfer history [L001](../../docs/adr/L001-append-only-event-log.md).
```

The translation is mechanical. The ADR link `[L001](...)` is added because the
"current-state snapshot vs full history" trade-off is a real decision that
belongs in the append-only event log ADR, not inline in the comment.