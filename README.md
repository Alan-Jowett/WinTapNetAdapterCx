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
the approved `CHG-015` through `CHG-020` plus `CHG-022` alignment corrections
are applied. `CHG-021` was superseded by the consolidated D0 change.
Hosted validation covers repository artifacts, Rust WDK tool provisioning,
CMake configure/build/package, and package shape for x64 and ARM64. The privileged
runtime harness requires an installed test-signed driver and an elevated
administrator session; hosted CI does not claim packet-path, power, removal,
or Driver Verifier coverage. See [`specs/current-status.md`](specs/current-status.md)
for deferred evidence and WDK findings.

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

The hosted workflow invokes the same command and uploads diagnostics. Windows
hosted runners normally cannot reboot after `bcdedit /set testsigning on`;
therefore a runner that is not already test-signed fails explicitly rather
than reporting a capability-only pass.

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
