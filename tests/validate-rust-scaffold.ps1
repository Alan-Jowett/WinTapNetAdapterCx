[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

function Assert-Text($Path, [string]$Pattern, [string]$Message) {
    $content = Get-Content -Raw -Path (Join-Path $root $Path)
    if ($content -notmatch $Pattern) {
        throw $Message
    }
}

Assert-Text "Cargo.toml" 'panic\s*=\s*"abort"' "Cargo profiles must abort on panic."
Assert-Text "rust-toolchain.toml" 'channel\s*=\s*"1\.85\.0"' "Rust toolchain pin is missing."
Assert-Text "crates\netadaptercx-sys\build.rs" 'NETADAPTERCX_VERSION:\s*&str\s*=\s*"2\.5"' "NetAdapterCx binding version must be pinned to 2.5."
Assert-Text "crates\netadaptercx-sys\build.rs" 'allowlist_function\("Net\.\*"\)' "NetAdapterCx binding generation must include Net* functions."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'export_name\s*=\s*"DriverEntry"' "Rust driver must export DriverEntry."
Assert-Text "crates\wintap-netadaptercx-driver\wintap_netadaptercx_driver.inx" 'Root\\WinTapRust' "Rust package must use the WinTapRust hardware ID."
Assert-Text "CMakeLists.txt" 'build-rust-driver.ps1' "CMake must package the Rust driver with cargo-wdk."
Assert-Text "scripts\build-rust-driver.ps1" '& cargo @arguments' "The Rust package wrapper must invoke cargo."
Assert-Text "scripts\build-rust-driver.ps1" '"wdk", "build"' "The Rust package wrapper must invoke cargo-wdk."

Write-Host "Rust migration scaffold validation passed."
