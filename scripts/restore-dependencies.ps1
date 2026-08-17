param(
    [ValidateSet("x64", "ARM64")]
    [string]$Architecture = "x64",
    [string]$Version = "10.0.28000.2526",
    [string]$OutputDirectory = "$PSScriptRoot\..\out\packages"
)

$ErrorActionPreference = "Stop"

$nuget = Get-Command nuget.exe -ErrorAction SilentlyContinue
if (-not $nuget) {
    throw "nuget.exe is required to restore the WDK and SDK packages."
}

$wdkPackage = "Microsoft.Windows.WDK.$Architecture"
$sdkBasePackage = "Microsoft.Windows.SDK.CPP"
$sdkPackage = "Microsoft.Windows.SDK.CPP.$Architecture"

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

& $nuget.Source install $wdkPackage -Version $Version -OutputDirectory $OutputDirectory -NonInteractive
if ($LASTEXITCODE -ne 0) {
    throw "Failed to restore $wdkPackage $Version."
}

& $nuget.Source install $sdkBasePackage -Version $Version -OutputDirectory $OutputDirectory -NonInteractive
if ($LASTEXITCODE -ne 0) {
    throw "Failed to restore $sdkBasePackage $Version."
}

& $nuget.Source install $sdkPackage -Version $Version -OutputDirectory $OutputDirectory -NonInteractive
if ($LASTEXITCODE -ne 0) {
    throw "Failed to restore $sdkPackage $Version."
}

Write-Host "Restored $wdkPackage, $sdkBasePackage, and $sdkPackage version $Version to $OutputDirectory."
