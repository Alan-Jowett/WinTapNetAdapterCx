<!-- SPDX-License-Identifier: MIT
  Copyright (c) 2026 WinTapNetAdapterCx contributors -->

# WinTapNetAdapterCx Validation Specification

**Workflow:** `/evolve`  
**Phase:** Phase 2 — Specification Changes
**Status:** Dynamic-bus validation changes proposed; awaiting approval
**Trace source:** `specs/requirements.md` and `specs/design.md`

## Acceptance criteria

| ID | Requirement | Validation |
| --- | --- | --- |
| VAL-001 | REQ-001 | Build and install the NetAdapterCx driver; verify one virtual Ethernet adapter appears with the expected capabilities and identity. |
| VAL-002 | REQ-002 | Write valid Ethernet frames through the device handle and verify delivery to the Windows networking stack; verify invalid nonzero lengths complete with error 87 without enqueuing a frame and zero-byte writes complete as Win32 no-ops; transmit frames through the stack and verify complete reads in user mode without crossing the two directions. |
| VAL-037 | REQ-002 | With the maximum valid configuration, verify the adapter reports MTU 65,521 and complete-frame maximum 65,535; write and forward a 65,535-byte frame successfully, and reject a 65,536-byte frame with `ERROR_INVALID_PARAMETER` without destabilizing the adapter. |
| VAL-038 | REQ-002, REQ-046 | Verify omitted creation MTU defaults to 1,500 and frame maximum 1,514; valid creation values including 1,500 and 65,521 produce matching capabilities and packet bounds; below-minimum and above-maximum values fail before child publication; recreated children accept a different MTU; active children have no runtime MTU mutation path; and switch startup rejects endpoints with mismatched effective MTUs. |
| VAL-003 | REQ-003 | Exercise start, pause, restart, stop, surprise removal, owner close, process termination, and cancellation; verify no hangs, double completions, or leaked objects. |
| VAL-004 | REQ-004 | Build and execute the supported x64 and ARM64 packages on Windows 10 version 2004+ and reject unsupported platform combinations explicitly. |
| VAL-005 | REQ-005 | Verify non-administrator open/control attempts fail; verify malformed nonzero lengths complete with error 87 and invalid I/O requests cannot corrupt memory or disclose data. |
| VAL-006 | REQ-006 | Run the complete build, install, packet-path, concurrency, cancellation, power, malformed-input, and cleanup suite with Driver Verifier-compatible settings. |
| VAL-007 | REQ-007 | Configure and build from a clean environment with CMake and the Visual Studio generator for x64 and ARM64; verify NuGet WDK/SDK dependencies resolve reproducibly and missing prerequisites fail at configuration. |
| VAL-008 | REQ-008, REQ-029, REQ-030 | Create one test GUID child through the manager and run the complete privileged ICMP Echo Request/Echo Reply round trip through its discovered Ethernet/TAP interface using `192.0.2.1/30` and `192.0.2.2`; verify packet fields, checksums, stack completion, timeout behavior, and cleanup. |
| VAL-009 | REQ-009 | Execute the full REQ-008, REQ-015, and REQ-016 assertion sets in a GitHub-hosted Windows job and manually in a Hyper-V-capable Windows VM using the same entry points; fail on unavailable privileged operations rather than skipping. |
| VAL-010 | REQ-010 | Build the Rust driver and generated NetAdapterCx bindings from a clean pinned environment for x64 and ARM64; verify binding regeneration, ABI/layout checks, panic-abort configuration, and package production. |
| VAL-011 | REQ-011 | Verify the repository, CMake targets, workflow, harness, and package validation contain no C/C++ driver source, project, INF, fallback, or selector. |
| VAL-012 | REQ-033 | Build each package and verify separate bus and child services, their intended parent/child identities, and the absence of supported legacy root runtime identities. |
| VAL-013 | REQ-013 | Load the test-signed Rust package with NetAdapterCx verifier enabled; verify directed, broadcast, multicast, all-multicast, and promiscuous capability initialization with a nonzero multicast capacity does not trigger `0x19E/0xB`, and TCP/IP binds successfully. |
| VAL-014 | REQ-014 | Verify the harness captures native overlapped-I/O errors within its C# wrappers and reports pending and cancelled requests accurately. |
| VAL-015 | REQ-015, REQ-029, REQ-030 | In a clean elevated environment, create two GUID-keyed children through the manager, discover their distinct TAP interfaces, verify independent exclusive handles, install reciprocal IPv4/IPv6 routes and static neighbors, relay frames in both directions, and verify complete removal. |
| VAL-016 | REQ-016 | With a destination TAP read already pending, inject a routed request into that destination and fail if the destination's reverse-direction TAP read returns the byte-identical injected request. Record/rearm unrelated traffic; accept only a validated stack-originated reply for the round trip. Exercise notification arming across owner close/reopen, RX ring-capacity boundaries, cancellation, and teardown under NetAdapterCx verifier without a bugcheck or ownership violation. |
| VAL-017 | REQ-017, REQ-036 | Run the two-TAP switch with two GUID-selected dynamic endpoints; verify source MAC/VLAN learning, known-unicast forwarding, peer-only flooding, immediate source movement, no reflection, and fixed 4,096-entry full-table preservation behavior. |
| VAL-018 | REQ-018 | On every target OS, record I/O-ring maximum version and read/write/scatter/gather support, require successful read/write capability probes before startup, verify bounded registered buffers and operation depths, and verify explicit startup failure when required support is absent. Exercise slot generations, terminal completions, cancellation, endpoint removal, and resource release ordering. |
| VAL-019 | REQ-036 | Verify GUID/interface discovery populates a selected endpoint collection without static paths or PnP-order assumptions, preserving stable identity and teardown isolation; arbitrary-N forwarding remains out of scope. |
| VAL-020 | REQ-020 | Verify one positive even shared depth is split equally between both endpoints, completion metadata uniquely represents every allocated slot and operation state, and startup fails explicitly for zero, odd, overflowed, unrepresentable, unallocatable, unsupported, or unregistered depths without silently reducing the request. |
| VAL-021 | REQ-021 | Verify valid writes are captured and completed entirely in the write callback without entering a WDF write queue or scheduling a write work item; verify callback execution level, nonpaged allocation, queue ownership, notification reentrancy, teardown synchronization, and explicit initialization failure when the required inline contract is unavailable. |
| VAL-022 | REQ-024 | Verify passive-level TX callbacks deliver complete frames directly to compatible pending READ IRPs before returning ring entries; verify elevated-level callbacks use nonpaged capture and passive deferred completion, no TX entry is held indefinitely, too-small reads preserve frame ownership, and cancellation/teardown complete each request exactly once. |
| VAL-023 | REQ-025 | Verify passive READ delivery claims frame/request ownership under the state lock but performs WDF buffer access, copying, requeue, and completion only after releasing it; verify no duplicate claims across packet callbacks, `evt_io_read`, and the work item, including too-small-buffer, cancellation, stop, and teardown races. |
| VAL-024 | REQ-026 | Install the bus and child packages; create, enumerate, and remove children; verify one bus parent and independently managed adapters per active child. |
| VAL-025 | REQ-027, REQ-046 | Verify non-administrator manager requests and malformed version, length, opcode, GUID, and MTU fields fail without child-list mutation, disclosure, leak, or bugcheck; verify version 1 requests are rejected and version 2 is required. |
| VAL-026 | REQ-028, REQ-031 | Create at least three distinct GUID children concurrently; verify one child per GUID, isolated TAP I/O, and explicit resource-exhaustion failure without partial publication. |
| VAL-027 | REQ-029, REQ-032 | Race create, explicit remove, surprise removal, and bus teardown; verify terminal operation correlation, interface withdrawal, and exactly-once child I/O completion. |
| VAL-028 | REQ-030 | Verify distinct GUID-correlated TAP interfaces and independent exclusive owners; reject fixed-path and ordinal-discovery assumptions. |
| VAL-029 | REQ-033 | Verify legacy root devices are neither adopted nor bound as dynamic children; verify explicit migration/cleanup and package service selection. |
| VAL-030 | REQ-034, REQ-035 | Verify lifecycle diagnostics omit packet data and manager restart preserves, enumerates, and reattaches active children without duplication. |
| VAL-031 | REQ-032 | On independently created children, execute the REQ-002, REQ-013, REQ-016, REQ-021, REQ-024, and REQ-025 packet-path, IRQL, RX-ring, cancellation, power, and teardown regressions while create/remove operations occur on other children. |
| VAL-036 | REQ-045 | Inject or observe transient I/O-ring `ERROR_BUSY` (`0x800700AA`) read/write completions during sustained two-endpoint traffic; verify the operation is retried with its original slot, buffer, endpoint, and completion identity up to the finite retry limit, exhaustion terminates explicitly with cleanup, traffic continues through transient events, and an unrelated fatal HRESULT still terminates explicitly. |
| VAL-032 | REQ-037, REQ-038, REQ-039, REQ-042, REQ-044 | Run SPDX validation in full-tree and staged modes over the exact governed extension/path policy. Verify valid source, script, metadata, Markdown, INX, shebang, encoding, and front-matter cases pass; missing, malformed, misplaced, wrong-syntax, and newly introduced governed extensions fail; verify exclusions are explicit, version-controlled, and diagnosed. |
| VAL-033 | REQ-040, REQ-044 | Invoke the pre-commit hook with compliant and noncompliant staged files. Verify compliant commits proceed and noncompliant commits are rejected before commit creation, including partial staging and renames. |
| VAL-034 | REQ-041, REQ-044 | Run CI with a missing or malformed governed-file header in a pull request and protected-branch push context. Verify the stable required `SPDX headers` check fails and merge eligibility/update acceptance is rejected; verify a compliant baseline passes. |
| VAL-035 | REQ-043 | Review contributor documentation and validator diagnostics. Verify every supported file category has an example, the exclusion policy is explicit, and remediation identifies the expected header form. |
| VAL-039 | REQ-049 | Verify OS-managed nonpaged lookaside frame allocation, bounded queue ownership, allocation-failure ring release, in-flight frame drain, and safe pool teardown across normal, cancellation, power, stop, and removal paths. |
| VAL-040 | REQ-047 | Enable adaptive-polling mode on two exclusive TAP handles; verify immediate empty READ, inline busy WRITE, level-sensitive queue-change waits, bounded one-wait-per-handle behavior, legacy compatibility, adaptive switch batching, cancellation, PnP/power teardown, and no lost notification or per-frame I/O-ring wait. |
| VAL-041 | REQ-048 | Verify per-endpoint adaptive negotiation diagnostics, periodic idle wait statistics, and a GUID-correlated two-TAP probe. Reject stale TAP interfaces; prove enable, wait registration, readable capture, wait completion, resumed read, and peer forwarding before recording performance data. |

