<!-- SPDX-License-Identifier: MIT
  Copyright (c) 2026 WinTapNetAdapterCx contributors -->

# WinTapNetAdapterCx Requirements

**Workflow:** `/evolve`  
**Phase:** Phase 2 — Specification Changes
**Status:** Dynamic-bus specification changes proposed; awaiting approval
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
- Remove the artificial 256-slot pending-I/O limit while retaining an even,
  shared total depth across both endpoints and explicit resource-limit
  failures.
- Deliver captured TX frames directly to pending READ IRPs when packet
  callback execution is at passive level, while retaining deferred delivery
  for elevated-IRQL callbacks.
- Preserve an endpoint abstraction that can accommodate future dynamically
  provisioned devices without implementing dynamic provisioning in this
  change.
- Establish a repository-wide SPDX MIT header policy for governed text files,
  with syntax-preserving rules for source, scripts, metadata, and Markdown.
- Enforce the SPDX policy at staged-commit time and in required pull-request
  and push CI checks, rejecting noncompliant commits and pull requests.
- Do not modify C source, headers, INF files, project files, tests, generated
  artifacts, or build configuration during discovery.
- Replace the fixed root-enumerated adapter model with a separate-service KMDF
  bus and dynamically enumerated, GUID-keyed TAP child adapters.

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
- **UI-023 (KNOWN):** The user supplied an approved dynamic-bus requirements
  baseline and selected separate KMDF bus and TAP-child driver services.
- **UI-024 (KNOWN):** The user requested SPDX headers on every eligible file
  and enforcement that rejects noncompliant commits and pull requests.

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
**After:** A privileged integration test shall load the test-signed package,
create one test GUID child through the manager, wait for its GUID-correlated
TAP interface, assign `192.0.2.1/30` without creating an unintended default
route, and open that discovered interface with an overlapped Win32 device
handle. It shall cause the Windows networking stack to generate an ICMP Echo
Request to `192.0.2.2`. It shall service the required Ethernet ARP exchange
through the TAP handle so the stack can resolve the peer, then read and
validate the Ethernet/IPv4/ICMP request from the TAP handle, write a correctly
formed Echo Reply through the handle, and verify that the Windows stack
receives the matching reply. It shall restore addressing, routes, handles,
child, and package state on success and failure.

The test shall use documentation-only TEST-NET space and shall not depend on
an external peer, internet connectivity, bridge, NAT, or production route.

**Trace:** User-requested `/evolve` change; selected address pair
`192.0.2.1/30` and `192.0.2.2`; extends REQ-001, REQ-002, REQ-003, REQ-005,
REQ-006, REQ-029, and REQ-030.
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
4. The inline write callback shall notify NetAdapterCx of queued injection
   work only when receive notification is enabled, at most once for each
   enable cycle, and without invoking a user-read completion path. Owner-only
   cleanup that leaves the RX queue running shall preserve an armed
   notification cycle so a later owner write can request RX polling; queue
   stop, cancellation, D0 exit, and release may disarm it.
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

### REQ-020 — Resource-bounded scalable pending I/O depth

**Before:** The switch limits pending reads and writes to an application-level
maximum of 256 operations because completion metadata reserves only 8 bits for
the buffer slot. The configured depth is otherwise treated as separate
per-endpoint capacity.

**After:** The switch shall accept one positive, even total pending
read/write-depth value shared across both endpoints. Each endpoint shall
receive equal capacity, equal to half the configured total, and the
completion identity representation shall support every allocated buffer slot
without an artificial fixed maximum. The effective depth shall be limited
only by representable sizes, checked arithmetic, available memory, I/O-ring
API/resource limits, and successful registration of the required buffers and
operations. Invalid values, overflow, allocation failure, unsupported API
limits, and registration failure shall produce explicit startup errors; the
switch shall not silently clamp, wrap, or fall back to a smaller depth.

The existing two-endpoint behavior and teardown ordering remain unchanged:
all operations referencing a slot must reach terminal completion before the
slot or its registered buffer is reused or released.

**Trace:** User request to increase the maximum pending reads/writes beyond
256; Phase 1 discovery decision for one shared even total limited by available
memory and runtime resources; extends REQ-018 and REQ-019.

**Invariant impact:** Equal endpoint capacity is derived from one validated
total. Completion identity remains unique for every live operation, and
resource exhaustion fails before partial publication of the data plane.

### REQ-021 — Inline TAP write processing

