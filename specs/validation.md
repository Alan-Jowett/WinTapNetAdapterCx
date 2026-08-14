# WinTapNetAdapterCx Validation Specification

**Workflow:** `/evolve`  
**Phase:** Phase 2 — Specification Changes  
**Status:** Pending specification audit and user approval  
**Trace source:** `specs/requirements.md` and `specs/design.md`

## Acceptance criteria

| ID | Requirement | Validation |
| --- | --- | --- |
| VAL-001 | REQ-001 | Build and install the NetAdapterCx driver; verify one virtual Ethernet adapter appears with the expected capabilities and identity. |
| VAL-002 | REQ-002 | Write valid Ethernet frames through the device handle and verify delivery to the Windows networking stack; transmit frames through the stack and verify complete reads in user mode. |
| VAL-003 | REQ-003 | Exercise start, pause, restart, stop, surprise removal, owner close, process termination, and cancellation; verify no hangs, double completions, or leaked objects. |
| VAL-004 | REQ-004 | Build and execute the supported x64 and ARM64 packages on Windows 10 version 2004+ and reject unsupported platform combinations explicitly. |
| VAL-005 | REQ-005 | Verify non-administrator open/control attempts fail; verify malformed lengths and invalid I/O requests cannot corrupt memory or disclose data. |
| VAL-006 | REQ-006 | Run the complete build, install, packet-path, concurrency, cancellation, power, malformed-input, and cleanup suite with Driver Verifier-compatible settings. |
| VAL-007 | REQ-007 | Configure and build from a clean environment with CMake and the Visual Studio generator for x64 and ARM64; verify NuGet WDK/SDK dependencies resolve reproducibly and missing prerequisites fail at configuration. |
| VAL-008 | REQ-008 | Run the complete privileged ICMP Echo Request/Echo Reply round trip through the Ethernet/TAP handle using `192.0.2.1/30` and `192.0.2.2`; verify packet fields, checksums, stack completion, timeout behavior, and cleanup. |
| VAL-009 | REQ-009 | Execute the full REQ-008 assertion set in a GitHub-hosted Windows job and manually in a Hyper-V-capable Windows VM using the same test entry point; fail on unavailable privileged operations rather than skipping. |
| VAL-010 | REQ-010 | Build the Rust driver and generated NetAdapterCx bindings from a clean pinned environment for x64 and ARM64; verify binding regeneration, ABI/layout checks, panic-abort configuration, and package production. |
| VAL-011 | REQ-011 | Verify the repository, CMake targets, workflow, harness, and package validation contain no C/C++ driver source, project, INF, fallback, or selector. |
| VAL-012 | REQ-012 | Build each package and verify `wintap_netadaptercx_driver.inf`, `wintap_netadaptercx_driver.cat`, service `WinTapRust`, and hardware ID `ROOT\WinTapRust`. |
| VAL-013 | REQ-013 | Load the test-signed Rust package with NetAdapterCx verifier enabled and verify receive-filter capability initialization does not trigger `0x19E/0xB`. |

| Test | Coverage |
|---|---|
| TC-015 | Verify the control context exists before adapter start and early packet callbacks are not dropped. |
| TC-016 | Verify pending read/write limits reject excess requests deterministically. |
| TC-017 | Verify pending-operation counters remain correct across retrieval, cancellation, purge, and requeue. |
| TC-018 | Verify packet callbacks schedule passive completion and never access user buffers at DISPATCH_LEVEL. |
| TC-019 | Verify D0 exit/entry request, frame, callback, and work-item transitions. |
| TC-020 | Verify an undersized pending read fails without losing the queued frame. |
| TC-022 | Verify hosted/runtime readiness status matches the evidence actually available. |
| TC-023 | Verify the test-signed driver loads, the intended TAP interface is uniquely identified, and `192.0.2.1/30` is assigned without an unintended default route. |
| TC-024 | Generate the ARP request for `192.0.2.2`, read it from the Win32 handle, validate it, write the matching ARP reply, then read and validate the resulting Ethernet/IPv4/ICMP Echo Request and checksums. |
| TC-025 | Construct and write the matching ICMP Echo Reply, then verify the Windows networking stack reports the successful reply within the bounded timeout. |
| TC-026 | Exercise malformed, unrelated, truncated, invalid-ARP, fragmented, mismatched, and checksum-invalid frames during the ICMP test and verify deterministic rejection or filtering. |
| TC-027 | Interrupt the ICMP test at provisioning, read, write, timeout, driver-stop, and cleanup stages and verify idempotent restoration plus preserved diagnostics. |
| TC-028 | Execute TC-023 through TC-027 on a GitHub-hosted runner and in a Hyper-V VM; verify no capability-only skip is reported. |
| TC-029 | Verify generated NetAdapterCx bindings match the pinned WDK declarations for sizes, offsets, constants, calling conventions, callback signatures, and status values. |
| TC-030 | Verify every Rust framework callback has the required IRQL/pageability annotation and no callback can unwind across the FFI boundary. |
| TC-031 | Run Rust ownership, queue, cancellation, adapter-stop, surprise-removal, and power-transition tests under Driver Verifier-compatible settings; verify no use-after-free, double completion, leaked reference, or retained framework packet. |
| TC-032 | Remove or make unavailable the WDK headers, Rust target, or binding-generation input and verify configuration fails with an actionable diagnostic rather than using stale or partial bindings. |
| TC-036 | Install the test-signed root-enumerated adapter with NetAdapterCx verifier enabled; verify `WintapEvtPrepareHardware` succeeds without bugcheck `0x19E/0xB` and the advertised receive-filter capabilities omit multicast. |
| TC-037 | Build Rust x64 and ARM64 packages through CMake and verify each contains the Rust driver binary, `wintap_netadaptercx_driver.inf`, and `wintap_netadaptercx_driver.cat`. |
| TC-038 | Install `ROOT\WinTapRust` after removing any stale C package; verify service `WinTapRust` starts and no C device or service is selected. |

