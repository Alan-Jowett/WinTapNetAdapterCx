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
cancellation, and successful overlapped writes; packet exchange and lifecycle
stress remain separate acceptance tests.

The implementation shall use CMake 3.25 or later, the Visual Studio 18
generator, the four architecture-specific WDK/SDK NuGet packages listed in
`specs/design.md`, and version `10.0.28000.2526`. The harness is implemented
in PowerShell using P/Invoke to Win32 overlapped I/O.
