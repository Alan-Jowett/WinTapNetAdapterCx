param(
    [Parameter(Mandatory = $true)]
    [string]$WdkVersion,

    [ValidateSet("x64", "ARM64")]
    [string]$Architecture = "x64"
)

$ErrorActionPreference = "Stop"

$packageRoot = Join-Path $env:USERPROFILE ".nuget\packages"
$wdkPackage = if ($Architecture -eq "ARM64") {
    "microsoft.windows.wdk.arm64"
} else {
    "microsoft.windows.wdk.x64"
}
$toolRoot = Join-Path $packageRoot "$wdkPackage\$WdkVersion\c\bin\10.0.28000.0"
$hostToolRoot = Join-Path $toolRoot "x64"
$stampInf = Join-Path $hostToolRoot "stampinf.exe"

if (-not (Test-Path -LiteralPath $stampInf -PathType Leaf)) {
    throw "stampinf.exe was not found in the restored WDK package: $stampInf"
}

if ($env:GITHUB_PATH) {
    $hostToolRoot | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
}

$env:Path = "$hostToolRoot;$env:Path"
if (-not (Get-Command stampinf.exe -ErrorAction SilentlyContinue)) {
    throw "stampinf.exe is not discoverable after provisioning: $hostToolRoot"
}

$apiValidator = Join-Path $toolRoot "x86\ApiValidator.exe"
$apiValidatorEnabled = Test-Path -LiteralPath $apiValidator -PathType Leaf
if (-not $apiValidatorEnabled) {
    Write-Warning "ApiValidator.exe is not present in the restored WDK package; API validation remains deferred."
}

if ($env:GITHUB_ENV) {
    "WINTAP_API_VALIDATOR_ENABLE=$($apiValidatorEnabled.ToString().ToLowerInvariant())" |
        Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
}

Write-Host "WDK tools provisioned from $hostToolRoot"
Write-Host "stampinf.exe: $stampInf"