## Functional tests

1. **Adapter publication:** install, enumerate, enable, disable, and uninstall
   the adapter; verify INF device identity and cleanup.
2. **Valid user write:** submit minimum-size, normal-size, and maximum-supported
   Ethernet frames with overlapped `WriteFile`; verify one completion and exact
   frame contents at the networking boundary.
3. **Valid user read:** submit pending overlapped `ReadFile` requests and
   deliver frames from the networking boundary; verify exact byte count and
   contents.
4. **Multiple outstanding requests:** issue concurrent reads and writes from
   the exclusive owner; verify ordering guarantees documented by the final
   design and absence of cross-request data.
5. **Exclusive ownership:** open the adapter from one elevated process, reject
   a second open, then allow a new owner after clean close and after abnormal
   process termination.
6. **Backpressure:** fill each bounded frame queue, verify new operations wait,
   cancel correctly, and resume when capacity is released.
7. **Boundary validation:** test zero-length, undersized, oversized, malformed,
   and partially invalid requests; verify explicit failure and no state damage.

## Lifecycle and concurrency tests

- Cancel a pending read while a frame is arriving.
- Cancel a pending write while the transmit queue is full.
- Close the owner handle with pending reads, pending writes, queued frames, and
  active framework callbacks.
- Pause and restart with every queue state and with requests in flight.
- Stop or remove the adapter during each allocation, enqueue, dequeue, copy,
  and completion stage.
- Race second-open, close, cancellation, pause, restart, and removal operations.
- Repeat stress cycles until Driver Verifier, pool tracking, and handle
  tracking remain clean.

## Power and failure tests

- Exercise sleep, hibernate, resume, fast startup where applicable, and
  device disable/enable while I/O is pending.
- Inject allocation failures at every documented allocation site and verify
  reverse-order cleanup.
- Force framework callback failure and verify adapter state transitions to a
  safe terminal state.
- Verify no request remains pending after stop, removal, owner close, or
  cancellation completes.

## Security tests

- Attempt open and control operations from a standard user account.
- Verify the device security descriptor does not expose unintended access.
- Fuzz frame lengths, I/O control metadata, cancellation timing, and queue
  limits within a test-signing environment.
- Confirm user buffers are never retained after request completion and kernel
  memory is never copied beyond the requested output length.

## Verification tooling

The implementation validation package shall include:

- CMake configure/build/package commands using the Visual Studio generator for
  x64 and ARM64.
- NuGet restore commands and recorded WDK/SDK package versions.
- INF installation, removal, and test-signing instructions.
- A user-mode overlapped-I/O test harness.
- Driver Verifier configuration appropriate for WDF, pool, I/O, and deadlock
  detection.
- ETW/WPP or equivalent diagnostics sufficient to correlate request, frame,
  queue, callback, and teardown transitions.

The current harness is `tests/run-wintap-harness.ps1`. It requires an elevated
administrator PowerShell session and an installed test-signed driver. It
validates exclusive device open, malformed frame rejection, overlapped read
cancellation, and successful overlapped writes. The REQ-008 implementation
shall extend or compose this harness with interface discovery/address
provisioning, Ethernet/IPv4/ICMP parsing and construction, stack-triggered
request generation, reply verification, bounded timeouts, diagnostics, and
idempotent cleanup.

The implementation shall use CMake 3.25 or later and a supported Visual
Studio generator. The repository presets target Visual Studio 18 2026; hosted
CI uses Visual Studio 17 2022 when that is the runner-provided generator. The
four architecture-specific WDK/SDK NuGet packages listed in `specs/design.md`
remain pinned to version `10.0.28000.2526`. The harness is implemented in
PowerShell using P/Invoke to Win32 overlapped I/O.

## Required hosted and privileged execution

Hosted CI shall continue to validate artifact presence, PowerShell syntax, WDK
tool provisioning, CMake configure/build/package, and INF/driver package shape
for x64 and ARM64. In addition, a privileged Windows job shall install/load
the test-signed driver and execute the hosted-runner instance of VAL-008 and
VAL-009 using the same test entry point as the manual VM path. The job must
upload diagnostics and fail if driver installation, address configuration,
ARP/ICMP packet exchange, or cleanup is blocked.

The elevated harness remains runnable manually in a Hyper-V-capable VM. It
requires an installed test-signed driver and validates the existing I/O
contract plus the complete REQ-008 round trip. Queue saturation, power,
removal, and verifier scenarios remain additional privileged acceptance gates.

The hosted job and VM procedure must report environment failures explicitly;
they must not classify an unexecuted packet-path test as passed. VAL-009 is
complete only after both the hosted-runner result and the manual-VM result are
recorded; the hosted job alone cannot claim VM coverage.

TC-015, TC-016, TC-017, TC-018, TC-019, TC-020, and TC-022 are implementation
and specification trace points for the approved maintenance corrections.
TC-023 through TC-028 provide trace points for REQ-008 and REQ-009.
