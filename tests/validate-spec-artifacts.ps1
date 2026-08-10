$ErrorActionPreference = "Stop"

$required = @(
    "specs\requirements.md",
    "specs\design.md",
    "specs\validation.md",
    "specs\audit.md",
    "CMakeLists.txt",
    "CMakePresets.json",
    "driver\WinTapNetAdapterCx.vcxproj",
    "driver\WinTapNetAdapterCx.inf",
    "driver\wintap.cpp",
    "driver\wintap.h",
    "tests\run-wintap-harness.ps1"
)

foreach ($path in $required) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required artifact is missing: $path"
    }
}

$cmake = Get-Content -Raw CMakeLists.txt
if ($cmake -notmatch 'Visual Studio' -or $cmake -notmatch 'MSBuild') {
    throw "CMake does not enforce the required Visual Studio/MSBuild workflow."
}

$inf = Get-Content -Raw driver\WinTapNetAdapterCx.inf
if ($inf -notmatch 'NTamd64' -or $inf -notmatch 'NTarm64') {
    throw "INF does not declare both supported architectures."
}

$harness = Get-Content -Raw tests\run-wintap-harness.ps1
if ($harness -notmatch 'CreateFile' -or
    $harness -notmatch 'CancelIoEx' -or
    $harness -notmatch 'Administrator') {
    throw "The overlapped administrator harness is incomplete."
}

Write-Host "Specification and implementation artifacts are present."
