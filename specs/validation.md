# WinTapNetAdapterCx Validation Specification

**Workflow:** `/evolve`  
**Phase:** Phase 8 — Create Deliverable
**Status:** Specification package approved; implementation and validation
changes are being delivered
**Trace source:** `specs/requirements.md` and `specs/design.md`

## Acceptance criteria

| ID | Requirement | Validation |
| --- | --- | --- |
| VAL-001 | REQ-001 | Build and install the NetAdapterCx driver; verify one virtual Ethernet adapter appears with the expected capabilities and identity. |
| VAL-002 | REQ-002 | Write valid Ethernet frames through the device handle and verify delivery to the Windows networking stack; verify invalid nonzero lengths complete with error 87 without enqueuing a frame and zero-byte writes complete as Win32 no-ops; transmit frames through the stack and verify complete reads in user mode without crossing the two directions. |
| VAL-003 | REQ-003 | Exercise start, pause, restart, stop, surprise removal, owner close, process termination, and cancellation; verify no hangs, double completions, or leaked objects. |
| VAL-004 | REQ-004 | Build and execute the supported x64 and ARM64 packages on Windows 10 version 2004+ and reject unsupported platform combinations explicitly. |
| VAL-005 | REQ-005 | Verify non-administrator open/control attempts fail; verify malformed nonzero lengths complete with error 87 and invalid I/O requests cannot corrupt memory or disclose data. |
| VAL-006 | REQ-006 | Run the complete build, install, packet-path, concurrency, cancellation, power, malformed-input, and cleanup suite with Driver Verifier-compatible settings. |
| VAL-007 | REQ-007 | Configure and build from a clean environment with CMake and the Visual Studio generator for x64 and ARM64; verify NuGet WDK/SDK dependencies resolve reproducibly and missing prerequisites fail at configuration. |
| VAL-008 | REQ-008 | Run the complete privileged ICMP Echo Request/Echo Reply round trip through the Ethernet/TAP handle using `192.0.2.1/30` and `192.0.2.2`; verify packet fields, checksums, stack completion, timeout behavior, and cleanup. |
| VAL-009 | REQ-009 | Execute the full REQ-008, REQ-015, and REQ-016 assertion sets in a GitHub-hosted Windows job and manually in a Hyper-V-capable Windows VM using the same entry points; fail on unavailable privileged operations rather than skipping. |
| VAL-010 | REQ-010 | Build the Rust driver and generated NetAdapterCx bindings from a clean pinned environment for x64 and ARM64; verify binding regeneration, ABI/layout checks, panic-abort configuration, and package production. |
| VAL-011 | REQ-011 | Verify the repository, CMake targets, workflow, harness, and package validation contain no C/C++ driver source, project, INF, fallback, or selector. |
| VAL-012 | REQ-012 | Build each package and verify `wintap_netadaptercx_driver.inf`, `wintap_netadaptercx_driver.cat`, service `WinTapRust`, and hardware IDs `ROOT\WinTapRust` and `ROOT\WinTapRust2`. |
| VAL-013 | REQ-013 | Load the test-signed Rust package with NetAdapterCx verifier enabled; verify directed, broadcast, multicast, all-multicast, and promiscuous capability initialization with a nonzero multicast capacity does not trigger `0x19E/0xB`, and TCP/IP binds successfully. |
| VAL-014 | REQ-014 | Verify the harness captures native overlapped-I/O errors within its C# wrappers and reports pending and cancelled requests accurately. |
| VAL-015 | REQ-015 | In a clean elevated environment, provision two WinTap adapters, verify their identity and independently exclusive TAP handles, install reciprocal IPv4/IPv6 host routes and static neighbors, relay frames in both directions, and verify unbound IPv4 ICMP and IPv6 ICMPv6 round trips plus complete cleanup. |
| VAL-016 | REQ-016 | With a destination TAP read already pending, inject a routed request into that destination and fail if the destination's reverse-direction TAP read returns the byte-identical injected request. Record/rearm unrelated traffic; accept only a validated stack-originated reply for the round trip. Exercise notification arming across owner close/reopen, RX ring-capacity boundaries, cancellation, and teardown under NetAdapterCx verifier without a bugcheck or ownership violation. |
| VAL-017 | REQ-017 | Run the two-TAP switch with the two existing static endpoints; verify source MAC/VLAN learning, known-unicast forwarding, unknown-unicast/broadcast/multicast flooding to only the peer, immediate source movement, no source reflection, and fixed 4,096-entry full-table preservation behavior. |
| VAL-018 | REQ-018 | On every target OS, record I/O-ring maximum version and read/write/scatter/gather support, require successful read/write capability probes before startup, verify bounded registered buffers and operation depths, and verify explicit startup failure when required support is absent. Exercise slot generations, terminal completions, cancellation, endpoint removal, and resource release ordering. |
| VAL-019 | REQ-019 | Verify the first release discovers and opens exactly the two existing static endpoints through a collection-oriented endpoint model, and confirm the model has no dynamic provisioning or arbitrary-N behavior while preserving stable endpoint identity and teardown isolation. |
| VAL-020 | REQ-020 | Verify one positive even shared depth is split equally between both endpoints, completion metadata uniquely represents every allocated slot and operation state, and startup fails explicitly for zero, odd, overflowed, unrepresentable, unallocatable, unsupported, or unregistered depths without silently reducing the request. |

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
| TC-036 | Install the test-signed root-enumerated adapter with NetAdapterCx verifier enabled; verify `WintapEvtPrepareHardware` succeeds without bugcheck `0x19E/0xB`, the capability structure advertises directed, broadcast, multicast, all-multicast, and promiscuous filtering with capacity 64, and TCP/IP appears in the adapter's active NDIS protocol bindings. |
| TC-039 | Set directed, broadcast, multicast, all-multicast, and promiscuous receive-filter configurations through the Windows stack; verify the driver accepts each supported configuration and stores no more than 64 multicast addresses without corrupting active filter state. |
| TC-037 | Build Rust x64 and ARM64 packages through CMake and verify each contains the Rust driver binary, `wintap_netadaptercx_driver.inf`, and `wintap_netadaptercx_driver.cat`. |
| TC-038 | Install `ROOT\WinTapRust` after removing any stale C package; verify service `WinTapRust` starts and no C device or service is selected. |
| TC-040 | Issue an empty-queue overlapped read and verify `ReadFile` returns false with error 997; cancel it and verify `GetOverlappedResult` returns false with error 995. Repeat this error-observation path before ARP/ICMP assertions. |
| TC-041 | Verify a 0-byte overlapped write completes as a Win32 no-op. Issue 1-byte, 13-byte, and 1515-byte overlapped writes; verify each completes with error 87, transfers no bytes, leaves no queued frame or retained pending request, and is followed by a successful valid-frame write. |
| TC-042 | In a clean environment, use the pinned WDK DevCon tool to create `ROOT\WinTapRust` and `ROOT\WinTapRust2`; verify exactly two adapters, MAC/control-endpoint mapping, service identity, and independent exclusive opens. Verify any pre-existing matching adapter causes a non-destructive failure. |
| TC-043 | Assign the REQ-015 IPv4 and IPv6 test addresses, static peer neighbors, exact reciprocal `/32` and `/128` active-store routes, and run-scoped firewall rules. Verify no default route is created and the exact host routes select the opposite egress interface. |
| TC-044 | Start an unbound IPv4 ICMP Echo to B. Verify the request is read from A, relayed to B, the reply is read from B, relayed to A, and the stack reports success with matching Ethernet/IP/ICMP identities, payload, and checksums. |
| TC-045 | Start an unbound IPv6 ICMPv6 Echo to B. Verify the same A-to-B and B-to-A relay path, IPv6 endpoint identities, payload, and ICMPv6 pseudo-header checksum. |
| TC-046 | Exercise malformed/truncated frames, write/read cancellation, route/neighbor/firewall/address failure, partial provisioning, timeout, and device removal. Verify both handles complete before release, only created state is removed, diagnostics persist, and primary failure is retained. |
| TC-047 | Execute TC-042 through TC-046 using `tests\run-wintap-dual-adapter-harness.ps1` on a GitHub-hosted Windows job and a manual Hyper-V/WinDbg VM; verify shared assertions and no capability-only skip. |
| TC-048 | Pre-post a TAP read on B, relay A's valid IPv4 Echo Request into B, and fail if B's reverse-direction read returns that byte-identical request. Record/rearm unrelated frames; require B's valid Echo Reply to be relayed to A and reported successful by the unbound Ping client. Repeat for ICMPv6. |
| TC-049 | Exercise injection while RX polling is active and while receive notification is armed. Close and reopen the TAP owner while RX remains running, then verify a later write requests a new RX advance. Send enough valid routed frames to cross at least one RX-ring capacity handoff, then cancel/stop during queued injection. Under NetAdapterCx verifier, verify packet and fragment ownership remains synchronized, no ring entry is returned twice, and no frame leaks into the TAP read path. |
| TC-050 | With static peer neighbors installed, present valid ARP, multicast Neighbor Solicitation, unicast Neighbor Unreachability Detection Solicitation without a source link-layer option, and Duplicate Address Detection frames to each relay direction. Verify the harness validates and counts them, performs no peer write, remains free of a reflection loop, and still completes the IPv4 and IPv6 Echo tests. |
| TC-051 | Feed the switch known, unknown, broadcast, multicast, VLAN-tagged, source-move, source-destination, malformed, and unsupported frames on both static endpoints; verify FDB learning and bounded full-table behavior, peer-only flooding, and no reflection. |
| TC-052 | Probe I/O-ring capabilities on each target OS, verify required contiguous read/write operations before starting, record the selected version, and verify v4 scatter/gather is used only when separately supported and validated. |
| TC-053 | Saturate configured read/write slots and the 4,096-entry FDB; verify deterministic bounded backpressure or rejection, slot-generation protection, no cross-frame corruption, and recovery after terminal completions. |
| TC-054 | Cancel and remove either endpoint during pending reads and peer writes; verify no new reads are posted, every original completion is consumed before deregistration/close, stale generations cannot free reused slots, and cleanup preserves the primary failure. |
| TC-055 | Verify the endpoint collection accepts the two existing static identities without provisioning additional devices, and inspect the endpoint-selection path for collection-based identity lookup rather than a two-branch-only contract. |
| TC-056 | Configure several positive even shared depths, including a value greater than 256, and verify equal per-endpoint capacity, successful allocation/registration, saturation behavior, and recovery after terminal completions. |
| TC-057 | Exercise zero, odd, maximum-integer, arithmetic-overflow, and otherwise unrepresentable depth values; verify deterministic explicit startup errors and no partially published ring, endpoint, or buffer state. |
| TC-058 | Force buffer allocation failure and I/O-ring depth/resource-limit failure; verify the requested depth is not clamped or wrapped, all partial resources unwind, and the primary failure is preserved. |
| TC-059 | Force registered-buffer or operation-registration failure after partial progress; verify startup fails, every previously allocated resource is released exactly once, and no endpoint enters `Running`. |
| TC-060 | Submit and complete operations using the highest allocated slot IDs, both directions, multiple generations, and cancellation markers; verify encode/decode round trips, rejection of truncation/collision/stale identities, and no release of a reused slot. |
| TC-061 | Cancel, remove, and shut down with a depth above 256 and outstanding reads/writes on both endpoints; verify all original completions are consumed before deregistration or close and no buffer is reused early. |

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
   design, absence of cross-request data, and that a pending read cannot
   consume a queued user-write frame.
