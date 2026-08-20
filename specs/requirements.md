# WinTapNetAdapterCx Requirements

**Workflow:** `/evolve`  
**Phase:** Phase 2 — Specification Changes  
**Status:** Baseline requirements approved; CHG-032 audit revision approved
for specification audit
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
- Add a routed, dual-adapter IPv4/IPv6 relay acceptance test modeled on the
  DuoNIC topology.
- Reconcile the Rust package identity with its existing two root-enumerated
  hardware IDs.
- Replace the C driver implementation with Rust, reusing the
  `windows-drivers-rs` WDF ecosystem and adding generated NetAdapterCx FFI
  bindings.
- Correct the discovered TAP directional-ownership and receive-notification
  defect without changing the public Win32 read/write contract.
- Reconcile the permanent-neighbor relay policy by validating and suppressing
  ARP and IPv6 Neighbor Discovery instead of forwarding control traffic.
- Define a first-release user-mode two-TAP switch using the two existing
  statically defined adapters.
- Require bounded I/O-ring operation with explicit startup failure when the
  required runtime capabilities are unavailable.
- Preserve an endpoint abstraction that can accommodate future dynamically
  provisioned devices without implementing dynamic provisioning in this
  change.
- Do not modify C source, headers, INF files, project files, tests, generated
  artifacts, or build configuration during discovery.

## User-intent references

- **UI-001 (KNOWN):** The repository purpose is to create a software device
  driver using NetAdapterCx that implements the Linux TAP adapter concept on
  Windows.
- **UI-002 (KNOWN):** The project should expose a practical user-mode path for
  exchanging Ethernet frames with the Windows networking stack.
- **UI-003 (KNOWN):** The repository is licensed under MIT.
- **UI-015 (KNOWN):** The user requested a WinTapNetAdapterCx test script
  modeled on DuoNIC's two-NIC routing setup.
- **UI-016 (KNOWN):** The test must force traffic through the NIC datapath by
  routing rules rather than local loopback delivery.
- **UI-017 (KNOWN):** The user selected a full two-adapter relay test with
  IPv4 and IPv6 coverage.
- **UI-018 (KNOWN):** The user selected GitHub-hosted Windows CI and a manual
  Hyper-V/WinDbg VM as required execution environments.
- **UI-019 (KNOWN):** The user selected a dedicated
  `tests\run-wintap-dual-adapter-harness.ps1` entry point.
- **UI-020 (KNOWN):** The user selected always provisioning and removing the
  two test adapters, while failing without modification when matching adapters
  already exist.
- **UI-021 (KNOWN):** The user selected removal of a driver-store package only
  when the current test run added it.
- **UI-022 (KNOWN):** The user selected validation and suppression of ARP and
  IPv6 Neighbor Discovery frames when permanent neighbors are configured.

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

The adapter shall advertise directed, broadcast, multicast, all-multicast, and
promiscuous receive filters. It shall declare a finite multicast-address
capacity of at least 64 addresses and apply the framework-provided
receive-filter configuration before accepting receive traffic. This capability
set is required for TCP/IP to bind successfully through NetAdapterCx.

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

Nonzero writes shorter than 14 bytes or longer than 1514 bytes shall complete
promptly without enqueuing a frame, report `ERROR_INVALID_PARAMETER` (87), and
leave subsequent valid read/write I/O operational. A zero-byte `WriteFile` is a
native Win32 no-op that completes before dispatching to the driver.

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

Malformed Ethernet-frame lengths shall be rejected according to the REQ-002
completion contract.

### REQ-006 — Verification

**Before:** No tests or acceptance criteria exist.  
**After:** The project shall define build, installation, adapter lifecycle,
packet-path, concurrency, cancellation, power-management, malformed-input, and
cleanup verification before implementation is approved.

Verification shall include the routed dual-adapter provisioning, route
precedence, bidirectional relay, IPv4/IPv6 protocol exchange, partial-failure,
and cleanup behavior required by REQ-015.

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
`\\.\WinTapRust` with an overlapped Win32 device handle. It shall
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
**After:** The complete REQ-008 and REQ-015 flows shall run without changing
their respective assertions in both a GitHub-hosted Windows CI/CD runner and a
manually operated Windows VM on a Hyper-V-capable development machine. The
hosted workflow shall provision the test-signed package, install/load the
driver, configure each required interface, execute the packet exchanges,
collect diagnostics, and clean up. The VM path shall use the same entry point
and assertions for each flow.