**Before:** A valid TAP write is admitted to a pending-write counter,
forwarded to a WDF manual queue, and later processed by
`evt_write_drain_work_item`. The worker retrieves the request, copies the
frame into driver-owned storage, completes the request, and may notify
NetAdapterCx.

**After:** A valid TAP write shall be processed synchronously by the write
I/O callback. The callback shall validate the request, capture the frame into
driver-owned storage, enqueue it into the bounded injection queue, complete
the request exactly once, and issue at most one receive notification outside
the state/frame lock. Valid writes shall not be forwarded to a WDF manual
queue or require a write work item.

The implementation shall verify that the WDF callback execution-level and
memory-allocation contracts permit every inline operation. If the required
contract is unavailable, driver initialization shall fail explicitly rather
than restoring deferred write processing.

The request input buffer shall not be retained after request completion.
Injection-queue ownership, queue-full and closed-queue rejection, allocation
failure, adapter stop, owner teardown, and notification state transitions
shall remain deterministic and exactly-once.

**Trace:** User request to eliminate write-path work-item queueing; repository
evidence in `evt_io_write`, `evt_write_drain_work_item`,
`enqueue_injection_frame`, and `notify_more_received_packets`; extends
REQ-002, REQ-003, REQ-006, and REQ-016.

**Invariant impact:** Inline execution must not race adapter teardown, queue
closure, cancellation, or notification reentrancy. A request is completed
only after its frame has been safely captured or rejected, and no user buffer
or request may remain reachable after completion.

### REQ-024 — IRQL-aware inline TAP read delivery

**Before:** Captured TX frames are always copied into a driver-owned
`Frame`, placed in `capture_queue`, and delivered to pending READ IRPs by a
passive-level WDF work item.

**After:** When the TX packet callback executes at `PASSIVE_LEVEL` and a
compatible pending READ IRP is available, the driver shall validate and copy
the captured frame directly from the NetAdapterCx TX fragment(s) into the
IRP output buffer before returning the framework-owned ring entries. When the
callback executes above `PASSIVE_LEVEL`, or when no compatible READ IRP is
available, the driver shall preserve the frame in nonpaged driver-owned
storage, return the framework-owned ring entries, and complete the READ IRP
from a passive-level drain path.

The implementation shall not manipulate NetAdapterCx ring ownership from the
WDF read callback or another context outside the packet callback contract.
It shall not leave framework-owned TX entries indefinitely pending solely
because no READ IRP is currently available. A bounded capture queue remains
the backpressure boundary.

**Trace:** User request to deliver directly when possible while retaining a
work-item fallback for elevated IRQL; extends REQ-002, REQ-003, REQ-006,
REQ-016, and REQ-021.

**Invariant impact:** Direct delivery must occur before the packet ring
entries are returned and only at an IRQL that permits WDF output-buffer
access and request completion. Elevated-IRQL callbacks must never access
user buffers. A too-small output buffer shall be completed with
`STATUS_BUFFER_TOO_SMALL`; the frame shall then be staged in the bounded
capture queue and the packet ring entry returned rather than held pending.
Request cancellation, queue exhaustion, callback reentrancy, adapter stop,
and owner teardown must retain exactly-once completion and frame ownership.

### REQ-025 — Lock-bounded passive READ drain

**Before:** A passive `evt_io_read` callback may retain the state lock while
claiming a captured frame and READ request, accessing the request buffer, and
completing or requeueing the operation through the passive work-item path.

**After:** When `evt_io_read` can pair a queued captured frame with a pending
READ at `PASSIVE_LEVEL`, it shall claim the frame and request while holding
the state lock, release the lock, and only then retrieve/access the output
buffer, copy the frame, and complete the request. If the output buffer is
too small or retrieval fails, it shall preserve frame ownership by requeueing
the frame under the state lock and complete the request outside the lock.
The passive work item shall remain available for elevated-IRQL capture and
for work that cannot be completed by direct pairing.

The implementation shall ensure that packet callbacks, `evt_io_read`, and
the passive work item cannot claim the same frame or request concurrently.
Cancellation, adapter stop, surprise removal, queue closure, and teardown
shall preserve exactly-once request completion and frame ownership.

**Trace:** User request to release the state lock before WDF buffer access and
request completion; repository evidence in `InstanceStateGuard`,
`evt_io_read`, `deliver_transmit_packet_to_read`, and
`evt_read_completion_work_item`; refines REQ-024.

**Invariant impact:** The state lock protects ownership transitions only.
No WDF buffer retrieval, user-buffer access, frame copy, or request
completion may occur while that lock is held. Every claimed frame and
request must have one owner, and all failure paths must either complete the
request or return the frame to the bounded capture queue.

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
  contract, forwarding database, resource-bounded scalable I/O-ring lifecycle,
  and validation using the two existing static adapter identities.
