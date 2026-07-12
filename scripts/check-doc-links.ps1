<#
scripts/check-doc-links.ps1
Cross-reference integrity check for the markdown documentation in this repo.

Strategy (NO full-document read):
  1. Collect every .md file (skip target/, .git/, .workbuddy/).
  2. For each file, extract every `[label](target)` via Select-String + regex.
  3. Skip http(s) / mailto / pure-anchor targets.
  4. Resolve the relative target against the file's directory and Test-Path.
  5. Cross-check ADR index links in docs/adr/README.md.
  6. Cross-check .rules/RULES.md local links.

Exit: 0 = clean; 1 = at least one broken link.
Run via `just check-links` (see .rules/06-justfile.md).
#>

$ErrorActionPreference = 'Stop'

# Repo root: walk up to the nearest .git. If absent, use cwd.
$root = (& git rev-parse --show-toplevel 2>$null) | Out-String
$root = $root.Trim()
if (-not $root) { $root = (Get-Location).Path }
Set-Location $root

$exit = 0
$fails = 0

function Resolve-RelativePath([string]$base, [string]$rel) {
    $rel = $rel -replace '^\./', ''
    $rel = $rel -replace '^/', ''
    if ([string]::IsNullOrEmpty($rel)) { return '.' }
    $combined = if ($base) { "$base/$rel" } else { $rel }
    $parts = $combined -split '/' | Where-Object { $_ -ne '' }
    $stack = New-Object System.Collections.Generic.Stack[string]
    foreach ($p in $parts) {
        if ($p -eq '.' -or [string]::IsNullOrEmpty($p)) { continue }
        if ($p -eq '..') {
            if ($stack.Count -gt 0) { $stack.Pop() | Out-Null }
            continue
        }
        $stack.Push($p)
    }
    if ($stack.Count -eq 0) { return '.' }
    return ($stack -join '/')
}

function Record-Fail([string]$file, [string]$target, [string]$resolved) {
    Write-Host ("[FAIL] {0}: broken link -> {1} (resolved: {2})" -f $file, $target, $resolved)
    $script:fails += 1
    $script:exit = 1
}

# 1. Collect .md files
$files = Get-ChildItem -Path . -Recurse -File -Filter '*.md' |
    Where-Object { $_.FullName -notmatch '[\\/](target|\.git|\.workbuddy|node_modules)[\\/]' }

if (-not $files -or $files.Count -eq 0) {
    Write-Host "[FAIL] no .md files found under $root"
    exit 1
}

# 2. Scan all .md files for inline links
$linkRegex = [regex]'\[(?<label>[^\]]*)\]\((?<target>[^)]+)\)'
foreach ($f in $files) {
    $relFile = $f.FullName.Substring((Get-Location).Path.Length + 1)
    $fileDir = Split-Path -Parent $relFile
    if ($fileDir -eq '.') { $fileDir = '' }
    $content = Get-Content -Raw -Path $relFile -ErrorAction SilentlyContinue
    if (-not $content) { continue }
    $matchesFound = $linkRegex.Matches($content)
    foreach ($m in $matchesFound) {
        $target = $m.Groups['target'].Value.Trim()
        if ($target -match '^(https?:|mailto:|ftp:|//)') { continue }
        if ([string]::IsNullOrEmpty($target) -or $target -like '#*') { continue }
        $noAnchor = ($target -split '#')[0]
        $clean = ($noAnchor -split '\s+')[0]
        if ([string]::IsNullOrEmpty($clean)) { continue }
        $resolved = Resolve-RelativePath $fileDir $clean
        if (-not (Test-Path -LiteralPath $resolved)) {
            Record-Fail $relFile $target $resolved
        }
    }
}

# 3. ADR index consistency
$adrIndex = 'docs/adr/README.md'
if (Test-Path -LiteralPath $adrIndex) {
    $content = Get-Content -Raw -Path $adrIndex
    $adrRegex = [regex]'\]\((?<id>[FMCNBTLR][0-9]{3}-[^)]+\.md)\)'
    $idMatches = $adrRegex.Matches($content)
    foreach ($m in $idMatches) {
        $id = $m.Groups['id'].Value
        $candidate = "docs/adr/$id"
        if (-not (Test-Path -LiteralPath $candidate)) {
            Record-Fail $adrIndex $id $candidate
        }
    }
}

# 4. .rules index consistency
$rulesIndex = '.rules/RULES.md'
if (Test-Path -LiteralPath $rulesIndex) {
    $content = Get-Content -Raw -Path $rulesIndex
    $rmatches = $linkRegex.Matches($content)
    foreach ($m in $rmatches) {
        $target = $m.Groups['target'].Value.Trim()
        if ($target -match '^\.\./') { continue }   # parent-relative, validated in step 2
        if ($target -match '^(https?:|mailto:|#)') { continue }
        $clean = $target -replace '^\./', ''
        if ([string]::IsNullOrEmpty($clean)) { continue }
        if (-not (Test-Path -LiteralPath $clean)) {
            Record-Fail $rulesIndex $target $clean
        }
    }
}

# 5. Summary
if ($exit -ne 0) {
    Write-Host ""
    Write-Host ("Check failed: {0} broken link(s) found across {1} .md files." -f $fails, $files.Count)
    exit 1
}
Write-Host ("[OK] no broken links among {0} .md files." -f $files.Count)
exit 0
