# Current Project Status

**Status date:** 2026-08-10  
**Requirements:** Approved baseline `REQ-001` through `REQ-009`
**Implementation:** Corrective patch sets `CHG-001` through `CHG-014` and
`CHG-015` through `CHG-020` plus `CHG-022` applied without changing the
approved Ethernet/TAP scope. `CHG-021` was superseded by `CHG-019`.

## Verified in the repository

- x64 and ARM64 project configurations and INF declarations are present.
- Hosted validation covers specification artifacts, PowerShell syntax, WDK
  `stampinf.exe` provisioning, CMake configure/build/package, and package
  artifact checks.
- Driver lifetime paths now synchronize control-context acquisition, packet
  callbacks, power quiescence, frame cleanup, and the write-drain work item.
- Fragment metadata is checked before every packet copy.
- The user-mode harness covers administrator access, exclusive open behavior,
  malformed writes, overlapped cancellation, and valid writes.
- The opt-in privileged harness path discovers the root-enumerated adapter,
  isolates `192.0.2.1/30`, handles ARP, validates/builds IPv4 ICMP frames with
  checksums, verifies the Windows Ping result, captures diagnostics, and
  performs idempotent address/device cleanup.
- The maintenance alignment corrections bound pending operations, defer
  packet-path user-buffer completion to passive work, preserve frames for
  undersized reads, and define D0 request/frame behavior.

## Deferred evidence

- The harness requires an elevated administrator session and test signing for
  test-signed packages. Hosted GitHub runners may reject the required
  test-signing/reboot policy; the privileged job fails with that platform
  error and uploads diagnostics instead of skipping packet-path coverage.
- The pinned NetAdapterCx 2.5 headers expose packet queue start/stop/advance
  callbacks and `NetAdapterStart`/`NetAdapterStop`; no separate adapter
  pause/restart callback API was found. Pause/restart behavior remains
  deferred to verified framework callbacks or a documented future WDK baseline.
- RX ring ownership/index semantics were not changed. The current
  `BeginIndex` advancement must be validated against an installed runtime or a
  verified NetAdapterCx sample before making a semantic change.
- Catalog generation and production signing, installation, enumeration,
  removal, power transition, and Driver Verifier execution remain
  self-hosted/manual gates. Hosted/local builds do produce test-signed `.sys`
  files.
- The REQ-009 hosted job is wired to run the same privileged entry point as the
  VM path. A hosted result remains blocked until a runner already configured
  for test signing permits driver installation and virtual-interface setup.
- The restored `10.0.28000.2526` WDK package on this host does not include
  `ApiValidator.exe`; the default build disables that target and provisioning
  reports the limitation rather than claiming API validation. A runner that
  supplies the tool can explicitly enable the target.

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

F-017, F-018, F-022, and F-023 remain deferred pending privileged or
authoritative NetAdapterCx evidence.

## Post-alignment verification

- Specification artifact validation passed.
- PowerShell syntax validation passed.
- x64 Debug direct MSBuild build and test signing passed.
- ARM64 Debug direct MSBuild build and test signing passed.
- x64 and ARM64 package artifact validation passed.
- CMake x64 package build passed.
- Catalog generation, installation, packet-path, power-transition, removal,
  and Driver Verifier execution remain deferred as documented above.