5. **Exclusive ownership:** open the adapter from one elevated process, reject
   a second open, then allow a new owner after clean close and after abnormal
   process termination.
6. **Backpressure:** fill each bounded frame queue, verify new operations wait,
   cancel correctly, and resume when capacity is released.
7. **Boundary validation:** verify a zero-byte write completes as a Win32
   no-op; test undersized, oversized, malformed, and partially invalid
   requests; verify invalid nonzero write lengths complete with error 87,
   transfer no bytes, and cause no state damage.
8. **Routed dual-adapter relay:** provision two clean root-enumerated
   adapters, relay complete frames between their independent TAP handles, and
   verify IPv4 and IPv6 stack round trips use the configured adapter routes
   rather than loopback. With permanent neighbors, validate/count and suppress
   ARP and IPv6 Neighbor Discovery rather than relaying those control frames.

## Lifecycle and concurrency tests

- Cancel a pending read while a frame is arriving.
- Cancel a pending write while the transmit queue is full.
- Cancel a pending TAP read while an injection frame is awaiting RX queue
  advance; verify the injection frame cannot be redirected into that read.
- Cancel or remove one switch endpoint while the other has pending reads,
  pending writes, and queued forwarding work; verify the endpoint collection
  drains independently and does not release shared forwarding resources early.
