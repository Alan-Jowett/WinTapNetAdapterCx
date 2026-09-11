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
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'rx_callback_leases:\s*AtomicU64' "RX packet callbacks must take a callback-lifetime lease."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'tx_callback_leases:\s*AtomicU64' "TX packet callbacks must take a callback-lifetime lease."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'DATAPATH_CLOSED_ANY:\s*u64\s*=\s*\r?\n?\s*DATAPATH_CLOSED_POWER \| DATAPATH_CLOSED_HARDWARE \| DATAPATH_CLOSED_OWNER' "Callback-lifetime admission and lease count must share one atomic word with per-scope closers."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'wait_state:\s*AtomicU64' "Adaptive waits must use atomic publication state."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'WAIT_RECORD_SATISFIED_MASK:\s*u64' "The adaptive wait state and its satisfied mask must share one atomic word."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'WAIT_RECORD_SEQUENCE_SHIFT:\s*u32' "The adaptive wait record must carry a monotonic registration sequence."
Assert-NotText "crates\wintap-netadaptercx-driver\src\lib.rs" 'wait_ready_satisfied' "The satisfied mask must not live in a word separate from the wait state."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'wait_request:\s*AtomicPtr<c_void>' "Adaptive waits must publish exactly one request slot."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'wait_cancel_handoff:\s*AtomicU8' "Adaptive wait cancellation must use an exact-once handoff word."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'WdfRequestMarkCancelableEx' "Adaptive waits must mark requests before publication."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" 'WdfRequestUnmarkCancelable' "Adaptive wait claimants must unmark requests."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)if status == STATUS_CANCELLED \{\s*// MarkCancelableEx does not invoke.*?finish_wait\(state\);\s*complete_request\(request, STATUS_CANCELLED\);' "Mark-time cancellation must be completed by registration."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)extern "C" fn evt_file_cleanup.*?DatapathQuiesceGuard::acquire\(state, DATAPATH_CLOSED_OWNER\);.*?clear_frame_queues\(state\);' "Owner cleanup must drain packet-callback leases under its own closer before clearing frame queues."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)extern "C" fn evt_file_cleanup.*?compare_exchange\(\s*INSTANCE_OPEN,\s*INSTANCE_OWNER_CLOSING,.*?reopen_frame_queues\(state\);.*?compare_exchange\(\s*INSTANCE_OWNER_CLOSING,\s*INSTANCE_OPEN,.*?if resumed \{.*?resume_manual_queue\(read_queue\);' "Owner cleanup must claim and revalidate an owner-specific lifecycle state before publishing OPEN and resuming the manual queue."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)fn evt_device_d0_exit.*?quiesce_datapath_callbacks\(state, DATAPATH_CLOSED_POWER\)' "D0 exit must quiesce packet callbacks under the power closer."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)fn evt_device_release_hardware.*?quiesce_datapath_callbacks\(state, DATAPATH_CLOSED_HARDWARE\)' "Release hardware must quiesce packet callbacks under the hardware closer."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)extern "C" fn evt_io_read.*?acquire_capture_lease\(state\).*?owner_generation\.load\(Ordering::Acquire\).*?dequeue_capture_frame\(state\)' "Passive READ delivery must hold the capture lease across its owner snapshot and dequeue."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)extern "C" fn evt_read_completion_work_item.*?acquire_capture_lease\(state\).*?owner_generation\.load\(Ordering::Acquire\).*?WdfIoQueueRetrieveNextRequest.*?dequeue_capture_frame\(state\)' "Passive capture-drain work must hold the capture lease across its owner snapshot, dequeue, and delivery."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)fn requeue_capture_frame_and_schedule_wait.*?acquire_capture_lease\(state\)' "Capture requeue must revalidate ownership under a callback-lifetime lease."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)fn deliver_transmit_packet_to_read.*?LegacyDirectReadGuard::try_acquire\(state\)' "Direct TX delivery must not block a packet callback on the legacy read lock."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)let mut control_queue_attributes = WDF_OBJECT_ATTRIBUTES \{.*?WdfExecutionLevelPassive,.*?WdfSynchronizationScopeQueue,' "The control queue must serialize its request handlers with EvtIoStop at PASSIVE_LEVEL."
Assert-Text "crates\wintap-netadaptercx-driver\src\lib.rs" '(?s)fn handle_enable_adaptive_polling.*?LegacyDirectReadGuard::try_acquire\(state\)' "Control dispatch must not block on a driver lock while holding the queue synchronization lock."

