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

function Invoke-GitText([string[]]$Arguments) {
    $output = & git @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $LASTEXITCODE."
    }
    return @($output)
}

$root = (Invoke-GitText @("rev-parse", "--show-toplevel")).Trim()
Set-Location $root
$policyManifest = Import-PowerShellDataFile (Join-Path $root "scripts/spdx-policy.psd1")

$slashExtensions = @(".rs", ".c", ".h", ".cpp", ".hpp")
$hashExtensions = @(".ps1", ".psm1", ".psd1", ".sh", ".yml", ".yaml", ".toml", ".ini", ".txt")
$semicolonExtensions = @(".inx")
$markdownExtensions = @(".md")
$specialHashFiles = @(".gitignore", ".gitattributes", "CMakeLists.txt", "Cargo.lock")
$specialHashFiles += @("hooks/pre-commit", "crates/wdk-sys/Cargo.toml.orig")

function Get-Policy([string]$Path) {
    $name = [IO.Path]::GetFileName($Path)
    $extension = [IO.Path]::GetExtension($Path).ToLowerInvariant()
    $normalizedPath = $Path.Replace("\", "/")
    if (@($policyManifest.ExplicitExclusions | Where-Object {
        $normalizedPath -like $_.Pattern -or $name -like $_.Pattern
    }).Count -gt 0) {
        return "excluded"
    }
    if (@($policyManifest.BinaryExtensions | Where-Object {
        $extension -eq $_.Extension
    }).Count -gt 0) {
        return "excluded"
    }
    if ($slashExtensions -contains $extension) {
        if (@($policyManifest.VendoredDualLicensePatterns | Where-Object {
            $normalizedPath -like $_
        }).Count -gt 0) {
            return "slash-dual"
        }
        return "slash"
    }
    if ($hashExtensions -contains $extension -or
        $specialHashFiles -contains $name -or
        $specialHashFiles -contains $Path.Replace("\", "/")) { return "hash" }
    if ($semicolonExtensions -contains $extension) { return "semicolon" }
    if ($markdownExtensions -contains $extension) { return "markdown" }
    return $null
}

function Read-File([string]$Path) {
    if ($Staged) {
        return @(Invoke-GitText @("show", ":$Path"))
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
    if ($index -lt $Lines.Count -and $Lines[$index] -match "^#.*(?:coding|encoding)[:=]") {
        $index++
    }
    if ($Policy -eq "markdown" -and $index -eq 0 -and $Lines.Count -ge 1 -and $Lines[0] -eq "---") {
        $closing = [Array]::IndexOf($Lines, "---", 1)
        if ($closing -lt 0) { throw "Missing YAML front matter terminator in $Path." }
        $index = $closing + 1
    }

    $expected = switch ($Policy) {
        "slash" { "// SPDX-License-Identifier: MIT" }
        "slash-dual" { "// SPDX-License-Identifier: MIT OR Apache-2.0" }
        "hash" { "# SPDX-License-Identifier: MIT" }
        "semicolon" { "; SPDX-License-Identifier: MIT" }
        "markdown" { "<!-- SPDX-License-Identifier: MIT" }
    }
    if ($index -ge $Lines.Count -or $Lines[$index].TrimEnd("`r") -ne $expected) {
        Write-Error "Missing or misplaced SPDX header in $Path. Expected '$expected' after required preamble."
        return $false
    }
    if ($Policy -eq "slash-dual" -and
        (($index + 2) -ge $Lines.Count -or
         $Lines[$index + 1].TrimEnd("`r") -ne "// Copyright (c) Microsoft Corporation" -or
         $Lines[$index + 2].TrimEnd("`r") -ne "// License: MIT OR Apache-2.0")) {
        Write-Error "Malformed dual-license SPDX header in $Path."
        return $false
    }
    if ($Policy -eq "markdown" -and
        (($index + 1) -ge $Lines.Count -or
         $Lines[$index + 1].TrimEnd("`r") -ne "  Copyright (c) 2026 WinTapNetAdapterCx contributors -->")) {
        Write-Error "Malformed Markdown SPDX header in $Path. Expected the complete two-line header block."
        return $false
    }
    return $true
}

if ($All) {
    $paths = @(Invoke-GitText @("ls-files"))
} else {
    $paths = @(Invoke-GitText @("diff", "--cached", "--name-only", "--diff-filter=ACMRT"))
}

$failed = $false
if ($All) {
    foreach ($exclusion in $policyManifest.ExplicitExclusions) {
        Write-Host "EXCLUDED: $($exclusion.Pattern) ($($exclusion.Reason))"
    }
    foreach ($exclusion in $policyManifest.BinaryExtensions) {
        Write-Host "EXCLUDED: *$($exclusion.Extension) ($($exclusion.Reason))"
    }
}
foreach ($path in $paths) {
    $policy = Get-Policy $path
    if ($null -eq $policy) {
        Write-Error "Unclassified tracked file $path. Add a policy or explicit exclusion."
        $failed = $true
        continue
    }
    if (-not (Test-Header $path (Read-File $path) $policy)) {
        $failed = $true
    }
}
if ($failed) { exit 1 }
