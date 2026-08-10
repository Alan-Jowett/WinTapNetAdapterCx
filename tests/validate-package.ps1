param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("x64", "ARM64")]
    [string]$Architecture,

    [string]$Configuration = "Release",

    [string]$PackageRoot = ".\out\driver",

    [string]$PackageDirectory
)

$ErrorActionPreference = "Stop"

$artifactRoot = if ($PackageDirectory) {
    (Resolve-Path -LiteralPath $PackageDirectory).Path
} else {
    Join-Path $PackageRoot "$Architecture\$Configuration"
}
$driver = Join-Path $artifactRoot "WinTapNetAdapterCx.sys"
$inf = if ($PackageDirectory) {
    Join-Path $artifactRoot "WinTapNetAdapterCx.inf"
} else {
    Join-Path (Resolve-Path ".\driver").Path "WinTapNetAdapterCx.inf"
}

if (-not (Test-Path -LiteralPath $driver -PathType Leaf)) {
    throw "Driver artifact was not produced: $driver"
}
if (-not (Test-Path -LiteralPath $inf -PathType Leaf)) {
    throw "INF artifact is missing: $inf"
}

$infText = Get-Content -Raw -LiteralPath $inf
foreach ($required in @("CatalogFile", "WinTap_CopyFiles", "WinTap_Service", "NTamd64", "NTarm64")) {
    if ($infText -notmatch [regex]::Escape($required)) {
        throw "INF is missing the required package declaration: $required"
    }
}

$catalog = Join-Path $artifactRoot "WinTapNetAdapterCx.cat"
if (Test-Path -LiteralPath $catalog -PathType Leaf) {
    Write-Host "Catalog artifact present: $catalog"
} else {
    Write-Warning "Catalog artifact is not produced because EnableInf2cat is disabled; signing/install validation is deferred."
}

Write-Host "Package artifacts validated for $Architecture/$Configuration."
