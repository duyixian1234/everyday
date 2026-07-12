# 06-justfile.md — Justfile Recipes

> The [`Justfile`](../Justfile) is the project's command catalog. Underneath each
> recipe is a real `cargo` command — `just` exists for ergonomics and for
> cross-platform shell handling.

## Cross-platform shells

The Justfile declares two shells:

```just
set shell := ["bash", "-c"]
set windows-shell := ["powershell.exe", "-NoProfile", "-NoLogo", "-Command"]
```

- On Linux / macOS / WSL / Git-Bash, recipes run under `bash`.
- On Windows native, recipes run under `powershell.exe` (no profile, no logo).

This means a recipe body that uses `&&` to chain commands works on both — bash's
`&&` runs in the `bash -c` subshell on Windows, and PowerShell's `; if ($?) { ... }`
works on native Windows. Use bash chaining (`&&`) for portability.

## Recipes

| Recipe | Cargo equivalent | Notes |
| --- | --- | --- |
| `just` | `just --list` | List all recipes |
| `just format` | `cargo fmt` | Format all code |
| `just check` | `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | Lint; **fail-fast on `fmt --check`** (does not run clippy if formatting is wrong) |
| `just test` | `cargo test -q` | Run all tests; `-q` suppresses progress lines |
| `just build` | `cargo build -q` | Build the binary; `-q` suppresses progress lines |
| `just ci` | `check` → `test` → `build` | Full local CI |
| `just check-links` | (custom) | Cross-reference integrity check — see below |

## Quiet output convention

`cargo`-backed recipes (`test`, `build`) carry the `-q` (quiet) flag by default.
This suppresses the "Compiling / Finished" progress chatter and keeps CI logs to
only errors, test failures, and the final summary. Favor `-q` for any new
`cargo` recipe added to this file unless the recipe's purpose is to surface the
full build trace.

## `just check-links`

This recipe validates every markdown link in the repo resolves to an existing
file or anchor. It is **the gate that catches ADR / `.rules` / cross-doc link
rot** before it reaches CI.

### What it checks

For every `.md` file in the repo (excluding `target/`, `.git/`, `.workbuddy/`):

1. Every `[label](path)` link (both inline and reference-style) where `path` is
   relative — verify the file exists at the resolved path.
2. Every `<id>.md` style anchor in a `README.md` index — verify the target
   file exists.
3. Every cross-doc index entry under an "ADR index" or "Index" section — sample
   a few entries against the directory layout.
4. Inline anchors (`#heading-slug`) are best-effort: only flagged when the
   heading text is unambiguously missing (regex match).

It does **not** read every document line-by-line. It uses `grep` + `sed` to
extract link patterns, then `test -e` for each.

### Source

The recipe lives in [Justfile](../Justfile). It dispatches to one of two
scripts:

- [scripts/check-doc-links.sh](../scripts/check-doc-links.sh) — bash (Unix /
  Git-Bash on Windows).
- [scripts/check-doc-links.ps1](../scripts/check-doc-links.ps1) — PowerShell
  (Windows native).

Both produce identical output. Either may be invoked directly:

```sh
bash scripts/check-doc-links.sh
```

```powershell
pwsh -File scripts/check-doc-links.ps1
```

### Failure modes

| Output | Meaning |
| --- | --- |
| `[OK] No broken links.` | Pass |
| `[FAIL] <file>:<line> broken link -> '<path>'` | Path does not resolve |
| `[FAIL] ADR id '<id>' not in docs/adr/` | ADR referenced but no file with that id exists |
| `[FAIL] .rules/ file '<name>' missing` | A file promised by index is missing |
| Exit code 0 on pass, 1 on any FAIL |

## Adding a new recipe

1. Edit the [Justfile](../Justfile).
2. Keep the body minimal — `just` is a thin shell.
3. For long bodies, prefer extracting to a script under `scripts/`.
4. Add a row to the table above.
5. Run `just --list` locally to verify formatting.
