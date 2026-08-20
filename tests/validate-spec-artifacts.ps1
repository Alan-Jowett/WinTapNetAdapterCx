$ErrorActionPreference = "Stop"

$required = @(
    "specs\requirements.md",
    "specs\design.md",
    "specs\validation.md",
    "specs\audit.md",
    "specs\current-status.md",
    "CMakeLists.txt",
    "CMakePresets.json",
    "crates\wintap-netadaptercx-driver\wintap_netadaptercx_driver.inx",
    "crates\wintap-netadaptercx-driver\src\lib.rs",
    "tests\run-wintap-harness.ps1",
    "tests\run-wintap-dual-adapter-harness.ps1",
    "tests\validate-package.ps1",
    "scripts\prepare-wdk-tools.ps1",
    "scripts\build-rust-driver.ps1"
)

foreach ($path in $required) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required artifact is missing: $path"
    }
}

$cmake = Get-Content -Raw CMakeLists.txt
if ($cmake -notmatch 'build-rust-driver.ps1' -or
    $cmake -notmatch 'CARGO_WDK_EXECUTABLE' -or
    $cmake -notmatch 'restore-dependencies.ps1') {
    throw "CMake does not enforce the required Rust/cargo-wdk workflow."
}

$harness = Get-Content -Raw tests\run-wintap-harness.ps1
if ($harness -notmatch 'CreateFile' -or
    $harness -notmatch 'CancelIoEx' -or
    $harness -notmatch 'Administrator' -or
    $harness -notmatch 'Extended' -or
    $harness -notmatch 'exclusive') {
    throw "The overlapped administrator harness is incomplete."
}

$dualHarness = Get-Content -Raw tests\run-wintap-dual-adapter-harness.ps1
if ($dualHarness -notmatch 'devcon.exe' -or
    $dualHarness -notmatch 'ROOT\\WinTapRust2' -or
    $dualHarness -notmatch 'New-NetNeighbor' -or
    $dualHarness -notmatch 'New-NetRoute' -or
    $dualHarness -notmatch 'ICMPv6' -or
    $dualHarness -notmatch 'CancelIoEx' -or
    $dualHarness -notmatch 'Win32_PnPEntity' -or
    $dualHarness -notmatch 'Assert-ArpFrame' -or
    $dualHarness -notmatch 'Assert-IPv6Icmpv6Structure' -or
    $dualHarness -notmatch 'Assert-NoReflectedInjection' -or
    $dualHarness -notmatch 'InjectedFrameKeys' -or
    $dualHarness -notmatch 'Convert-FrameToKey' -or
    $dualHarness -notmatch 'Test-IPv6NeighborDiscoveryFrame' -or
    $dualHarness -notmatch 'New-RelayUnicastNeighborSolicitationFrame' -or
    $dualHarness -notmatch 'Assert-OwnerReopen' -or
    $dualHarness -notmatch 'OwnerReopenValidated' -or
    $dualHarness -notmatch 'ControlFrames' -or
    $dualHarness -notmatch '(?s)if \(\$script:PnpRemovalConfirmed\) \{.*?/delete-driver') {
    throw "The routed dual-adapter harness is incomplete."
}

$source = Get-Content -Raw crates\wintap-netadaptercx-driver\src\lib.rs
if ($source -notmatch 'export_name = "DriverEntry"' -or
    $source -notmatch 'evt_driver_device_add' -or
    $source -notmatch 'NetPacketFilterFlagMulticast' -or
    $source -notmatch 'NetPacketFilterFlagAllMulticast' -or
    $source -notmatch 'NetPacketFilterFlagPromiscuous' -or
    $source -notmatch 'MaximumMulticastAddresses:\s*MAXIMUM_MULTICAST_ADDRESSES' -or
    $source -notmatch 'evt_set_receive_filter' -or
    $source -notmatch 'injection_queue' -or
    $source -notmatch 'capture_queue' -or
    $source -notmatch 'resume_manual_queues' -or
    $source -notmatch 'WdfIoQueueStart' -or
    $source -notmatch 'take_rx_notification' -or
    $source -notmatch 'set_Offset\(0\)' -or
    $source -notmatch 'set_Ignore\(0\)' -or
    $source -notmatch 'set_Ignore\(1\)') {
    throw "The Rust driver identity or receive-filter contract is incomplete."
}

$workflow = Get-Content -Raw .github\workflows\driver-validation.yml
if ($workflow -notmatch 'cargo-wdk' -or
    $workflow -notmatch 'wintap_package' -or
    $workflow -notmatch 'validate-package.ps1' -or
    $workflow -notmatch 'run-wintap-harness.ps1' -or
    $workflow -notmatch 'run-wintap-dual-adapter-harness.ps1') {
    throw "The workflow does not cover Rust driver packaging and package validation."
}

Write-Host "Specification and implementation artifacts are present."
