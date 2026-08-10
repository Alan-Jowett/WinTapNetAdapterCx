param(
    [string]$DevicePath = "\\.\WinTapNetAdapterCx",
    [switch]$Extended
)

$ErrorActionPreference = "Stop"

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this harness from an elevated administrator PowerShell session."
}

Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class WinTapNative {
    public const uint GenericRead = 0x80000000;
    public const uint GenericWrite = 0x40000000;
    public const uint OpenExisting = 3;
    public const uint FileFlagOverlapped = 0x40000000;
    public const uint ErrorIoPending = 997;
    public const uint ErrorOperationAborted = 995;
    public const uint ErrorInvalidParameter = 87;
    public const uint WaitObject0 = 0;

    [StructLayout(LayoutKind.Sequential)]
    public struct OVERLAPPED {
        public IntPtr Internal;
        public IntPtr InternalHigh;
        public uint Offset;
        public uint OffsetHigh;
        public IntPtr hEvent;
    }

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern IntPtr CreateFile(
        string name,
        uint access,
        uint share,
        IntPtr security,
        uint creation,
        uint flags,
        IntPtr template);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool ReadFile(
        IntPtr handle,
        byte[] buffer,
        uint length,
        out uint transferred,
        ref OVERLAPPED overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool WriteFile(
        IntPtr handle,
        byte[] buffer,
        uint length,
        out uint transferred,
        ref OVERLAPPED overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetOverlappedResult(
        IntPtr handle,
        ref OVERLAPPED overlapped,
        out uint transferred,
        bool wait);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CancelIoEx(IntPtr handle, IntPtr overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr CreateEvent(
        IntPtr attributes,
        bool manualReset,
        bool initialState,
        string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr handle);
}
"@

function Get-Win32Error {
    [ComponentModel.Win32Exception]::new([Runtime.InteropServices.Marshal]::GetLastWin32Error()).Message
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

$handle = [WinTapNative]::CreateFile(
    $DevicePath,
    ([WinTapNative]::GenericRead -bor [WinTapNative]::GenericWrite),
    0,
    [IntPtr]::Zero,
    [WinTapNative]::OpenExisting,
    [WinTapNative]::FileFlagOverlapped,
    [IntPtr]::Zero)

Assert-True ($handle -ne [IntPtr]::new(-1)) "Open failed: $(Get-Win32Error)"
try {
    $secondHandle = [WinTapNative]::CreateFile(
        $DevicePath,
        ([WinTapNative]::GenericRead -bor [WinTapNative]::GenericWrite),
        0,
        [IntPtr]::Zero,
        [WinTapNative]::OpenExisting,
        [WinTapNative]::FileFlagOverlapped,
        [IntPtr]::Zero)
    Assert-True ($secondHandle -eq [IntPtr]::new(-1)) `
        "Exclusive device open unexpectedly succeeded twice."

    $invalid = [byte[]]::new(13)
    $invalidOverlapped = [WinTapNative+OVERLAPPED]::new()
    $invalidEvent = [WinTapNative]::CreateEvent(
        [IntPtr]::Zero, $true, $false, $null)
    $invalidOverlapped.hEvent = $invalidEvent
    $invalidTransferred = 0
    $invalidResult = [WinTapNative]::WriteFile(
        $handle, $invalid, $invalid.Length, [ref]$invalidTransferred,
        [ref]$invalidOverlapped)
    if (-not $invalidResult) {
        $invalidError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        if ($invalidError -eq [WinTapNative]::ErrorIoPending) {
            [WinTapNative]::WaitForSingleObject($invalidEvent, 5000) | Out-Null
            $invalidTransferred = 0
            $invalidResult = [WinTapNative]::GetOverlappedResult(
                $handle, [ref]$invalidOverlapped, [ref]$invalidTransferred, $false)
            $invalidError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        }
        Assert-True (-not $invalidResult -and
            $invalidError -eq [WinTapNative]::ErrorInvalidParameter) `
            "Undersized frame returned unexpected status: $invalidError"
    } else {
        throw "Undersized frame was unexpectedly accepted."
    }
    [WinTapNative]::CloseHandle($invalidEvent) | Out-Null

    $readBuffer = [byte[]]::new(1514)
    $readOverlapped = [WinTapNative+OVERLAPPED]::new()
    $readEvent = [WinTapNative]::CreateEvent(
        [IntPtr]::Zero, $true, $false, $null)
    $readOverlapped.hEvent = $readEvent
    $readTransferred = 0
    $readResult = [WinTapNative]::ReadFile(
        $handle, $readBuffer, $readBuffer.Length, [ref]$readTransferred,
        [ref]$readOverlapped)
    if (-not $readResult) {
        $readError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        Assert-True ($readError -eq [WinTapNative]::ErrorIoPending) `
            "Read did not pend as expected: $readError"
        [WinTapNative]::CancelIoEx($handle, [IntPtr]::Zero) | Out-Null
        [WinTapNative]::WaitForSingleObject($readEvent, 5000) | Out-Null
        $cancelTransferred = 0
        $cancelResult = [WinTapNative]::GetOverlappedResult(
            $handle, [ref]$readOverlapped, [ref]$cancelTransferred, $false)
        Assert-True (-not $cancelResult) `
            "Cancelled read unexpectedly completed successfully."
        $cancelError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        Assert-True ($cancelError -eq [WinTapNative]::ErrorOperationAborted) `
            "Unexpected cancellation status: $cancelError"
    } else {
        throw "Read completed synchronously; cancellation coverage was not exercised."
    }
    [WinTapNative]::CloseHandle($readEvent) | Out-Null

    $frame = [byte[]]::new(60)
    for ($i = 0; $i -lt $frame.Length; ++$i) {
        $frame[$i] = [byte]$i
    }
    $writeOverlapped = [WinTapNative+OVERLAPPED]::new()
    $writeEvent = [WinTapNative]::CreateEvent(
        [IntPtr]::Zero, $true, $false, $null)
    $writeOverlapped.hEvent = $writeEvent
    $writeTransferred = 0
    $writeResult = [WinTapNative]::WriteFile(
        $handle, $frame, $frame.Length, [ref]$writeTransferred,
        [ref]$writeOverlapped)
    if (-not $writeResult) {
        $writeError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        Assert-True ($writeError -eq [WinTapNative]::ErrorIoPending) `
            "Valid write failed unexpectedly: $writeError"
        [WinTapNative]::WaitForSingleObject($writeEvent, 5000) | Out-Null
        $writeResult = [WinTapNative]::GetOverlappedResult(
            $handle, [ref]$writeOverlapped, [ref]$writeTransferred, $false)
    }
    Assert-True $writeResult "Valid write did not complete: $(Get-Win32Error)"
    Assert-True ($writeTransferred -eq $frame.Length) `
        "Valid write completed with $writeTransferred bytes."
    [WinTapNative]::CloseHandle($writeEvent) | Out-Null

    if ($Extended) {
        $extendedBuffer0 = [byte[]]::new(1514)
        $extendedBuffer1 = [byte[]]::new(1514)
        $extendedEvent0 = [WinTapNative]::CreateEvent(
            [IntPtr]::Zero, $true, $false, $null)
        $extendedEvent1 = [WinTapNative]::CreateEvent(
            [IntPtr]::Zero, $true, $false, $null)
        $extendedOverlapped0 = [WinTapNative+OVERLAPPED]::new()
        $extendedOverlapped1 = [WinTapNative+OVERLAPPED]::new()
        $extendedOverlapped0.hEvent = $extendedEvent0
        $extendedOverlapped1.hEvent = $extendedEvent1
        $extendedTransferred0 = 0
        $extendedTransferred1 = 0
        $extendedPending0 = $false
        $extendedPending1 = $false

        $extendedResult0 = [WinTapNative]::ReadFile(
            $handle, $extendedBuffer0, $extendedBuffer0.Length,
            [ref]$extendedTransferred0, [ref]$extendedOverlapped0)
        if (-not $extendedResult0) {
            $extendedError0 = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            Assert-True ($extendedError0 -eq [WinTapNative]::ErrorIoPending) `
                "Extended read 0 failed unexpectedly: $extendedError0"
            $extendedPending0 = $true
        }

        $extendedResult1 = [WinTapNative]::ReadFile(
            $handle, $extendedBuffer1, $extendedBuffer1.Length,
            [ref]$extendedTransferred1, [ref]$extendedOverlapped1)
        if (-not $extendedResult1) {
            $extendedError1 = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            Assert-True ($extendedError1 -eq [WinTapNative]::ErrorIoPending) `
                "Extended read 1 failed unexpectedly: $extendedError1"
            $extendedPending1 = $true
        }

        [WinTapNative]::CancelIoEx($handle, [IntPtr]::Zero) | Out-Null
        if ($extendedPending0) {
            [WinTapNative]::WaitForSingleObject($extendedEvent0, 5000) | Out-Null
            $extendedResult0 = [WinTapNative]::GetOverlappedResult(
                $handle, [ref]$extendedOverlapped0,
                [ref]$extendedTransferred0, $false)
            Assert-True (-not $extendedResult0) "Extended read 0 was not cancelled."
            Assert-True (
                [Runtime.InteropServices.Marshal]::GetLastWin32Error() `
                    -eq [WinTapNative]::ErrorOperationAborted) `
                "Extended read 0 returned an unexpected cancellation status."
        }
        if ($extendedPending1) {
            [WinTapNative]::WaitForSingleObject($extendedEvent1, 5000) | Out-Null
            $extendedResult1 = [WinTapNative]::GetOverlappedResult(
                $handle, [ref]$extendedOverlapped1,
                [ref]$extendedTransferred1, $false)
            Assert-True (-not $extendedResult1) "Extended read 1 was not cancelled."
            Assert-True (
                [Runtime.InteropServices.Marshal]::GetLastWin32Error() `
                    -eq [WinTapNative]::ErrorOperationAborted) `
                "Extended read 1 returned an unexpected cancellation status."
        }
        [WinTapNative]::CloseHandle($extendedEvent0) | Out-Null
        [WinTapNative]::CloseHandle($extendedEvent1) | Out-Null
        Write-Host "Extended outstanding-read cancellation checks passed."
    }
}
finally {
    [WinTapNative]::CloseHandle($handle) | Out-Null
}

Write-Host "WinTap overlapped I/O harness passed."
