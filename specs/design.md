# WinTapNetAdapterCx Design Specification

**Workflow:** `/evolve`  
**Phase:** Phase 2 — Specification Changes  
**Status:** Pending specification audit and user approval  
**Trace source:** `specs/requirements.md`

## Design principles

- The adapter is Ethernet/TAP only; IP/TUN mode is not part of the initial
  contract.
- One adapter has one exclusive elevated-administrator user-mode owner.
- User mode uses a Win32 device handle with overlapped read/write I/O.
- Queues are bounded and use cancellation-aware backpressure.
- Every asynchronous operation has one terminal completion and one owner at
  every transition.

## Build and dependency design

The project shall use CMake as its build-system entry point and shall support
the Visual Studio generator required by the selected Visual Studio/WDK
toolchain.

- CMake configuration shall identify the target architecture explicitly for
  x64 and ARM64.
- WDK and SDK packages shall be acquired through NuGet rather than relying on
  undeclared machine-global paths.
- Package versions, package sources, and any required workload/toolset
  versions shall be pinned or captured in repository configuration so a clean
  machine can reproduce the dependency graph.
- Configuration shall fail with an actionable error when the required WDK,
  SDK, Visual Studio generator, or architecture toolchain is unavailable.
- Build targets shall distinguish driver binaries, INF/package artifacts, and
  user-mode validation tools.
- The implementation shall document the exact CMake configure, build, package,
  install, and test-signing commands for each supported architecture.

The initial NuGet package IDs are `Microsoft.Windows.WDK.x64`,
`Microsoft.Windows.WDK.ARM64`, `Microsoft.Windows.SDK.CPP.x64`, and
`Microsoft.Windows.SDK.CPP.ARM64`, all pinned to version `10.0.28000.2526`.
They are required implementation dependencies, not implicit machine paths.

## Component boundaries

### Control/device boundary

The driver shall expose a named control/device interface through which an
administrator opens the adapter and performs the documented read/write
operations.

- The device security descriptor shall restrict open and control access to
  elevated administrators.
- Device naming and symbolic-link details shall be defined by the INF and
  driver design together; no undocumented path may be relied upon.
- A second open shall fail deterministically while an owner is active.
- Closing the owner handle, process termination, or cancellation shall begin
  owner teardown and complete all outstanding requests.

### NetAdapterCx adapter boundary

The driver shall create one NetAdapterCx adapter representing one virtual
Ethernet interface. Framework initialization, queue creation, adapter start,
pause, restart, stop, and deletion shall follow the WDK contract for the
selected NetAdapterCx version.

The implementation shall record the exact WDK/SDK and NetAdapterCx API
contracts used. Any callback whose IRQL or pageability depends on framework
state shall be annotated and placed accordingly.

## Packet ownership and direction

### User write to Windows networking stack

1. A completed overlapped write request owns its input buffer until validation
   and enqueueing finish.
2. The driver validates frame length and required Ethernet constraints before
   accepting the frame.
3. Once queued, ownership transfers to a nonpaged frame object owned by the
   adapter receive-injection path.
4. The user request completes only after the driver has copied or otherwise
   safely captured the frame; it shall not retain a user buffer.
5. The frame is submitted to the Windows networking stack using the verified
   NetAdapterCx receive/injection contract.
6. Completion, rejection, cancellation, adapter stop, or owner teardown
   releases the frame exactly once.

### Windows networking stack to user read

1. A frame arriving from the adapter transmit path is represented by a
   driver-owned frame object or framework-owned packet until copied.
2. If a pending overlapped read can accept the frame, the driver copies the
   complete frame into the user output buffer and completes the read.
3. If no read is available, the frame enters a bounded nonpaged receive queue.
4. A queued frame remains driver-owned until copied into a read buffer or
   discarded during an explicitly defined stop/error path.
5. Framework packet ownership is returned at the framework-required completion
   point and never retained across adapter teardown.
6. If a pending read cannot accept the frame because its output buffer is too
   small, that request completes with `STATUS_BUFFER_TOO_SMALL` and the frame
   remains queued for a later compatible read.

The implementation uses the installed WDK ring iterator contract:
`NetTxQueueGetRingCollection`, `NetRxQueueGetRingCollection`,
`NetRingGetPacketAtIndex`, `NetRingGetFragmentAtIndex`,
`NetRingIncrementIndex`, and `NetRingAdvanceIndex`. Fragment virtual
addresses are obtained through the `ms_fragment_virtualaddress` fragment
extension. This document specifies ownership transitions in addition to the
verified API names.

The pinned WDK 10.0.28000.2526 NetAdapterCx 2.5 headers verify
`EVT_PACKET_QUEUE_START`, `EVT_PACKET_QUEUE_STOP`, and
`EVT_PACKET_QUEUE_ADVANCE` callbacks, with queue advance allowed up to
DISPATCH_LEVEL. They also expose `NetAdapterStart` and `NetAdapterStop`.
No separate NetAdapter pause/restart callback API was found in the installed
headers, so pause/restart remains deferred rather than being represented by
an invented callback.

## Queue state and backpressure

