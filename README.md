# WinTapNetAdapterCx

WinTapNetAdapterCx is a Windows software network adapter project built with the
[NetAdapterCx](https://learn.microsoft.com/windows-hardware/drivers/netcx/)
framework. Its goal is to implement the core concept of the Linux
[TUN/TAP](https://docs.kernel.org/networking/tuntap.html) adapter on Windows:
providing a software device that allows user-mode applications to exchange
Ethernet frames with the Windows networking stack.

## Project goals

- Create a virtual Ethernet adapter using NetAdapterCx.
- Provide a driver-managed data path for transmitting and receiving Ethernet
  frames.
- Expose a practical user-mode interface for applications that need TAP-style
  packet access.
- Follow Windows driver development, signing, installation, and security
  practices.

## Status

The approved `REQ-008`/`REQ-009` privileged integration change and the
`CHG-001` through `CHG-014` maintenance patch set are applied, and
the approved `CHG-015` through `CHG-020`, `CHG-022`, and `CHG-032` alignment
corrections are applied. `CHG-021` was superseded by the consolidated D0
change. `CHG-032` keeps stack-RX injection and stack-TX capture in separate
frame queues so a successful TAP write cannot complete a TAP read. `CHG-033`
restarts purged manual control queues before a recovered owner or D0 lifecycle
re-enters the open state.
Hosted validation covers repository artifacts, Rust WDK tool provisioning,
CMake configure/build/package, and package shape for x64 and ARM64. The hosted
workflow runs the existing ICMP TAP and routed dual-adapter harnesses, uploading
diagnostics for each. A runner that cannot satisfy test-signing or privileged
device operations fails explicitly; it does not claim packet-path coverage as
passed. See [`specs/current-status.md`](specs/current-status.md) for deferred
evidence and WDK findings.

## Privileged integration harness

The existing entry point preserves the basic overlapped-I/O checks and adds an
opt-in Ethernet/ICMP round trip:

```powershell
.\tests\run-wintap-harness.ps1 -Integration `
  -InstallDriver -RequireTestSigning `
  -PackageDirectory .\out\rust-target\x86_64-pc-windows-msvc\release\wintap_netadaptercx_driver_package `
  -DiagnosticsPath .\artifacts\wintap-harness
```

The integration path discovers the root-enumerated adapter, assigns only
`192.0.2.1/30`, services ARP for `192.0.2.2`, validates and replies to ICMP
Echo, verifies the Windows `Ping` result, and removes only the address/device
state it created. Run it elevated in a Hyper-V VM with test signing enabled.

This repository's `windows-latest` and `windows-2022` hosted runners are
already test-signed. If that runner configuration regresses, the harness fails
explicitly rather than reporting a capability-only pass.

## Routed dual-adapter relay harness

The DuoNIC-style integration harness provisions two disposable root-enumerated
WinTap adapters, installs reciprocal endpoint host routes, and relays Ethernet
frames between their independently exclusive TAP handles. It tests unbound
IPv4 ICMP and IPv6 ICMPv6 traffic through the adapter datapaths rather than
loopback, detects byte-identical reflected injections, and validates/counts
then suppresses ARP and IPv6 Neighbor Discovery/DAD control frames:

```powershell
.\tests\run-wintap-dual-adapter-harness.ps1 `
  -PackageDirectory .\out\rust-target\x86_64-pc-windows-msvc\release\wintap_netadaptercx_driver_package `
  -Architecture x64 `
  -DiagnosticsPath .\artifacts\wintap-dual-adapter-harness
```

Run it elevated in a clean, test-signed Hyper-V/WinDbg VM. It refuses to touch
pre-existing WinTap adapters, resolves `devcon.exe` from the pinned WDK, uses
`198.51.100.1/30`/`198.51.100.2/30` and
`2001:db8:515:1::1/64`/`2001:db8:515:1::2/64`, and removes the two devices,
network state, and only a driver-store package added by that run.
It runs 257 IPv4/IPv6 relay iterations by default; use `-RelayIterations` to
select a bounded alternative.

## Two-TAP switch core

The `wintap-switch-core` crate implements the first-release switch contract
above the two existing control endpoints. It provides collection-oriented
endpoint identities, MAC/VLAN learning with a bounded 4,096-entry FDB,
peer-only forwarding decisions, and generation-protected bounded buffer-slot
state. Dynamic PnP provisioning and arbitrary-N forwarding remain deferred.

## Deploying to a test VM

The following procedure deploys the x64 Rust package and the `wintap-switch`
executable to a clean, test-signed Windows VM over SSH. The example VM is
`alanjo-ssp`; replace the VM name, address, and user as needed. Run the local
PowerShell commands from an elevated 64-bit PowerShell session.

### 1. Build the driver package and switch

Restore the pinned WDK/SDK packages and build the x64 Release package:

```powershell
cmake --preset vs18-x64-debug
cmake --build .\out\build\vs2022-x64-debug --config Release --target wintap_package
cargo build -p wintap-switch --release
```

The driver package is normally under
`out\rust-target\x86_64-pc-windows-msvc\release\wintap_netadaptercx_driver_package`.
The switch executable is under
`target\release\wintap-switch.exe` unless the WDK build configuration redirects
Cargo output to `out\rust-target`.

Resolve the pinned x64 DevCon tool:

```powershell
$DevCon = (Get-ChildItem .\out\packages -Recurse -Filter devcon.exe |
    Where-Object FullName -Match 'WDK\.x64.*\\x64\\devcon\.exe' |
    Select-Object -First 1 -ExpandProperty FullName)
if (-not $DevCon) { throw "Pinned x64 devcon.exe was not found." }
```

### 2. Resolve the VM address and verify SSH

For a local Hyper-V VM:

```powershell
$VmName = 'alanjo-ssp'
$VmIp = Get-VMNetworkAdapter -VMName $VmName |
    Select-Object -ExpandProperty IPAddresses |
    Where-Object { $_ -match '^\d+\.\d+\.\d+\.\d+$' } |
    Select-Object -First 1
if (-not $VmIp) { throw "No IPv4 address was reported for $VmName." }

$VmUser = 'administrator'
ssh -o StrictHostKeyChecking=accept-new "$VmUser@$VmIp" hostname
```

SSH must authenticate as an administrator. The VM must be test-signed and
configured to permit driver installation. Do not use a production machine for
this procedure.

### 3. Copy the package, DevCon, and switch

```powershell
$RemoteRoot = 'C:/Temp/WinTapSwitch'
ssh "$VmUser@$VmIp" "cmd /c if not exist C:\Temp\WinTapSwitch\package mkdir C:\Temp\WinTapSwitch\package"

scp -r .\out\rust-target\x86_64-pc-windows-msvc\release\wintap_netadaptercx_driver_package\* `
    "$VmUser@${VmIp}:$RemoteRoot/package/"
scp $DevCon "$VmUser@${VmIp}:$RemoteRoot/devcon.exe"

$SwitchExe = '.\target\release\wintap-switch.exe'
if (-not (Test-Path $SwitchExe)) {
    $SwitchExe = '.\out\rust-target\release\wintap-switch.exe'
}
if (-not (Test-Path $SwitchExe)) { throw "wintap-switch.exe was not found." }
scp $SwitchExe "$VmUser@${VmIp}:$RemoteRoot/wintap-switch.exe"
```

Verify the transfer before running it:

```powershell
Get-FileHash $SwitchExe -Algorithm SHA256
ssh "$VmUser@$VmIp" `
    "powershell -NoProfile -Command Get-FileHash -LiteralPath C:\Temp\WinTapSwitch\wintap-switch.exe -Algorithm SHA256"
```

### 4. Provision the two static adapters

The first release intentionally provisions exactly `ROOT\WinTapRust` and
`ROOT\WinTapRust2`; it does not dynamically create arbitrary adapters:

```powershell
ssh "$VmUser@$VmIp" `
    "C:\Temp\WinTapSwitch\devcon.exe install C:\Temp\WinTapSwitch\package\wintap_netadaptercx_driver.inf ROOT\WinTapRust"
ssh "$VmUser@$VmIp" `
    "C:\Temp\WinTapSwitch\devcon.exe install C:\Temp\WinTapSwitch\package\wintap_netadaptercx_driver.inf ROOT\WinTapRust2"
```

If either device already exists, stop and use the cleanup commands below
instead of replacing an unrelated test instance.

### 5. Run the switch startup and shutdown smoke test

The switch opens both TAP endpoints exclusively, probes the required I/O-ring
read/write operations, registers its handles and buffers, and remains in its
completion loop until Ctrl+C or console close. Start it in the VM:

```powershell
ssh "$VmUser@$VmIp" C:\Temp\WinTapSwitch\wintap-switch.exe --read-depth 128
```

`--read-depth` controls the total number of pending read buffers. It must be a
positive even number; buffers are divided equally between the two TAP
endpoints. The default for this experiment branch is `128`, so use
`--read-depth 64` or another even value to compare configurations.

Leave it running long enough to confirm it remains alive, then press Ctrl+C.
A missing endpoint, unavailable I/O-ring capability, registration failure, or
stale completion is reported as a nonzero startup/termination error. The
switch currently uses the contiguous v3 path; v4 scatter/gather remains
disabled until its dedicated validation is complete.

### 6. Run the routed driver relay validation

The existing harness validates the provisioned driver, TAP ownership, routed
IPv4/IPv6 traffic, cleanup, and Driver Verifier-facing ownership assertions.
It owns the TAP handles itself, so stop `wintap-switch.exe` before running it:

```powershell
scp .\tests\run-wintap-dual-adapter-harness.ps1 `
    "$VmUser@$VmIp:$RemoteRoot/run-wintap-dual-adapter-harness.ps1"

ssh "$VmUser@$VmIp" `
    "powershell -NoProfile -ExecutionPolicy Bypass -File C:\Temp\WinTapSwitch\run-wintap-dual-adapter-harness.ps1 -PackageDirectory C:\Temp\WinTapSwitch\package -Architecture x64 -DevConPath C:\Temp\WinTapSwitch\devcon.exe -DiagnosticsPath C:\Temp\WinTapSwitch\diagnostics -RelayIterations 257"
```

The harness uses documentation-only IPv4/IPv6 addresses, exact host routes,
static neighbors, and run-scoped firewall rules. It refuses to modify
pre-existing matching adapters and removes only state created by that run.
Inspect `C:\Temp\WinTapSwitch\diagnostics` after a failure and copy it back
with `scp -r` before cleaning the VM.

### 7. Clean up the VM

If the harness was not used, remove only the two devices created for this
test:

```powershell
ssh "$VmUser@$VmIp" `
    "C:\Temp\WinTapSwitch\devcon.exe remove ROOT\WinTapRust"
ssh "$VmUser@$VmIp" `
    "C:\Temp\WinTapSwitch\devcon.exe remove ROOT\WinTapRust2"
```

Confirm both identities are gone:

```powershell
ssh "$VmUser@$VmIp" "C:\Temp\WinTapSwitch\devcon.exe find ROOT\WinTapRust"
ssh "$VmUser@$VmIp" "C:\Temp\WinTapSwitch\devcon.exe find ROOT\WinTapRust2"
```

The harness performs this cleanup automatically, including addresses, routes,
neighbors, firewall rules, PnP instances, and any driver-store package added
by that invocation. Do not remove a pre-existing Driver Store package by
hand.

## Intended platform

- Windows 10 and later
- Windows Driver Kit (WDK) with NetAdapterCx support
- Rust `1.85.0`, `cargo-wdk`, LLVM/libclang, and NuGet

The project targets Windows 10 version 2004 and later on x64 and ARM64. Build
dependencies use the pinned WDK/SDK NuGet version `10.0.28000.2526`.

## Related technologies

- [NetAdapterCx](https://learn.microsoft.com/windows-hardware/drivers/netcx/)
- [Windows networking drivers](https://learn.microsoft.com/windows-hardware/drivers/network/)
- [Linux TUN/TAP documentation](https://docs.kernel.org/networking/tuntap.html)

## License

This project is licensed under the [MIT License](LICENSE).
