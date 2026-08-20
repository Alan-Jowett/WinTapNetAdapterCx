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
