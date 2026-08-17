# Rust NetAdapterCx migration scaffold

This workspace captures the approved Phase 5 Rust migration boundary without
replacing the verified C package path until the Rust toolchain, `cargo-wdk`,
LLVM/libclang, and generated NetAdapterCx bindings are available and validated.

- `netadaptercx-sys` generates raw NetAdapterCx 2.5 bindings from the pinned
  WDK/SDK NuGet headers (`10.0.28000.2526`).
- `wintap-netadaptercx-driver` is a KMDF `cdylib` using `windows-drivers-rs`
  WDF crates with `panic = "abort"`.
- The scaffold deliberately returns `STATUS_NOT_SUPPORTED` from device-add so
  it cannot be mistaken for the complete TAP datapath port.

Build prerequisites: Rust `1.85.0`, `cargo-wdk`, LLVM/libclang usable by
`bindgen`, and `nuget.exe`. On the first `cargo build`, the workspace restores
the pinned WDK/SDK NuGet packages for the selected architecture into
`out\packages`; it does not use a machine-installed SDK or WDK.
