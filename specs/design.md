<!-- SPDX-License-Identifier: MIT
  Copyright (c) 2026 WinTapNetAdapterCx contributors -->

# WinTapNetAdapterCx Design Specification

**Workflow:** `/evolve`  
**Phase:** Phase 2 — Specification Changes
**Status:** Direction-isolated packet-queue advancement specification changes
proposed; awaiting approval
**Trace source:** `specs/requirements.md`

## Design principles

- The adapter is Ethernet/TAP only; IP/TUN mode is not part of the initial
  contract.
- One adapter has one exclusive elevated-administrator user-mode owner.
- User mode uses a Win32 device handle with overlapped read/write I/O.
- Queues are bounded and use cancellation-aware backpressure.
- Every asynchronous operation has one terminal completion and one owner at
  every transition.
- TAP children are discovered through immutable GUID-correlated device
  interfaces; the initial relay and switch acceptance topology selects two
  endpoints without imposing a static-root identity.
- Protocol tests exercise the existing Ethernet/TAP boundary and do not add
  an IP/TUN mode or test-only driver path.

## SPDX header policy and enforcement

All governed repository text files carry `SPDX-License-Identifier: MIT` in a
comment syntax valid for the file type. The initial governed policy is:

- `*.rs`, `*.c`, `*.h`, `*.cpp`, and `*.hpp`: `//`.
- `*.ps1`, `*.psm1`, `*.psd1`, `*.sh`, `*.yml`, `*.yaml`, `*.toml`, `*.ini`,
  `*.txt`, and `CMakeLists.txt`: `#`.
- `*.md`: an HTML comment.
- `*.inx`: `;`.
- `.gitignore`, `.gitattributes`, and other hash-comment metadata:
  `#`.

`Cargo.lock` is governed as TOML-like metadata. `CMakePresets.json` and
other strict JSON files are explicit exclusions because JSON comments are not
semantically valid. `LICENSE`, binary files, and generated outputs are
explicit exclusions unless a future policy entry defines a safe syntax.
Shebangs, encoding declarations, and YAML front matter remain in their
required positions, with the SPDX header placed immediately after those
preambles.

The validator has full-tree and staged modes. Full-tree mode enumerates all
tracked governed files; staged mode examines added, copied, renamed, and
modified index entries. Both modes fail closed for absent, malformed,
wrong-syntax, or misplaced headers and use the same policy data locally and
in CI.

The exact governed extension and path rules, together with the exclusion
manifest, are version-controlled beside the validator. New text file types
are governed by default when a safe comment form is defined; adding an
exclusion requires a documented policy-manifest change and review.

The staged validator runs from a local pre-commit hook and rejects the commit
before creation when any staged governed file fails. This local control can
be bypassed and is not the remote authority. CI runs full-tree validation on
pull requests and protected-branch pushes, exposes the stable required check
name `SPDX headers`, and branch protection requires that check before merge.
Thus a commit may exist remotely only when the push or pull-request policy
accepts it; a bypassed local hook cannot bypass remote rejection.

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

### Rust driver and binding design

The production driver shall be a Rust kernel-mode crate built with the
`windows-drivers-rs` WDF ecosystem. The repository shall add a
`netadaptercx-sys`-style raw FFI crate generated from the pinned WDK
NetAdapterCx headers using the existing `wdk-build`/bindgen workflow.

Generated bindings shall be treated as an ABI boundary. Higher-level Rust
modules may wrap the raw bindings, but every wrapper shall document required
IRQL and pageability, framework-versus-driver ownership, callback lifetime,
status and failure behavior, and nonpaged allocation requirements.

The Rust target shall use `panic = "abort"` and shall not permit unwinding
across WDF, NetAdapterCx, or C ABI callbacks. Unsafe code shall be isolated
around FFI, raw packet-ring access, pointer validation, and kernel memory
operations. Rust references and ownership types shall never outlive the
framework object they represent.

Binding generation shall be reproducible from checked-in configuration and
pinned headers. Generated source may be checked in only if regeneration is
validated as equivalent; otherwise the build shall generate it deterministically
and fail when required inputs are missing.

### Rust-only package design

`cargo wdk build` is the sole driver build and package operation. CMake shall
remain a thin Rust-only wrapper that restores the pinned NuGet packages,
places the pinned `stampinf` x64 and `inf2cat` x86 tool directories on `PATH`,
and invokes `cargo wdk build` for the selected target architecture. Debug uses
the cargo-wdk default profile; Release passes `--profile release`.

The Rust package flow shall produce distinct bus and child INF/catalog/service
artifacts as specified in the dynamic KMDF bus and child design. No C/C++
driver project, INF, source, service, hardware ID, package fallback, or
selection switch remains in this branch. Legacy `ROOT\WinTapRust` and
`ROOT\WinTapRust2` identities are migration inputs only, not runtime models.

## Component boundaries

### Control/device boundary

The driver shall expose a named control/device interface for each supported
adapter instance through which an administrator opens that adapter and performs
the documented read/write operations.

- The device security descriptor shall restrict open and control access to
  elevated administrators.
- Device naming and symbolic-link details shall be defined by the INF and
  driver design together; no undocumented path may be relied upon.
- A second open of the same control device shall fail deterministically while
  an owner is active.
- Each GUID-keyed child retains an independently exclusive control interface;
  a process may hold one exclusive handle to each selected child concurrently.
- The routed dual-adapter harness shall validate GUID/interface mapping and
  fail rather than infer identity from PnP enumeration order.
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

The receive-filter capability structure shall advertise directed, broadcast,
multicast, all-multicast, and promiscuous filters with a multicast-address
capacity of 64. This is required because NetAdapterCx fails an upper layer's
packet-filter OID when the requested filters are not advertised; TCP/IP did
not create an IP interface after multicast-only or all-multicast builds were
deployed. The `EvtSetReceiveFilter` callback shall atomically replace the
active packet-filter flags and multicast-address list with the
framework-provided configuration. The list shall never exceed the declared
capacity.

The TAP data path has no hardware receive filter and shall continue to deliver
user-injected frames without software filtering. The cached filter state
satisfies the NetAdapterCx/upper-layer control-plane contract and makes the
accepted configuration available for diagnostics; it does not change
TAP-style frame delivery. The callback state shall be nonpaged, bounded, and
synchronized safely for its callback IRQL and any diagnostic readers.

The Rust implementation shall use the generated NetAdapterCx declarations for
all framework calls. It shall not duplicate C declarations manually or invent
Rust-specific lifecycle callbacks. The adapter shall be created during device
addition, configured and started at the framework-required preparation stage,
and stopped or deleted only through the verified NetAdapterCx/WDF lifecycle.

