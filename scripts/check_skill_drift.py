#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Check that the agent skill reference covers every CLI (module, action).

Run with ``uv run scripts/check_skill_drift.py`` (or ``just check-skill-drift``).

The expected inventory is read from the binary itself — ``everyday mcp tools
--json`` projects every ``(module, action)`` pair with a JSON Schema, generated
from ``module_arg_spec`` (single source of truth, see ADR F014).  This script
asserts each projected action is documented in the skill reference
(``skills/everyday-cli/references/COMMANDS.md``), so a new action cannot ship
without its documentation.  See ADR F018.

Excluded modules: ``mcp`` cannot project itself, and ``health`` is a root-level
command rather than a module — neither has tools to enumerate.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

# Modules with no projected tools by construction (see module docstring).
EXCLUDED_MODULES = frozenset({"mcp", "health"})

REFERENCE_PATH = Path("skills/everyday-cli/references/COMMANDS.md")


@dataclass(frozen=True)
class Drift:
    """A projected CLI action that the skill reference does not document."""

    module: str
    action: str

    @property
    def key(self) -> str:
        return f"{self.module} {self.action}"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="repository root (defaults to the Git root, or the current directory)",
    )
    parser.add_argument(
        "--binary",
        default="everyday",
        help="everyday binary to interrogate (default: everyday on PATH)",
    )
    parser.add_argument(
        "--reference",
        type=Path,
        default=None,
        help=f"reference document (default: <root>/{REFERENCE_PATH})",
    )
    return parser.parse_args()


def repository_root(requested_root: Path | None) -> Path:
    if requested_root is not None:
        return requested_root.resolve()
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            check=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return Path.cwd().resolve()
    return Path(result.stdout.strip()).resolve()


def projected_actions(binary: str) -> list[tuple[str, str]]:
    """Return every ``(module, action)`` the CLI projects as an MCP tool."""
    if shutil.which(binary) is None and not Path(binary).exists():
        raise SystemExit(
            f"[FAIL] '{binary}' not found on PATH — build it first "
            f"(cargo build) or pass --binary <path>"
        )
    result = subprocess.run(
        [binary, "mcp", "tools", "--json"],
        capture_output=True,
        check=True,
    )
    # The CLI emits UTF-8 (tool descriptions are often non-ASCII), but the default
    # text decoding follows the locale — GBK on a Chinese Windows console. Decode
    # explicitly instead of letting `text=True` mangle it.
    tools = json.loads(result.stdout.decode("utf-8"))
    pairs: list[tuple[str, str]] = []
    for tool in tools:
        name = tool.get("name", "")
        # Split on the FIRST underscore only: the module name never contains one,
        # so any further underscores belong to the action (`rss_items-fetch`).
        module, separator, action = name.partition("_")
        if not separator or not action:
            raise SystemExit(f"[FAIL] projected tool '{name}' is not <module>_<action>")
        if module in EXCLUDED_MODULES:
            continue
        # A bare module with an empty/underscore-leading action would silently
        # produce a pair that can never match a doc line; fail loudly instead.
        if action.startswith("_") or action.endswith("_"):
            raise SystemExit(
                f"[FAIL] projected tool '{name}' has an unexpected action "
                f"'{action}' — the module/action split is ambiguous"
            )
        pairs.append((module, action))
    return sorted(pairs)


def documented_actions(text: str, modules: set[str]) -> set[str]:
    """Return every ``"<module> <action>"`` the reference documents.

    Two forms count as documentation:

    * a command example — ```` ```everyday <module> <action> ...``` ````, and
    * a table row — ``| `<module> <action>` | ... |``.

    Both are anchored on a known module name.  A looser "any ``everyday X Y``"
    pattern was rejected: it also matches prose ("everyday as an MCP server"),
    which would let an undocumented action pass on the strength of a sentence.
    """
    found: set[str] = set()
    # A module name from the live inventory keeps prose out of the match set.
    for module, action in re.findall(
        r"`?everyday\s+([a-z][a-z0-9-]*)\s+([a-z][a-z0-9-]*)", text
    ):
        if module in modules:
            found.add(f"{module} {action}")
    for module, action in re.findall(r"\|\s*`([a-z]+) ([a-z][a-z0-9-]*)[^`]*`", text):
        if module in modules:
            found.add(f"{module} {action}")
    return found


def main() -> int:
    args = parse_args()
    root = repository_root(args.root)
    reference = args.reference or root / REFERENCE_PATH

    if not reference.is_file():
        print(f"[FAIL] reference document not found: {reference}", file=sys.stderr)
        return 1

    pairs = projected_actions(args.binary)
    if not pairs:
        print("[FAIL] no projected actions found — is this the right binary?", file=sys.stderr)
        return 1

    documented = documented_actions(
        reference.read_text(encoding="utf-8"), {module for module, _ in pairs}
    )
    drift = [Drift(m, a) for m, a in pairs if f"{m} {a}" not in documented]

    if drift:
        print(
            f"[FAIL] {len(drift)} of {len(pairs)} projected actions are missing from "
            f"{reference.relative_to(root).as_posix()}:",
            file=sys.stderr,
        )
        for item in drift:
            print(f"  - everyday {item.key}", file=sys.stderr)
        print(
            "\nDocument them, or exclude the module in EXCLUDED_MODULES if it has "
            "no tools by construction (see ADR F018).",
            file=sys.stderr,
        )
        return 1

    print(f"[OK] all {len(pairs)} projected actions documented in {reference.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
