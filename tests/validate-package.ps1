param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("x64", "ARM64")]
    [string]$Architecture,

    [string]$Configuration = "Release",

    [string]$PackageRoot = ".\out\rust-target",

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
    Join-Path $PackageRoot "$target\$profile\wintap_netadaptercx_driver_package"
}
$driver = Join-Path $artifactRoot "wintap_netadaptercx_driver.sys"
$inf = Join-Path $artifactRoot "wintap_netadaptercx_driver.inf"

if (-not (Test-Path -LiteralPath $driver -PathType Leaf)) {
    throw "Driver artifact was not produced: $driver"
}
if (-not (Test-Path -LiteralPath $inf -PathType Leaf)) {
    throw "INF artifact is missing: $inf"
}

$infText = Get-Content -Raw -LiteralPath $inf
foreach ($required in @(
    "CatalogFile",
    "WinTap_CopyFiles",
    "WinTap_Service",
    "WinTapRust",
    "ROOT\WinTapRust",
    "ROOT\WinTapRust2",
    "NTamd64",
    "NTarm64"
)) {
    if ($infText -notmatch [regex]::Escape($required)) {
        throw "INF is missing the required package declaration: $required"
    }
}

$catalog = Join-Path $artifactRoot "wintap_netadaptercx_driver.cat"
if (-not (Test-Path -LiteralPath $catalog -PathType Leaf)) {
    throw "Catalog artifact was not produced: $catalog"
}

Write-Host "Package artifacts validated for $Architecture/$Configuration."