- **In scope:** Rust kernel-mode driver behavior, generated NetAdapterCx FFI,
  safe wrapper boundaries, Rust-specific panic and build configuration, and
  ABI/layout validation.
- **In scope:** Removing the C/C++ implementation and publishing an
  unambiguous Rust-only driver package.
- **In scope:** Opportunistic passive-level direct delivery from NetAdapterCx
  TX fragments to pending READ IRPs, with bounded nonpaged capture and
  passive deferred completion when direct delivery is unavailable, including
  lock-bounded pairing and completion outside the state lock.
- **Out of scope unless explicitly added:** IP/TUN mode (decision: excluded
  from the initial milestone), protocol-specific
  user-mode libraries, packet capture beyond the virtual adapter contract,
  driver-internal peer linking, bridging/NAT/routing policy, and production
  signing/distribution services, arbitrary-N forwarding, and an overlapped-I/O
  fallback for the switch.

## Dynamic bus amendment

This amendment supersedes the static-root portions of REQ-001, REQ-003,
REQ-005, REQ-008, REQ-009, REQ-012, REQ-015, REQ-017, and REQ-019. REQ-002,
REQ-013, REQ-016, REQ-018, REQ-020, REQ-021, REQ-024, and REQ-025 remain
per-child contracts and are not relaxed.

### REQ-026 — Bus and child topology

**Before:** The package exposes one service bound to the two fixed
root-enumerated identities `ROOT\WinTapRust` and `ROOT\WinTapRust2`.
**After:** The package shall install separate KMDF bus and TAP-child driver
services. The bus shall dynamically enumerate every active TAP adapter as an
independent PnP child PDO.

**Trace:** UI-023; BUS-REQ-001 and BUS-REQ-009.
**Invariant impact:** A bus operation does not own packet data, and failure to
enumerate one child cannot alter another child's packet, queue, or handle
state.

### REQ-027 — Privileged versioned bus control

The bus shall expose an administrator-only manager interface for create,
remove, enumerate, and query operations. Every request shall include a
protocol version and bounded length, and shall be fully validated before
allocation, child-list mutation, or state publication.

**Trace:** UI-023; BUS-REQ-002.
**Invariant impact:** An unprivileged or malformed request cannot create,
remove, enumerate privileged child state, disclose kernel memory, or mutate a
child lifecycle.

### REQ-028 — Immutable dynamic child identity

Every create request shall contain one unique immutable adapter GUID. The bus
shall use that GUID as the WDF child-identification key. Duplicate and
concurrent create requests for the same GUID shall deterministically produce
at most one child.

**Trace:** UI-023; BUS-REQ-003.
**Invariant impact:** GUID reuse during create or teardown cannot yield
ambiguous ownership, interface selection, or cleanup.

### REQ-029 — Asynchronous lifecycle completion

Create and explicit remove shall be asynchronous manager operations correlated
by request ID and adapter GUID. Create shall not report success until the
matching child and direct TAP interface are observable. Remove shall not
report success until the matching child PnP instance and TAP interface are
gone. Surprise removal and bus teardown shall serialize with these operations
per GUID.

**Trace:** UI-023; BUS-REQ-004 and BUS-REQ-008.
**Invariant impact:** Each child I/O request has one terminal completion
before removal succeeds; no operation can report false success during PnP
delay or failure.

### REQ-030 — Direct TAP interface discovery

Every child shall publish a unique discoverable TAP device interface. The
manager shall return the GUID and resolved interface identity; TAP callers
shall open that child directly and shall not rely on ordinal DOS paths or PnP
enumeration order.

**Trace:** UI-023; BUS-REQ-005.
**Invariant impact:** Two or more children retain distinct endpoints and
independently exclusive owners, without the manager brokering frame I/O.

### REQ-031 — Dynamic child-owned state

Each child shall own its NetAdapterCx adapter, queues, work items, locks,
filter state, and bounded frame queues. The implementation shall remove fixed
two-instance global registries and shall not impose an arbitrary adapter-count
limit. Resource exhaustion shall fail explicitly before partial child
publication.

**Trace:** UI-023; BUS-REQ-006; repository evidence: two-element
`INSTANCE_IDS` and `INSTANCE_STATES` arrays.
**Invariant impact:** At least three concurrent children have isolated TAP
I/O; no state or frame crosses GUID boundaries.

### REQ-032 — Preserve per-child TAP contracts

