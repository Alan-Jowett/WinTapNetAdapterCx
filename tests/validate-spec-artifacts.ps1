$ErrorActionPreference = "Stop"

$required = @(
    "specs\requirements.md",
    "specs\design.md",
    "specs\validation.md",
    "specs\audit.md",
    "specs\current-status.md",
    "CMakeLists.txt",
    "CMakePresets.json",
    "driver\WinTapNetAdapterCx.vcxproj",
    "driver\WinTapNetAdapterCx.inf",
    "driver\wintap.cpp",
    "driver\wintap.h",
    "tests\run-wintap-harness.ps1",
    "tests\validate-package.ps1",
    "scripts\prepare-wdk-tools.ps1"
)

foreach ($path in $required) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required artifact is missing: $path"
    }
}

$cmake = Get-Content -Raw CMakeLists.txt
if ($cmake -notmatch 'Visual Studio' -or
    $cmake -notmatch 'MSBUILD_EXECUTABLE' -or
    $cmake -notmatch 'STAMPINF_EXECUTABLE') {
    throw "CMake does not enforce the required Visual Studio/MSBuild/StampInf workflow."
}

$inf = Get-Content -Raw driver\WinTapNetAdapterCx.inf
if ($inf -notmatch 'NTamd64' -or $inf -notmatch 'NTarm64') {
    throw "INF does not declare both supported architectures."
}

$harness = Get-Content -Raw tests\run-wintap-harness.ps1
if ($harness -notmatch 'CreateFile' -or
    $harness -notmatch 'CancelIoEx' -or
    $harness -notmatch 'Administrator' -or
    $harness -notmatch 'Extended' -or
    $harness -notmatch 'exclusive') {
    throw "The overlapped administrator harness is incomplete."
}

$source = Get-Content -Raw driver\wintap.cpp
if ($source -notmatch 'WintapValidateFragment' -or
    $source -notmatch 'g_ControlContextLock' -or
    $source -notmatch 'WintapWaitForWriteDrain' -or
    $source -notmatch 'EvtDeviceD0Exit') {
    throw "The driver lifetime, power, and fragment validation hardening is incomplete."
}

$workflow = Get-Content -Raw .github\workflows\driver-validation.yml
if ($workflow -notmatch 'prepare-wdk-tools.ps1' -or
    $workflow -notmatch 'cmake -S' -or
    $workflow -notmatch 'validate-package.ps1') {
    throw "The workflow does not cover WDK tool provisioning, CMake, and package validation."
}

Write-Host "Specification and implementation artifacts are present."
