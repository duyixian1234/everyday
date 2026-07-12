#!/usr/bin/env bash
# scripts/check-doc-links.sh
# Cross-reference integrity check for the markdown documentation in this repo.
#
# Strategy (NO full-document parse):
#   1. Collect every .md file (skip target/, .git/, .workbuddy/, node_modules/).
#   2. For each file, strip fenced code blocks (```...```) and inline backtick
#      spans so that placeholder syntax is not mistaken for a broken link.
#   3. Extract every `[label](target)` link via grep -oE.
#   4. Skip http(s) / mailto / pure-anchor / placeholder targets.
#   5. Resolve the relative target against the file's directory and test -e.
#   6. Cross-check ADR index rows in docs/adr/README.md.
#   7. Cross-check .rules/RULES.md local links.
#
# Exit: 0 = clean; 1 = at least one broken link.
# Run via `just check-links` (see .rules/06-justfile.md).

set -u

# Repo root: walk up to the nearest `.git`. If absent, use cwd.
ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT" || exit 2

EXIT=0
FAILS=0
TMPDIR_LOCAL="$(mktemp -d "${TMPDIR:-/tmp}/check-links.XXXXXX" 2>/dev/null || echo "${TMPDIR:-/tmp}/check-links.$$")"
mkdir -p "$TMPDIR_LOCAL"
trap 'rm -rf "$TMPDIR_LOCAL"' EXIT

# ---- 1. File list -------------------------------------------------------
# Strip the leading "./" so internal path math stays clean.
mapfile -t FILES < <(find . -type f -name '*.md' \
  -not -path './target/*' \
  -not -path './.git/*' \
  -not -path './.workbuddy/*' \
  -not -path './node_modules/*' \
  -not -path './skills/*/.workbuddy/*' \
  | sed 's|^\./||')

if [[ ${#FILES[@]} -eq 0 ]]; then
  echo "[FAIL] no .md files found under $ROOT"
  exit 1
fi

# ---- 2. Path resolver (POSIX, no realpath dependency) -------------------
# usage: resolve_path <base_dir> <relative_target> -> prints normalized path
resolve_path() {
  local base="$1" rel="$2"
  rel="${rel#./}"
  rel="${rel#/}"
  if [[ -z "$rel" ]]; then
    echo "."
    return
  fi
  local combined="$rel"
  if [[ -n "$base" ]]; then
    combined="$base/$rel"
  fi
  local -a stack=()
  local IFS='/'
  local p
  for p in $combined; do
    case "$p" in
      ""|.) ;;
      ..)
        if (( ${#stack[@]} > 0 )); then
          unset 'stack[${#stack[@]}-1]'
        fi
        ;;
      *)
        stack+=("$p")
        ;;
    esac
  done
  local out
  out=$(IFS='/'; echo "${stack[*]}")
  [[ -z "$out" ]] && out="."
  echo "$out"
}

record_fail() {
  local file="$1" target="$2" resolved="$3"
  printf '[FAIL] %s: broken link -> %s (resolved: %s)\n' \
    "${file#./}" "$target" "$resolved"
  FAILS=$((FAILS + 1))
  EXIT=1
}

is_placeholder() {
  # Treat <…>, …-… and "..." as placeholders — never resolve.
  local s="$1"
  case "$s" in
    *"<"*) return 0 ;;
    *">"*) return 0 ;;
    "..."|"…"*) return 0 ;;
    *) return 1 ;;
  esac
}

# ---- 3. Scan all .md files ---------------------------------------------
# Strip fenced code blocks AND inline backtick spans before grep so that
# placeholder syntax inside ```…``` blocks and `…` is ignored.
strip_code_fences() {
  awk 'BEGIN { in_code = 0 }
       /^```/ { in_code = !in_code; next }
       !in_code {
         # Remove inline backtick spans (paired, non-greedy)
         gsub(/`[^`]*`/, "")
         print
       }' "$1"
}

for f in "${FILES[@]}"; do
  file_dir="$(dirname "$f")"
  [[ "$file_dir" == "." ]] && file_dir=""
  stripped="$TMPDIR_LOCAL/$(printf '%s' "$f" | tr '/ ' '__')"
  strip_code_fences "$f" > "$stripped"
  while IFS= read -r target; do
    target="${target//$'\r'/}"
    # Skip external / pure anchor
    if [[ "$target" =~ ^(https?:|mailto:|ftp:|//) ]]; then
      continue
    fi
    if [[ -z "$target" || "$target" == \#* ]]; then
      continue
    fi
    # Drop #fragment and "title" (after space)
    target_no_anchor="${target%%#*}"
    target_clean="${target_no_anchor%%[[:space:]]*}"
    [[ -z "$target_clean" ]] && continue
    is_placeholder "$target_clean" && continue
    resolved="$(resolve_path "$file_dir" "$target_clean")"
    [[ -z "$resolved" ]] && resolved="."
    if [[ ! -e "$resolved" ]]; then
      record_fail "$f" "$target" "$resolved"
    fi
  done < <(grep -oE '\[[^]]*\]\([^)]+\)' "$stripped" 2>/dev/null \
            | sed -E 's/.*\]\(([^)]+)\).*/\1/')
done

# ---- 4. ADR index consistency ------------------------------------------
# Verify every [F001](F001-...), [M001](M001-...), etc. row in docs/adr/README.md
# actually points to a real file under docs/adr/.
ADR_INDEX="docs/adr/README.md"
if [[ -f "$ADR_INDEX" ]]; then
  stripped="$TMPDIR_LOCAL/adr-index"
  strip_code_fences "$ADR_INDEX" > "$stripped"
  while IFS= read -r target; do
    target="${target//$'\r'/}"
    [[ -z "$target" ]] && continue
    is_placeholder "$target" && continue
    candidate="docs/adr/$target"
    if [[ ! -e "$candidate" ]]; then
      record_fail "$ADR_INDEX" "$target" "$candidate"
    fi
  done < <(grep -oE '\]\(([FMCNBTLR][0-9]{3}-[^)]+\.md)\)' "$stripped" \
            | sed -E 's/.*\(([^)]+)\).*/\1/')
fi

# ---- 5. .rules index consistency ---------------------------------------
# .rules/RULES.md is already scanned in step 3, so no separate pass needed.
# Step 4 covers ADR index linkage. Local .rules/ file references were validated
# by step 3 against each file's own directory.

# ---- 6. Summary ---------------------------------------------------------
if [[ $EXIT -ne 0 ]]; then
  printf '\nCheck failed: %d broken link(s) found across %d .md files.\n' \
    "$FAILS" "${#FILES[@]}"
  exit 1
fi

printf '[OK] no broken links among %d .md files.\n' "${#FILES[@]}"
exit 0