The design shall maintain separate bounded queues for:

- pending user writes awaiting injection into the networking stack;
- received Ethernet frames awaiting user reads;
- pending overlapped reads and writes.

The pending read and write queues each have a finite limit of 256 requests.
Requests beyond the limit fail with an explicit busy status. Request counters
are owned by the queue transition that marks a request pending and are
decremented exactly once when the request is retrieved, cancelled, purged, or
removed after a forwarding failure.

Each queue shall have explicit states: `OPEN`, `CLOSING`, and `CLOSED`.

- `OPEN`: new work may be accepted.
- `CLOSING`: no new work is accepted; queued and pending work is drained or
  cancelled according to the stop reason.
- `CLOSED`: all queue references and requests are released; new work fails.

When a frame queue is full, a request may wait only while the queue is `OPEN`.
Waiting requests must be cancellable. Queue limits shall be finite and
configuration shall reject zero, overflowed, or unsupported sizes.

The implementation resumes blocked user writes from a passive WDF work item
after RX ring capacity is consumed; this keeps request-buffer capture on a WDF
I/O/work-item path rather than doing it from the packet callback.

Packet callbacks only manipulate nonpaged driver-owned state and schedule
passive work for user-buffer access and request completion.

Pending reads and writes are held by WDF manual queues. WDF owns cancellation
while a request is queued, and synchronous queue purge owns terminal
completion during cleanup; the driver does not register a second cancellation
owner for those requests.

RX ring index ownership is intentionally not changed by the maintenance
patches. The existing `BeginIndex` advancement requires runtime or verified
sample evidence before a semantic correction is safe.

## Synchronization

- A per-adapter lock shall protect adapter state, ownership state, queue
  membership, and transitions between `OPEN`, `CLOSING`, and `CLOSED`.
- Queue operations shall use one consistent lock ordering; the adapter state
  lock must not be reacquired from a completion path that already owns it.
- Cancellation shall atomically remove a request from its queue or mark it for
  completion by the owning worker.
- Adapter and frame objects shall use reference counting or an equivalent
  lifetime mechanism so teardown waits for in-flight callbacks and framework
  completions.
- No user buffer, framework packet, or queue node may be freed while a pending
  callback can still access it.

## IRQL and pageability

- Dispatch and cancellation paths shall obey the IRQL contract of the WDF and
  NetAdapterCx callbacks used.
- Data structures reachable at DISPATCH_LEVEL shall be nonpaged.
- Blocking queue waits and file-system/user-buffer operations shall run only
  at permitted IRQL and in a context that supports waiting.
- Pageable code shall not be called from a high-IRQL callback.
- The implementation shall use SAL annotations and verifier-friendly lock
  discipline for every public callback and asynchronous completion path.

## Adapter lifecycle

### Start

Create device state, queues, synchronization, adapter capabilities, and the
virtual Ethernet identity. Publish the adapter only after all required
resources are initialized. A partial start must unwind in reverse order.

### Pause/restart

Pause shall stop accepting new network traffic while preserving the owner and
user handle when the framework contract permits. Pending operations shall
remain cancellable and must not complete successfully with uninitialized
data. Restart shall resume only after queues and callbacks are in a consistent
state.

### Stop/removal

Transition the adapter and all queues to `CLOSING`, reject new work, cancel
pending user requests, stop new framework traffic, drain or fail queued
frames, return framework-owned resources, release the owner, and finally
destroy synchronization and memory. Surprise removal must use the same
ownership rules without waiting on user cooperation.

## Power management

The driver shall define behavior for system and device power transitions:
pause network traffic before resources become unavailable, preserve or fail
user requests deterministically, and restart only after hardware-independent
software state is valid. Since the adapter is software-only, no power-state
shortcut may bypass framework-required pause, stop, or restart callbacks.

On D0 exit, the control path enters `Suspended`, pending user requests fail
with `STATUS_DEVICE_NOT_READY`, queued frames are discarded through the
documented stop/error path, and callbacks and passive work items quiesce before
frame cleanup. D0 entry clears `Suspended` only after state is valid and
reschedules required passive drain/completion work.

## Error handling and cleanup

- Invalid frame lengths, unsupported flags, closed queues, unavailable owner
  state, and cancelled requests shall return explicit, documented errors.
- Allocation failure shall fail the affected operation and preserve all other
  queue invariants.
- Every failure path shall release resources in reverse acquisition order.
- Cleanup must be idempotent and safe when start or owner acquisition fails
  partway through.
- No broad catch-all or silent success fallback is permitted.

## Unresolved implementation details

- The selected WDK baseline uses `EVT_PACKET_QUEUE_ADVANCE` for both directions
  and the ring iterator APIs listed above.
- The initial frame contract is 14 through 1514 bytes, with a 1500-byte
  Ethernet payload/MTU. VLAN-tagged frames remain subject to the fixed maximum.
- The default directional frame queue limit is 256 frames and is not yet
  registry-configurable.
- **[ASSUMPTION]** A copy at the user/kernel boundary is acceptable for the
  initial implementation; zero-copy is not required by the approved
  requirements.