## Packet ownership and direction

### User write to Windows networking stack

1. A completed overlapped write request owns its input buffer until validation
   and inline capture finish.
2. The driver validates frame length and required Ethernet constraints before
   accepting the frame.
3. A nonzero write shorter than 14 bytes or longer than the negotiated maximum
   frame size completes
   with `STATUS_INVALID_PARAMETER` before it enters a manual queue, consumes
   pending I/O capacity, or creates a frame object. A zero-byte `WriteFile`
   completes as a Win32 no-op before the request reaches this callback.
4. Once captured, ownership transfers to a nonpaged frame object owned by the
   adapter injection queue. The injection queue is distinct from the queue
   used for stack transmit capture and user reads.
5. The user request completes only after the driver has copied or otherwise
   safely captured the frame; it shall not retain a user buffer.
6. The frame is submitted to the Windows networking stack using the verified
   NetAdapterCx receive/injection contract.
7. Completion, rejection, cancellation, adapter stop, or owner teardown
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

### NetAdapterCx RX indication

The injection queue is the software adapter's receive-completion source. The
write I/O callback captures a validated user frame into that queue, completes
the write after capture, and never wakes the read-completion worker for that
frame. The transmit queue capture path is the only producer for the bounded
queue consumed by TAP reads.

When the RX queue is polling, `EVT_PACKET_QUEUE_ADVANCE` consumes injection
frames. When polling is disabled, `EVT_PACKET_QUEUE_SET_NOTIFICATION_ENABLED`
records whether notification is armed; an injection producer may request more
RX polling only while that state is armed and only once per enable cycle. The
notification call shall occur outside a lock that could be reentered by the
serialized packet-queue callbacks. `EVT_PACKET_QUEUE_SET_NOTIFICATION_ENABLED`
does not copy an injection frame or mutate ring indices itself.
Owner-only TAP cleanup that leaves the RX queue running preserves an armed
notification cycle; D0 exit, RX queue stop, RX cancellation, and release
hardware disarm it.

Within RX queue advance, the driver owns entries beginning at `BeginIndex` and
ending immediately before `EndIndex`. It copies a complete injection frame
only to a driver-owned packet/fragment pair. Before copying, it initializes
the fragment `Offset` to zero, bounds the complete frame against capacity, and
sets `ValidLength` to the copied length. It clears `Ignore`, initializes
`FragmentIndex`, `FragmentCount`, and all applicable `NET_PACKET_LAYOUT`
fields, then advances both packet and fragment `BeginIndex` together. It does
not modify `EndIndex` or advance either begin index beyond the corresponding
end index. `NextIndex` remains optional queue-local post bookkeeping and is
not a cross-callback ownership signal.

RX cancellation first marks every unindicated packet ignored, then returns
the outstanding packet and fragment entries to NetAdapterCx by advancing their
`BeginIndex` values to their corresponding `EndIndex` values. This is the
only permitted RX ring mutation outside `EVT_PACKET_QUEUE_ADVANCE`. Adapter
stop, owner teardown, and queue deletion close and release the injection and
capture queues independently. No queued injection frame may be exposed by a
TAP read, and no captured TX frame may be injected into the networking stack.

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

### TX capture and READ delivery

TX capture shall preserve the NetAdapterCx ring ownership contract. A packet
callback owns entries in `[BeginIndex, EndIndex)` and must finish any
inspection and data copy before advancing `BeginIndex`. It shall not leave
framework-owned entries indefinitely pending while waiting for a user READ
IRP, because doing so can exhaust the TX ring and stall the networking stack.

When the TX packet callback is executing at `PASSIVE_LEVEL`, it may match a
compatible pending READ IRP, retrieve the output buffer, copy the complete
frame directly from the framework TX fragment(s), complete the request, and
then return the framework-owned entries. The callback must release any
lifecycle/control or queue-local lock before calling WDF routines that may access request
buffers or complete requests.

When the callback executes above `PASSIVE_LEVEL`, it must not access a user
buffer or complete a request requiring user-buffer processing. It shall copy
the frame into nonpaged driver-owned storage, advance the framework ring, and
use the existing passive-level work item to drain captured frames into
pending READ IRPs. A READ callback may opportunistically drain an already
queued capture frame at passive level, but it shall not inspect or mutate TX
ring indices.

If no compatible READ IRP is available, the captured frame enters the bounded
`capture_queue`. If the capture queue is full, the driver shall apply the
documented deterministic backpressure/error behavior without retaining the
framework TX entry indefinitely. If a pending READ output buffer is too
small, the request completes with `STATUS_BUFFER_TOO_SMALL`; the driver
stages the frame in `capture_queue`, advances the framework ring, and leaves
the frame available for a later compatible read.

The passive READ callback and completion work item shall use capture-queue
synchronization only to transition ownership. The sequence for a
queued-frame/READ pair is:

1. Under capture-queue synchronization, dequeue or claim one captured frame and one READ
   request, or leave both available if pairing is not possible.
2. Release capture-queue synchronization before retrieving the WDF output buffer, copying
   bytes, requeueing a frame, or completing the request.
3. If the output buffer is too small or cannot be retrieved, re-acquire
   capture-queue synchronization to requeue the still-owned frame, then complete the request
   outside the lock with the documented error.

Packet callbacks, the passive READ callback, and the work item must use the
same ownership transitions so a frame or request cannot be claimed twice.
Teardown closes the capture queue before draining it and prevents new claims;
already claimed pairs finish through the outside-the-lock completion path.

Adaptive-polling mode does not retain READ requests for this direct-delivery
path. In that mode, TX capture always copies a valid frame into the bounded
nonpaged `capture_queue` before returning the framework-owned packet entries.
The enqueue transition records whether the queue was empty before insertion.
An empty-to-nonempty transition evaluates and claims a registered readable
`WAIT_FOR_CHANGE` request; it does not access a user buffer or complete that
request while holding queue-local synchronization. This preserves the
DISPATCH_LEVEL-safe packet callback contract while making readiness visible to
user mode without a pending READ IRP.

### Driver-owned frame storage

Driver-owned frames shall be represented by fixed-size elements from an OS
nonpaged lookaside list. Each element contains a `FRAME_MAXIMUM`-byte payload
area, a valid-length field, and only the metadata required to identify its
current ownership. The lookaside list owns element caching and obtains backing
storage from the OS when needed; the driver does not maintain a parallel free
list or impose a second capacity limit.