Each dynamic child shall preserve the REQ-002, REQ-003, REQ-013, REQ-016,
REQ-021, REQ-024, and REQ-025 directional frame isolation, bounded queues,
cancellation, receive filtering, RX-ring ownership, power, IRQL, and teardown
contracts.

**Trace:** UI-023; BUS-REQ-007.
**Invariant impact:** Refactoring topology cannot relax packet ownership or
completion guarantees.

### REQ-033 — Legacy migration and package identity

The package shall use one bus-parent identity and one child
hardware/compatible identity. `ROOT\WinTapRust` and `ROOT\WinTapRust2` shall
not remain supported runtime adapter models. Install, upgrade, uninstall, and
validation shall detect stale legacy devices and remove them only through an
explicit migration or cleanup operation.

**Trace:** UI-023; BUS-REQ-009.
**Invariant impact:** A dynamic child cannot bind accidentally to a legacy
root device or leave an ambiguous service selection.

### REQ-034 — Lifecycle diagnostics

The manager and bus shall record request ID, adapter GUID, PnP state,
interface identity, and primary cleanup failure. Packet contents shall not be
recorded by default.

**Trace:** UI-023; BUS-REQ-010.
**Invariant impact:** Lifecycle failure is diagnosable without TAP payload
disclosure.

### REQ-035 — Manager restart behavior

Active children shall survive manager exit and restart until explicitly
removed. A restarted manager shall enumerate and reattach to existing GUIDs.
This requirement does not require persistence across bus unload or reboot.

**Trace:** UI-023; BUS-REQ-011.
**Invariant impact:** Manager availability is independent of child packet
ownership and does not create duplicate children.

### REQ-036 — Dynamic endpoint collection

The switch-facing endpoint contract shall use GUID-correlated interface
discovery and support a selected dynamic collection. The initial relay and
switch acceptance topology remains two selected endpoints; arbitrary-N
forwarding policy is not introduced by this requirement.

**Trace:** UI-023; supersedes the static selection portions of REQ-017 and
REQ-019.
**Invariant impact:** Dynamic discovery cannot change existing endpoint,
buffer-slot, completion-generation, or source-reflection rules.

### REQ-037 — SPDX headers on governed text files

Every tracked governed text file shall contain the repository-approved
`SPDX-License-Identifier: MIT` header using the comment syntax assigned by the
SPDX policy. The policy shall cover all repository source, scripts, build and
package metadata, workflows, specifications, and Markdown documentation.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Headers are comment-only metadata and shall not alter
build, packaging, runtime, or documentation semantics.

### REQ-038 — Preamble-preserving header placement

Header placement shall preserve required shebangs, encoding declarations, and
YAML front matter. The SPDX header shall be the first permitted comment after
such a preamble, and a file shall not pass validation when the identifier is
present only in an invalid location or comment syntax.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Executable scripts, parsers, and front-matter consumers
continue to interpret files exactly as before.

### REQ-039 — Fail-closed SPDX validation

The repository shall provide one validator with full-tree and staged modes.
Full-tree mode shall inspect every tracked governed file. Staged mode shall
inspect added, copied, renamed, and modified governed files from the Git
index. Both modes shall return nonzero and identify the file and expected
form for absent, malformed, misplaced, or wrong-syntax headers.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Validation cannot silently pass incomplete coverage.

### REQ-040 — Local commit rejection

A repository-provided pre-commit hook shall invoke staged SPDX validation and
reject a commit before it is created when any staged governed file fails.
The hook shall validate the index contents, not an unrelated working-tree
copy, and shall be documented as a required contributor control.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Noncompliant local commits are prevented without
changing commit contents or history.

### REQ-041 — Required remote CI enforcement

CI shall invoke full-tree SPDX validation for pull requests and protected
branch pushes. The workflow shall expose a stable named check, and branch
protection shall require that check before merge. A failed check shall block
the pull request or protected-branch update; local-hook bypasses shall not
weaken remote enforcement.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Repository acceptance is fail-closed even when a local
hook is unavailable or bypassed.

### REQ-042 — Explicit exclusions and diagnostics

The validator shall maintain an explicit version-controlled exclusion
manifest for binary files, generated outputs, and formats that cannot safely
contain comments. Each exclusion shall name its path or deterministic rule
and reason. Exclusions shall be reported in full-tree diagnostics and shall
not match source, script, metadata, specification, or documentation files
implicitly.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Unsupported formats remain semantically valid without
creating an unreviewed enforcement bypass.

### REQ-043 — Contributor-facing policy documentation

