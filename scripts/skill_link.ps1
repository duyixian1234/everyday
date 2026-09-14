# Link the everyday agent skill into the host skills directory instead of copying it.
#
# A copy goes stale silently: editing the repo would not change what an agent
# actually loads. A link means there is one copy, and it is the one under version
# control. See docs/adr/F018-agent-skill-distribution.md.
#
# Idempotent: re-running replaces an existing link, and refuses to clobber a real
# directory that contains files (so a hand-made copy is never destroyed silently).

[CmdletBinding()]
param(
    [string]$Target = "$HOME/.agents/skills",
    [string]$Name = "everyday-cli"
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$source = Join-Path $repoRoot "skills/$Name"

if (-not (Test-Path -LiteralPath $source -PathType Container)) {
    Write-Error "source skill directory not found: $source"
    exit 1
}

if (-not (Test-Path -LiteralPath $Target -PathType Container)) {
    New-Item -ItemType Directory -Path $Target -Force | Out-Null
    Write-Host "created skills directory: $Target"
}

$linkPath = Join-Path $Target $Name

if (Test-Path -LiteralPath $linkPath) {
    $item = Get-Item -LiteralPath $linkPath -Force
    $isLink = $item.LinkType -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)

    if (-not $isLink) {
        # A real directory: only replace it if it holds nothing.
        $children = @(Get-ChildItem -LiteralPath $linkPath -Force)
        if ($children.Count -gt 0) {
            Write-Error @"
$linkPath is a real directory with $($children.Count) entries, not a link.
Refusing to delete it. If it is a hand-made copy of the skill, diff it against
$source first, then remove the COPY and re-run:
  Get-ChildItem -Recurse -Force '$linkPath' | Select-Object FullName
  Remove-Item -LiteralPath '$linkPath' -Recurse -Force
Do NOT run Remove-Item -Recurse on a junction: it would follow the link and
delete the repository's own files. Inspect with
  (Get-Item '$linkPath' -Force).LinkType
first — this script only deletes a path once it has confirmed it is a link.
"@
            exit 1
        }
        Remove-Item -LiteralPath $linkPath -Force
    }
    else {
        Write-Host "replacing existing link: $linkPath"
        # Remove the link only; never recurse through it into the source.
        if ($item.PSIsContainer) {
            [IO.Directory]::Delete($linkPath, $false)
        }
        else {
            Remove-Item -LiteralPath $linkPath -Force
        }
    }
}

$onWindows = $IsWindows -or ($PSVersionTable.PSEdition -eq "Desktop")
if ($onWindows) {
    # Junction: no admin rights or developer mode needed, unlike a symlink.
    & cmd /c mklink /J "$linkPath" "$source" | Out-Null
    if ($LASTEXITCODE -ne 0) { Write-Error "mklink /J failed ($LASTEXITCODE)"; exit 1 }
}
else {
    New-Item -ItemType SymbolicLink -Path $linkPath -Target $source | Out-Null
}

$created = Get-Item -LiteralPath $linkPath -Force
Write-Host "linked: $linkPath"
Write-Host "     -> $source"
Write-Host ""
Write-Host "Edits under skills/$Name are now live in every agent session that loads"
Write-Host "the skill. There is no staging gate: a work-in-progress edit is visible"
Write-Host "immediately. See docs/adr/F018-agent-skill-distribution.md."