The injection and capture queues remain separate bounded owners. A producer
acquires a lookaside element before copying data from a user request or a
NetAdapterCx fragment into storage that must outlive the current callback. If
acquisition fails, the producer reports the existing explicit resource
exhaustion result, completes or rejects the operation according to its queue
contract, and returns any framework-owned ring entry without retaining it.
Direct passive-level TX delivery may continue to copy directly into a
compatible user output buffer and does not require a lookaside element when
the framework entry is returned in the same callback.

The element's valid length is set only after a complete frame copy succeeds.
On dequeue, delivery, rejection, cancellation, or queue drain, ownership
transitions are serialized by the owning queue's local synchronization and the
element is returned exactly once. Reuse must not expose bytes from a previous frame;
the implementation shall clear the prior valid payload range before release
or establish an equivalent non-observability guarantee.

Lookaside creation occurs before adapter datapath publication and uses a
nonpaged element suitable for the highest IRQL at which frame acquisition or
release occurs. Teardown first closes both frame queues and prevents new
acquisitions, then drains queued and in-flight elements, and only then deletes
the lookaside list. Adapter stop, D0 exit, owner close, surprise removal, and
failed initialization follow the same ordering and preserve exactly-once
release.

## Queue state and backpressure

The design shall maintain separate bounded queues for:

- received Ethernet frames awaiting user reads;
- pending overlapped reads.

The owning queue-local lock protects frame-queue and request-ownership
transitions, not WDF request buffer access or request completion. A path that
claims a frame or request under the capture or injection lock shall release
that lock before invoking WDF buffer APIs, copying to a user buffer, or
completing the request. The lifecycle/control lock is excluded from ordinary
queue ownership transitions.

Write admission shall use a bounded counter and the bounded injection queue;
valid writes shall not wait in a WDF manual queue.

The injection and captured-frame queues each use the configured frame limit
independently. Each has queue-local synchronization; neither queue operation
may acquire a lock held by the other packet direction. Their fullness, close,
reopen, dequeue, and teardown transitions shall not affect one another.

The switch's pending read and write capacity is one validated positive even
total configured value shared across both endpoints. Each endpoint receives
half of that total capacity. Requests beyond the allocated capacity fail with
an explicit busy/resource status. Request counters are owned by the queue
transition that marks a request pending and are decremented exactly once when
the request is retrieved, cancelled, purged, or removed after a forwarding
failure. The implementation shall not impose an additional fixed maximum;
allocation and arithmetic must be checked before resources are published.

Each queue shall have explicit states: `OPEN`, `CLOSING`, and `CLOSED`.

- `OPEN`: new work may be accepted.
- `CLOSING`: no new work is accepted; queued and pending work is drained or
  cancelled according to the stop reason.
- `CLOSED`: all queue references and requests are released; new work fails.

When the injection queue is full or closed, an inline write fails
deterministically; it does not wait in a WDF queue. Queue limits shall be
finite for a given run and configuration shall reject zero, odd, overflowed,
or unsupported sizes. A requested depth that cannot be represented, allocated,
registered, or supported by the I/O-ring API shall fail explicitly rather than
being clamped or wrapped.

Packet callbacks only manipulate nonpaged driver-owned state and schedule
passive work for packet-driven user-buffer access and request completion,
except for the explicitly permitted passive-level direct READ delivery.
The inline write callback performs its verified capture and request completion
at its own WDF execution level. RX ring mutation is confined to
`EVT_PACKET_QUEUE_ADVANCE` for indication and `EVT_PACKET_QUEUE_CANCEL` for
return; a notification callback may only arm/disarm notification and request
a subsequent advance.

For a control handle that has not enabled adaptive-polling mode, pending reads
are held by a WDF manual queue. WDF owns cancellation while a read request is
queued, and synchronous queue purge owns terminal completion during cleanup.
Valid writes are completed inline and therefore are not owned by a pending
write queue.

### Adaptive-polling control contract

Adaptive-polling mode is negotiated through a versioned driver-defined control
operation on the exclusive control handle. The enable request carries a
version and flags, and the successful response identifies the supported
protocol version and accepted flags. The switch must enable the mode on every
selected endpoint before it uses the adaptive path. A failed or unsupported
negotiation leaves the handle in its legacy mode; it must not partially alter
READ, WRITE, queue, or cancellation semantics.

The switch falls back to its existing I/O-ring pending-READ loop only when an
endpoint reports that adaptive-polling mode is unsupported or its protocol
version is incompatible. It reports all other negotiation failures and does
not start a partially adaptive data plane. This fallback preserves additive
deployment with an older driver while requiring explicit successful opt-in
before using the new contract.

Adaptive enable diagnostics identify the manager-returned endpoint GUID and
record one of: accepted version/flags, unsupported Win32 error, incompatible
version/flags, or fatal negotiation failure. The diagnostics contain only
control-plane values; they never include a frame or interface payload.

The driver-defined `WAIT_FOR_CHANGE` operation has a versioned input
containing a readable/writable interest mask and a fixed-size output containing
the satisfied mask. It is valid only after successful mode enable. The driver
permits at most one wait request per exclusive handle. A second request fails
explicitly and cannot cancel, replace, or steal the registered request.

The driver shall use a nonblocking atomic wait-publication state machine
instead of the state lock for `WAIT_FOR_CHANGE` coordination. Readability is
true while the capture queue is nonempty. Writability is true while the
injection queue has capacity. Each queue publishes its readiness state and a
monotonic transition generation with release semantics after its queue-local
transition.

Wait registration shall atomically claim a nonclaimable `REGISTERING` record
before it publishes the single request slot, then publish the requested
readiness metadata and call `WdfRequestMarkCancelableEx`. A packet-queue stop
that precedes request-slot publication is detected by rechecking the started
state of every direction named by the wait interest mask; a stop after slot
publication can atomically change `REGISTERING` to teardown ownership. The
wait shall not become claimable until marking succeeds. If `WdfRequestMarkCancelableEx`
returns `STATUS_CANCELLED` or any other failure, WDF does not invoke the
cancellation callback; registration removes `REGISTERING` metadata, completes
the request with that status, and does not publish it. If cancellation runs
while registration is in progress after successful marking, the callback
completes the request, records its claim in the shared wait state, and
registration retires that state without completing the request again. The
same rule applies when teardown has already claimed the `REGISTERING` record,
so a cancelled teardown handoff can never leave the wait state stuck and
reject later waits as busy. If marking succeeds and registration wins the
publish transition, it publishes a record containing its interest mask and the
observed generations, then re-reads readiness. It claims immediately if an
interested condition was already or becomes satisfied; otherwise it remains
published.

