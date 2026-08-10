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

This project is under active development. APIs, installation procedures, and
behavior may change as the driver implementation evolves.

## Intended platform

- Windows 10 and later
- Windows Driver Kit (WDK) with NetAdapterCx support
- Visual Studio with the Windows driver development workload

The exact supported Windows versions and build instructions will be documented
as the implementation is established.

## Related technologies

- [NetAdapterCx](https://learn.microsoft.com/windows-hardware/drivers/netcx/)
- [Windows networking drivers](https://learn.microsoft.com/windows-hardware/drivers/network/)
- [Linux TUN/TAP documentation](https://docs.kernel.org/networking/tuntap.html)

## License

This project is licensed under the [MIT License](LICENSE).
