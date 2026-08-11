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
- Add a privileged end-to-end ICMP round-trip acceptance test over the
  Ethernet/TAP boundary.
- Require the same full test on a GitHub-hosted Windows runner and manually in
  a Hyper-V-capable development VM.
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

### REQ-008 — ICMP/TAP end-to-end round trip

**Before:** Packet exchange is specified only as generic Ethernet frame
read/write behavior; no protocol-level test proves traversal through the
Windows networking stack in both directions.
**After:** A privileged integration test shall load the test-signed driver,
identify the resulting TAP Ethernet interface, assign `192.0.2.1/30`
without creating an unintended default route, and open
`\\.\WinTapNetAdapterCx` with an overlapped Win32 device handle. It shall
cause the Windows networking stack to generate an ICMP Echo Request to
`192.0.2.2`. It shall service the required Ethernet ARP exchange through the
TAP handle so the stack can resolve the peer, then read and validate the
Ethernet/IPv4/ICMP request from the TAP handle, write a correctly formed Echo
Reply through the handle, and verify that the Windows stack receives the
matching reply. It shall restore addressing, routes, handles, and driver
state on success and failure.

The test shall use documentation-only TEST-NET space and shall not depend on
an external peer, internet connectivity, bridge, NAT, or production route.

**Trace:** User-requested `/evolve` change; selected address pair
`192.0.2.1/30` and `192.0.2.2`; extends REQ-001, REQ-002, REQ-003, REQ-005,
and REQ-006.
**Invariant impact:** The test preserves Ethernet framing and driver
ownership rules, distinguishes timeout from malformed-packet failure, and
leaves no test-created network or driver state after cleanup.

### REQ-009 — Dual-mode privileged integration execution

**Before:** Hosted CI validates build and package artifacts only; privileged
packet-path validation is manual/self-hosted.
**After:** The complete REQ-008 flow shall run without changing its assertions
in both a GitHub-hosted Windows CI/CD runner and a manually operated Windows
VM on a Hyper-V-capable development machine. The hosted workflow shall
provision the test-signed package, install/load the driver, configure the
interface, execute the packet exchange, collect diagnostics, and clean up.
The VM path shall use the same test entry point and assertions.

The test shall fail if required privileged operations are unavailable. It
shall not silently downgrade to a capability check or skip packet-path
assertions. Provisioning, signing, and cleanup may be parameterized by
environment, but the ICMP/TAP assertions shall remain identical.

**Trace:** Additional user requirement; extends REQ-004, REQ-006, and
REQ-007.
**Invariant impact:** Provisioning and cleanup must be deterministic,
idempotent, isolated to the test interface, and diagnostic-preserving. A
hosted-platform policy that blocks required execution is a validation failure,
not a pass.

## Scope boundaries

- **In scope:** A NetAdapterCx software Ethernet adapter and a TAP-style
  user-mode frame path on Windows.
- **In scope:** Driver lifecycle, packet ownership, synchronization, IRQL and
  pageable-code rules, power management, access control, installation, and
  verification specifications.
- **In scope:** The complete privileged ICMP Echo Request/Echo Reply test and
  its GitHub-hosted runner and Hyper-V VM execution environments.
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
10. **Resolved:** The ICMP integration test performs a complete Echo
    Request/Echo Reply round trip.
11. **Resolved:** The test network is `192.0.2.1/30` with peer
    `192.0.2.2`.
12. **Resolved:** Full privileged execution is required on both a
    GitHub-hosted Windows runner and a Hyper-V-capable development VM.

## Discovery gate

Requirements discovery is complete. Phase 3 remains blocked until the
requirements, design, and validation patches are approved together.