The wait record is a single atomic word holding the claim state, the satisfied
readiness mask, and a monotonic registration sequence. Publishing a
`REGISTERING` record advances the sequence and clears the mask in one store,
and retiring a record advances it again. Every state transition is a
compare-exchange on the whole word, so:

- a claim publishes its readiness bits and its `SCHEDULED` transition
  indivisibly, and the passive worker reads the mask from the very word whose
  transition it won;
- a claim that stalled across a retirement observes a different sequence, so
  its compare-exchange fails and its readiness bits can neither leak into a
  later registration nor schedule one;
- concurrent transitions accumulate into the same record: the first moves it
  to `SCHEDULED` and enqueues the worker, and a later one only ORs its bits in
  without enqueueing again.

A readable or writable transition may atomically change the published record
from `PENDING` to `SCHEDULED` and enqueue the passive worker. That atomic
claim and the work-item enqueue are the only wait operations permitted in a
packet-queue advance callback: it shall not acquire a wait lock, call
`WdfRequestUnmarkCancelable`, or complete a request. Cancellation and teardown
use the same single-winner atomic claim; teardown may also claim a `SCHEDULED`
record. A claimant that takes terminal ownership moves the record to
`UNMARKING`, calls `WdfRequestUnmarkCancelable`, and:

- on `STATUS_SUCCESS`, retires the wait and then completes the request;
- on `STATUS_CANCELLED`, publishes an `UNMARK_CANCELLED` handoff. If the
  cancellation callback had already published `CANCEL_ARRIVED`, this claimant
  retires the wait and completes the request; otherwise the record stays
  published and the cancellation callback resolves and completes it. Because
  the cancellation callback never completes a request while a claimant is
  unmarking, `WdfRequestUnmarkCancelable` is never called on a completed
  request;
- on any other status, retires the wait and completes the request with that
  status, because the cancellation callback will not run and would otherwise
  leave the record permanently claimed.

Retirement before completion is the general rule: as soon as terminal
ownership is resolved, the owning path clears the wait state and the published
request slot before calling `WdfRequestComplete`, so a handle the framework
recycles after completion can never match the slot. The two paths whose
ownership is still unresolved — the `STATUS_CANCELLED` unmark handoff and a
cancellation that observes a `REGISTERING` or teardown-claimed record — leave
the record published for the path that still owes resolution and must not be
reordered.

The WDF cancellation callback removes or observes the claim through the same
atomic state and participates in exactly one completion. All completion occurs
outside queue and lifecycle locks. A losing path observes the terminal claim
and does not complete the request. The registration record remains valid until
the mark/publish race and any cancellation callback have reached a terminal
outcome. This protocol prevents a readiness change between the initial
observation and publication from being lost without requiring RX and TX queue
advances to acquire a common lock.

The control request queue is created with queue-level automatic
synchronization and passive execution, so the framework serializes its request
handlers with `EvtIoStop`. `EvtIoStop` therefore never observes a control
request that the dispatch callback still owns, and it never observes the
pre-publication or `REGISTERING` windows. For the published wait it performs
the same single-winner atomic claim as any other teardown path:

- if the claim wins, it holds exclusive ownership, unmarks, and completes the
  request, which satisfies the stop;
- if the claim loses to the cancellation callback or the passive completion
  worker, that owner is free to be manipulating the handle concurrently.
  `EvtIoStop` takes no action at all — neither `WdfRequestStopAcknowledge` nor
  completion — because the framework treats the owner's bounded completion as
  satisfying the stop.

Because a callback holding the queue synchronization lock can be waited on by
a path that completes a request on the same queue, no such callback may block
on a driver lock a stop, power, owner-cleanup, or removal path can hold. The
adaptive-enable handler therefore acquires the direct-read serialization lock
without blocking and reports `STATUS_DEVICE_BUSY` when it is unavailable.

When adaptive-polling mode is enabled, an empty READ completes with
`STATUS_NO_MORE_ENTRIES`; it is never placed on the manual read queue. An
accepted WRITE continues to complete inline, and a full injection queue
returns `STATUS_DEVICE_BUSY`. A transition remains level-sensitive: a wait
submitted after a queue becomes readable or writable completes immediately,
which prevents a lost wakeup between polling and blocking.

Owner close, file cleanup, D0 exit, adapter stop, surprise removal, queue
closure, and release hardware first prevent a new wait from being published,
then claim/cancel the published wait through the atomic wait-publication
protocol. Owner close additionally retires the owner generation, closes
packet-callback admission, and drains the direction-specific callback leases
before it clears or reopens the frame queues, so an in-flight packet callback
can neither indicate nor requeue a retired owner's frame into the next owner's
queues.
They purge legacy manual READ requests only for legacy mode. Adaptive-mode
teardown neither leaves a wait request published nor resumes a manual queue
for an adaptive READ.

The installed NetAdapterCx ring guidance verifies that a client driver owns
`[BeginIndex, EndIndex)` and returns completed RX entries by advancing
`BeginIndex`; `EndIndex` remains framework-owned. The receive path shall
follow that contract and retain runtime/Driver Verifier validation for every
ring-capacity and cancellation boundary.

## Synchronization

- A per-adapter lifecycle/control lock shall protect control-handle state,
  owner generation, receive-filter state, and transitions between `OPEN`,
  `CLOSING`, and `CLOSED`. It shall not protect packet-ring traversal or
  ordinary directional frame-queue operations.
- Injection and capture queues shall have independent queue-local locks.
  RX advance may acquire only injection-queue synchronization and TX advance
  may acquire only capture-queue synchronization. Neither advance callback
  may acquire the lifecycle/control lock or a shared adaptive-wait lock while
  processing packet rings.
- Queue identity, ring collection, fragment extension, immutable frame limit,
  and frame-pool identity shall be published before the corresponding queue
  starts. They remain immutable until the framework has stopped and quiesced
  that queue. Release hardware shall invalidate them only after all callbacks
  are quiescent; if this cannot be proven from the callback contract, a
  lock-free callback lease shall prevent invalidation until all leases drain.
