# SPDX-License-Identifier: MIT
# Copyright (c) 2026 WinTapNetAdapterCx contributors
Set-StrictMode -Version Latest

$script:ManagerPath = "\\.\Global\WinTapBusMgr"
$script:ManagerIoctl = 0x00222004
$script:ProtocolVersion = 1
$script:RequestLength = 40
$script:ResponseHeaderLength = 32
$script:RecordLength = 560
$script:LifecycleAbsent = 0
$script:LifecycleCreating = 1
$script:LifecycleActive = 2
$script:LifecycleRemoving = 3
$script:LifecycleFailed = 4
$script:StatusSuccess = 0
$script:StatusPending = 0x103

if (-not ("WinTapBusNative" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class WinTapBusNative {
    public const uint GenericRead = 0x80000000;
    public const uint GenericWrite = 0x40000000;
    public const uint OpenExisting = 3;
    public const uint ErrorInsufficientBuffer = 122;

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern IntPtr CreateFile(
        string name, uint access, uint share, IntPtr security, uint creation,
        uint flags, IntPtr template);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool DeviceIoControl(
        IntPtr device, uint controlCode, byte[] input, uint inputLength,
        [Out] byte[] output, uint outputLength, out uint returned,
        IntPtr overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr handle);
}
"@
}

if (-not ("WinTapBusSetupApi" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public sealed class WinTapBusDeviceInterface
{
    public string DeviceInstanceId { get; set; }
    public string DevicePath { get; set; }
}

public static class WinTapBusSetupApi
{
    private const uint DIGCF_PRESENT = 0x2;
    private const uint DIGCF_DEVICEINTERFACE = 0x10;
    private const int ERROR_NO_MORE_ITEMS = 259;
    private const int ERROR_INSUFFICIENT_BUFFER = 122;
    private static readonly IntPtr InvalidHandle = new IntPtr(-1);

    [StructLayout(LayoutKind.Sequential)]
    private struct SP_DEVICE_INTERFACE_DATA
    {
        public int cbSize;
        public Guid InterfaceClassGuid;
        public int Flags;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct SP_DEVINFO_DATA
    {
        public int cbSize;
        public Guid ClassGuid;
        public int DevInst;
        public IntPtr Reserved;
    }

    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern IntPtr SetupDiGetClassDevs(
        ref Guid classGuid, IntPtr enumerator, IntPtr hwndParent, uint flags);

    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiEnumDeviceInterfaces(
        IntPtr deviceInfoSet, IntPtr deviceInfoData, ref Guid interfaceClassGuid,
        uint memberIndex, ref SP_DEVICE_INTERFACE_DATA deviceInterfaceData);

    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern bool SetupDiGetDeviceInterfaceDetail(
        IntPtr deviceInfoSet, ref SP_DEVICE_INTERFACE_DATA deviceInterfaceData,
        IntPtr deviceInterfaceDetailData, uint deviceInterfaceDetailDataSize,
        out uint requiredSize, ref SP_DEVINFO_DATA deviceInfoData);

    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern bool SetupDiGetDeviceInstanceId(
        IntPtr deviceInfoSet, ref SP_DEVINFO_DATA deviceInfoData,
        StringBuilder deviceInstanceId, int deviceInstanceIdSize, out int requiredSize);

    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr deviceInfoSet);

    public static WinTapBusDeviceInterface[] EnumeratePresentInterfaces(Guid interfaceClassGuid)
    {
        IntPtr deviceInfoSet = SetupDiGetClassDevs(
            ref interfaceClassGuid, IntPtr.Zero, IntPtr.Zero, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if (deviceInfoSet == InvalidHandle)
        {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }

        var interfaces = new List<WinTapBusDeviceInterface>();
        try
        {
            for (uint index = 0; ; ++index)
            {
                var interfaceData = new SP_DEVICE_INTERFACE_DATA();
                interfaceData.cbSize = Marshal.SizeOf(typeof(SP_DEVICE_INTERFACE_DATA));
                if (!SetupDiEnumDeviceInterfaces(
                    deviceInfoSet, IntPtr.Zero, ref interfaceClassGuid, index, ref interfaceData))
                {
                    int error = Marshal.GetLastWin32Error();
                    if (error == ERROR_NO_MORE_ITEMS)
                    {
                        break;
                    }
                    throw new System.ComponentModel.Win32Exception(error);
                }

                uint requiredSize;
                var deviceInfoData = new SP_DEVINFO_DATA();
                deviceInfoData.cbSize = Marshal.SizeOf(typeof(SP_DEVINFO_DATA));
                SetupDiGetDeviceInterfaceDetail(
                    deviceInfoSet, ref interfaceData, IntPtr.Zero, 0, out requiredSize, ref deviceInfoData);
                if (Marshal.GetLastWin32Error() != ERROR_INSUFFICIENT_BUFFER || requiredSize == 0)
                {
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                }

                IntPtr detailData = Marshal.AllocHGlobal((int)requiredSize);
                try
                {
                    Marshal.WriteInt32(detailData, IntPtr.Size == 8 ? 8 : 6);
                    deviceInfoData = new SP_DEVINFO_DATA();
                    deviceInfoData.cbSize = Marshal.SizeOf(typeof(SP_DEVINFO_DATA));
                    if (!SetupDiGetDeviceInterfaceDetail(
                        deviceInfoSet, ref interfaceData, detailData, requiredSize,
                        out requiredSize, ref deviceInfoData))
                    {
                        throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                    }

                    string path = Marshal.PtrToStringUni(
                        IntPtr.Add(detailData, 4));
                    var instanceId = new StringBuilder(512);
                    int instanceIdLength;
                    if (!SetupDiGetDeviceInstanceId(
                        deviceInfoSet, ref deviceInfoData, instanceId, instanceId.Capacity, out instanceIdLength))
                    {
                        throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                    }
                    interfaces.Add(new WinTapBusDeviceInterface {
                        DeviceInstanceId = instanceId.ToString(),
                        DevicePath = path
                    });
                }
                finally
                {
                    Marshal.FreeHGlobal(detailData);
                }
            }
        }
        finally
        {
            SetupDiDestroyDeviceInfoList(deviceInfoSet);
        }
        return interfaces.ToArray();
    }
}
"@
}

function Assert-WinTapBusCondition([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Open-WinTapBusManager {
    $handle = [WinTapBusNative]::CreateFile(
        $script:ManagerPath,
        ([WinTapBusNative]::GenericRead -bor [WinTapBusNative]::GenericWrite),
        0, [IntPtr]::Zero, [WinTapBusNative]::OpenExisting, 0, [IntPtr]::Zero)
    if ($handle -eq [IntPtr]::new(-1)) {
        $error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "Open of administrator WinTap bus manager '$script:ManagerPath' failed with Win32 error $error."
    }
    return $handle
}

function Wait-WinTapBusManager([int]$TimeoutSeconds = 30) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        try {
            $handle = Open-WinTapBusManager
            [WinTapBusNative]::CloseHandle($handle) | Out-Null
            return
        } catch {
            Start-Sleep -Milliseconds 200
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "WinTap administrator manager '$script:ManagerPath' was not published before timeout."
}

function New-WinTapBusRequest([uint16]$Operation, [Guid]$Guid, [uint32]$Cursor = 0) {
    $request = [byte[]]::new($script:RequestLength)
    [BitConverter]::GetBytes([uint16]$script:ProtocolVersion).CopyTo($request, 0)
    [BitConverter]::GetBytes($Operation).CopyTo($request, 2)
    [BitConverter]::GetBytes([uint32]$script:RequestLength).CopyTo($request, 4)
    [BitConverter]::GetBytes([uint64][DateTime]::UtcNow.Ticks).CopyTo($request, 8)
    $Guid.ToByteArray().CopyTo($request, 16)
    [BitConverter]::GetBytes($Cursor).CopyTo($request, 32)
    return $request
}

function ConvertFrom-WinTapBusResponse([byte[]]$Buffer, [uint32]$Returned) {
    Assert-WinTapBusCondition ($Returned -ge $script:ResponseHeaderLength) `
        "WinTap manager returned only $Returned bytes; the response header is incomplete."
    $version = [BitConverter]::ToUInt16($Buffer, 0)
    $length = [BitConverter]::ToUInt32($Buffer, 4)
    Assert-WinTapBusCondition ($version -eq $script:ProtocolVersion) `
        "WinTap manager returned unsupported protocol version $version."
    Assert-WinTapBusCondition ($length -ge $script:ResponseHeaderLength -and $length -le $Returned) `
        "WinTap manager returned invalid response length $length."
    $count = [BitConverter]::ToUInt32($Buffer, 20)
    $nextCursor = [BitConverter]::ToUInt32($Buffer, 24)
    Assert-WinTapBusCondition ($script:ResponseHeaderLength + ($count * $script:RecordLength) -le $length) `
        "WinTap manager response record count exceeds the bounded response."
    $records = @()
    for ($index = 0; $index -lt $count; ++$index) {
        $offset = $script:ResponseHeaderLength + ($index * $script:RecordLength)
        $guidBytes = [byte[]]$Buffer[$offset..($offset + 15)]
        $interfaceLength = [BitConverter]::ToUInt16($Buffer, $offset + 32)
        Assert-WinTapBusCondition ($interfaceLength -lt 260) `
            "WinTap manager returned overlong interface identity for record $index."
        $interface = if ($interfaceLength -eq 0) {
            ""
        } else {
            [Text.Encoding]::Unicode.GetString($Buffer, $offset + 36, $interfaceLength * 2)
        }
        if ($interface.StartsWith('\??\')) {
            $interface = '\\?\' + $interface.Substring(4)
        }
        $records += [pscustomobject]@{
            Guid = [Guid]::new($guidBytes)
            Lifecycle = [BitConverter]::ToUInt32($Buffer, $offset + 16)
            TerminalStatus = [BitConverter]::ToInt32($Buffer, $offset + 20)
            RequestId = [BitConverter]::ToUInt64($Buffer, $offset + 24)
            InterfacePath = $interface
        }
    }
    return [pscustomobject]@{
        RequestId = [BitConverter]::ToUInt64($Buffer, 8)
        Status = [BitConverter]::ToInt32($Buffer, 16)
        NextCursor = $nextCursor
        Records = @($records)
    }
}

function Invoke-WinTapBusRequest(
    [ValidateSet("Create", "Remove", "Enumerate", "Query")]
    [string]$Operation,
    [Guid]$Guid = [Guid]::Empty,
    [uint32]$Cursor = 0
) {
    $opcode = @{
        Create = [uint16]1
        Remove = [uint16]2
        Enumerate = [uint16]3
        Query = [uint16]4
    }[$Operation]
    if ($Operation -ne "Enumerate" -and $Guid -eq [Guid]::Empty) {
        throw "$Operation requires a non-empty immutable child GUID."
    }
    $handle = Open-WinTapBusManager
    try {
        $input = New-WinTapBusRequest $opcode $Guid $Cursor
        $output = [byte[]]::new(65536)
        [uint32]$returned = 0
        $ok = [WinTapBusNative]::DeviceIoControl(
            $handle, $script:ManagerIoctl, $input, $input.Length, $output, $output.Length,
            [ref]$returned, [IntPtr]::Zero)
        if (-not $ok) {
            $error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            throw "WinTap manager $Operation failed with Win32 error $error."
        }
        return ConvertFrom-WinTapBusResponse $output $returned
    } finally {
        [WinTapBusNative]::CloseHandle($handle) | Out-Null
    }
}

function Test-WinTapBusInterfaceOpenable([string]$InterfacePath) {
    $handle = [WinTapBusNative]::CreateFile(
        $InterfacePath,
        ([WinTapBusNative]::GenericRead -bor [WinTapBusNative]::GenericWrite),
        0, [IntPtr]::Zero, [WinTapBusNative]::OpenExisting, 0, [IntPtr]::Zero)
    if ($handle -ne [IntPtr]::new(-1)) {
        [WinTapBusNative]::CloseHandle($handle) | Out-Null
        return $true
    }
    return $false
}

function Find-WinTapBusChildInterface([Guid]$Guid) {
    $interfaceClass = [Guid]'25D32EDF-7C8C-4F09-901F-650B232E864D'
    $instancePrefix = "WINTAPBUS\{$($Guid.ToString().ToUpperInvariant())}\"
    $matches = @(
        [WinTapBusSetupApi]::EnumeratePresentInterfaces($interfaceClass) |
            Where-Object {
                $_.DeviceInstanceId.StartsWith(
                    $instancePrefix, [StringComparison]::OrdinalIgnoreCase)
            }
    )
    if ($matches.Count -gt 1) {
        throw "More than one TAP interface matched dynamic child $Guid."
    }
    if ($matches.Count -eq 1) {
        return $matches[0].DevicePath
    }
    return $null
}

function Wait-WinTapBusChild(
    [Guid]$Guid,
    [ValidateSet("Active", "Absent")]
    [string]$Target,
    [int]$TimeoutSeconds = 30
) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $response = Invoke-WinTapBusRequest Query $Guid
        $record = @($response.Records | Where-Object Guid -eq $Guid)
        if ($record.Count -eq 1) {
            $record = $record[0]
            $interfacePath = Find-WinTapBusChildInterface $Guid
            if (($Target -eq "Active" -and
                    $record.Lifecycle -eq $script:LifecycleActive -and
                    $record.TerminalStatus -eq $script:StatusSuccess -and
                    -not [string]::IsNullOrWhiteSpace($interfacePath) -and
                    (Test-WinTapBusInterfaceOpenable $interfacePath)) -or
                ($Target -eq "Absent" -and
                    $record.Lifecycle -eq $script:LifecycleAbsent -and
                    [string]::IsNullOrWhiteSpace($interfacePath))) {
                $record.InterfacePath = $interfacePath
                return $record
            }
            if ($record.Lifecycle -eq $script:LifecycleFailed) {
                $status = [uint32]([int64]$record.TerminalStatus -band 0xffffffffL)
                throw "WinTap child $Guid reached Failed with NTSTATUS 0x$('{0:X8}' -f $status)."
            }
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "WinTap child $Guid did not reach $Target before timeout."
}

function New-WinTapBusChild([Guid]$Guid, [int]$TimeoutSeconds = 30) {
    Wait-WinTapBusManager $TimeoutSeconds
    $result = Invoke-WinTapBusRequest Create $Guid
    if ($result.Status -notin @($script:StatusSuccess, $script:StatusPending)) {
        throw "WinTap create for $Guid returned NTSTATUS 0x$('{0:X8}' -f [uint32]$result.Status)."
    }
    return Wait-WinTapBusChild $Guid Active $TimeoutSeconds
}

function Remove-WinTapBusChild([Guid]$Guid, [int]$TimeoutSeconds = 30) {
    $result = Invoke-WinTapBusRequest Remove $Guid
    if ($result.Status -notin @($script:StatusSuccess, $script:StatusPending)) {
        throw "WinTap remove for $Guid returned NTSTATUS 0x$('{0:X8}' -f [uint32]$result.Status)."
    }
    return Wait-WinTapBusChild $Guid Absent $TimeoutSeconds
}

function Get-WinTapBusChildren {
    $cursor = [uint32]0
    $children = @()
    do {
        $response = Invoke-WinTapBusRequest Enumerate ([Guid]::Empty) $cursor
        if ($response.Status -ne $script:StatusSuccess) {
            throw "WinTap enumerate returned NTSTATUS 0x$('{0:X8}' -f [uint32]$response.Status)."
        }
        $children += $response.Records
        $cursor = $response.NextCursor
    } while ($cursor -ne 0)
    return @($children)
}

Export-ModuleMember -Function @(
    "Get-WinTapBusChildren",
    "Invoke-WinTapBusRequest",
    "New-WinTapBusChild",
    "Remove-WinTapBusChild",
    "Wait-WinTapBusManager",
    "Wait-WinTapBusChild"
)
