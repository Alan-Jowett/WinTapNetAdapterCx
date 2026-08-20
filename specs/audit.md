# WinTapNetAdapterCx Specification Audit

**Workflow:** `/evolve`  
**Phase:** Phase 3 — Specification Audit  
**Verdict:** PASS, with implementation prerequisites tracked as `[UNKNOWN]`

## Scope examined

- `specs/requirements.md`
- `specs/design.md`
- `specs/validation.md`
- `README.md`
- Repository layout, which currently contains no implementation, project, INF,
  or test artifacts

## Traceability audit

| Requirement | Design coverage | Validation coverage | Result |
| --- | --- | --- | --- |
| REQ-001 software Ethernet adapter | Adapter boundary, start/stop, identity, cleanup | VAL-001, adapter publication | PASS |
| REQ-002 TAP frame exchange | Bidirectional ownership paths, Win32 overlapped I/O, bounded queues | VAL-002, functional and backpressure tests | PASS |
| REQ-003 lifecycle | Start, pause/restart, stop/removal, owner teardown, power | VAL-003, lifecycle/concurrency/power tests | PASS |
| REQ-004 platform compatibility | x64/ARM64 target and explicit WDK/SDK contract | VAL-004 | PASS |
| REQ-005 security/access | Administrator-only device boundary, validation and buffer rules | VAL-005, security tests | PASS |
| REQ-006 verification | Ownership, synchronization, failure, and cleanup design constraints | VAL-006 and functional/adversarial suites | PASS |
| REQ-007 build system | CMake, Visual Studio generator, NuGet dependency design | VAL-007 and reproducible-build checks | PASS |

## Adversarial findings

### Ownership and completion

- **Challenge:** A user buffer could be retained after an overlapped write
  completes.
- **Control:** The design requires capture into a driver-owned nonpaged frame
  before completion and explicitly forbids retaining the user buffer.
- **Result:** Neutralized by design; validation includes buffer-retention and
  cleanup checks.

- **Challenge:** A framework packet could be returned after the adapter or its
  backing memory is destroyed.
- **Control:** The design requires framework ownership return before final
  teardown and lifetime references around callbacks/completions.
- **Result:** Neutralized by design; callback/removal races are tested.

### Queue and cancellation races

- **Challenge:** Cancellation could race with dequeue or completion and cause
  double completion or a leaked request.
- **Control:** Queue states, atomic removal/marking, one lock ordering, and one
  terminal completion are explicit.
- **Result:** Neutralized by design; cancellation races are required tests.

- **Challenge:** Backpressure could deadlock teardown.
- **Control:** Waiting is permitted only in `OPEN`; `CLOSING` rejects new work
  and cancellation drains pending requests.
- **Result:** Neutralized by design; full-queue close and cancellation tests are
  required.

### Lifecycle and power

- **Challenge:** Pause, stop, or surprise removal could leave requests
  indefinitely pending.
- **Control:** All transitions enter `CLOSING`, reject new work, cancel pending
  requests, and release framework resources before final destruction.
- **Result:** Neutralized by design; lifecycle and power tests cover pending I/O.

### IRQL and memory safety

- **Challenge:** A high-IRQL callback could call pageable code or touch pageable
  data.
- **Control:** The design requires IRQL-aware placement, nonpaged data for
  high-IRQL access, and SAL annotations.
- **Result:** Covered by design and Driver Verifier validation; exact callback
  IRQL contracts remain an implementation prerequisite.

### Build and platform reproducibility

- **Challenge:** A machine-global WDK or SDK could silently produce a different
  binary or package.
- **Control:** CMake, Visual Studio generator, NuGet acquisition, and package
  version capture are normative requirements.
- **Result:** Neutralized by design; clean-environment VAL-007 is required.

## Open evidence and gates

The following are intentionally unresolved and must be verified before Phase 5
implementation approval:

- **[UNKNOWN]** Exact NetAdapterCx packet, queue, and callback APIs and their
  IRQL/ownership contracts for the selected WDK.
- **[UNKNOWN]** Exact NuGet package IDs and versions for WDK/SDK dependencies.
- **[UNKNOWN]** Ethernet maximum frame size and VLAN-tag policy.
- **[UNKNOWN]** Queue depth defaults and configuration mechanism.
- **[UNKNOWN]** Exact CMake and Visual Studio generator versions.
- **[UNKNOWN]** User-mode test harness language and test-signing command details.

These unknowns do not contradict the approved requirements or current design;
each is assigned to a later implementation prerequisite and has validation
coverage or an explicit evidence requirement.

## Audit verdict

**PASS.** The approved requirements have forward coverage into design and
validation, the principal packet/lifetime/concurrency failure modes have
explicit controls, and no implementation behavior is being asserted without
an evidence gate. Phase 4 requires user review of this audit and the complete
specification set.

---

## CHG-032 Specification Audit Revision

**Workflow:** `/evolve`
**Phase:** Phase 3 — Specification Audit
**Verdict:** PASS
**Scope:** REQ-015, REQ-016, the corresponding design, and VAL-016 /
TC-048 through TC-050.

### Evidence examined

