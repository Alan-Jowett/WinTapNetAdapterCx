# WinTapNetAdapterCx Implementation Audit

**Workflow:** `/evolve`  
**Phase:** Phase 6 — Implementation Audit  
**Verdict:** REVISE-IMPLEMENTATION

## Evidence examined

- `CMakeLists.txt`
- `CMakePresets.json`
- `scripts/restore-dependencies.ps1`
- `driver/WinTapNetAdapterCx.vcxproj`
- `driver/wintap.cpp`
- `driver/wintap.h`
- `driver/WinTapNetAdapterCx.inf`
- `tests/run-wintap-harness.ps1`
- `tests/validate-spec-artifacts.ps1`
- Local validation result: artifact validation passed.
- Local build result: Visual Studio 18 with WDK/SDK NuGet
  `10.0.28000.2526` successfully built, test-signed, and API-validated both
  x64 and ARM64 targets through the CMake-driven MSBuild targets.

## Forward traceability

| Approved area | Current implementation | Result |
| --- | --- | --- |
| REQ-001 adapter | Creates a WDF device and NetAdapterCx adapter with Ethernet MTU/link capabilities and packet queues | PARTIAL |
| REQ-002 TAP I/O | Named WDF control device, overlapped-compatible read/write dispatch, bounded directional frame storage, and TX/RX ring copying are implemented | PARTIAL |
| REQ-003 lifecycle | File cleanup and release-hardware teardown stop the adapter, purge pending requests, reject new opens after removal, free queued frames, gate packet callbacks, and wait for active datapath callbacks; full adapter pause/restart state coverage remains incomplete | PARTIAL |
| REQ-004 platform | CMake presets and INF declare x64/ARM64 and Windows 10 minimum; both targets build and pass API validation | PASS |
| REQ-005 security | Control device uses an administrator-only SDDL and exclusive WDF open; malformed frame sizes are rejected | PASS |
| REQ-006 verification | Repository artifact validation and an administrator-only overlapped-I/O harness now exist; installation, packet-path, lifecycle, power, and verifier execution remain unavailable | PARTIAL |
| REQ-007 build | CMake invokes MSBuild; WDK/SDK packages are pinned, restored, isolated by platform, and consumed by the project | PASS |

## Adversarial findings

1. **Network bridge needs hardening:** `WintapEvtPacketQueueAdvance` drains TX
   rings into the user read queue and posts user writes into RX fragments.
   The bridge still relies on a global control context and lacks a fully
   reference-counted adapter state object; the control context now tracks active
   datapath callbacks and release-hardware waits for them to quiesce.
2. **Lifecycle incomplete:** Release-hardware teardown and packet queue
   start/stop callbacks now gate traffic and purge control requests, but the
   adapter is still started directly from device add and lacks complete
   pause/restart state transitions.
3. **Queue backpressure partial:** Writes that encounter a full user-to-stack
   frame queue are held in a cancellable manual WDF queue and a passive WDF
   work item resumes them after RX ring capacity is consumed; framework-side
   drops and pause/removal coordination remain incomplete.
4. **Verification incomplete:** Build-time WDK/API validation now passes for
   x64 and ARM64, and the harness covers administrator access, malformed
   writes, overlapped read cancellation, and valid writes. It has not been run
   against an installed driver and does not cover packet exchange, power, or
   Driver Verifier.

## Audit verdict

**REVISE-IMPLEMENTATION.** The packet-ring bridge is now present and the
implementation builds, test-signs, and API-validates for x64 and ARM64, but
adapter lifecycle synchronization, runtime tests, and verifier coverage remain
incomplete. Do not create a commit or present Phase 7 as approved until those
remaining requirements are implemented and tested.
