# WinTapNetAdapterCx Maintenance Drift Report

**Workflow:** `/maintain`  
**Audit date:** 2026-08-10  
**Phase 6 status:** Corrective patch set applied

## Classified findings

| Finding | Classification | Corrective change | Status |
| --- | --- | --- | --- |
| F-001 | fix-impl | CHG-001 | Applied; lifecycle gaps remain deferred where no verified callback contract exists |
| F-002 | fix-impl | CHG-002 | Applied; hosted and privileged validation are separated |
| F-003 | fix-impl | CHG-003 | Applied; D0 power callbacks added, framework pause/restart API remains deferred |
| F-004 | fix-impl | CHG-004 | Applied; control-device teardown and global clearing are synchronized |
| F-005 | fix-impl | CHG-005 | Applied; callback acquisition is protected by a global spin lock and active-callback count |
| F-006 | fix-impl | CHG-006 | Deferred semantic change; RX index ownership requires runtime or verified sample evidence |
| F-007 | fix-impl | CHG-007 | Applied; fragment address, offset, capacity, and cumulative-length checks added |
| F-008 | fix-impl | CHG-008 | Applied; packet queue start/stop state uses an interlocked `LONG` |
| F-009 | fix-impl | CHG-009 | Applied; queue and purge cancellation behavior is documented and exercised by the harness scope |
| F-010 | fix-impl | CHG-010 | Applied; write-drain running/idle state is synchronized during teardown |
| F-011 | fix-impl | CHG-011 | Partial; harness scope is expanded, privileged packet/lifecycle tests remain self-hosted/manual |
| F-012 | fix-impl | CHG-012 | Applied; CI provisions `stampinf.exe` and runs direct MSBuild plus CMake |
| F-013 | fix-impl | CHG-013 | Partial; package shape is checked, catalog and installation remain deferred |
| F-014 | fix-spec | CHG-014 | Applied through `specs/current-status.md` without rewriting historical phase records |

## Verification evidence

- Repository artifact validation passed.
- PowerShell syntax validation passed for all repository scripts.
- x64 Debug direct MSBuild build passed with test signing.
- ARM64 Debug direct MSBuild build passed with test signing.
- x64 CMake configure/build/package passed.
- ARM64 CMake configure/build/package passed.
- x64 and ARM64 package-shape validation passed.
- Hosted CI repository validation passed.
- The earlier hosted CI build failed because `stampinf.exe` was not on `PATH`;
  the workflow now provisions it from the restored WDK package.

## Deferred evidence and limitations

- The runtime harness requires an installed test-signed driver and elevation.
- Packet exchange, installation/removal, power transitions, pause/restart,
  process termination, queue saturation, and Driver Verifier remain
  self-hosted/manual acceptance tests.
- RX ring ownership/index semantics remain unchanged pending verified
  NetAdapterCx evidence.
- Catalog generation and production signing are not enabled.
- `ApiValidator.exe` is not present in the restored WDK package on the current
  environment; API validation remains explicitly deferred.

## Current audit conclusion

The corrective patch set reduces specification-to-implementation drift and
hardens callback, power, queue, fragment-copy, and teardown paths. The project
remains **REVISE-IMPLEMENTATION** until the deferred privileged runtime evidence
and RX ownership verification are completed.

## Alignment audit continuation

The subsequent `/maintain` audit used collision-free finding identifiers
`F-015` through `F-026` and approved changes `CHG-015` through `CHG-022`.
`CHG-021` was superseded by the consolidated D0 semantics in `CHG-019`.
The approved rationale was: "Align the implementation with the spec."

Applied changes are traced by `specs/current-status.md` and validation cases
`TC-015`, `TC-016`, `TC-017`, `TC-018`, `TC-019`, `TC-020`, and `TC-022`.
Findings F-017, F-018, F-022, and F-023 remain deferred.
