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
    try {
        $stream = [System.IO.File]::Open(
            $InterfacePath,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None)
        $stream.Dispose()
        return $true
    } catch {
        return $false
    }
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
            if (($Target -eq "Active" -and
                    $record.Lifecycle -eq $script:LifecycleActive -and
                    $record.TerminalStatus -eq $script:StatusSuccess -and
                    -not [string]::IsNullOrWhiteSpace($record.InterfacePath) -and
                    (Test-WinTapBusInterfaceOpenable $record.InterfacePath)) -or
                ($Target -eq "Absent" -and $record.Lifecycle -eq $script:LifecycleAbsent)) {
                return $record
            }
            if ($record.Lifecycle -eq $script:LifecycleFailed) {
                throw "WinTap child $Guid reached Failed with NTSTATUS 0x$('{0:X8}' -f [uint32]$record.TerminalStatus)."
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
