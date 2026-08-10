# WinTapNetAdapterCx Requirements

**Workflow:** `/evolve`  
**Phase:** Phase 2 — Specification Changes  
**Status:** Requirements approved; design and validation propagation in progress  
**Evidence scope:** `README.md`, repository layout, and user-provided project purpose

## Change manifest

- Establish the initial requirements baseline for a Windows software network
  adapter built with NetAdapterCx.
- Define the intended TAP-style user-mode packet exchange contract.
- Define lifecycle, compatibility, security, and verification decisions needed
  before implementation specifications are approved.
- Do not modify C source, headers, INF files, project files, tests, generated
  artifacts, or build configuration during discovery.

## User-intent references

- **UI-001 (KNOWN):** The repository purpose is to create a software device
  driver using NetAdapterCx that implements the Linux TAP adapter concept on
  Windows.
- **UI-002 (KNOWN):** The project should expose a practical user-mode path for
  exchanging Ethernet frames with the Windows networking stack.
- **UI-003 (KNOWN):** The repository is licensed under MIT.

## Baseline requirements

### REQ-001 — Software Ethernet adapter

**Before:** No driver implementation or adapter contract exists in the
repository.  
**After:** The project shall provide a Windows software network adapter
implemented as a NetAdapterCx miniport and presented to the Windows networking
stack as an Ethernet-capable interface.

**Trace:** UI-001; `README.md` project goals.  
**Invariant impact:** Adapter creation and teardown must leave no registered
device, queue, packet, or user handle after failure or removal.

### REQ-002 — TAP-style frame exchange

**Before:** No user-mode packet interface is defined.  
**After:** A user-mode application shall be able to submit Ethernet frames to
the virtual adapter and receive Ethernet frames delivered by the Windows
networking stack through a Win32 device handle using read/write I/O, subject to
the selected access, buffering, and queueing contract. The interface shall
support overlapped I/O and cancellation.

**Trace:** UI-001, UI-002; `README.md` purpose and project goals.  
**Invariant impact:** Every frame must have one unambiguous owner at each stage,
with bounded buffering, backpressure when full, and deterministic completion or
cancellation.

### REQ-003 — Windows driver lifecycle

**Before:** No lifecycle behavior is specified.  
**After:** The driver shall define behavior for installation, adapter start,
pause, restart, stop, surprise removal, system power transitions, user-handle
closure, and process termination.

**Trace:** UI-001; `README.md` Windows driver development goal.  
**Invariant impact:** Teardown must synchronize with in-flight I/O and packet
processing without use-after-free, double completion, or leaked references.

### REQ-004 — Compatibility target

**Before:** The README names Windows 10 and later but does not identify a
minimum build, architecture, or NetAdapterCx version.  
**After:** The initial project shall target Windows 10 version 2004 and later
on x64 and ARM64, and shall publish the WDK/SDK baseline and NetAdapterCx
dependency used by the implementation.

**Trace:** `README.md` intended platform.  
**Invariant impact:** Unsupported platform combinations must fail explicitly at
build, install, or initialization rather than silently degrading behavior.

### REQ-005 — Security and access control

**Before:** No device name, security descriptor, privilege model, or isolation
boundary is specified.  
**After:** Only elevated administrators may open or control a TAP device. The
project shall define the device security descriptor and how malformed or
hostile frames and I/O requests are bounded and rejected.

**Trace:** UI-001; `README.md` Windows security-practices goal.  
**Invariant impact:** User-mode access must not permit unauthorized control,
kernel memory disclosure, buffer overrun, or cross-device frame access.

### REQ-006 — Verification

**Before:** No tests or acceptance criteria exist.  
**After:** The project shall define build, installation, adapter lifecycle,
packet-path, concurrency, cancellation, power-management, malformed-input, and
cleanup verification before implementation is approved.

**Trace:** UI-001, UI-002; lifecycle and safety implications of REQ-001 through
REQ-005.  
**Invariant impact:** Each ownership, synchronization, and failure-path
requirement must have an observable acceptance test or documented analysis.

### REQ-007 — Reproducible build system

**Before:** No build system, generator, dependency acquisition method, or
package version policy exists.  
**After:** The project shall use CMake with a Visual Studio generator. The WDK
and SDK dependencies shall be acquired through NuGet and pinned or otherwise
resolved reproducibly for x64 and ARM64 builds.

**Trace:** User-approved workflow decision.  
**Invariant impact:** Build and packaging must use a known toolchain and
dependency set; unsupported or unresolved dependencies must fail during
configuration rather than producing an ambiguous driver package.

## Scope boundaries

- **In scope:** A NetAdapterCx software Ethernet adapter and a TAP-style
  user-mode frame path on Windows.
- **In scope:** Driver lifecycle, packet ownership, synchronization, IRQL and
  pageable-code rules, power management, access control, installation, and
  verification specifications.
- **Out of scope unless explicitly added:** IP/TUN mode (decision: excluded
  from the initial milestone), protocol-specific
  user-mode libraries, packet capture beyond the virtual adapter contract,
  bridging/NAT/routing policy, and production signing/distribution services.

## Open questions requiring user decisions

1. **Resolved:** The initial milestone supports Ethernet/TAP mode only; Linux-
   style IP/TUN mode is out of scope.
2. **Resolved:** User-mode frame exchange uses a Win32 device handle with
   read/write I/O, including overlapped I/O and cancellation.
3. **Resolved:** Each adapter has one exclusive user-mode owner.
4. **Resolved:** Queues are bounded and apply backpressure by blocking until
   space is available or the request is cancelled.
5. **Resolved:** The minimum target is Windows 10 version 2004 and later on
   x64 and ARM64.
6. **Resolved:** The first milestone includes the driver INF/installer and
   test-signing instructions, in addition to the test harness.
7. **Resolved:** Only elevated administrators may open or control an adapter.
8. **Resolved:** Phase 8 shall produce a consolidated design and
   implementation patch set.
9. **Resolved:** The project shall use CMake with a Visual Studio generator and
   NuGet-managed WDK/SDK dependencies.

## Discovery gate

Phase 2 is blocked until the user resolves material open questions and
explicitly indicates that discovery is complete, for example by replying
`READY` or `proceed`.
