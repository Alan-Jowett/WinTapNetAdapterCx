cmake_minimum_required(VERSION 3.25)

set(CMAKE_VS_WINDOWS_TARGET_PLATFORM_VERSION
    "10.0.26100.0"
    CACHE STRING "Installed Windows SDK used by the CMake wrapper project")

project(WinTapNetAdapterCx LANGUAGES NONE)

if (NOT CMAKE_GENERATOR MATCHES "Visual Studio")
    message(FATAL_ERROR "Use a Visual Studio generator, for example -G \"Visual Studio 18 2026\".")
endif()

if (NOT CMAKE_GENERATOR_PLATFORM MATCHES "^(x64|ARM64)$")
    message(FATAL_ERROR "CMAKE_GENERATOR_PLATFORM must be x64 or ARM64.")
endif()

find_program(MSBUILD_EXECUTABLE NAMES MSBuild.exe msbuild REQUIRED)

set(WINTAP_WDK_VERSION "10.0.28000.2526" CACHE STRING "WDK and SDK NuGet version")
set(WINTAP_CONFIGURATION "Debug" CACHE STRING "Visual Studio configuration")

set(WINTAP_DRIVER_PROJECT "${CMAKE_SOURCE_DIR}/driver/WinTapNetAdapterCx.vcxproj")
set(WINTAP_DRIVER_OUTPUT
    "${CMAKE_BINARY_DIR}/driver/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/WinTapNetAdapterCx.sys")

add_custom_target(wintap_driver ALL
    COMMAND "${MSBUILD_EXECUTABLE}" "${WINTAP_DRIVER_PROJECT}"
        /t:Restore,Build
        /p:Configuration=${WINTAP_CONFIGURATION}
        /p:Platform=${CMAKE_GENERATOR_PLATFORM}
        /p:WintapWdkVersion=${WINTAP_WDK_VERSION}
        /p:WindowsTargetPlatformVersion=10.0.28000.0
        /p:OutDir=${CMAKE_BINARY_DIR}/driver/${CMAKE_GENERATOR_PLATFORM}/${WINTAP_CONFIGURATION}/
        /m
    BYPRODUCTS "${WINTAP_DRIVER_OUTPUT}"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}/driver"
    VERBATIM)

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
