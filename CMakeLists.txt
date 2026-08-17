cmake_minimum_required(VERSION 3.25)

project(WinTapNetAdapterCx LANGUAGES NONE)

if (NOT CMAKE_GENERATOR_PLATFORM MATCHES "^(x64|ARM64)$")
    message(FATAL_ERROR "CMAKE_GENERATOR_PLATFORM must be x64 or ARM64.")
endif()

set(WINTAP_WDK_VERSION "10.0.28000.2526" CACHE STRING "WDK and SDK NuGet version")
set(WINTAP_CONFIGURATION "Debug" CACHE STRING "Rust driver configuration")
find_program(CARGO_EXECUTABLE NAMES cargo REQUIRED)
find_program(CARGO_WDK_EXECUTABLE NAMES cargo-wdk REQUIRED)

if (CMAKE_GENERATOR_PLATFORM STREQUAL "x64")
    set(WINTAP_RUST_ARCH amd64)
    set(WINTAP_RUST_TARGET x86_64-pc-windows-msvc)
    set(WINTAP_WDK_PACKAGE x64)
elseif (CMAKE_GENERATOR_PLATFORM STREQUAL "ARM64")
    set(WINTAP_RUST_ARCH arm64)
    set(WINTAP_RUST_TARGET aarch64-pc-windows-msvc)
    set(WINTAP_WDK_PACKAGE ARM64)
endif()

if (WINTAP_CONFIGURATION STREQUAL "Release")
    set(WINTAP_RUST_PROFILE release)
    set(WINTAP_RUST_PROFILE_ARG --profile release)
else()
    set(WINTAP_RUST_PROFILE debug)
    set(WINTAP_RUST_PROFILE_ARG)
endif()

set(WINTAP_RUST_PACKAGE_DIR
    "${CMAKE_SOURCE_DIR}/out/rust-target/${WINTAP_RUST_TARGET}/${WINTAP_RUST_PROFILE}/wintap_netadaptercx_driver_package")
set(WINTAP_PACKAGE_DIR
    "${CMAKE_BINARY_DIR}/package/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}")

set(WINTAP_HOST_TOOL_RESTORE)
if (CMAKE_GENERATOR_PLATFORM STREQUAL "ARM64")
    list(APPEND WINTAP_HOST_TOOL_RESTORE
        COMMAND powershell -NoProfile -ExecutionPolicy Bypass
            -File "${CMAKE_SOURCE_DIR}/scripts/restore-dependencies.ps1"
            -Architecture x64
            -Version ${WINTAP_WDK_VERSION})
endif()

add_custom_target(wintap_rust_driver ALL
    COMMAND powershell -NoProfile -ExecutionPolicy Bypass
        -File "${CMAKE_SOURCE_DIR}/scripts/restore-dependencies.ps1"
        -Architecture ${CMAKE_GENERATOR_PLATFORM}
        -Version ${WINTAP_WDK_VERSION}
    ${WINTAP_HOST_TOOL_RESTORE}
    COMMAND powershell -NoProfile -ExecutionPolicy Bypass
        -File "${CMAKE_SOURCE_DIR}/scripts/build-rust-driver.ps1"
        -Architecture ${CMAKE_GENERATOR_PLATFORM}
        -Configuration ${WINTAP_CONFIGURATION}
        -Version ${WINTAP_WDK_VERSION}
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
    VERBATIM)

add_custom_target(wintap_package
    COMMAND "${CMAKE_COMMAND}" -E rm -rf "${WINTAP_PACKAGE_DIR}"
    COMMAND "${CMAKE_COMMAND}" -E make_directory "${WINTAP_PACKAGE_DIR}"
    COMMAND "${CMAKE_COMMAND}" -E copy_directory
        "${WINTAP_RUST_PACKAGE_DIR}"
        "${WINTAP_PACKAGE_DIR}"
    DEPENDS wintap_rust_driver
    VERBATIM)