- Each datapath direction shall have its own callback-lifetime lease word that
  packs per-scope admission closers with the outstanding lease count, so
  admission closure and lease acquisition are one atomic operation and no
  callback can be admitted after admission closes. Lease acquisition shall
  never block and is therefore safe at DISPATCH_LEVEL. Owner cleanup, D0 exit,
  and release hardware shall close admission for both directions, wait at
  `PASSIVE_LEVEL` for both lease counts to drain while holding no lock that a
  leased callback can wait on, and only then clear, reopen, or invalidate queue
  state. During hardware preparation, the hardware closer may be released
  before NetAdapterCx creates replacement queues only while lifecycle remains
  non-`OPEN`; any admitted callback returns without touching queue state, and
  each queue context is fully published before that queue can invoke its
  callback.
- Quiescence shall nest. Owner cleanup, power transition, and hardware
  transition each own a distinct closer bit of the lease word, so a scope
  readmits only its own closure and callbacks stay denied while any other
  scope is quiesced. Owner cleanup releases its closer through a scope guard so
  an early return cannot strand admission closed, and releasing it cannot
  readmit callbacks while D0 exit or release hardware is still quiesced. The
  power closer is set by D0 exit and released by D0 entry; the hardware closer
  is held from device creation until prepare hardware, and thereafter is set by
  release hardware and released by prepare hardware. Because each scope owns a
  distinct bit, unbalanced or repeated readmission cannot underflow a count or
  readmit another scope.
- A leased callback observes the owner generation and lifecycle state under
  its lease, so the ownership check and the subsequent enqueue or requeue are
  atomic with respect to owner cleanup. A frame that fails that check belongs
  to a retired owner and is released rather than requeued.
- Passive READ delivery and capture-drain work hold the capture-direction
  lease from the lifecycle/owner snapshot through dequeue, WDF buffer
  delivery, and any requeue. Owner cleanup reopens queues first, then changes
  lifecycle from an owner-specific closing state to `OPEN` with a
  compare-exchange. Cleanup claims that state only from `OPEN`, so a power or
  hardware transition that started before or during cleanup retains lifecycle
  ownership and prevents cleanup from resuming the manual queue.
- Adaptive wait publication and claiming shall use the atomic state machine
  above. It shall not impose a shared lock acquisition on packet advancement.
- Cancellation shall atomically remove a request from its queue or mark it for
  completion by the owning worker.
- Adapter and frame objects shall use reference counting or an equivalent
  lifetime mechanism so teardown waits for in-flight callbacks and framework
  completions.
- No user buffer, framework packet, or queue node may be freed while a pending
  callback can still access it.
- Adapter teardown shall wait for an executing inline write callback before
  closing the injection queue, destroying its lock, or releasing adapter state.
- Adapter teardown shall also wait for direct passive-level READ delivery and
  passive capture-drain work before releasing pending requests, capture
  frames, or packet-ring state.
- Direct passive-level TX delivery shall acquire the direct-read serialization
  lock without blocking. A packet-queue advance callback that cannot acquire
  it immediately falls back to bounded nonpaged capture, so no packet callback
  ever waits on a lock that owner cleanup, D0 exit, or release hardware holds.
- No callback that runs under the control queue's automatic synchronization
  lock may block on a driver lock that a path completing a control request can
  hold, because such a completion needs the same queue lock.

## IRQL and pageability

- Dispatch and cancellation paths shall obey the IRQL contract of the WDF and
  NetAdapterCx callbacks used.
- Data structures reachable at DISPATCH_LEVEL shall be nonpaged.
- Blocking queue waits and file-system/user-buffer operations shall run only
  at permitted IRQL and in a context that supports waiting.
- Pageable code shall not be called from a high-IRQL callback.
- The write callback's execution-level and allocation contract shall be
  verified before the adapter is published; unsupported inline requirements
  shall fail initialization explicitly.
- The TX capture callback shall branch on its actual execution IRQL. Only a
  passive-level callback may retrieve READ output buffers or complete a read
  through direct delivery; elevated-level callbacks shall use nonpaged
  capture and passive deferred completion.
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
  Nonzero control writes outside the 14-byte-to-negotiated-maximum frame range shall
  complete with `STATUS_INVALID_PARAMETER`, which the Win32 caller observes as
  `ERROR_INVALID_PARAMETER` (87). Zero-byte `WriteFile` calls are native
  Win32 no-ops and do not dispatch to the driver.
- Allocation failure shall fail the affected operation and preserve all other
  queue invariants.
- Every failure path shall release resources in reverse acquisition order.
- Cleanup must be idempotent and safe when start or owner acquisition fails
  partway through.
- No broad catch-all or silent success fallback is permitted.
- The PowerShell harness shall receive the native error for `ReadFile`,
  `WriteFile`, `GetOverlappedResult`, and `CancelIoEx` through an explicit
  C# wrapper output captured in the same managed call. It shall not query
  last-error state independently after crossing the C#/PowerShell boundary.

## ICMP/TAP integration-test design

The REQ-008 test is an external user-mode acceptance workflow. It shall create
one test GUID child through the manager, use that child's discovered TAP
interface and overlapped read/write contract, and shall not add test-only ARP
or ICMP handling to the driver.

### Provisioning and isolation

1. Install and start the test-signed package, create one test GUID child
   through the manager, then uniquely identify the resulting virtual Ethernet
   and TAP interfaces by GUID rather than interface index or fixed path.
2. Record the interface address, routes, administrative state, and driver
   state needed for restoration.
3. Assign `192.0.2.1/30` only to the test interface. Do not add a default
   route; reject address collisions or ambiguous adapter matches.
4. Open the GUID-correlated TAP interface exclusively with overlapped I/O
   before the protocol exchange.

### Request/reply packet flow

The test shall cause the Windows networking stack to issue an Echo Request to
`192.0.2.2`, then use the TAP handle as the observation and injection
boundary. Because the test network has no external peer, the workflow shall
first handle address resolution:

1. A pending overlapped read receives the Ethernet ARP request generated for
   `192.0.2.2`.
2. The test validates the ARP request fields and writes the corresponding
   Ethernet ARP reply for the test interface and peer address.
3. The Windows stack then emits the ICMP Echo Request, which the workflow
   reads and validates.

4. The test validates Ethernet endpoints and EtherType, IPv4 version/header
   length/total length/addresses/TTL/protocol, and ICMP type/code,
   identifier/sequence, payload, and checksum.
5. The reply swaps IPv4 addresses and Ethernet endpoints, changes ICMP type
   to Echo Reply, preserves identifier, sequence, and payload, and
   recomputes IPv4 and ICMP checksums.
6. The reply is submitted with an overlapped write and completes with the
   complete frame length.
7. The test verifies that the Windows stack reports the matching successful
   Echo Reply within a bounded timeout.

