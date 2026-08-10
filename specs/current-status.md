# Current Project Status

**Status date:** 2026-08-10  
**Requirements:** Approved baseline `REQ-001` through `REQ-007`  
**Implementation:** Corrective patch set `CHG-001` through `CHG-014` applied
without changing the approved Ethernet/TAP scope.

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

## Deferred evidence

- The harness requires an installed, test-signed driver and an elevated
  administrator session. Hosted GitHub runners do not claim privileged
  installation, packet-path, power, removal, or Driver Verifier coverage.
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
- The restored `10.0.28000.2526` WDK package on this host does not include
  `ApiValidator.exe`; the default build disables that target and provisioning
  reports the limitation rather than claiming API validation. A runner that
  supplies the tool can explicitly enable the target.

This page is the current status record. Historical phase and audit documents
remain unchanged for traceability.
