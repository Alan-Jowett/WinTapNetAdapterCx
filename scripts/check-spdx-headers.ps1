# SPDX-License-Identifier: MIT
# Copyright (c) 2026 WinTapNetAdapterCx contributors

[CmdletBinding()]
param(
    [switch]$All,
    [switch]$Staged
)

$ErrorActionPreference = "Stop"
if (($All -and $Staged) -or (-not $All -and -not $Staged)) {
    throw "Specify exactly one mode: -All or -Staged."
}

$root = (git rev-parse --show-toplevel).Trim()
Set-Location $root

$slashExtensions = @(".rs", ".c", ".h", ".cpp", ".hpp")
$hashExtensions = @(".ps1", ".psm1", ".sh", ".yml", ".yaml", ".toml", ".ini", ".txt")
$semicolonExtensions = @(".inx")
$markdownExtensions = @(".md")
$specialHashFiles = @(".gitignore", ".gitattributes", "CMakeLists.txt", "Cargo.lock")
$specialHashFiles += "hooks/pre-commit"
$explicitExclusions = @("CMakePresets.json", "LICENSE")

function Get-Policy([string]$Path) {
    $name = [IO.Path]::GetFileName($Path)
    $extension = [IO.Path]::GetExtension($Path).ToLowerInvariant()
    if ($explicitExclusions -contains $Path -or $explicitExclusions -contains $name) {
        return "excluded"
    }
    if ($slashExtensions -contains $extension) { return "slash" }
    if ($hashExtensions -contains $extension -or $specialHashFiles -contains $name) { return "hash" }
    if ($semicolonExtensions -contains $extension) { return "semicolon" }
    if ($markdownExtensions -contains $extension) { return "markdown" }
    return $null
}

function Read-File([string]$Path) {
    if ($Staged) {
        return @(git show ":$Path")
    }
    return @(Get-Content -LiteralPath $Path)
}

function Test-Header([string]$Path, [string[]]$Lines, [string]$Policy) {
    if ($Policy -eq "excluded") {
        Write-Output "EXCLUDED: $Path"
        return $true
    }
    if ($null -eq $Policy) {
        return $true
    }

    $index = 0
    if ($Lines.Count -gt 0 -and $Lines[0] -match "^#!") { $index = 1 }
    if ($Policy -eq "markdown" -and $index -eq 0 -and $Lines.Count -ge 1 -and $Lines[0] -eq "---") {
        $closing = [Array]::IndexOf($Lines, "---", 1)
        if ($closing -lt 0) { throw "Missing YAML front matter terminator in $Path." }
        $index = $closing + 1
    }

    $expected = switch ($Policy) {
        "slash" { "// SPDX-License-Identifier: MIT" }
        "hash" { "# SPDX-License-Identifier: MIT" }
        "semicolon" { "; SPDX-License-Identifier: MIT" }
        "markdown" { "<!-- SPDX-License-Identifier: MIT" }
    }
    if ($index -ge $Lines.Count -or $Lines[$index].TrimEnd("`r") -ne $expected) {
        Write-Error "Missing or misplaced SPDX header in $Path. Expected '$expected' after required preamble."
        return $false
    }
    return $true
}

if ($All) {
    $paths = @(git ls-files)
} else {
    $paths = @(git diff --cached --name-only --diff-filter=ACMR)
}

$failed = $false
foreach ($path in $paths) {
    $policy = Get-Policy $path
    if ($null -eq $policy -and $explicitExclusions -notcontains $path) {
        continue
    }
    if (-not (Test-Header $path (Read-File $path) $policy)) {
        $failed = $true
    }
}
if ($failed) { exit 1 }
