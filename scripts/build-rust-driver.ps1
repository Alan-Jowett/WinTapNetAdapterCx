param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("x64", "ARM64")]
    [string]$Architecture,

    [Parameter(Mandatory = $true)]
    [ValidateSet("Debug", "Release")]
    [string]$Configuration,

    [string]$Version = "10.0.28000.2526"
)

$ErrorActionPreference = "Stop"

$cargoArchitecture = if ($Architecture -eq "x64") { "amd64" } else { "arm64" }
$packageRoot = Join-Path $PSScriptRoot "..\out\packages\Microsoft.Windows.WDK.x64.$Version\c"
$toolDirectories = @(
    (Join-Path $packageRoot "bin\10.0.28000.0\x64"),
    (Join-Path $packageRoot "bin\10.0.28000.0\x86"),
    (Join-Path $packageRoot "tools\10.0.28000.0\x64")
)

foreach ($directory in $toolDirectories) {
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
        throw "Pinned WDK tool directory is missing: $directory"
    }
}

$env:PATH = ($toolDirectories -join ";") + ";" + $env:PATH
$arguments = @("wdk", "build", "--target-arch", $cargoArchitecture)
if ($Configuration -eq "Release") {
    $arguments += @("--profile", "release")
}

& cargo @arguments
if ($LASTEXITCODE -ne 0) {
    throw "cargo-wdk failed with exit code $LASTEXITCODE."
}