The parser rejects truncated headers, inconsistent lengths, invalid ARP
fields, fragments, unexpected protocols, invalid checksums, and packets that
do not match the request identity. Unrelated well-formed frames may be
ignored only while the bounded timeout remains enforceable; unrelated
malformed frames fail the test.

### Cleanup and failure handling

Cleanup runs from a guaranteed finalization path and is idempotent. It
cancel/completes pending operations before closing the handle, removes only
the test address and any test-created route, removes the recorded test child,
restores the interface and driver state, removes the test package where
permitted, and retains command output, packet bytes, driver status, and event
logs on failure.

Provisioning, packet-validation, timeout, or cleanup errors are test
failures. Cleanup failures are reported in addition to the primary failure
and cannot be converted into success.

## Routed dual-adapter relay-test design

REQ-015 is an external user-mode acceptance workflow. It does not add a
driver-internal peer link or alter TAP frame semantics. The dedicated
`tests\run-wintap-dual-adapter-harness.ps1` script owns the test topology and
uses the two existing control-device contracts independently.

### Clean-environment preflight and provisioning

1. Require elevation and test-signing policy before modifying device or network
   state.
2. Enumerate PnP devices matching `ROOT\WinTapRust` and
   `ROOT\WinTapRust2`. If either exists, fail without disabling, removing, or
   reconfiguring it.
3. Snapshot matching driver-store package identities before installation.
   Resolve the host-appropriate `devcon.exe` from the pinned WDK package
   (`microsoft.windows.wdk.<architecture>\10.0.28000.2526`); fail if it is not
   available.
4. Use the Microsoft-documented `devcon install <INF> <HardwareId>` operation
   to create `ROOT\WinTapRust` followed by `ROOT\WinTapRust2`. Record the
   exact PnP instance IDs, package installation result, and command output.
5. Wait for exactly two enabled WinTap adapters, validate their hardware IDs,
   service, and permanent/current MAC addresses
   `02-57-54-41-50-01` and `02-57-54-41-50-02`. Map those MAC identities to
   `\\.\WinTapRust` and `\\.\WinTapRust2`, respectively. Any missing,
   duplicate, unexpected, or ambiguous identity is a failure.

The script may remove a driver-store package only when comparison with the
pre-install snapshot proves that the current run added it. A pre-existing
package is retained even when no matching device existed at preflight.

### Isolated routed topology

The default topology uses documentation-only addresses that do not overlap
REQ-008:

| Endpoint | IPv4 | IPv6 | Control endpoint |
| --- | --- | --- | --- |
| A | `198.51.100.1/30` | `2001:db8:515:1::1/64` | `\\.\WinTapRust` |
| B | `198.51.100.2/30` | `2001:db8:515:1::2/64` | `\\.\WinTapRust2` |

The script assigns the addresses only after verifying no system-wide
collision. It must not add a default route. It creates active-store static
neighbor entries mapping B's IPv4/IPv6 addresses to B's MAC on A, and A's
addresses to A's MAC on B.

Following the DuoNIC model, the script installs exact on-link host routes:

| Destination | Egress interface | Next hop |
| --- | --- | --- |
| A's IPv4 `/32` and IPv6 `/128` | B | `0.0.0.0` / `::` |
| B's IPv4 `/32` and IPv6 `/128` | A | `0.0.0.0` / `::` |

The script uses active-store route and neighbor configuration so it is not
persistent across reboot. It verifies the exact routes win over the connected
prefix routes and records the route table before and after configuration.
Narrow inbound firewall rules use a unique run identifier and permit only the
two test endpoints/prefixes; no global firewall profile or unrelated rule is
changed.

### Bidirectional relay and protocol assertions

The harness opens both control endpoints with overlapped I/O and maintains
independent outstanding reads. A completed Ethernet frame from A is validated
for the supported frame bounds before it is written to B; a completed frame
from B is handled symmetrically. The source read buffer remains live until the
destination write reaches a terminal completion. On timeout, cancellation, or
failure, all outstanding operations are cancelled and completed before their
buffers or handles are released.

The relay forwards validated IPv4/IPv6 data traffic without modifying Ethernet
bytes. Because active-store permanent peer neighbors eliminate discovery
dependency, it validates and counts ARP and ICMPv6 Neighbor Discovery
(including Duplicate Address Detection) but suppresses them instead of writing
them to the peer endpoint. It rejects malformed or out-of-contract frames
rather than forwarding them. The IPv4 client sends an unbound ICMP Echo to B
and the IPv6 client sends an unbound ICMPv6 Echo to B. Captured traffic must
prove each request originated on A, crossed A-to-B, and each reply crossed
B-to-A. A byte-identical A-to-B request returned by the B-to-A TAP read is a
directional-isolation failure and shall not be silently filtered. Unrelated
traffic may be recorded, suppressed when it is ARP/NDP, or discarded and
rearmed according to the relay filter; only a validated B-to-A Echo Reply
satisfies the round-trip assertion. Assertions validate Ethernet addresses, IP
version, addresses, header length and total length where applicable,
ICMP/ICMPv6 type, code, identifier, sequence, payload, IPv4 checksum, and
ICMP or ICMPv6 checksum including the IPv6 pseudo-header.

Neighbor Solicitation validation follows RFC 4861. Address-resolution
solicitations use a solicited-node multicast destination and include the
source link-layer option on Ethernet. A valid unicast Neighbor Unreachability
Detection probe may use the target's unicast destination and omit that option.
Duplicate Address Detection uses the unspecified source, a solicited-node
multicast destination, and no source link-layer option. Each valid control
frame is counted, suppressed, and rearmed without a peer write.

### Cleanup and diagnostics

Finalization preserves the first failure and reports cleanup failures
separately. It cancels/completes all I/O, closes both handles, removes only
recorded test-created firewall rules, routes, neighbor entries, and addresses,
then removes the two recorded PnP device instances. If and only if this run
added the driver-store package, it removes the recorded published INF after
both devices are gone. It captures adapter, address, route, neighbor,
firewall, PnP, service, driver-event, command, and packet diagnostics on every
failure path.

## Two-TAP user-mode switch design

The switch is a privileged user-mode component above the existing TAP control
devices. It does not add a driver-internal peer link, change the driver packet
contract, or provision PnP devices. The first release constructs an endpoint
collection containing exactly `\\.\WinTapRust` and `\\.\WinTapRust2`, maps each
endpoint to its stable adapter identity, and opens both handles exclusively.
Endpoint selection is represented by endpoint identity rather than by a
hard-coded destination branch so a future dynamic provisioning effort can
extend the collection without changing slot ownership rules.