The test shall fail if required privileged operations are unavailable. It
shall not silently downgrade to a capability check or skip packet-path
assertions. Provisioning, signing, and cleanup may be parameterized by
environment, but the REQ-008 and REQ-015 protocol, route, relay, and cleanup
assertions shall remain identical.

**Trace:** Additional user requirement; UI-018; extends REQ-004, REQ-006,
REQ-007, and REQ-015.
**Invariant impact:** Provisioning and cleanup must be deterministic,
idempotent, isolated to the test interfaces, and diagnostic-preserving. A
hosted-platform policy that blocks required execution is a validation failure,
not a pass.

### REQ-010 — Rust NetAdapterCx implementation

**Before:** The driver implementation is written in C and consumes WDF and
NetAdapterCx APIs through the C toolchain.  
**After:** Production driver behavior shall be implemented in Rust as a
Windows kernel-mode NetAdapterCx miniport. The implementation shall reuse the
`windows-drivers-rs` WDF crates and shall add a generated Rust
`netadaptercx-sys` binding layer for the pinned WDK NetAdapterCx headers.

The binding layer shall cover adapter initialization and creation,
lifecycle/start/stop, link-layer and link-state configuration, datapath and
receive-filter capabilities, TX/RX queues, packet rings, callback types,
constants, structures, and status values. Any safe Rust wrapper shall
preserve the underlying framework ABI and lifecycle contract.

Binding-generation inputs, WDK/SDK headers, Rust toolchain, bindgen
configuration, target triples, and generated-output policy shall be pinned or
captured so a clean environment can reproduce the same bindings. Rust panics
shall not unwind across kernel or framework callbacks.

**Trace:** User-requested Rust implementation; ecosystem inspection confirmed
that `windows-drivers-rs` provides WDF crates but no NetAdapterCx binding
crate. Extends REQ-001, REQ-003, REQ-004, REQ-006, and REQ-007.
**Invariant impact:** Rust FFI must preserve callback ABI and IRQL contracts,
structure layout, packet ownership, queue cancellation, synchronization,
nonpaged allocation, exactly-once completion, and teardown safety. Unsupported
Rust, WDK, SDK, binding, or architecture combinations must fail explicitly.

### REQ-011 — Rust-only production tree

**Before:** The branch contains both a C/C++ driver project and an optional
Rust driver path.
**After:** The branch shall contain only the Rust production driver, its
generated bindings, and its Rust package flow. C/C++ driver source, Visual
Studio driver project, C driver INF, C package fallback, and C-specific CI or
harness selection shall be removed.

**Trace:** User request: "remove the c/c++ impelmentation in this branch as
well."
**Invariant impact:** Every build, package, install, and validation entry point
selects the Rust implementation; no artifact can accidentally deploy the
obsolete C service.

### REQ-012 — Rust package identity

**Before:** The Rust package partially shares the C driver naming scheme.
**After:** The package shall use `ROOT\WinTapRust` and `ROOT\WinTapRust2` as
its supported root-enumerated test-adapter identities, service `WinTapRust`,
and `wintap_netadaptercx_driver.inf`/`wintap_netadaptercx_driver.cat`. It
shall not reuse C hardware, service, INF, or catalog identities.

**Trace:** User direction that this branch work on the Rust driver.
**Invariant impact:** Installation and removal unambiguously target the Rust
driver and cannot select a stale C package. The two adapter identities share
one service but retain separately exclusive control endpoints.

### REQ-013 — Receive-filter verifier compatibility

**Before:** The Rust driver omits multicast receive filtering, causing
NetAdapterCx to reject TCP/IP's `OID_GEN_CURRENT_PACKET_FILTER` request and
preventing TCP/IP from binding to the adapter.
**After:** The Rust driver shall advertise directed, broadcast, multicast,
all-multicast, and promiscuous receive filtering; declare a multicast-address
capacity of at least 64; and apply every framework-provided receive-filter
configuration.

