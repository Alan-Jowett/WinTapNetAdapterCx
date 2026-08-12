cmake_minimum_required(VERSION 3.25)

set(CMAKE_VS_WINDOWS_TARGET_PLATFORM_VERSION
    "10.0.26100.0"
    CACHE STRING "Installed Windows SDK used by the CMake wrapper project")
set(WINTAP_WINDOWS_SDK_VERSION
    "10.0.28000.0"
    CACHE STRING "Windows SDK package version used by MSBuild")

project(WinTapNetAdapterCx LANGUAGES NONE)

if (NOT CMAKE_GENERATOR MATCHES "Visual Studio")
    message(FATAL_ERROR "Use a Visual Studio generator, such as \"Visual Studio 17 2022\" or newer.")
endif()

if (NOT CMAKE_GENERATOR_PLATFORM MATCHES "^(x64|ARM64)$")
    message(FATAL_ERROR "CMAKE_GENERATOR_PLATFORM must be x64 or ARM64.")
endif()

set(WINTAP_WDK_VERSION "10.0.28000.2526" CACHE STRING "WDK and SDK NuGet version")
set(WINTAP_CONFIGURATION "Debug" CACHE STRING "Visual Studio configuration")
set(WINTAP_API_VALIDATOR_ENABLE "false" CACHE STRING "Enable WDK API validation when the tool is installed")
set(WINTAP_RUST_DRIVER "AUTO" CACHE STRING "Rust driver build mode: AUTO, ON, or OFF")
set_property(CACHE WINTAP_RUST_DRIVER PROPERTY STRINGS AUTO ON OFF)
find_program(MSBUILD_EXECUTABLE NAMES MSBuild.exe msbuild)
if (NOT MSBUILD_EXECUTABLE)
    message(FATAL_ERROR "MSBuild.exe was not found. Install the Visual Studio driver workload or add MSBuild to PATH.")
endif()

set(WINTAP_WDK_PACKAGE "microsoft.windows.wdk.x64" CACHE STRING "Restored WDK NuGet package used for build tools")
set(WINTAP_WDK_TOOL_ROOT
    "$ENV{USERPROFILE}/.nuget/packages/${WINTAP_WDK_PACKAGE}/${WINTAP_WDK_VERSION}/c/bin/10.0.28000.0/x64"
    CACHE PATH "Directory containing the host WDK tools")
find_program(
    STAMPINF_EXECUTABLE
    NAMES stampinf.exe
    HINTS "${WINTAP_WDK_TOOL_ROOT}"
)
if (NOT STAMPINF_EXECUTABLE)
    message(FATAL_ERROR
        "stampinf.exe was not found. Restore WDK package ${WINTAP_WDK_PACKAGE} "
        "${WINTAP_WDK_VERSION}, then run scripts/prepare-wdk-tools.ps1 or set "
        "WINTAP_WDK_TOOL_ROOT.")
endif()
get_filename_component(WINTAP_WDK_TOOL_DIR "${STAMPINF_EXECUTABLE}" DIRECTORY)

set(WINTAP_DRIVER_PROJECT "${CMAKE_SOURCE_DIR}/driver/WinTapNetAdapterCx.vcxproj")
set(WINTAP_DRIVER_OUTPUT
    "${CMAKE_BINARY_DIR}/driver/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/WinTapNetAdapterCx.sys")
string(TOUPPER "${WINTAP_RUST_DRIVER}" WINTAP_RUST_DRIVER_MODE)
if (NOT WINTAP_RUST_DRIVER_MODE MATCHES "^(AUTO|ON|OFF)$")
    message(FATAL_ERROR "WINTAP_RUST_DRIVER must be AUTO, ON, or OFF.")
endif()
find_program(CARGO_EXECUTABLE NAMES cargo)
if (CMAKE_GENERATOR_PLATFORM STREQUAL "x64")
    set(WINTAP_RUST_ARCH "amd64")
elseif (CMAKE_GENERATOR_PLATFORM STREQUAL "ARM64")
    set(WINTAP_RUST_ARCH "arm64")
endif()
if (WINTAP_CONFIGURATION STREQUAL "Release")
    set(WINTAP_RUST_PROFILE "release")
    set(WINTAP_RUST_PROFILE_ARG --profile release)
else()
    set(WINTAP_RUST_PROFILE "debug")
    set(WINTAP_RUST_PROFILE_ARG)
endif()
set(WINTAP_RUST_PACKAGE_DIR
    "${CMAKE_SOURCE_DIR}/out/rust-target/${WINTAP_RUST_PROFILE}/wintap-netadaptercx-driver-package")
