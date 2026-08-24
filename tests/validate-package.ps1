param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("x64", "ARM64")]
    [string]$Architecture,

    [string]$Configuration = "Release",

    [string]$PackageRoot = ".\out\cmake",

    [string]$PackageDirectory
)

$ErrorActionPreference = "Stop"

$artifactRoot = if ($PackageDirectory) {
    (Resolve-Path -LiteralPath $PackageDirectory).Path
} else {
    $target = if ($Architecture -eq "x64") {
        "x86_64-pc-windows-msvc"
    } else {
        "aarch64-pc-windows-msvc"
    }
    $profile = if ($Configuration -eq "Debug") { "debug" } else { "release" }
    Join-Path $PackageRoot "$Architecture\package\$Architecture\$Configuration"
}
$childDriver = Join-Path $artifactRoot "wintap_netadaptercx_driver.sys"
$childInf = Join-Path $artifactRoot "wintap_netadaptercx_driver.inf"
$busDriver = Join-Path $artifactRoot "wintap_bus_driver.sys"
$busInf = Join-Path $artifactRoot "wintap_bus_driver.inf"

if (-not (Test-Path -LiteralPath $childDriver -PathType Leaf)) {
    throw "TAP-child driver artifact was not produced: $childDriver"
}
if (-not (Test-Path -LiteralPath $busDriver -PathType Leaf)) {
    throw "Bus driver artifact was not produced: $busDriver"
}
foreach ($inf in @($childInf, $busInf)) {
    if (-not (Test-Path -LiteralPath $inf -PathType Leaf)) {
        throw "INF artifact is missing: $inf"
    }
}

$childInfText = Get-Content -Raw -LiteralPath $childInf
$busInfText = Get-Content -Raw -LiteralPath $busInf
foreach ($required in @("CatalogFile", "WinTapChild", "WINTAPBUS\WinTapChild", "NTamd64", "NTarm64")) {
    if ($childInfText -notmatch [regex]::Escape($required)) {
        throw "TAP-child INF is missing the required package declaration: $required"
    }
}
foreach ($required in @("CatalogFile", "WinTapBus", "ROOT\WinTapBus", "NTamd64", "NTarm64")) {
    if ($busInfText -notmatch [regex]::Escape($required)) {
        throw "Bus INF is missing the required package declaration: $required"
    }
}
if ($childInfText -match 'ROOT\\WinTapRust' -or $busInfText -match 'ROOT\\WinTapRust') {
    throw "Legacy WinTapRust root identities must not remain in dynamic runtime package INF files."
}
foreach ($catalog in @(
        (Join-Path $artifactRoot "wintap_netadaptercx_driver.cat"),
        (Join-Path $artifactRoot "wintap_bus_driver.cat")
    )) {
    if (-not (Test-Path -LiteralPath $catalog -PathType Leaf)) {
        throw "Catalog artifact was not produced: $catalog"
    }
}

Write-Host "Package artifacts validated for $Architecture/$Configuration."
