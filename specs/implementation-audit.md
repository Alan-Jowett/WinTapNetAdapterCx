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
- `specs/current-status.md`
- `specs/maintenance-drift-report.md`
- Local validation result: artifact, PowerShell syntax, and package-shape
  validation passed.
- Local build result: Visual Studio 18 with WDK/SDK NuGet
  `10.0.28000.2526` successfully built and test-signed both x64 and ARM64
  targets through direct MSBuild and CMake package targets.

## Forward traceability

| Approved area | Current implementation | Result |
| --- | --- | --- |
| REQ-001 adapter | Creates a WDF device and NetAdapterCx adapter with Ethernet MTU/link capabilities and packet queues; control-device teardown is synchronized | PARTIAL |
| REQ-002 TAP I/O | Named WDF control device, overlapped-compatible read/write dispatch, bounded directional frame storage, fragment bounds checks, and TX/RX ring copying are implemented | PARTIAL |
| REQ-003 lifecycle | File cleanup, D0 power callbacks, release-hardware teardown, callback lifetime gating, and write-drain quiescence are implemented; framework pause/restart and full runtime coverage remain incomplete | PARTIAL |
| REQ-004 platform | CMake presets and INF declare x64/ARM64 and Windows 10 minimum; both targets build and pass API validation | PASS |
| REQ-005 security | Control device uses an administrator-only SDDL and exclusive WDF open; malformed frame sizes are rejected | PASS |
| REQ-006 verification | Hosted artifact/syntax/package validation and an administrator-only overlapped-I/O harness exist; privileged packet-path, lifecycle, power, and verifier execution remain unavailable | PARTIAL |
| REQ-007 build | CMake invokes MSBuild; WDK/SDK packages are pinned, restored, isolated by platform, and consumed by the project | PASS |

## Adversarial findings

1. **Network bridge remains partially verified:** Callback acquisition now uses
   synchronized global-context lookup, active-callback tracking, and fragment
   bounds checks. RX ring ownership/index semantics remain unverified.
2. **Lifecycle remains incomplete:** D0 entry/exit and teardown quiescence are
   implemented, but no separately verified NetAdapterCx pause/restart callback
   contract was available in the selected baseline.
3. **Queue backpressure remains partial:** Full-queue writes use manual WDF
   queues and a synchronized passive drain work item; privileged saturation and
   removal races remain untested.
4. **Verification remains incomplete:** Direct MSBuild and CMake builds pass for
   x64 and ARM64, but installation, packet exchange, power, removal, catalog,
   and Driver Verifier execution remain deferred.

## Audit verdict

**REVISE-IMPLEMENTATION.** The approved maintenance corrections are applied
and local build/package evidence is available for x64 and ARM64, but privileged
runtime tests, catalog/install validation, Driver Verifier coverage, and RX
ring ownership verification remain incomplete.