- Close the owner handle with pending reads, pending writes, queued frames, and
  active framework callbacks.
- Pause and restart with every queue state and with requests in flight.
- Stop or remove the adapter during each allocation, enqueue, dequeue, copy,
  and completion stage.
- Race second-open, close, cancellation, pause, restart, and removal operations.
- Cancel one or both relay reads/writes while the paired topology is active,
  then verify both handles and all route/neighbor/firewall/device state are
  restored without affecting a pre-existing adapter.
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
- Probe unavailable I/O-ring read/write support and verify explicit switch
  startup failure without partially opened or registered resources.

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

The existing harness is `tests/run-wintap-harness.ps1`. It requires an
elevated administrator PowerShell session and an installed test-signed driver.
It validates exclusive device open, malformed frame rejection, overlapped read
cancellation, and successful overlapped writes. REQ-008 remains implemented
through that harness.

The dedicated REQ-015 entry point is
`tests/run-wintap-dual-adapter-harness.ps1`. It shall resolve `devcon.exe`
from the pinned WDK package, create both root devices, map their identities to
the two control endpoints, configure the routed IPv4/IPv6 topology, run the
bidirectional relay, preserve diagnostics, and clean up its devices and any
driver package it added.

The first-release switch validation uses the same two statically defined
control endpoints but a collection-oriented endpoint model. It must not add
dynamic PnP provisioning, arbitrary-N forwarding, or an overlapped-I/O
fallback. Its I/O-ring capability and completion tests are separate from the
existing driver overlapped-I/O harness.