set(WINTAP_RUST_DRIVER_OUTPUT
    "${WINTAP_RUST_PACKAGE_DIR}/wintap_netadaptercx_driver.sys")

set(WINTAP_BUILD_RUST_DRIVER FALSE)
if (WINTAP_RUST_DRIVER_MODE STREQUAL "ON")
    if (NOT CARGO_EXECUTABLE)
        message(FATAL_ERROR "WINTAP_RUST_DRIVER=ON requires rustc/cargo, cargo-wdk, LLVM/libclang, and pinned WDK/SDK packages.")
    endif()
    set(WINTAP_BUILD_RUST_DRIVER TRUE)
elseif (WINTAP_RUST_DRIVER_MODE STREQUAL "AUTO" AND CARGO_EXECUTABLE)
    set(WINTAP_BUILD_RUST_DRIVER TRUE)
elseif (WINTAP_RUST_DRIVER_MODE STREQUAL "AUTO")
    message(STATUS "Rust toolchain not found; preserving the existing C driver package path. Set WINTAP_RUST_DRIVER=ON to fail explicitly.")
endif()

add_custom_target(wintap_driver ALL
    COMMAND "${CMAKE_COMMAND}" -E env
        "PATH=${WINTAP_WDK_TOOL_DIR}"
        "${MSBUILD_EXECUTABLE}" "${WINTAP_DRIVER_PROJECT}"
        /t:Restore,Build
        /p:Configuration=${WINTAP_CONFIGURATION}
        /p:Platform=${CMAKE_GENERATOR_PLATFORM}
        /p:WintapWdkVersion=${WINTAP_WDK_VERSION}
        /p:WindowsTargetPlatformVersion=${WINTAP_WINDOWS_SDK_VERSION}
        /p:TargetPlatformVersion=${WINTAP_WINDOWS_SDK_VERSION}
        /p:WindowsSDKInstalled=true
        /p:WindowsSDK_Desktop_Support=true
        /p:WindowsSDK_UAP_Support=true
        /p:ApiValidator_Enable=${WINTAP_API_VALIDATOR_ENABLE}
        /p:OutDir=${CMAKE_BINARY_DIR}/driver/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/
        /m
    BYPRODUCTS "${WINTAP_DRIVER_OUTPUT}"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}/driver"
    VERBATIM)

if (WINTAP_BUILD_RUST_DRIVER)
    add_custom_target(wintap_rust_driver
        COMMAND "${CMAKE_COMMAND}" -E env
            "WINTAP_WDK_VERSION=${WINTAP_WDK_VERSION}"
            "WINTAP_WDK_PACKAGE_ROOT=${CMAKE_SOURCE_DIR}/out/packages"
            "${CARGO_EXECUTABLE}" wdk build
            --target-arch ${WINTAP_RUST_ARCH}
            ${WINTAP_RUST_PROFILE_ARG}
            --sign-mode test
        WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
        VERBATIM)

    add_custom_target(wintap_rust_package
        COMMAND "${CMAKE_COMMAND}" -E make_directory
            "${CMAKE_BINARY_DIR}/package/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}"
        COMMAND "${CMAKE_COMMAND}" -E copy_if_different
            "${CMAKE_SOURCE_DIR}/driver/WinTapNetAdapterCx.inf"
            "${CMAKE_BINARY_DIR}/package/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/"
        COMMAND "${CMAKE_COMMAND}" -E copy_if_different
            "${WINTAP_RUST_DRIVER_OUTPUT}"
            "${CMAKE_BINARY_DIR}/package/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/WinTapNetAdapterCx.sys"
        DEPENDS wintap_rust_driver
        VERBATIM)
endif()

add_custom_target(wintap_package
    COMMAND "${CMAKE_COMMAND}" -E make_directory
        "${CMAKE_BINARY_DIR}/package/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}"
    COMMAND "${CMAKE_COMMAND}" -E copy_if_different
        "${CMAKE_SOURCE_DIR}/driver/WinTapNetAdapterCx.inf"
        "${CMAKE_BINARY_DIR}/package/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/"
    COMMAND "${CMAKE_COMMAND}" -E copy_if_different
        "${WINTAP_DRIVER_OUTPUT}"
        "${CMAKE_BINARY_DIR}/package/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/"
    DEPENDS wintap_driver
    VERBATIM)

message(STATUS "WinTapNetAdapterCx: ${CMAKE_GENERATOR_PLATFORM} ${WINTAP_CONFIGURATION}")
message(STATUS "WDK/SDK NuGet version: ${WINTAP_WDK_VERSION}")
message(STATUS "stampinf.exe: ${STAMPINF_EXECUTABLE}")
message(STATUS "Rust driver mode: ${WINTAP_RUST_DRIVER_MODE}")
