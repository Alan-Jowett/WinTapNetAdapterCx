# SPDX-License-Identifier: MIT
# Copyright (c) 2026 WinTapNetAdapterCx contributors

@{
    ExplicitExclusions = @(
        @{
            Pattern = "CMakePresets.json"
            Reason = "Strict JSON does not permit comments."
        }
        @{
            Pattern = "LICENSE"
            Reason = "The complete license text is maintained in this file."
        }
        @{
            Pattern = "out/*"
            Reason = "Generated build, package, and diagnostic output."
        }
        @{
            Pattern = "target/*"
            Reason = "Generated Rust build output."
        }
    )
    VendoredDualLicensePatterns = @(
        "crates/wdk-sys/src/*",
        "crates/wdk-sys/build.rs"
    )
    BinaryExtensions = @(
        @{
            Extension = ".cat"
            Reason = "Catalog files are binary signatures."
        }
        @{
            Extension = ".dll"
            Reason = "Dynamic-link libraries are binary executables."
        }
        @{
            Extension = ".exe"
            Reason = "Executables are binary."
        }
        @{
            Extension = ".pdb"
            Reason = "Debug symbol files are binary."
        }
        @{
            Extension = ".sys"
            Reason = "Driver binaries are binary."
        }
    )
}
