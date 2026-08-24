// SPDX-License-Identifier: MIT
// Copyright (c) 2026 WinTapNetAdapterCx contributors
fn main() -> Result<(), wdk_build::ConfigError> {
    let _wdk_root = wintap_wdk_bootstrap::configure();
    wdk_build::configure_wdk_binary_build()
}
