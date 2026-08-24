# SPDX-License-Identifier: MIT
# Copyright (c) 2026 WinTapNetAdapterCx contributors
[CmdletBinding()]
param(
    [switch]$CleanupLegacy,

    [string]$DiagnosticsPath = ".\artifacts\wintap-legacy-migration"
)

$ErrorActionPreference = "Stop"
$legacyHardwareIds = @("ROOT\WinTapRust", "ROOT\WinTapRust2")

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run legacy migration from an elevated administrator PowerShell session."
}

New-Item -ItemType Directory -Path $DiagnosticsPath -Force | Out-Null
$DiagnosticsPath = (Resolve-Path -LiteralPath $DiagnosticsPath).Path

$legacy = @(
    Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction Stop | Where-Object {
        $hardwareIds = @($_.HardwareID | ForEach-Object { [string]$_ })
        @($hardwareIds | Where-Object { $legacyHardwareIds -contains $_ }).Count -gt 0
    } | ForEach-Object {
        [pscustomobject]@{
            InstanceId = [string]$_.PNPDeviceID
            HardwareIds = @($_.HardwareID | ForEach-Object { [string]$_ })
            Name = [string]$_.Name
        }
    }
)
$legacy | ConvertTo-Json -Depth 4 | Out-File `
    -LiteralPath (Join-Path $DiagnosticsPath "legacy-devices-before.json") -Encoding utf8 -Force

if (-not $CleanupLegacy) {
    Write-Host "Detected $($legacy.Count) legacy WinTap root device(s). No mutation was requested."
    exit 0
}

foreach ($device in $legacy) {
    $output = & pnputil.exe /remove-device $device.InstanceId 2>&1
    $output | Out-File `
        -LiteralPath (Join-Path $DiagnosticsPath ("remove-" + ($device.InstanceId -replace '[\\/:*?`"<>|]', '_') + ".txt")) `
        -Encoding utf8 -Force
    if ($LASTEXITCODE -ne 0) {
        throw "pnputil failed to remove legacy instance $($device.InstanceId) with exit code $LASTEXITCODE."
    }
}

$remaining = @(
    Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction Stop | Where-Object {
        $hardwareIds = @($_.HardwareID | ForEach-Object { [string]$_ })
        @($hardwareIds | Where-Object { $legacyHardwareIds -contains $_ }).Count -gt 0
    }
)
$remaining | ConvertTo-Json -Depth 4 | Out-File `
    -LiteralPath (Join-Path $DiagnosticsPath "legacy-devices-after.json") -Encoding utf8 -Force
if ($remaining.Count -ne 0) {
    throw "Legacy WinTap root devices remain after explicit cleanup."
}

Write-Host "Removed $($legacy.Count) legacy WinTap root device(s)."
