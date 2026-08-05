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

# Build the project (cargo build)
build:
    cargo build -q

# Validate cross-document links (agents.md / .rules/ / docs/adr/).
# Requires uv and uses one process per CPU by default.
check-links:
    uv run scripts/check_doc_links.py

# Full CI pipeline: check → check-links → test → build
ci: check check-links test build