### Forwarding database and frame policy

The switch maintains a bounded 4,096-entry table keyed by source MAC address
and VLAN identifier. A learned entry records the endpoint on which the source
was observed. Learning occurs before destination resolution; an observation
on the other endpoint immediately replaces the prior endpoint. Existing
entries are retained when the table is full, and entries do not age in the
first release.

Known unicast is sent to the learned destination endpoint. Unknown unicast,
broadcast, and multicast are sent to every eligible endpoint other than the
source endpoint. With the two-endpoint first-release collection, each flood
has one recipient. A frame whose destination resolves to its source endpoint
is not written. Frame validation, VLAN parsing, and forwarding decisions run
before a write is built; malformed or unsupported frames fail or are recorded
according to the switch validation contract and are never forwarded.

### I/O-ring resources and completion state

Before creating the data plane, the switch calls `QueryIoRingCapabilities` and
`IsIoRingOpSupported` for the required read and write operations. It records
the maximum supported version and creates the newest usable ring that meets
the required contiguous read/write contract. Version-3 operations are the
initial baseline. Version-4 scatter/gather operations are selected only when
runtime probes and dedicated validation confirm support; otherwise the switch
continues with the validated contiguous path. If required read/write support
is absent, startup fails explicitly.

The switch registers both handles and a pool of negotiated-maximum-sized buffers sized from
the validated shared total. The total is split equally between the two
endpoints, with checked multiplication and allocation before ring
registration. FDB capacity remains 4,096 entries. Each buffer slot has the
states `Free`, `ReadPending`, `Dispatching`, `WritePending`, and `Free`, with a
generation counter incremented on reuse. Completion `userData` uses bits
0-30 for the slot, bits 31-62 for the generation, and bit 63 for cancellation.
The endpoint is derived from the slot partition and the operation direction is
retained with the active slot, so every live operation remains uniquely
identified without truncation or collision. Encoding and decoding shall use
checked operations and reject unknown or out-of-range values. A source slot
remains unavailable for repost until its read and every peer write using that
slot have terminal completions.

Startup validates the positive even total, derives equal endpoint capacity,
checks all size calculations, allocates the complete pool, configures ring
depths, and registers every buffer before entering `Running`. Any failure
unwinds all allocated resources and reports the primary error explicitly.

An I/O-ring completion result of Win32 `ERROR_BUSY` (`HRESULT 0x800700AA`)
is transient resource contention, equivalent to the existing device-busy
retry cases. The runtime clears the submitted state without cancelling the
slot, waits using the bounded exponential backoff, rebuilds the same read or
write operation with the same completion identity and registered buffer, and
resubmits it. The runtime permits at most eight busy retries for one
operation; exhaustion cancels and releases the consumed slot before
reporting an explicit error, allowing normal shutdown to complete without
re-enqueuing that operation. The retry path shall not allocate a new slot,
release the buffer, change the endpoint, or hide a retry-build failure. Other
non-success completion results remain fatal and preserve the primary error
for shutdown.

Shutdown, endpoint removal, and cancellation stop new reads, submit operation
cancellation, drain each original completion, and only then deregister buffers
and handles or close the ring. A completion with an unknown slot, direction,
or generation is rejected as stale and cannot release a current slot.
Cancellation markers must identify the same live operation as normal
completion metadata. No completion path may free a buffer before all
operations referencing it have terminated.

### Switch lifecycle and synchronization

The switch lifecycle is `Created -> Probing -> Open -> Running -> Draining ->
Closed`. Capability failure transitions to `Closed` without publishing a
partially initialized data plane. Endpoint close, device removal, owner close,
and process shutdown all enter `Draining`, prevent new reads and writes, and
preserve the original failure while reporting cleanup failures separately.

### Adaptive-polling switch execution

The switch enables the driver-defined adaptive-polling protocol independently
on each selected endpoint before posting adaptive I/O. If all endpoints
negotiate the same supported protocol version, it uses adaptive polling;
otherwise it uses the documented all-legacy fallback and does not mix models
within one relay run. Negotiation is transactional: if any endpoint has
entered adaptive mode before the common protocol selection fails, the switch
closes every negotiating handle, allowing each driver's owner-cleanup path to
cancel waits and restore legacy state, then reopens every endpoint before
posting legacy I/O. It retains one
overlapped `WAIT_FOR_CHANGE` operation per endpoint, separate from registered
frame-buffer slots and normal I/O-ring completion identities. A wait result
only grants permission to resume polling; it does not transfer a frame or
buffer ownership.

While the data plane makes progress, the switch submits READ and WRITE
operations in batches with an I/O-ring minimum completion count of zero and
drains immediately available completions. It treats the adaptive empty-READ
status as reported by the I/O-ring completion and `ERROR_BUSY` write result as
no-progress/backpressure outcomes, not as fatal slot failures. The affected
slot reaches a terminal completion before it is reposted or retried according
to the documented bounded policy.

After no progress for its configured microsecond polling budget, the switch
submits `WAIT_FOR_CHANGE` for the conditions relevant to its current reads
and writes through an overlapped control request, then blocks on that request
rather than on an I/O-ring completion. It resumes polling after the wait
completes. The adaptive normal-traffic path must not issue
`SubmitIoRing(..., 1, ...)` merely to wait for an individual packet
completion. The switch cancels and drains its outstanding waits before
closing endpoint handles, I/O-ring resources, or registered buffers.

When statistics are requested, the switch reports wait submission, signaled
wake, and read/write batch counters at the configured interval even while an
adaptive wait remains pending. This exposes an idle, unsignaled wait without
changing the wait or completion path.

The experiment records a GUID-to-device-interface-to-network-interface mapping
before assigning an address or route. Every probe explicitly uses the mapped
network interface and is rejected if its address, route, or PnP hardware ID
does not match the manager-created GUID supplied to the switch. It performs
functional enable, wait-registration, readable-transition, wait-completion,
read, and forwarding checks before treating any adaptive run as a performance
sample.

The endpoint collection, FDB, slot states, and pending-operation counters use
one documented lock order. Completion callbacks do not reacquire a lock that
they already hold, and notification/submission calls that may reenter the
completion path occur outside the state lock. User buffers and ring resources
are accessed only at permitted user-mode execution contexts; all completion
and cancellation paths are idempotent.

## Dynamic KMDF bus and child design

This section supersedes the fixed-root package, control-path, relay
provisioning, and endpoint-selection details elsewhere in this document.

### Package and PnP topology

