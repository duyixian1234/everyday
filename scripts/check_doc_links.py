#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Check local Markdown links in documentation and Rust doc comments.

Run with ``uv run scripts/check_doc_links.py``.  Files are checked in parallel;
use ``--jobs 1`` when debugging a single-process failure.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

DEFAULT_EXCLUDES = ("target", ".git", ".workbuddy", "node_modules")
LINK_RE = re.compile(r"\[[^]]*\]\(([^)]+)\)")
ADR_LINK_RE = re.compile(r"\]\(([FMCNBTLR][0-9]{3}-[^)]+\.md)\)")
RS_DOC_RE = re.compile(r"^\s*//[/!]\s?(.*)$")


@dataclass(frozen=True)
class LinkFailure:
    """A local documentation link that does not resolve within the repository."""

    file: str
    target: str
    resolved: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="repository root (defaults to the Git root, or the current directory)",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=os.cpu_count() or 1,
        help="number of worker processes (default: CPU count)",
    )
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        metavar="DIRECTORY",
        help="additional directory name to exclude; may be repeated",
    )
    return parser.parse_args()


def repository_root(requested_root: Path | None) -> Path:
    if requested_root is not None:
        return requested_root.resolve()
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return Path.cwd().resolve()
    return Path(result.stdout.strip()).resolve()


def find_files(root: Path, excluded_directories: set[str]) -> tuple[list[Path], list[Path]]:
    markdown_files: list[Path] = []
    rust_files: list[Path] = []
    for path in root.rglob("*"):
        if not path.is_file() or any(part in excluded_directories for part in path.parts):
            continue
        if path.suffix == ".md":
            markdown_files.append(path)
        elif path.suffix == ".rs" and path.is_relative_to(root / "src"):
            rust_files.append(path)
    return sorted(markdown_files), sorted(rust_files)


def strip_markdown(content: str) -> str:
    in_code_block = False
    retained_lines: list[str] = []
    for line in content.splitlines():
        if line.startswith("```"):
            in_code_block = not in_code_block
            continue
        if not in_code_block:
            retained_lines.append(re.sub(r"`[^`]*`", "", line))
    return "\n".join(retained_lines)


def strip_rust(content: str) -> str:
    return "\n".join(
        match.group(1)
        for line in content.splitlines()
        if (match := RS_DOC_RE.match(line)) is not None
    )


def resolve_target(file: Path, root: Path, target: str) -> Path:
    relative_target = target.removeprefix("./").lstrip("/")
    parts: list[str] = []
    for part in PurePosixPath(file.relative_to(root).parent, relative_target).parts:
        if part in ("", "."):
            continue
        if part == "..":
            if parts:
                parts.pop()
            continue
        parts.append(part)
    return root.joinpath(*parts)


def is_skipped_target(target: str) -> bool:
    return (
        not target
        or target.startswith(("http:", "https:", "mailto:", "ftp:", "//", "#"))
        or "<" in target
        or ">" in target
        or target == "..."
        or target.startswith("…")
    )


def scan_file(root: Path, file: Path) -> list[LinkFailure]:
    content = file.read_text(encoding="utf-8", errors="replace")
    stripped = strip_rust(content) if file.suffix == ".rs" else strip_markdown(content)
    failures: list[LinkFailure] = []
    for match in LINK_RE.finditer(stripped):
        target = match.group(1).replace("\r", "")
        if is_skipped_target(target):
            continue
        target = target.split("#", maxsplit=1)[0].split(maxsplit=1)[0]
        if not target or is_skipped_target(target):
            continue
        resolved = resolve_target(file, root, target)
        if not resolved.exists():
            failures.append(
                LinkFailure(
                    file.as_posix().removeprefix(root.as_posix()).lstrip("/"),
                    match.group(1),
                    resolved.relative_to(root).as_posix(),
                )
            )
    return failures


def scan_adr_index(root: Path) -> list[LinkFailure]:
    index = root / "docs" / "adr" / "README.md"
    if not index.is_file():
        return []
    failures: list[LinkFailure] = []
    for target in ADR_LINK_RE.findall(strip_markdown(index.read_text(encoding="utf-8", errors="replace"))):
        candidate = index.parent / target
        if not candidate.exists():
            failures.append(LinkFailure(index.relative_to(root).as_posix(), target, candidate.relative_to(root).as_posix()))
    return failures


def main() -> int:
    args = parse_args()
    if args.jobs < 1:
        print("[FAIL] --jobs must be at least 1", file=sys.stderr)
        return 2

    root = repository_root(args.root)
    if not root.is_dir():
        print(f"[FAIL] repository root does not exist: {root}", file=sys.stderr)
        return 2

    markdown_files, rust_files = find_files(root, set(DEFAULT_EXCLUDES).union(args.exclude))
    files = [*markdown_files, *rust_files]
    if not files:
        print(f"[FAIL] no .md or .rs files found under {root}")
        return 1

    with ProcessPoolExecutor(max_workers=args.jobs) as executor:
        failures = [failure for file_failures in executor.map(scan_file, [root] * len(files), files) for failure in file_failures]
    failures.extend(scan_adr_index(root))

    for failure in sorted(failures, key=lambda item: (item.file, item.target, item.resolved)):
        print(f"[FAIL] {failure.file}: broken link -> {failure.target} (resolved: {failure.resolved})")
    if failures:
        print(f"\nCheck failed: {len(failures)} broken link(s) found across {len(files)} files.")
        return 1

    print(f"[OK] no broken links among {len(files)} files ({len(markdown_files)} .md + {len(rust_files)} .rs).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())