| Test | Coverage |
|---|---|
| TC-015 | Verify the control context exists before adapter start and early packet callbacks are not dropped. |
| TC-016 | Verify pending read/write limits reject excess requests deterministically. |
| TC-017 | Verify pending-operation counters remain correct across retrieval, cancellation, purge, and requeue. |
| TC-018 | Verify packet callbacks schedule passive completion and never access user buffers at DISPATCH_LEVEL. |
| TC-019 | Verify D0 exit/entry request, frame, callback, and work-item transitions. |
| TC-020 | Verify an undersized pending read fails without losing the queued frame. |
| TC-022 | Verify hosted/runtime readiness status matches the evidence actually available. |
| TC-023 | Create one test GUID child, verify its TAP interface is uniquely identified by GUID, and assign `192.0.2.1/30` without an unintended default route. |
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
| TC-038 | Install the separate bus and child packages after identifying stale legacy root devices; verify only the intended bus and child services bind dynamic children. |
| TC-040 | Issue an empty-queue overlapped read and verify `ReadFile` returns false with error 997; cancel it and verify `GetOverlappedResult` returns false with error 995. Repeat this error-observation path before ARP/ICMP assertions. |
| TC-041 | Verify a 0-byte overlapped write completes as a Win32 no-op. At the selected MTU, issue 1-byte, 13-byte, and maximum-frame-plus-one (65,536 bytes at MTU 65,521) overlapped writes; verify each completes with error 87, transfers no bytes, leaves no queued frame or retained pending request, and is followed by a successful maximum-frame write. |
| TC-042 | In a clean environment, use the manager control interface to create two test GUID children; verify terminal create results, GUID/interface mapping, service identity, and independent exclusive opens. |
| TC-043 | Assign the REQ-015 IPv4 and IPv6 test addresses, static peer neighbors, exact reciprocal `/32` and `/128` active-store routes, and run-scoped firewall rules. Verify no default route is created and the exact host routes select the opposite egress interface. |
| TC-044 | Start an unbound IPv4 ICMP Echo to B. Verify the request is read from A, relayed to B, the reply is read from B, relayed to A, and the stack reports success with matching Ethernet/IP/ICMP identities, payload, and checksums. |
| TC-045 | Start an unbound IPv6 ICMPv6 Echo to B. Verify the same A-to-B and B-to-A relay path, IPv6 endpoint identities, payload, and ICMPv6 pseudo-header checksum. |
| TC-046 | Exercise malformed/truncated frames, write/read cancellation, route/neighbor/firewall/address failure, partial provisioning, timeout, and device removal. Verify both handles complete before release, only created state is removed, diagnostics persist, and primary failure is retained. |
| TC-047 | Execute TC-042 through TC-046 using the manager-based dual-adapter harness on a GitHub-hosted Windows job and a manual Hyper-V/WinDbg VM; verify shared assertions and no capability-only skip. |
| TC-048 | Pre-post a TAP read on B, relay A's valid IPv4 Echo Request into B, and fail if B's reverse-direction read returns that byte-identical request. Record/rearm unrelated frames; require B's valid Echo Reply to be relayed to A and reported successful by the unbound Ping client. Repeat for ICMPv6. |
| TC-049 | Exercise injection while RX polling is active and while receive notification is armed. Close and reopen the TAP owner while RX remains running, then verify a later write requests a new RX advance. Send enough valid routed frames to cross at least one RX-ring capacity handoff, then cancel/stop during queued injection. Under NetAdapterCx verifier, verify packet and fragment ownership remains synchronized, no ring entry is returned twice, and no frame leaks into the TAP read path. |
| TC-050 | With static peer neighbors installed, present valid ARP, multicast Neighbor Solicitation, unicast Neighbor Unreachability Detection Solicitation without a source link-layer option, and Duplicate Address Detection frames to each relay direction. Verify the harness validates and counts them, performs no peer write, remains free of a reflection loop, and still completes the IPv4 and IPv6 Echo tests. |
| TC-051 | Feed the switch known, unknown, broadcast, multicast, VLAN-tagged, source-move, source-destination, malformed, and unsupported frames on both static endpoints; verify FDB learning and bounded full-table behavior, peer-only flooding, and no reflection. |
| TC-052 | Probe I/O-ring capabilities on each target OS, verify required contiguous read/write operations before starting, record the selected version, and verify v4 scatter/gather is used only when separately supported and validated. |
| TC-053 | Saturate configured read/write slots and the 4,096-entry FDB; verify deterministic bounded backpressure or rejection, slot-generation protection, no cross-frame corruption, and recovery after terminal completions. |
| TC-054 | Cancel and remove either endpoint during pending reads and peer writes; verify no new reads are posted, every original completion is consumed before deregistration/close, stale generations cannot free reused slots, and cleanup preserves the primary failure. |
| TC-055 | Verify the endpoint collection accepts two GUID-correlated interfaces created through the manager and selects by identity rather than fixed path or two-branch-only logic. |
| TC-056 | Configure several positive even shared depths, including a value greater than 256, and verify equal per-endpoint capacity, successful allocation/registration, saturation behavior, and recovery after terminal completions. |
| TC-057 | Exercise zero, odd, maximum-integer, arithmetic-overflow, and otherwise unrepresentable depth values; verify deterministic explicit startup errors and no partially published ring, endpoint, or buffer state. |
| TC-058 | Force buffer allocation failure and I/O-ring depth/resource-limit failure; verify the requested depth is not clamped or wrapped, all partial resources unwind, and the primary failure is preserved. |
| TC-059 | Force registered-buffer or operation-registration failure after partial progress; verify startup fails, every previously allocated resource is released exactly once, and no endpoint enters `Running`. |
| TC-060 | Submit and complete operations using the highest allocated slot IDs, both directions, multiple generations, and cancellation markers; verify encode/decode round trips, rejection of truncation/collision/stale identities, and no release of a reused slot. |
| TC-061 | Cancel, remove, and shut down with a depth above 256 and outstanding reads/writes on both endpoints; verify all original completions are consumed before deregistration or close and no buffer is reused early. |
| TC-062 | Submit minimum, normal, and maximum valid frames under idle, full-injection-queue, notification-armed, notification-disarmed, stop, close, and concurrent RX-advance conditions; verify each write completes exactly once, no write enters a WDF manual queue, and accepted frames remain exclusively in the injection path. |
| TC-063 | Instrument WDF callback execution level and allocation paths; verify every inline write operation is valid at the observed IRQL, uses nonpaged-safe state, and fails adapter initialization explicitly when the required callback contract is unavailable. |
| TC-064 | Race inline writes with adapter stop, owner close, cancellation, surprise removal, injection-queue close, and notification enable/disable; verify no use-after-free, double completion, retained request, stale notification, or frame leak under NetAdapterCx verifier. |
| TC-065 | Exercise TX capture with a pending compatible READ at `PASSIVE_LEVEL`; verify direct fragment-to-output-buffer delivery and ring advancement. Repeat at elevated IRQL and verify nonpaged capture, passive work-item delivery, bounded backpressure, no indefinite ring retention, too-small-buffer retry behavior, cancellation, and teardown. |
| TC-066 | Instrument `evt_io_read` and passive completion paths while a captured frame and READ request are available; verify ownership is claimed under the state lock, the lock is released before WDF buffer retrieval/copy/completion, too-small or failed retrieval requeues the frame under the lock, and concurrent callback/work-item/cancellation/teardown races complete exactly once. |
| TC-067 | Install the separate bus and child packages; verify one bus parent appears, manager create produces one child PDO and one NetAdapterCx adapter per GUID, and remove returns both to absent state. |
| TC-068 | Send manager create, remove, enumerate, and query requests as a standard user and with malformed version, opcode, length, request ID, and GUID values. Verify explicit failure before allocation or child-list mutation. |
| TC-069 | Submit duplicate and concurrent creates for one GUID, then concurrent creates for at least three distinct GUIDs. Verify one child per GUID, stable identity, isolated interfaces, and explicit failure rather than partial publication on exhaustion. |
| TC-070 | For one active child with pending TAP reads, writes, frames, and callbacks, request explicit remove. Verify operation correlation, rejection of duplicate lifecycle requests, exactly-once I/O terminal completion, child PDO removal, and interface withdrawal before successful remove completion. |
| TC-071 | Trigger surprise removal and bus teardown while create or remove is pending and while independent children remain active. Verify per-GUID serialization, no cross-child teardown, primary-failure preservation, and complete cleanup. |
| TC-072 | Open two or more GUID-correlated TAP interfaces independently, reject a second owner per interface, and verify manager restart followed by enumerate/reattach neither removes nor duplicates active children. |
| TC-073 | Verify manager create does not report success before the child interface is observable, and remove does not report success while the child PnP instance or interface remains observable. |
| TC-074 | Install with stale `ROOT\WinTapRust` and `ROOT\WinTapRust2` devices. Verify the bus does not adopt them; exercise explicit migration/cleanup and verify no legacy device is selected as a dynamic child. |
| TC-075 | Capture manager and bus diagnostics across invalid requests, create failure, remove failure, surprise removal, and cleanup failure. Verify request ID, GUID, PnP state, interface identity, and primary cleanup failure are present while packet payload bytes are absent. |
| TC-076 | Create at least three children. Exercise valid and invalid TAP I/O, receive filtering, directional isolation, notification arming, passive/elevated TX delivery, cancellation, D0 transition, and teardown on one child while creating and removing the others. Verify no cross-child frame, queue, completion, lock, or callback state. |
| TC-077 | For every tracked governed file, verify the policy-approved SPDX expression is present with the correct comment syntax: `MIT` for repository-owned files and `MIT OR Apache-2.0` for the documented vendored WDK bindings. Verify no governed file is missing its assigned identifier. |
| TC-078 | Validate files with shebangs, encoding declarations, YAML front matter, and comment-sensitive preambles. Verify headers preserve interpreter behavior and front-matter parsing. |
| TC-079 | Run full-tree and staged validation against missing, malformed, wrong-language, misplaced, deleted, renamed, and newly added governed files. Verify each failure is nonzero and diagnostic. |
| TC-080 | Stage a compliant file and a file with its SPDX header removed. Invoke the pre-commit hook and verify only the compliant commit succeeds; verify the hook checks the index rather than an unrelated working-tree version. |
| TC-081 | Execute CI with a deliberately noncompliant governed file in pull-request and protected-branch push contexts. Verify the required SPDX job fails and the compliant baseline passes. |
| TC-082 | Present binary and generated files covered by the exclusion list. Verify exclusions are explicit, reported, and cannot cause source or configuration files to be skipped. |
| TC-083 | Verify contributor documentation states the MIT policy, supported comment forms, preamble rules, local hook usage, CI behavior, and the process for requesting a justified exclusion. |
| TC-084 | Add governed files in every repository directory and supported extension family. Verify full-tree, staged, pre-commit, and CI paths apply one consistent policy without directory-specific bypasses. |
| TC-085 | Run the switch with both dynamic endpoints while forcing transient `ERROR_BUSY` I/O-ring completions. Verify bounded backoff retries the same operation no more than eight times without slot reuse, frame loss caused by premature buffer release, stale completions, or unbounded looping; verify retry exhaustion cancels and releases the consumed slot exactly once before shutdown, and fatal completion errors remain surfaced. |
| TC-086 | Submit a version-2 create request with `requested_mtu=0`; verify the manager reports the selected MTU as 1,500, the adapter advertises frame maximum 1,514, and normal packet acceptance/rejection bounds apply. |
| TC-087 | Create children with valid MTU properties of 1,500, an intermediate value, and 65,521; verify the selected value is reported and the advertised MTU, `MaximumFrameSize`, queue limits, maximum accepted frame, and maximum-plus-one rejection all match. |
| TC-088 | Submit version-2 create requests with zero, below-minimum, above-maximum, malformed, truncated, and unsupported MTU fields; submit version-1 requests and non-create requests with nonzero `requested_mtu`; verify explicit failure before child publication and no partial child state. |
| TC-089 | Remove and recreate a child with a different MTU; verify the new value applies, an active child has no runtime MTU mutation path, and the switch rejects a pair whose effective MTUs differ. |
| TC-092 | Enable adaptive-polling mode on one exclusive TAP handle. Verify an empty READ completes with `STATUS_NO_MORE_ENTRIES`, an accepted WRITE completes inline, and a full injection queue returns `STATUS_DEVICE_BUSY` without retaining either request. Verify a legacy handle retains pending-READ behavior. |
| TC-093 | For each requested readiness mask, race an empty-to-nonempty capture transition and a full-to-nonfull injection transition against `WAIT_FOR_CHANGE` registration, cancellation, and a second simultaneous wait. Verify immediate level-triggered completion when already ready, exactly one completion for the registered wait, explicit rejection of the second wait, no lost wakeup, and no stale wait after owner close. |
| TC-094 | Run the adaptive switch on two dynamic endpoints under sustained and bursty traffic. Verify it uses zero-minimum I/O-ring submissions while making progress, polls only within its configured microsecond budget, blocks only on an overlapped `WAIT_FOR_CHANGE` request after idle, resumes after readable/writable notification, and preserves slot generation and frame forwarding ownership. Force one endpoint to reject or not support negotiation after its peer has enabled it; verify all negotiating handles close, both driver instances return to legacy state, the switch reopens both endpoints, and bidirectional legacy forwarding continues without a mixed-mode relay. Record `KeSetEvent`/`HalpInterruptSendIpi` attribution against the legacy one-completion-wait baseline as performance evidence; it is not a pass/fail threshold. |
| TC-095 | During an outstanding adaptive `WAIT_FOR_CHANGE`, exercise cancellation, owner cleanup, D0 exit/entry, queue stop/start, surprise removal, and release hardware. Verify exactly one terminal wait completion, no request or frame leak, no use of the manual READ queue for adaptive empty READs, and safe legacy-mode purge/resume behavior. |
| TC-090 | Provision two manager-created TAP children while stale TAP instances are present. Verify each test address, route, PnP hardware ID, and switch endpoint maps to the same created GUID. Force unsupported, incompatible, and accepted adaptive enable results; verify endpoint-correlated diagnostics. For the accepted case, generate ARP/ICMP traffic and verify idle wait submission, readable capture, wait completion, resumed read, peer forwarding, and bidirectional reply before collecting performance counters. |
| TC-091 | Exercise minimum, normal, maximum, burst, queue-full, allocation-failure, cancellation, D0, stop, owner-close, surprise-removal, and teardown paths with OS nonpaged lookaside-backed full-size frame elements. Verify no normal frame path uses a general-purpose per-frame allocation, queue limits remain the only admission boundary, every element is acquired and released exactly once, prior payload bytes are not observable after reuse, NetAdapterCx entries and user buffers are never retained on acquisition failure, and lookaside deletion occurs only after all frame ownership drains. |

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
8. **Routed dual-adapter relay:** create two clean GUID-keyed children through
   the manager, relay complete frames between their independently discovered
   TAP interfaces, and verify IPv4 and IPv6 stack round trips use the
   configured adapter routes rather than loopback. With permanent neighbors,
   validate/count and suppress ARP and IPv6 Neighbor Discovery rather than
   relaying those control frames.

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
`tests/run-wintap-dual-adapter-harness.ps1`. It shall use the administrator
manager interface to create two test GUID children, map their returned GUIDs
to distinct TAP interfaces, configure the routed IPv4/IPv6 topology, run the
bidirectional relay, preserve diagnostics, and remove only the children and
package state it created.

The first-release switch validation uses two manager-created GUID-correlated
interfaces through a collection-oriented endpoint model. It does not add
arbitrary-N forwarding or an overlapped-I/O fallback. Its I/O-ring capability
and completion tests are separate from the existing driver overlapped-I/O
harness.

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
TC-062 through TC-064 provide trace points for REQ-021. TC-065 provides the
trace point for REQ-024. TC-066 provides the trace point for REQ-025.
TC-086 through TC-089 provide trace points for REQ-046. TC-092 through TC-095 provide trace points for REQ-047. TC-090 provides the
trace point for REQ-048, and TC-091 provides the trace point for REQ-049.
