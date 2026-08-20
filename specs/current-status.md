# Current Project Status

**Status date:** 2026-08-19
**Requirements:** Approved baseline `REQ-001` through `REQ-016`
**Implementation:** Rust-only package migration is in progress. The obsolete
C/C++ implementation has been removed from the branch. CHG-032 directional
frame isolation, RX notification handling, and routed-relay hardening are
implemented and manually validated on the test-signed `alanjo-ssp` VM.
CHG-033 resumes purged manual control queues before owner or D0 recovery
publishes the open lifecycle state.

## Verified in the repository

- The Rust driver exports `DriverEntry` and uses generated NetAdapterCx
  bindings from the pinned NuGet WDK.
- CMake invokes `cargo wdk build` after restoring the pinned NuGet packages
  and exposes both `stampinf` and `inf2cat` to cargo-wdk.
- Rust packages use `ROOT\WinTapRust` and `ROOT\WinTapRust2`, service `WinTapRust`,
  `wintap_netadaptercx_driver.inf`, and `wintap_netadaptercx_driver.cat`.
- Receive-filter capabilities advertise directed, broadcast, multicast,
  all-multicast, and promiscuous filtering with a multicast-address capacity
  of 64. TCP/IP binds successfully and creates IPv4 and IPv6 interfaces.
- The user-mode harness covers administrator access, exclusive open behavior,
  malformed writes, overlapped cancellation, and valid writes.
- The opt-in privileged harness path discovers the root-enumerated adapter,
  isolates `192.0.2.1/30`, handles ARP, validates/builds IPv4 ICMP frames with
  checksums, verifies the Windows Ping result, captures diagnostics, and
  performs idempotent address/device cleanup.
- The dedicated dual-adapter harness parses and compiles its native P/Invoke
  wrapper. It provisions the two root identities through the pinned WDK
  DevCon tool, configures routed IPv4/IPv6 peers, relays frames between the
  two exclusive TAP endpoints, rejects byte-identical reflected injections,
  validates/suppresses ARP and IPv6 Neighbor Discovery/DAD control traffic,
  and records cleanup diagnostics.
- Each adapter keeps separate bounded injection and capture frame queues.
  TAP writes can request a receive notification only when it is armed; only
  RX queue advance mutates RX ring entries, and RX cancellation marks
  outstanding packets ignored before returning ring entries.
- Recoverable owner cleanup and D0 entry restart the purged manual read/write
  queues outside the state lock before reopening frame queues and publishing
  `INSTANCE_OPEN`.
- A local x64 Release package build and package validation passed. The latest
  privileged VM owner-reopen smoke and 257-iteration IPv4/IPv6 stress tests
  passed with `OwnerReopenValidated: true`, 516 injection frames in each
  direction, no primary failure, no cleanup error, no reflected injection,
  and no remaining test adapter.

## Deferred evidence

- The harness requires an elevated administrator session and test signing for
  test-signed packages. The repository maintainer confirms that the
  `windows-latest` and `windows-2022` hosted runners are already test-signed;
  a configuration regression still fails explicitly and uploads diagnostics
  instead of skipping packet-path coverage.
- The pinned NetAdapterCx 2.5 headers expose packet queue start/stop/advance
  callbacks and `NetAdapterStart`/`NetAdapterStop`; no separate adapter
  pause/restart callback API was found. Pause/restart behavior remains
  deferred to verified framework callbacks or a documented future WDK baseline.
- CHG-032/CHG-033 verified RX ring ownership/index behavior and owner-reopen
  manual-queue recovery against the installed WDF/NetAdapterCx documentation
  and manual VM relay tests. Driver Verifier coverage of notification,
  cancellation, ring-capacity, and D0 power-transition boundaries remains a
  self-hosted/manual gate.
- Catalog generation and production signing, installation, enumeration,
  removal, power transition, and Driver Verifier execution remain
  self-hosted/manual gates. Hosted/local builds do produce test-signed `.sys`
  files.
- The REQ-009 hosted jobs are wired to run the REQ-008 and REQ-015 privileged
  entry points used by the VM path. Hosted execution evidence remains pending
  the first test-signed runner result.
- The CHG-031 hosted job downloads the x64 package, restores the pinned DevCon
  tool, invokes the dual-adapter harness, and uploads diagnostics. Its
  privileged DevCon, route/neighbor/firewall, relay, and cleanup assertions
  remain pending the first hosted execution.
- The restored `10.0.28000.2526` WDK package on this host does not include
  `ApiValidator.exe`; the default build disables that target and provisioning
  reports the limitation rather than claiming API validation. A runner that
  supplies the tool can explicitly enable the target.
- Stable Cargo cannot run this kernel crate's unit tests because its required
  `panic = "abort"` profile needs the nightly `-Zpanic-abort-tests` option.
  The required unit-test coverage remains pending until it is run with a
  compatible nightly toolchain; this limitation is not a passing result.

This page is the current status record. Historical phase and audit documents
remain unchanged for traceability.

## Maintenance alignment trace

| Finding | Change | Verification | Status |
|---|---|---|---|
| F-015 | CHG-015 | TC-015 | Applied |
| F-016 | CHG-016 | TC-016, TC-017 | Applied |
| F-019 | CHG-017 | Specification trace | Applied |
| F-020 | CHG-018 | TC-018 | Applied |
| F-021, F-025 | CHG-019 | TC-019 | Applied |
| F-024 | CHG-020 | TC-020 | Applied |
| F-026 | CHG-022 | TC-022 | Applied |
| F-027 | CHG-027 | Specification trace | Applied |
| F-029 | CHG-028 | Design queue-limit contract | Applied |
| F-030 | CHG-029 | Teardown queue-drain contract | Applied |
| F-032 | CHG-030 | TC-031 | Applied |
| Runtime directional-frame defect | CHG-032 | TC-048, TC-050 | VM verified |
| F-033 | CHG-033 | TC-049 owner reopen | VM verified |

F-017, F-018, F-022, and F-023 remain deferred pending privileged or
authoritative NetAdapterCx evidence.

## Post-alignment verification

- Specification artifact validation passed.
- PowerShell syntax validation passed.
- x64 Debug direct MSBuild build and test signing passed.
- ARM64 Debug direct MSBuild build and test signing passed.
- x64 and ARM64 package artifact validation passed.
- CMake x64 package build and package validation passed.
- Manual VM owner-reopen smoke and 257-iteration IPv4/IPv6 relay stress tests
  passed.
- Catalog generation, installation, packet-path, power-transition, removal,
  and Driver Verifier execution remain deferred as documented above.
