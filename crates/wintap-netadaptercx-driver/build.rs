fn main() -> Result<(), wdk_build::ConfigError> {
    let wdk_root = wintap_wdk_bootstrap::configure();
    wdk_build::configure_wdk_binary_build()?;

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".into());
    let lib_arch = match arch.as_str() {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => panic!("unsupported Rust target architecture '{other}'"),
    };
    let netadapter_lib = wdk_root
        .join("Lib")
        .join("10.0.28000.0")
        .join("km")
        .join(lib_arch)
        .join("netcx")
        .join("kmdf")
        .join("adapter")
        .join("2.5");
    println!(
        "cargo:rustc-link-search=native={}",
        netadapter_lib.display()
    );
    println!("cargo:rustc-link-lib=static=netadaptercxstub");
    Ok(())
}