**Trace:** User approval: "Restore multicast filtering"; NetAdapterCx
`NET_PACKET_FILTER_FLAGS` documentation states that omitting a filter expected
by an upper layer makes `OID_GEN_CURRENT_PACKET_FILTER` fail and prevents that
layer from binding; observed absence of TCP/IP binding in the VM after both
multicast-only and all-multicast builds were deployed. User subsequently
selected promiscuous support.
**Invariant impact:** The capability structure declares a nonzero capacity
whenever multicast is advertised. Filter updates remain bounded and replace
the previously active filter state atomically.

### REQ-014 — Native I/O error preservation

**Before:** The PowerShell harness queries `Marshal.GetLastWin32Error()` after
returning from `ReadFile`, `WriteFile`, cancellation, and completion P/Invokes.
That later query can observe an unrelated error; a valid pending read was
reported as Win32 error 203.
**After:** The C# P/Invoke boundary shall capture the native error within the
same managed call as each relevant Win32 invocation and return it explicitly
to PowerShell. A queued overlapped read shall report `ERROR_IO_PENDING` (997),
and a cancelled request shall report `ERROR_OPERATION_ABORTED` (995) from its
completion result. Other I/O failures shall report their captured native
error.

**Trace:** WinDbg showed the control read reach KMDF, return `STATUS_PENDING`,
and remain queued; an isolated C# probe observed `ReadFile=false` with error
997 and cancellation with error 997. The existing PowerShell harness instead
reported 203.
**Invariant impact:** This changes only user-mode test error observation. It
does not alter driver I/O, packet ownership, queue semantics, IRQL, or adapter
lifecycle.

### REQ-015 — Routed dual-adapter IPv4/IPv6 TAP relay test

**Before:** No acceptance test proves that traffic addressed to another local
WinTap interface leaves one WinTap adapter, crosses the TAP boundary, enters a
second adapter, and returns over the reverse direction rather than being
delivered through loopback.

**After:** The repository shall provide
`tests\run-wintap-dual-adapter-harness.ps1`, a privileged test that:

1. Requires a clean environment with no pre-existing `ROOT\WinTapRust` or
   `ROOT\WinTapRust2` adapter, failing before it modifies state otherwise.
2. Provisions exactly those two root-enumerated adapters, verifies their
   stable identity, expected instance-specific MAC/control-endpoint mapping,
   and separate exclusive TAP handles.
3. Configures isolated documentation-only IPv4 and IPv6 peer addresses,
   static peer-neighbor mappings, reciprocal on-link `/32` and `/128` host
   routes, and only narrowly scoped firewall rules needed for the actual
   inbound test path.
4. Starts IPv4 ICMP Echo and IPv6 ICMPv6 Echo clients without explicit
   source-address binding, verifies route selection sends each request through
   the opposite WinTap adapter rather than loopback, and relays complete
   validated data frames bidirectionally between the two TAP handles. With
   permanent peer-neighbor entries installed, valid ARP and IPv6 Neighbor
   Discovery frames (including Duplicate Address Detection) are recorded and
   suppressed rather than written to the peer endpoint.
5. Validates bounded successful round trips, packet identity, protocol
   headers, Ethernet endpoints, payloads, and applicable checksums; malformed,
   truncated, mismatched, cancelled, and timed-out traffic fails
   deterministically.
6. Retains diagnostics and removes all state created by the run. It removes
   both test-created devices and removes a driver-store package only when that
   same run added it.

The test shall not add a default route, bridge, NAT, external peer, production
routing policy, or source-address binding. It shall not modify a pre-existing
adapter or package.

**Trace:** UI-015 through UI-021; DuoNIC setup behavior examined through the
Bluebird source index; extends REQ-001, REQ-002, REQ-003, REQ-005, REQ-006,
REQ-009, and REQ-012.

**Invariant impact:** Each control handle remains independently exclusive.
Every relayed frame has one completed source read, one completed peer write,
and no retained user buffer after completion or cancellation. Cleanup is
idempotent, affects only recorded test-created objects, preserves the primary
failure, and reports cleanup failure separately. REQ-008 remains unchanged.

### REQ-016 — Directional frame isolation and receive indication

**Before:** The TAP write-to-stack and stack-transmit-to-read paths can share
one frame queue and a write can wake the user-read completion worker. The RX
ring ownership policy is deferred despite live evidence that a destination
TAP read can receive the same A-to-B Echo Request that was just injected into
that destination.

**After:** Each adapter shall maintain two distinct bounded frame queues:

