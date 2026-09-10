# SPDX-License-Identifier: MIT
# Copyright (c) 2026 WinTapNetAdapterCx contributors
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

function Assert-NotText($Path, [string]$Pattern, [string]$Message) {
    $content = Get-Content -Raw -Path (Join-Path $root $Path)
    if ($content -match $Pattern) {
        throw $Message
    }
}

Assert-Text "Cargo.toml" 'panic\s*=\s*"abort"' "Cargo profiles must abort on panic."
Assert-Text "rust-toolchain.toml" 'channel\s*=\s*"1\.85\.0"' "Rust toolchain pin is missing."
Assert-Text "crates\netadaptercx-sys\build.rs" 'NETADAPTERCX_VERSION:\s*&str\s*=\s*"2\.5"' "NetAdapterCx binding version must be pinned to 2.5."
Assert-Text "crates\netadaptercx-sys\build.rs" 'allowlist_function\("Net\.\*"\)' "NetAdapterCx binding generation must include Net* functions."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'export_name\s*=\s*"DriverEntry"' "Rust driver must export DriverEntry."
Assert-Text "crates\wintap-bus-driver\src\lib.rs" 'WdfFdoInitSetDefaultChildListConfig' "Bus driver must configure a KMDF child list."
Assert-Text "crates\wintap-bus-driver\src\lib.rs" 'WdfPdoInitAssignDeviceID' "Bus driver must assign GUID-keyed child identities."
Assert-Text "crates\wintap-netadaptercx-driver\wintap_netadaptercx_driver.inx" 'WINTAPBUS\\WinTapChild' "Child package must bind only the dynamic bus child identity."
Assert-Text "crates\wintap-bus-driver\wintap_bus_driver.inx" 'Root\\WinTapBus' "Bus package must define the bus-parent identity."
Assert-Text "CMakeLists.txt" 'build-rust-driver.ps1' "CMake must package the Rust driver with cargo-wdk."
Assert-Text "scripts\build-rust-driver.ps1" '& cargo @arguments' "The Rust package wrapper must invoke cargo."
Assert-Text "scripts\build-rust-driver.ps1" '"wdk", "build"' "The Rust package wrapper must invoke cargo-wdk."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'injection_lock:\s*WDFSPINLOCK' "RX injection queue must have a dedicated lock."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'capture_lock:\s*WDFSPINLOCK' "TX capture queue must have a dedicated lock."
Assert-NotText "crates\wintap-netadaptercx-driver\src\lib.rs" 'frame_lock' "Frame queues must not share a frame lock."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'wait_state:\s*AtomicU8' "Adaptive waits must use atomic publication state."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'WdfRequestMarkCancelableEx' "Adaptive waits must mark requests before publication."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'WdfRequestUnmarkCancelable' "Adaptive wait claimants must unmark requests."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)else if status == STATUS_CANCELLED \{\s*// MarkCancelableEx does not invoke.*?finish_wait\(state\);\s*complete_request\(request, STATUS_CANCELLED\);' "Mark-time cancellation must be completed by registration."

$driverSource = Get-Content -Raw -Path (Join-Path $root "crates\wintap-netadaptercx-driver\src\lib.rs")
$advance = [regex]::Match(
    $driverSource,
    'extern "C" fn evt_packet_queue_advance(?s:.*?)\nfn inject_receive_frames'
)
if (-not $advance.Success -or $advance.Value -match 'InstanceStateGuard|WdfSpinLockAcquire') {
    throw "Packet queue advance must not acquire the shared lifecycle lock."
}

Write-Host "Rust migration scaffold validation passed."