- `specs/requirements.md`, REQ-015 and REQ-016
- `specs/design.md`, RX indication, synchronization, and relay assertions
- `specs/validation.md`, VAL-016 and TC-048 through TC-050
- Pinned `netadaptercx-sys` generated bindings, which expose
  `NET_FRAGMENT::set_Offset` (KNOWN)
- RFC 4861 sections 7.1.1 and 7.2.2, which distinguish valid multicast
  address-resolution, unicast Neighbor Unreachability Detection, and
  Duplicate Address Detection solicitations (KNOWN)

### Forward traceability

| Requirement / finding | Design coverage | Validation coverage | Result |
| --- | --- | --- | --- |
| Directional injection/capture isolation | Separate producer/consumer ownership and TAP-read exclusion | VAL-016, TC-048 | PASS |
| RX fragment descriptor initialization | RX indication initializes Offset, bounds capacity, sets ValidLength, then indicates | VAL-016, TC-049 under verifier | PASS |
| RX cancellation ownership return | Cancellation marks `Ignore` and returns entries; documented exception to advance-only indication | VAL-016, TC-049 | PASS |
| Notification after owner-only cleanup | Preserved arm state while RX remains running; disarm boundaries are explicit | VAL-016, TC-049 | PASS |
| Valid relay control traffic | RFC 4861 multicast NS, unicast NUD NS, and DAD rules are distinct before suppression | TC-050 | PASS |

### Adversarial falsification

- **Ring-contract contradiction:** Falsified. The prior absolute
  queue-advance-only wording conflicted with mandatory cancellation return.
  The revised requirement and design limit cancellation to `Ignore` updates
  and returning outstanding entries; no callback may use that exception to
  indicate a new frame.
- **Stale RX descriptor state:** Falsified. RX indication now has an explicit
  Offset/ValidLength initialization contract before the frame copy, with
  capacity bounds and verifier coverage.
- **Lost notification after owner reopen:** Falsified. The design preserves
  an already armed notification cycle unless the framework queue/lifecycle
  boundary actually stops, cancels, suspends, or releases it.
- **Over-restrictive Neighbor Solicitation filtering:** Falsified. The relay
  validates only RFC-required DAD constraints, accepts valid unicast NUD
  probes without a source link-layer option, and still exercises multicast
  address resolution.
- **Unverifiable control suppression:** Falsified. TC-050 requires each
  validated control case to be counted, suppressed, rearmed, and followed by
  successful IPv4/IPv6 traffic.

### Known execution limitation

TC-049's Driver Verifier and explicit RX capacity/cancel/owner-reopen runtime
coverage remains a required implementation-validation gate. This is a
testable deferred execution item, not a specification contradiction or an
unbounded acceptance criterion.

### Audit verdict

**PASS.** Every revised requirement has coherent design and validation
coverage. The cancellation exception is narrowly constrained, descriptor and
notification lifetimes are explicit, and the control-frame contract admits
the RFC 4861-valid cases needed by the routed relay.

---

## CHG-033 Implementation Audit

**Workflow:** `/evolve`
**Phase:** Phase 6 — Implementation Audit
**Verdict:** PASS
**Finding:** F-033, classified `fix-impl`

### Scope and evidence

- `crates/wintap-netadaptercx-driver/src/lib.rs`
- `tests/validate-spec-artifacts.ps1`
- REQ-003, REQ-016, and TC-049
- Microsoft WDF documentation for `WdfIoQueuePurgeSynchronously` and
  `WdfIoQueueStart`
- Local x64 Release package build and package validation
- `alanjo-ssp` VM owner-reopen smoke and 257-iteration relay stress runs

### Forward traceability

`evt_file_cleanup` and `evt_device_d0_exit` synchronously purge the manual
read/write queues. WDF documents that purge stops a queue and requires
`WdfIoQueueStart` before it can receive requests again. CHG-033 resumes both
queues before `evt_file_cleanup` or `evt_device_d0_entry` publishes
`INSTANCE_OPEN`, satisfying REQ-003 lifecycle recovery and REQ-016's
post-owner-reopen RX-notification path.

### Adversarial checks

- **Lock reentrancy:** PASS. `WdfIoQueueStart` runs after
  `InstanceStateGuard` is dropped. This is required because WDF may
  synchronously dispatch request handlers while starting a queue.
- **Terminal teardown:** PASS. `evt_device_release_hardware` remains terminal
  and does not resume queues.
- **Lifecycle race:** PASS. Owner cleanup rechecks `INSTANCE_CLOSING` before
  reopening frame queues, preventing it from overriding a concurrent suspend
  or release transition.
- **Error masking:** PASS. The `WdfRequestForwardToIoQueue` diagnostic
  remains failure-only; CHG-033 adds no retry or success-shaped fallback.
- **Regression guard:** PASS. The static artifact validator requires
  `resume_manual_queues` and `WdfIoQueueStart`; TC-049's owner-reopen relay
  exercises the resulting post-reopen reads and writes.

### Runtime verification

The VM smoke and 257-iteration stress runs completed successfully.
`OwnerReopenValidated` was `true`; the stress run recorded 516 injection
frames in each direction, no primary failure, no cleanup error, and no
remaining WinTap adapter.

### Remaining limitations

Driver Verifier and explicit D0 power-transition execution remain manual
gates. The implementation was source-audited for the D0 path but this change
does not claim a completed power-transition runtime test.