1. An injection queue owns frames captured from successful TAP writes until
   they are indicated through the NetAdapterCx receive queue.
2. A capture queue owns frames copied from the NetAdapterCx transmit queue
   until they complete a TAP read.
3. No TAP write frame may complete a TAP read, and no stack-transmit frame
   may be indicated through the receive queue. The queues may share
   synchronization but shall not share storage, dequeue operations, capacity,
   or teardown ownership.
4. A write worker shall notify NetAdapterCx of queued injection work only
   when receive notification is enabled, at most once for each enable cycle,
   and without invoking a user-read completion path. Owner-only cleanup that
   leaves the RX queue running shall preserve an armed notification cycle so a
   later owner write can request RX polling; queue stop, cancellation, D0
   exit, and release may disarm it.
5. `EVT_PACKET_QUEUE_ADVANCE` is the only callback that may populate an RX
   frame or advance ring entries to indicate a new frame. It shall populate
   only driver-owned entries from `BeginIndex` up to, but not including,
   `EndIndex`; clear `Ignore`; explicitly initialize the fragment `Offset` and
   `ValidLength`; initialize each indicated packet's fragment and layout
   fields; and advance packet and fragment `BeginIndex` together after a
   complete frame is available. It shall never modify `EndIndex` or advance
   `BeginIndex` beyond it. `EVT_PACKET_QUEUE_CANCEL` is the sole exception:
   it may mark outstanding RX packets ignored and advance packet and fragment
   `BeginIndex` to `EndIndex` to return those entries to NetAdapterCx.
6. RX cancellation shall mark unindicated RX packets ignored before returning
   them to NetAdapterCx, and stop/removal shall release queued injection and
   captured frames exactly once.

**Trace:** Runtime evidence (KNOWN): the B endpoint returned the original
A-to-B IPv4 Echo Request instead of a B-to-A Echo Reply; source inspection
(KNOWN): the current write worker and transmit capture path use the same frame
queue; Microsoft NetAdapterCx `NET_RING` and RX element-management guidance
(KNOWN). Extends REQ-002, REQ-003, REQ-006, and REQ-015.

**Invariant impact:** A frame has exactly one directional owner at every
transition. A pending TAP read cannot steal a frame awaiting stack delivery.
RX indication mutations occur in queue advance and RX return mutations occur
only in queue cancellation. Notification remains edge-triggered across
owner-only cleanup, and teardown cannot leak, duplicate, or misdirect a
frame.

### REQ-017 — Two-TAP user-mode switch

**Before:** The repository specifies a routed dual-adapter relay harness, but
does not define a forwarding-database or user-mode switch contract.

**After:** The project shall define a privileged user-mode process that
exclusively opens the two existing WinTap TAP control endpoints, learns source
MAC/VLAN locations, and forwards valid Ethernet frames according to this
policy:

1. Known unicast traffic is forwarded to the learned destination endpoint.
2. Unknown unicast, broadcast, and multicast traffic is flooded to the other
   endpoint.
3. A source observed on the other endpoint immediately moves the learned
   entry.
4. The forwarding database has 4,096 entries, does not age entries in the
   first release, and preserves existing entries when full.
5. A frame is never forwarded to the endpoint from which it was read.

**Trace:** User-provided switch-feasibility argument; `tap-switch-feasibility.md`
Decision and Forwarding behavior; extends REQ-002, REQ-003, REQ-005, REQ-006,
and REQ-015.

**Invariant impact:** Forwarding state and pending work remain bounded. Each
frame has one forwarding decision and one user-mode ownership path at every
transition. Source-endpoint exclusion prevents reflection.

### REQ-018 — Bounded I/O-ring data plane

**Before:** The repository has overlapped-I/O relay evidence but no I/O-ring
contract or runtime capability policy.

**After:** The switch shall probe I/O-ring capabilities before starting its
data plane, require supported read and write operations, use a bounded
registered buffer pool and bounded read/write depth, and encode endpoint,
buffer slot, and generation in every completion. The initial path shall use
ordinary contiguous version-3 operations. Version-4 scatter/gather is
optional and may be enabled only after runtime support and operation
validation. If the required I/O-ring capability is unavailable, switch
startup shall fail explicitly; the existing overlapped relay is not a fallback
for this change.

