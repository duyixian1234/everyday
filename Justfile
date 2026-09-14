# Everyday — development workflow manager (just).
# Run `just` to list available recipes.
# Cross-platform shells: bash on Unix, powershell.exe on Windows.

set shell := ["bash", "-c"]
set windows-shell := ["powershell.exe", "-NoProfile", "-NoLogo", "-Command"]

# List all available recipes
default:
    @just --list

# Format all code (cargo fmt)
format:
    cargo fmt

# Check format and lint: fail-fast on fmt --check before running clippy
check: _fmt-check _clippy

_fmt-check:
    cargo fmt --check

_clippy:
    cargo clippy --all-targets -- -D warnings

# Run tests (cargo test)
test:
    cargo test -q

# Spell-check all docs/code (typos-cli; config: typos.toml)
typos:
    typos

# Dependency audit — licenses + RUSTSEC advisories (config: deny.toml)
deny:
    cargo deny check

# Build the project (cargo build)
build:
    cargo build -q

# Validate cross-document links (agents.md / .rules/ / docs/adr/).
# Requires uv and uses one process per CPU by default.
check-links:
    uv run scripts/check_doc_links.py

# Assert every CLI (module, action) is documented in the agent skill reference.
# Requires a built `everyday` (cargo build). Design: docs/adr/F018.
check-skill-drift:
    uv run scripts/check_skill_drift.py --binary ./target/debug/everyday

# Link the agent skill into the host skills directory (junction/symlink, not a copy).
# Idempotent. Target can be overridden: just skill-link /path/to/skills.
# Design: docs/adr/F018.
skill-link target="~/.agents/skills":
    pwsh -NoProfile -File scripts/skill_link.ps1 -Target "{{target}}"

# Full CI pipeline: check → check-links → check-skill-drift → test → build → typos → deny
ci: check check-links check-skill-drift test build typos deny
