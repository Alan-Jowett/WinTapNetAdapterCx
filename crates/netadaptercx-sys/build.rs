use std::{
    env,
    path::{Path, PathBuf},
};

const SDK_VERSION: &str = "10.0.28000.0";
const WDK_NUGET_VERSION: &str = "10.0.28000.2526";
const NETADAPTERCX_VERSION: &str = "2.5";

fn main() {
    let _wdk_root = wintap_wdk_bootstrap::configure();
    println!("cargo:rerun-if-changed=wrapper.h");

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| String::from("x86_64"));
    let target_define = match arch.as_str() {
        "x86_64" => "_AMD64_=1",
        "aarch64" => "_ARM64_=1",
        other => {
            panic!("unsupported Rust target architecture '{other}'; expected x86_64 or aarch64")
        }
    };
    let clang_target = match arch.as_str() {
        "x86_64" => "x86_64-pc-windows-msvc",
        "aarch64" => "aarch64-pc-windows-msvc",
        _ => unreachable!(),
    };

    let wdk_version =
        env::var("WINTAP_WDK_VERSION").unwrap_or_else(|_| WDK_NUGET_VERSION.to_string());
    if wdk_version != WDK_NUGET_VERSION {
        panic!(
            "unsupported WINTAP_WDK_VERSION '{wdk_version}'; this binding is pinned to {WDK_NUGET_VERSION}"
        );
    }

    let roots = locate_roots(&wdk_version, &arch);
    let wdk_include = roots.wdk.join("Include").join(SDK_VERSION);
    let sdk_include = roots.sdk.join("Include").join(SDK_VERSION);
    let sdk_arch = roots.sdk_arch;

    let include_dirs = [
        wdk_include.join("km"),
        wdk_include.join("shared"),
        wdk_include
            .join("km")
            .join("netcx")
            .join("kmdf")
            .join("adapter")
            .join(NETADAPTERCX_VERSION),
        roots
            .wdk
            .join("Include")
            .join("wdf")
            .join("kmdf")
            .join("1.33"),
        sdk_include.join("shared"),
        sdk_include
            .join("shared")
            .join("netcx")
            .join("shared")
            .join("1.0"),
        sdk_include.join("um"),
        sdk_arch.join("um"),
        sdk_arch.join("ucrt"),
    ];

    for include in &include_dirs {
        if !include.exists() {
            panic!(
                "required WDK/SDK include directory is missing: {}",
                include.display()
            );
        }
    }

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .use_core()
        .ctypes_prefix("core::ffi")
        .derive_default(true)
        // The WDK exposes NET_BUFFER_HEADER as a legacy anonymous union. Clang
        // reports its C layout using the inactive legacy member, while bindgen
        // intentionally omits that member, producing a false-positive layout
        // assertion. The driver crate performs explicit checks for the
        // NetAdapterCx structures it uses.
        .layout_tests(false)
        .generate_comments(true)
        .clang_arg("-fms-extensions")
        .clang_arg("-fms-compatibility")
        .clang_arg(format!("--target={clang_target}"))
        .clang_arg("-D_KERNEL_MODE=1")
        .clang_arg(format!("-D{target_define}"))
        .clang_arg("-DNET_VERSION_MAJOR=2")
        .clang_arg("-DNET_VERSION_MINOR=5")
        .clang_arg("-DNET_MINIMUM_VERSION_REQUIRED=4")
        .clang_arg("-DNETADAPTER_VERSION_MAJOR=2")
        .clang_arg("-DNETADAPTER_VERSION_MINOR=5")
        .clang_arg("-DNETADAPTER_MINIMUM_VERSION_REQUIRED=4")
        .allowlist_type("NET.*")
        .allowlist_type("EVT_NET.*")
        .allowlist_type("EVT_PACKET.*")
        .allowlist_function("Net.*")
        .allowlist_var("Net.*")
        .allowlist_var("STATUS_.*")
        .allowlist_type("NDIS_.*")
        .allowlist_type("WDF.*")
        .allowlist_function("Wdf.*")
        .allowlist_type("GUID")
        .blocklist_function(".*FORCEINLINE.*");

    for include in &include_dirs {
        builder = builder.clang_arg(format!("-I{}", include.display()));
    }

    let bindings = builder
        .generate()
        .unwrap_or_else(|error| panic!("failed to generate pinned NetAdapterCx bindings: {error}"));

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    bindings
        .write_to_file(out_dir.join("netadaptercx_bindings.rs"))
        .expect("write generated NetAdapterCx bindings");
}

struct Roots {
    wdk: PathBuf,
    sdk: PathBuf,
    sdk_arch: PathBuf,
}

fn locate_roots(version: &str, arch: &str) -> Roots {
    let package_root = package_root();
    let arch_package = match arch {
        "x86_64" => "x64",
        "aarch64" => "ARM64",
        _ => unreachable!(),
    };

    let wdk = find_package(&package_root, &format!("Microsoft.Windows.WDK.{arch_package}"), version)
        .unwrap_or_else(|| panic!("missing pinned WDK package Microsoft.Windows.WDK.{arch_package} {version}; restore NuGet packages or set WINTAP_WDK_PACKAGE_ROOT"))
        .join("c");
    let sdk = find_package(&package_root, "Microsoft.Windows.SDK.CPP", version)
        .unwrap_or_else(|| panic!("missing pinned SDK package Microsoft.Windows.SDK.CPP {version}; restore NuGet packages or set WINTAP_WDK_PACKAGE_ROOT"))
        .join("c");
    let sdk_arch = find_package(&package_root, &format!("Microsoft.Windows.SDK.CPP.{arch_package}"), version)
        .unwrap_or_else(|| panic!("missing pinned SDK architecture package Microsoft.Windows.SDK.CPP.{arch_package} {version}; restore NuGet packages or set WINTAP_WDK_PACKAGE_ROOT"))
        .join("c");

    Roots { wdk, sdk, sdk_arch }
}

fn package_root() -> PathBuf {
    let manifest_dir =
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is required for package lookup");
    let repo_root = Path::new(&manifest_dir)
        .ancestors()
        .nth(2)
        .expect("crate is under repo/crates/name");
    repo_root.join("out").join("packages")
}

fn find_package(root: &Path, id: &str, version: &str) -> Option<PathBuf> {
    let flat = root.join(format!("{id}.{version}"));
    if flat.exists() {
        return Some(flat);
    }
    let nested = root.join(id.to_ascii_lowercase()).join(version);
    if nested.exists() {
        return Some(nested);
    }
    let nested_original_case = root.join(id).join(version);
    if nested_original_case.exists() {
        return Some(nested_original_case);
    }
    None
}