The implementation shall use CMake 3.25 or later and a supported Visual
Studio generator. The repository presets target Visual Studio 18 2026; hosted
CI uses Visual Studio 17 2022 when that is the runner-provided generator. The
four architecture-specific WDK/SDK NuGet packages listed in `specs/design.md`
remain pinned to version `10.0.28000.2526`. The harness is implemented in
PowerShell using P/Invoke to Win32 overlapped I/O.

The kernel crate requires `panic = "abort"`. Stable Cargo therefore cannot
execute its unit tests; a compatible nightly toolchain with
`-Zpanic-abort-tests` is required. This tooling limitation does not satisfy
TC-031 or convert unexecuted unit tests into a passing result.

The harness captures native I/O errors within its C# P/Invoke wrappers. The
PowerShell layer consumes those explicit values and does not independently
query last-error state after a managed-boundary transition.

## Required hosted and privileged execution

Hosted CI shall continue to validate artifact presence, PowerShell syntax, WDK
tool provisioning, CMake configure/build/package, and INF/driver package shape
for x64 and ARM64. In addition, a privileged Windows job shall execute the
hosted-runner instances of VAL-008, VAL-009, and VAL-015 using the same entry
points as the manual VM path. The job must upload diagnostics and fail if
driver installation, two-device provisioning, address/route/neighbor/firewall
configuration, IPv4/IPv6 relay, packet exchange, or cleanup is blocked.

The elevated harnesses remain runnable manually in a Hyper-V-capable WinDbg
VM. They require a test-signed driver and validate the existing I/O contract,
the complete REQ-008 round trip, and the REQ-015 routed dual-adapter IPv4/IPv6
relay. Queue saturation, power, removal, and verifier scenarios remain
additional privileged acceptance gates.

The hosted job and VM procedure must report environment failures explicitly;
they must not classify an unexecuted packet-path test as passed. VAL-009 is
complete only after both the hosted-runner result and the manual-VM result are
recorded; the hosted job alone cannot claim VM coverage.

TC-040 is deferred: continuous transmit traffic from the live adapter prevents
the harness from establishing its required empty-queue cancellation fixture.
It does not pass until cancellation is validated with adapter traffic quiesced
at its source. TC-015, TC-016, TC-017, TC-018, TC-019, TC-020, and TC-022 are
implementation and specification trace points for the approved maintenance corrections.
TC-023 through TC-028 provide trace points for REQ-008 and REQ-009.
TC-042 through TC-047 provide trace points for REQ-015.
TC-051 through TC-055 provide trace points for REQ-017 through REQ-019.
TC-056 through TC-061 provide trace points for REQ-020.