Contributor documentation shall define the MIT identifier, supported comment
forms, preamble rules, governed-file policy, explicit exclusions, local hook
installation/use, CI check name, and remediation steps for failures.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Developers can satisfy the policy consistently across
all repository components.

### REQ-044 — Complete repository coverage

The full-tree, staged, pre-commit, and CI enforcement paths shall use one
shared policy and shall cover every governed directory and file category.
Adding a governed file shall require no directory-specific opt-in, and an
exclusion shall require a reviewed policy-manifest change.

**Trace:** UI-024; `specs/design.md` SPDX header policy.
**Invariant impact:** Enforcement strength is independent of directory,
workflow entry point, or file provenance.

### Dynamic-bus traceability

| Requirement | Design coverage | Validation coverage |
| --- | --- | --- |
| REQ-008 | ICMP/TAP integration-test design; dynamic relay and switch selection | VAL-008; TC-023 through TC-028 |
| REQ-026 | Package and PnP topology | VAL-024; TC-067 |
| REQ-027 | Bus manager control plane | VAL-025; TC-068 |
| REQ-028 | Package and PnP topology; bus manager control plane | VAL-026; TC-069 |
| REQ-029 | Bus manager control plane; synchronization, IRQL, and teardown | VAL-015, VAL-027; TC-070, TC-071, TC-073 |
| REQ-030 | Child lifetime and TAP interface | VAL-015, VAL-028; TC-042, TC-072 |
| REQ-031 | Child lifetime and TAP interface | VAL-026; TC-069 |
| REQ-032 | Child lifetime and TAP interface; synchronization, IRQL, and teardown | VAL-027, VAL-031; TC-070, TC-071, TC-076 |
| REQ-033 | Package and PnP topology; migration and diagnostics | VAL-012, VAL-029; TC-038, TC-074 |
| REQ-034 | Migration and diagnostics | VAL-030; TC-075 |
| REQ-035 | Child lifetime and TAP interface | VAL-030; TC-072 |
| REQ-036 | Dynamic relay and switch selection | VAL-017, VAL-019; TC-055 |
| REQ-037 | SPDX headers on governed text files | VAL-032; TC-077 |
| REQ-038 | Preamble-preserving header placement | VAL-032; TC-078 |
| REQ-039 | Fail-closed full-tree and staged validation | VAL-032; TC-079 |
| REQ-040 | Commit-time SPDX rejection | VAL-033; TC-080 |
| REQ-041 | Required CI SPDX enforcement | VAL-034; TC-081 |
| REQ-042 | Explicit exclusions and diagnostics | VAL-032; TC-082 |
| REQ-043 | Contributor-facing SPDX documentation | VAL-035; TC-083 |
| REQ-044 | Complete repository coverage | VAL-032, VAL-033, VAL-034; TC-084 |

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
19. **Superseded:** Dynamic GUID-correlated endpoint discovery replaces the
    two static root identities; the initial relay and switch acceptance
    topology still selects two endpoints.
20. **Resolved:** Missing required I/O-ring runtime capabilities fail switch
    startup; overlapped I/O is not a switch fallback.
21. **Superseded:** Dynamic provisioning is required through the bus manager;
    arbitrary-N forwarding policy remains out of scope.
22. **Resolved:** Pending reads and writes use one shared positive even total
    depth across both endpoints, with equal per-endpoint capacity. There is no
    artificial fixed maximum; available memory, checked arithmetic, and
    I/O-ring resource limits determine the effective maximum.
23. **Resolved:** Valid TAP writes are captured and completed inline by the
    passive WDF I/O callback; no write manual queue or write work item is
    required.
24. **Resolved:** Captured TX frames are delivered directly to pending READ
    IRPs only when the packet callback runs at `PASSIVE_LEVEL`; elevated-IRQL
    callbacks use bounded nonpaged capture followed by passive deferred
    completion, and TX ring entries are not held indefinitely waiting for a
    read.
25. **Resolved:** Passive READ delivery claims frame/request ownership under
    the state lock but performs WDF buffer access, copying, requeue, and
    request completion only after releasing that lock.
26. **Resolved:** SPDX enforcement uses MIT identifiers and comment syntax
    compatible with each governed file type, following the established
    LexonGraph and ebpf-for-windows patterns.
27. **Resolved:** Binary files and generated outputs that cannot contain
    comments are explicit validator exclusions; source, scripts, metadata,
    specifications, and documentation are not excluded by default.

## Specification approval gate

REQ-026 through REQ-044 require approval together with their design and
validation coverage before implementation.
