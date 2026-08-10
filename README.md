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

The approved `CHG-001` through `CHG-014` maintenance patch set is applied.
Hosted validation covers repository artifacts, WDK tool provisioning, CMake
configure/build/package, and package shape for x64 and ARM64. The privileged
runtime harness requires an installed test-signed driver and an elevated
administrator session; hosted CI does not claim packet-path, power, removal,
or Driver Verifier coverage. See [`specs/current-status.md`](specs/current-status.md)
for deferred evidence and WDK findings.

## Intended platform

- Windows 10 and later
- Windows Driver Kit (WDK) with NetAdapterCx support
- Visual Studio with the Windows driver development workload

The project targets Windows 10 version 2004 and later on x64 and ARM64. Build
dependencies use the pinned WDK/SDK NuGet version `10.0.28000.2526`.

## Related technologies

- [NetAdapterCx](https://learn.microsoft.com/windows-hardware/drivers/netcx/)
- [Windows networking drivers](https://learn.microsoft.com/windows-hardware/drivers/network/)
- [Linux TUN/TAP documentation](https://docs.kernel.org/networking/tuntap.html)

## License

This project is licensed under the [MIT License](LICENSE).
