# Two-TAP User-Mode Switch Feasibility Study

**Status:** Approved  
**Scope:** Two WinTap adapters in the first release  
**Date:** 2026-08-19

## Decision

The switch is feasible as a user-mode process. It will exclusively open two
TAP control-device handles, learn source MAC/VLAN locations, and forward each
captured Ethernet frame to the other TAP when required by the forwarding
database.

The data plane requires Windows I/O-ring version 3 support for reads and
writes. Version 4 scatter/gather support is optional and must be selected only
when runtime capability probes confirm it is available. The implementation
must use `QueryIoRingCapabilities` and `IsIoRingOpSupported` before starting
the data plane.

## Forwarding behavior

The forwarding database is keyed by source MAC address and VLAN identifier.
The first release has the following policy:

| Traffic | Action |
| --- | --- |
| Known unicast | Forward to the learned destination TAP adapter. |
| Unknown unicast | Flood to the other TAP adapter. |
| Broadcast | Flood to the other TAP adapter. |
| Multicast | Flood to the other TAP adapter. |
| Source learned on the other TAP | Immediately move the entry to that TAP. |
| FDB full | Preserve existing entries and do not learn new sources. |

The table has a fixed capacity of 4,096 entries. Entries do not age in the
first release. A frame is never forwarded to the TAP from which it was read.

With two TAP adapters, all flood behavior has one recipient. Generalizing the
driver and provisioning model to an arbitrary adapter list is explicitly
deferred to a separate effort.

## I/O-ring design

1. Exclusively open both TAP control handles and create one I/O ring.
2. Query the supported maximum I/O-ring version and create the newest usable
   ring. Require support for `IORING_OP_READ` and `IORING_OP_WRITE`.
3. Allocate a fixed pool of 1514-byte user buffers, register both handles and
   the buffer pool, and pre-post bounded reads.
4. On a read completion, validate the Ethernet frame, learn its source
   MAC/VLAN, resolve its destination, and build one I/O-ring write for the
   selected peer when forwarding is required.
5. Re-post the read only after the source buffer's read and all writes using
   that buffer have terminal completions.

Each completion `userData` identifies the endpoint, buffer slot, and slot
generation. A buffer slot is:

```text
Free -> ReadPending -> Dispatching -> WritePending(remaining writes) -> Free
```

On cancellation, device removal, or shutdown, the switch stops posting new
reads, submits cancellation for outstanding operations, drains their original
completions, and only then deregisters buffers and handles and closes the
ring. Generation values prevent late completions from being associated with a
recycled buffer.

## I/O-ring versioning

The public Windows SDK header defines:

- Version 3: write, flush, and drain support.
- Version 4: scatter/gather support.
- `BuildIoRingWriteFile` for writes.
- `BuildIoRingReadFileScatter` and `BuildIoRingWriteFileGather` for vectored
  operations.

Version 4 is not the initial data-path dependency. TAP frames are currently
contiguous and no larger than 1514 bytes, so ordinary v3 reads and writes
provide the required behavior. Scatter/gather should be evaluated only after a
proof of concept confirms operation support for the TAP control handles and a
measurable benefit.

## Copy and transition analysis

I/O rings reduce submission batching and registered-resource validation or
mapping overhead. A broadcast can reuse its one source user buffer for its
single peer write in this two-TAP scope, avoiding a user-mode frame copy.

They do not provide zero-copy end-to-end with the current driver:

1. The driver copies NetAdapterCx TX fragments into its capture `Vec`.
2. The driver copies the capture frame into the user read buffer.
3. The driver copies a user write into its injection frame.
4. The driver copies that injection frame into a NetAdapterCx RX fragment.

The performance claim is therefore reduced user/kernel submission overhead and
user-mode copy avoidance where a source slot can be reused, not elimination of
the driver's copies. The implementation must report I/O-ring and driver-copy
effects separately.

## Compatibility and risks

- The repository's existing Windows 10 version 2004+ baseline predates the
  documented I/O-ring API baseline. A product retaining that compatibility
  must provide overlapped-I/O fallback or fail startup explicitly when the
  required ring version or operations are unavailable.
- The current Rust driver is limited to two instances
  (`crates\wintap-netadaptercx-driver\src\lib.rs`, `INSTANCE_IDS`), matching
  this release scope.
- Existing overlapped I/O on the custom KMDF TAP handles is not proof that all
  I/O-ring versions and operations work on those handles. A target-OS proof of
  concept is required before production rollout.
- Unbounded pending work is prohibited. The switch must bound its registered
  buffer pool, read depth, write depth, and FDB capacity.

## Validation plan

| ID | Validation |
| --- | --- |
| TC-SW-001 | Record maximum I/O-ring version and support for read, write, read-scatter, and write-gather on every target OS. |
| TC-SW-002 | Open and register the two TAP handles; complete one I/O-ring read and write with exact byte and completion validation. |
| TC-SW-003 | Prove learned known-unicast forwarding to the peer and no source reflection. |
| TC-SW-004 | Prove unknown-unicast, broadcast, and multicast flood only to the peer. |
| TC-SW-005 | Verify immediate FDB move, no aging, 4,096-entry capacity behavior, and preservation of existing entries when full. |
| TC-SW-006 | Cancel and remove endpoints while reads and writes are pending; prove every original completion is consumed before resource release. |
| TC-SW-007 | Saturate the peer and verify bounded backpressure, no cross-frame corruption, and recovery after drain. |
| TC-SW-008 | Compare current overlapped relay and v3 I/O-ring relay with ETW, CPU, throughput, latency, allocation, and copy counters. |
| TC-SW-009 | When v4 support is present, validate page-aligned scatter/gather operations and retain v3 behavior as the baseline. |

## Repository evidence

- `README.md` documents the current routed dual-adapter relay and the Windows
  10 version 2004+ target.
- `tests\run-wintap-dual-adapter-harness.ps1` already uses independently
  exclusive TAP handles and bidirectional overlapped relay operations.
- `crates\wintap-netadaptercx-driver\src\lib.rs` has two instance slots,
  bounded frame queues, separate injection/capture directions, and control
  read/write completion paths.
- The Windows SDK `ntioring_x.h` and `ioringapi.h` define the versioned
  read/write/scatter/gather operations used by this design.