The switch shall repost a read only after its source read and all writes using
that buffer have terminal completions. On cancellation, removal, or shutdown,
it shall stop posting reads, cancel outstanding operations, consume original
completions, and only then deregister buffers and handles or close the ring.
Generation values shall prevent stale completions from being associated with a
recycled slot.

**Trace:** User-provided switch-feasibility argument; `tap-switch-feasibility.md`
I/O-ring design, versioning, compatibility, and risks; extends REQ-002,
REQ-003, REQ-004, and REQ-006.

**Invariant impact:** No user buffer is reused while an operation can still
reference it. Cancellation and teardown preserve completion and resource
release ordering. Read depth, write depth, registered buffers, and completion
state are finite.

### REQ-019 — Forward-compatible endpoint abstraction

**Before:** Current identities and relay behavior are explicitly two-adapter
and fixed-name oriented.

**After:** The switch-facing contract shall represent endpoints as a
collection with stable per-endpoint identity and peer-selection semantics,
while the first release supplies exactly the two existing statically defined
adapters. This change shall not provision or manage additional devices, but
its forwarding and lifecycle interfaces shall not encode a hard two-endpoint
assumption beyond the first-release flood policy. Dynamic PnP provisioning,
stable arbitrary-instance identity, and forwarding to more than one recipient
are deferred to a separate future change.

**Trace:** User-provided switch-feasibility argument; `tap-switch-feasibility.md`
current constraints; `multi-adapter.md` proposal; extends REQ-012 and REQ-015.

**Invariant impact:** Endpoint identity is independent of buffer-slot reuse.
Future endpoint addition must not invalidate ownership, teardown, or
completion-generation rules for existing endpoints.

## Scope boundaries

- **In scope:** A NetAdapterCx software Ethernet adapter and a TAP-style
  user-mode frame path on Windows.
- **In scope:** Driver lifecycle, packet ownership, synchronization, IRQL and
  pageable-code rules, power management, access control, installation, and
  verification specifications.
- **In scope:** The complete privileged ICMP Echo Request/Echo Reply test and
  its GitHub-hosted runner and Hyper-V VM execution environments.
- **In scope:** A dedicated dual-adapter IPv4/IPv6 TAP relay harness, its
  DevCon-based test provisioning, route/neighbor/firewall setup, diagnostics,
  cleanup, and hosted/VM execution.
- **In scope:** The first-release user-mode two-TAP switch data-plane
  contract, forwarding database, bounded I/O-ring lifecycle, and validation
  using the two existing static adapter identities.
- **In scope:** Rust kernel-mode driver behavior, generated NetAdapterCx FFI,
  safe wrapper boundaries, Rust-specific panic and build configuration, and
  ABI/layout validation.
- **In scope:** Removing the C/C++ implementation and publishing an
  unambiguous Rust-only driver package.
- **Out of scope unless explicitly added:** IP/TUN mode (decision: excluded
  from the initial milestone), protocol-specific
  user-mode libraries, packet capture beyond the virtual adapter contract,
  driver-internal peer linking, bridging/NAT/routing policy, and production
  signing/distribution services, dynamic adapter provisioning, arbitrary-N
  forwarding, and an overlapped-I/O fallback for the switch.

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
13. **Resolved:** REQ-015 uses a full two-adapter user-mode relay rather than
    a setup-only script or a one-adapter extension.
14. **Resolved:** REQ-015 covers both IPv4 and IPv6.
15. **Resolved:** REQ-015 uses a dedicated
    `tests\run-wintap-dual-adapter-harness.ps1` entry point.
16. **Resolved:** REQ-015 always provisions and removes its two adapters and
    fails without modification when matching adapters already exist.
17. **Resolved:** REQ-015 runs in hosted CI and a manual Hyper-V/WinDbg VM.
18. **Resolved:** REQ-015 removes a driver-store package only if that test run
    added it.
19. **Resolved:** The first switch release uses exactly the two existing
    statically defined adapters.
20. **Resolved:** Missing required I/O-ring runtime capabilities fail switch
    startup; overlapped I/O is not a switch fallback.
21. **Resolved:** Endpoint handling is collection-oriented for future dynamic
    devices, but dynamic provisioning and arbitrary-N forwarding are deferred.

## Discovery gate

The switch requirements change is complete for specification review. Phase 3
remains blocked until the requirements, design, and validation patches are
approved together.
