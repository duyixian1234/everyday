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

This recipe validates every local Markdown link in the repository resolves to
an existing file. It is **the gate that catches ADR / `.rules` / cross-doc link
rot** before it reaches CI.

### What it checks

For every `.md` file in the repo (excluding `target/`, `.git/`, `.workbuddy/`,
and `node_modules/`) and every Rust source file under `src/`:

1. Every inline `[label](path)` link where `path` is relative — verify the file
   exists at the resolved path.
2. Rust links are read only from `///` and `//!` doc comments.
3. Fenced Markdown blocks and inline code spans are ignored.
4. ADR index targets are cross-checked against `docs/adr/`.

The checker uses only the Python standard library, and scans source files in
multiple processes. It does not validate fragment/heading anchors.

### Source

The recipe lives in [Justfile](../Justfile) and dispatches to:

- [scripts/check_doc_links.py](../scripts/check_doc_links.py) — a standalone
   Python script run by `uv` on every supported platform.

It may be invoked directly; use `--jobs` to control the process count, `--root`
to select a repository, and repeat `--exclude` for extra directory names:

```bash
uv run scripts/check_doc_links.py --jobs 4
```

### Failure modes

| Output | Meaning |
| --- | --- |
| `[OK] no broken links among … files.` | Pass |
| `[FAIL] <file>: broken link -> <path> (resolved: <path>)` | Path does not resolve |
| `[FAIL] --jobs must be at least 1` | Invalid process-count argument |
| Exit code 0 on pass, 1 on any FAIL |

## Adding a new recipe

1. Edit the [Justfile](../Justfile).
2. Keep the body minimal — `just` is a thin shell.
3. For long bodies, prefer extracting to a script under `scripts/`.
4. Add a row to the table above.
5. Run `just --list` locally to verify formatting.