$driverSource = Get-Content -Raw -Path (Join-Path $root "crates\wintap-netadaptercx-driver\src\lib.rs")
$advance = [regex]::Match(
    $driverSource,
    'extern "C" fn evt_packet_queue_advance(?s:.*?)\nfn inject_receive_frames'
)
if (-not $advance.Success -or $advance.Value -match 'InstanceStateGuard|WdfSpinLockAcquire|WdfWaitLockAcquire') {
    throw "Packet queue advance must not acquire the shared lifecycle lock."
}
if ([regex]::Matches($advance.Value, 'acquire_(capture|receive)_lease\(state\)').Count -lt 2) {
    throw "Both packet queue advance directions must take a callback-lifetime lease."
}

$claim = [regex]::Match(
    $driverSource,
    'fn claim_wait_for_passive_completion(?s:.*?)\n/// Retires the wait record'
)
if (-not $claim.Success -or $claim.Value -match 'WdfRequestUnmarkCancelable|WdfRequestComplete|WdfSpinLockAcquire|WdfWaitLockAcquire') {
    throw "Adaptive wait claiming from packet queue advance must only make an atomic claim and schedule passive work."
}
if ($claim.Value -notmatch 'wait_record_satisfied\(observed\) \| satisfied,\s*\r?\n\s*WAIT_SCHEDULED,') {
    throw "A wait claim must publish its satisfied mask in the same atomic word as the WAIT_SCHEDULED transition."
}

$finish = [regex]::Match($driverSource, 'fn finish_wait\(state: \*mut InstanceState\)(?s:.*?)\n\}')
if (-not $finish.Success -or $finish.Value -notmatch 'wait_record_sequence\(observed\) \+ 1') {
    throw "Retiring a wait record must advance its registration sequence."
}

$stop = [regex]::Match($driverSource, 'extern "C" fn evt_io_stop(?s:.*?)\nfn forward_request')
if (-not $stop.Success -or $stop.Value -notmatch 'cancel_wait_request_for_teardown\(state, request\)') {
    throw "EvtIoStop must claim only the wait request it was given."
}
if ($stop.Value -match 'WdfRequestStopAcknowledge|acknowledge_stopped_request') {
    throw "EvtIoStop must not acknowledge a control request that a terminal owner can still be completing."
}
$stopWaitBranch = [regex]::Match(
    $stop.Value,
    'if wait_request_is_active\(state, request\) \{(?s:.*?)\n    \}'
)
if (-not $stopWaitBranch.Success -or $stopWaitBranch.Value -match 'acknowledge_stopped_request|complete_request') {
    throw "EvtIoStop must not acknowledge or complete a wait request whose completion another path owns."
}

$unmark = [regex]::Match($driverSource, 'fn unmark_and_complete_wait\((?s:.*?)\nfn cancel_wait_for_teardown')
if (-not $unmark.Success -or $unmark.Value -match '(?s)complete_wait_response\(request[^\)]*\);\s*finish_wait\(state\)') {
    throw "A resolved terminal wait owner must retire the wait record before completing the request."
}
if ($unmark.Value -notmatch '(?s)finish_wait\(state\);\s*complete_wait_response\(request') {
    throw "A successful unmark must retire the wait record before completing the request."
}

Write-Host "Rust migration scaffold validation passed."