The package contains separate KMDF bus and TAP-child services. The bus service
binds a bus-parent hardware identity and owns the WDF child list. The child
service binds only the bus child hardware/compatible identity and performs the
existing NetAdapterCx and TAP-child initialization. The design shall not bind
the child service to `ROOT\WinTapRust` or `ROOT\WinTapRust2`.

The bus's child identification description contains the immutable manager GUID
as its sole identity key. A child description is allocated and initialized
before it is added to the WDF child list. The bus publishes a child only after
all checked allocation and identity initialization succeeds. The child PDO
context owns its GUID and cannot reference mutable manager-owned request
storage.

### Bus manager control plane

The bus exposes an administrator-only control interface. Its request header
contains protocol version, operation, bounded total length, request ID, and
adapter GUID. The bus rejects unknown versions, invalid operation codes,
lengths smaller than the header, lengths exceeding the supplied buffer, and
invalid GUIDs before allocation or mutation.

The manager protocol advances to version 2; version 1 requests are rejected
and no compatibility shim is required because all supported consumers are in
this repository. The version-2 request retains the bounded fixed-size
envelope and uses its create-only `requested_mtu` field, where zero means the
1,500-byte default. For remove, enumerate, and query, `requested_mtu` must be
zero. The bus validates the requested MTU as 1,500 through 65,521 before
reserving lifecycle state or adding the child description. Invalid values fail
without child-list mutation. The selected MTU is copied into immutable
child/PDO context and is available to the child service during device
addition. Create then returns an in-progress result correlated by request ID
and GUID and reaches terminal success only after the child publishes its TAP
interface. Remove marks the GUID removing, prevents duplicate create/remove,
requests WDF child removal, and reaches terminal success only after child PnP
removal and interface withdrawal. Enumerate and query report GUID, lifecycle
state, interface identity, and the selected MTU; the manager never opens,
reads, or writes a TAP endpoint.

### Child lifetime and TAP interface

The child receives its GUID from immutable PDO context during device addition.
It allocates all NetAdapterCx, queue, frame, filter, lock, work-item, and
exclusive-owner state per child. No fixed instance ID, static state array, or
cross-child raw state lookup is permitted. The child creates and registers a
unique device interface and correlates it to the GUID for manager discovery.

The child interface replaces fixed `\\.\WinTapRust` paths. Its access control
continues to restrict TAP open/control to elevated administrators. A manager
restart loses only manager user-mode state: the bus retains child descriptions,
and enumerate permits reattachment. Bus unload or reboot does not promise
persistence.

### Synchronization, IRQL, and teardown

The bus serializes child-list mutation and per-GUID lifecycle transitions. A
per-GUID state machine is `Absent -> Creating -> Active -> Removing -> Absent`,
with terminal `Failed` reported only after partial resources are unwound.
Surprise removal and bus teardown enter `Removing`, reject new operations, and
wait for the child PnP lifecycle to withdraw its interface.

Bus control processing and PnP child-list mutation follow the verified KMDF
execution-level contract. The bus does not run packet processing. Child
NetAdapterCx callbacks preserve their existing IRQL/pageability requirements;
teardown first blocks new queue work, then terminally completes or cancels
child I/O, drains child frames and callbacks, unregisters the TAP interface,
and releases child state exactly once.

### Dynamic relay and switch selection

The relay harness creates two test GUIDs through the manager, waits for their
distinct interfaces, and selects them by returned GUID rather than device
order, MAC ordinal, or fixed DOS name. It retains the existing two-endpoint
IPv4/IPv6 route, neighbor, relay, directional-isolation, and cleanup
assertions. The switch enumerates a selected GUID/interface collection; its
initial forwarding policy still has two selected endpoints and therefore does
not specify arbitrary-N flooding behavior.

### Migration and diagnostics

Package installation, upgrade, and uninstall enumerate legacy
`ROOT\WinTapRust`/`ROOT\WinTapRust2` devices separately from dynamic children.
They never adopt a legacy device as a bus child. An explicit migration or
cleanup operation removes legacy instances and records the result.

The bus and manager emit request ID, GUID, lifecycle transition, child PnP
identity, interface identity, and primary cleanup failure. Diagnostics omit
packet contents by default.

## Execution-environment design

The hosted and VM paths share the REQ-008 and REQ-015 entry points, packet
parsers, packet builders, timeout policy, and acceptance assertions. Only
provisioning inputs such as package location, architecture, signing mode, and
cleanup policy may vary.

### GitHub-hosted Windows runner

The workflow shall provision the WDK/SDK and test packages, verify the
required test-signing/install state, use the manager control interface to
create and remove the REQ-015 children, run REQ-008 and REQ-015, upload
diagnostics, and restore the runner. The job must use the privileges required
by the driver and network commands. If the runner rejects any required
operation, the job fails with the operation and platform error.

### Manual Hyper-V VM

Documentation shall define a clean VM setup, supported Windows build,
architecture, administrator/test-signing prerequisites, package install,
test invocation, diagnostic collection, and cleanup. The VM tests use the
same assertions as hosted CI and do not rely on an external network peer.

## Unresolved implementation details

- The selected WDK baseline uses `EVT_PACKET_QUEUE_ADVANCE` for both directions
  and the ring iterator APIs listed above.
- The effective Ethernet payload/MTU is supplied in the version-2
  bus-manager child-create request. An omitted/zero `requested_mtu` selects
  1,500 bytes; valid values range from 1,500 through 65,521 bytes. Invalid
  values fail before child publication. The selected value is copied into
  immutable child/PDO context and remains fixed until child removal. Version 1
  requests are rejected.
- The complete frame contract is 14 through `MTU + 14` bytes. NetAdapterCx's
  complete-frame ceiling is 65,535 bytes; the implementation shall not
  advertise a larger `MaximumFrameSize`. VLAN-tagged frames remain subject
  to the selected maximum.
- Driver, queue, switch, and harness limits shall consume each child's
  effective MTU and complete-frame maximum so that a creation override cannot
  create mismatched packet bounds. The switch shall continue to reject
  endpoint pairs whose effective MTUs differ.
- The default directional frame queue limit is 256 frames and is not yet
  registry-configurable.
- **[ASSUMPTION]** A copy at the user/kernel boundary is acceptable for the
  initial implementation; zero-copy is not required by the approved
  requirements.
- **[KNOWN]** The repository maintainer confirms that the `windows-latest`
  and `windows-2022` GitHub-hosted runners used by CI are already test-signed
  for driver installation and virtual-interface configuration. A regression
  that rejects a required operation remains a required failure outcome under
  REQ-009 for both REQ-008 and REQ-015.
