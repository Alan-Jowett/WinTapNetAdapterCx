use std::{
    env,
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

const WDK_NUGET_VERSION: &str = "10.0.28000.2526";
const SDK_VERSION: &str = "10.0.28000.0";

pub fn configure() -> PathBuf {
    let repo_root = repo_root();
    let architecture = package_architecture();
    let package_root = repo_root.join("out").join("packages");
    let wdk_root = package_root
        .join(format!(
            "Microsoft.Windows.WDK.{architecture}.{WDK_NUGET_VERSION}"
        ))
        .join("c");

    println!(
        "cargo:rerun-if-changed={}",
        repo_root
            .join("scripts")
            .join("restore-dependencies.ps1")
            .display()
    );
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");

    if !is_complete_wdk_root(&wdk_root, architecture) {
        let restore_lock = acquire_restore_lock(&package_root);
        if !is_complete_wdk_root(&wdk_root, architecture) {
            restore_packages(&repo_root, &package_root, architecture);
        }
        drop(restore_lock);
    }
    if !is_complete_wdk_root(&wdk_root, architecture) {
        panic!(
            "pinned WDK package Microsoft.Windows.WDK.{architecture} {WDK_NUGET_VERSION} \
             is incomplete at {}; run .\\scripts\\restore-dependencies.ps1 \
             -Architecture {architecture} -Version {WDK_NUGET_VERSION}",
            wdk_root.display()
        );
    }

    // Override any machine-global WDK selection before wdk-build probes it.
    unsafe {
        env::set_var("WDKContentRoot", &wdk_root);
    }
    wdk_root
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("wintap-wdk-bootstrap must remain under the repository crates directory")
        .to_path_buf()
}

fn package_architecture() -> &'static str {
    match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "ARM64",
        Ok(other) => {
            panic!("unsupported Rust target architecture '{other}'; expected x86_64 or aarch64")
        }
        Err(_) => panic!("CARGO_CFG_TARGET_ARCH is required to select the pinned WDK package"),
    }
}

fn is_complete_wdk_root(root: &Path, architecture: &str) -> bool {
    let library_architecture = match architecture {
        "x64" => "x64",
        "ARM64" => "arm64",
        _ => unreachable!("package_architecture only returns supported architectures"),
    };
    [
        root.join("Include")
            .join(SDK_VERSION)
            .join("km")
            .join("crt"),
        root.join("Include")
            .join(SDK_VERSION)
            .join("km")
            .join("ntddk.h"),
        root.join("Include").join("wdf").join("kmdf").join("1.33"),
        root.join("Lib")
            .join(SDK_VERSION)
            .join("km")
            .join(library_architecture),
    ]
    .iter()
    .all(|path| path.exists())
}

fn restore_packages(repo_root: &Path, package_root: &Path, architecture: &str) {
    let script = repo_root.join("scripts").join("restore-dependencies.ps1");
    let status = Command::new("powershell")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .args(["-Architecture", architecture, "-Version", WDK_NUGET_VERSION])
        .arg("-OutputDirectory")
        .arg(package_root)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "failed to start PowerShell to restore pinned WDK package \
                 Microsoft.Windows.WDK.{architecture} {WDK_NUGET_VERSION}: {error}"
            )
        });

    if !status.success() {
        panic!(
            "failed to restore pinned WDK package Microsoft.Windows.WDK.{architecture} \
             {WDK_NUGET_VERSION}; run .\\scripts\\restore-dependencies.ps1 \
             -Architecture {architecture} -Version {WDK_NUGET_VERSION}"
        );
    }
}

struct RestoreLock(PathBuf);

impl Drop for RestoreLock {
    fn drop(&mut self) {
        fs::remove_file(&self.0).unwrap_or_else(|error| {
            panic!(
                "failed to remove restore lock {}: {error}",
                self.0.display()
            )
        });
    }
}

fn acquire_restore_lock(package_root: &Path) -> RestoreLock {
    fs::create_dir_all(package_root).unwrap_or_else(|error| {
        panic!(
            "failed to create pinned WDK package root {}: {error}",
            package_root.display()
        )
    });
    let lock_path = package_root.join(".wintap-wdk-restore.lock");

    for _ in 0..600 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => return RestoreLock(lock_path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                thread::sleep(Duration::from_secs(1));
            }
            Err(error) => panic!(
                "failed to acquire pinned WDK restore lock {}: {error}",
                lock_path.display()
            ),
        }
    }

    panic!(
        "timed out waiting for pinned WDK package restore at {}; remove the stale lock and retry",
        lock_path.display()
    );
}
