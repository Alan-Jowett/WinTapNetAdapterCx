# Proposal: Dynamically Provisioned WinTapRust Adapters

## Status

Proposal only. This document does not change the driver, INF, package, or
existing specifications.

## Goal

Allow an elevated user-mode manager to create and remove an arbitrary number
of WinTapRust adapters at runtime, using the supported Windows PnP
provisioning path (DevCon, PnPUtil, or equivalent SetupAPI calls).

This provides the practical Windows analogue to dynamic Linux TAP device
creation without making a live network adapter create another PnP device via
an IOCTL.

## Feasibility

This is feasible without a virtual-bus-driver architecture.

Windows PnP, rather than the NetAdapterCx adapter, creates a root-enumerated
device instance. The WinTapRust driver's `EvtDriverDeviceAdd` callback then
creates its WDF device and NetAdapterCx adapter. The existing dual-adapter
harness already uses this model:

1. `devcon install <inf> ROOT\WinTapRust` creates a root-enumerated device.
2. PnP loads `WinTapRust` and invokes `EvtDriverDeviceAdd`.
3. The driver creates the adapter and its TAP control endpoint.
4. The harness records the PnP instance ID and removes it with
   `pnputil /remove-device <instance-id>`.

Creation and removal should remain privileged user-mode PnP operations. An
adapter IOCTL cannot create a new PnP device, and no adapter control endpoint
exists when there are no adapters.

## INF Hardware IDs

The current INF lists both `ROOT\WinTapRust` and `ROOT\WinTapRust2`. That
does not itself impose a two-device limit. Repeated installation of
`ROOT\WinTapRust` can create distinct root-enumerated PnP instances that use
the same INF match.

The proposed package design uses one generic hardware ID:

```text
ROOT\WinTapRust
```

`ROOT\WinTapRust2` can be retained temporarily for compatibility with the
existing two-adapter test, but it is not needed for dynamic provisioning.
The unique identity of an installed adapter is its PnP instance ID, not a
unique INF hardware ID.

## Current Constraints

The current implementation is intentionally limited to two live instances:

- `INSTANCE_IDS` and `INSTANCE_STATES` are fixed-size arrays with two
  elements.
- `reserve_instance_id` returns `STATUS_DEVICE_BUSY` after both slots are in
  use.
- The assigned MAC address and control names are based on the transient slot
  number.
- Control paths are fixed as `\\.\WinTapRust` and `\\.\WinTapRust2`.
- The dual-adapter harness explicitly expects exactly the two INF hardware
  IDs and their fixed MAC/control-path mapping.

The fixed driver state and fixed endpoint naming, rather than the INF device
list, are the material blockers to arbitrary adapter count.

## Proposed Architecture

### User-mode manager

Provide a privileged manager CLI, service, or library which:

1. Ensures the signed driver package is available in the Driver Store.
2. Creates a root-enumerated `ROOT\WinTapRust` devnode through SetupAPI or
   the initially proven DevCon/PnPUtil tooling.
3. Records the returned PnP instance ID as the manager's adapter identity.
4. Waits for adapter arrival and resolves its TAP control interface.
5. Deletes only the recorded PnP instance through PnPUtil or equivalent
   SetupAPI functionality.
6. Waits for removal completion before reporting success.

The manager should not remove a package or a device that it did not create.
It should expose operation status because PnP arrival and removal are
asynchronous.

### Driver instance model

Change the driver from two static slots to dynamically allocated,
per-PnP-device instance state:

- Associate adapter state directly with the PnP WDF device context instead of
  searching fixed global arrays.
- Use a synchronized dynamic registry only where callbacks need
  adapter-to-instance lookup.
- Remove the two-instance admission limit and release each instance only when
  its PnP device teardown completes.
- Keep each instance's queues, locks, work items, NetAdapterCx adapter,
  exclusive-owner state, and cleanup independent.

### Stable identities

Do not use allocation order as the persistent adapter identity.

- Derive or persist a unique locally administered MAC address for every PnP
  instance.
- Publish a discoverable per-adapter control device interface instead of
  relying on sequential global DOS symbolic links.
- Correlate the discovered control interface with the PnP instance ID and/or
  permanent MAC address.
- Preserve the existing `\\.\WinTapRust` and `\\.\WinTapRust2` names only if
  a compatibility layer is explicitly required; they are unsuitable as the
  primary arbitrary-instance API.

## Lifecycle Requirements

Adapter deletion must remain PnP-driven:

1. Stop accepting new opens and I/O.
2. Cancel and complete pending reads and writes.
3. Drain or discard queued frames according to the existing ownership rules.
4. Stop the NetAdapterCx adapter and packet queues.
5. Delete the control endpoint and release per-instance state only after
   framework teardown makes callbacks impossible.
6. Notify the manager that the recorded PnP instance has been removed.

The implementation must tolerate races among create, remove, owner close,
cancelled I/O, D0 transitions, and packet queue callbacks. A slot or
interface name must not be reused until all previous references have been
released.

## Validation Scope

Add tests for:

- Repeated creation of the same `ROOT\WinTapRust` hardware ID.
- Concurrent create requests and a configurable instance quota.
- Discovery of the exact TAP interface for each recorded PnP instance.
- Unique, stable MAC assignment across removal and recreation.
- Independent exclusive opens and packet traffic across three or more
  adapters.
- Removal with no owner, an active owner, pending reads/writes, queued frames,
  and active RX/TX callbacks.
- Concurrent remove and owner close/cancellation.
- Failed installation, arrival timeout, failed removal, and manager restart.
- Confirmation that cleanup removes only manager-created PnP instances and
  never removes a pre-existing adapter or Driver Store package.
- Driver Verifier coverage for create/remove stress and teardown.

## Alternatives Considered

### Management IOCTL on an adapter

Not recommended. It cannot create a new root-enumerated PnP device, and it is
unavailable when no WinTap adapter exists.

### Permanent management device plus virtual bus

Feasible, but substantially more complex. A permanent root-enumerated bus
device would accept management IOCTLs and enumerate child PDOs through
KMDF's dynamic child-list support. Each child would load the adapter function
driver and enter the normal NetAdapterCx device-add lifecycle. This is only
appropriate if the product requires a kernel-resident IOCTL control plane;
it is not necessary when DevCon, PnPUtil, or SetupAPI provisioning is
acceptable.

### Multiple NETADAPTER objects on one WDF device

NetAdapterCx supports this in principle, but it conflicts with the current
one-PnP-device/one-control-device/one-queue-set design. It would require a
larger lifecycle and control-plane redesign than retaining PnP instances as
the adapter boundary.

## Recommendation

Adopt the user-mode PnP manager approach. First replace the driver's fixed
two-instance registry and endpoint naming with dynamic, per-PnP-device state
and discoverable interfaces. Then generalize the existing DevCon/PnPUtil
provisioning and cleanup logic into a supported manager API.

## Evidence

- `crates\wintap-netadaptercx-driver\src\lib.rs`: device addition creates
  per-instance WDF/NetAdapterCx objects; current registry and naming are
  limited to two instances.
- `crates\wintap-netadaptercx-driver\wintap_netadaptercx_driver.inx`: current
  root-enumerated hardware-ID entries.
- `tests\run-wintap-dual-adapter-harness.ps1`: documented DevCon creation and
  PnPUtil removal of recorded root-enumerated instances.
- Microsoft WDF documentation: dynamic child enumeration is available through
  `WDFCHILDLIST` and `EvtChildListCreateDevice` if a virtual-bus design is
  later required.
