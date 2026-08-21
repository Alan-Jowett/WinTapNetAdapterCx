param(
    [string]$DevicePath = "\\.\WinTapRust",
    [switch]$Extended,
    [switch]$Integration,
    [switch]$InstallDriver,
    [switch]$RemoveDevice,
    [switch]$RequireTestSigning,
    [string]$PackageDirectory,
    [string]$DiagnosticsPath = ".\artifacts\wintap-harness",
    [int]$TimeoutSeconds = 15
)

$ErrorActionPreference = "Stop"

$driverService = "WinTapRust"
$driverInf = "wintap_netadaptercx_driver.inf"
$driverHardwareId = "ROOT\WinTapRust"
$driverDescription = "WinTapRust"

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this harness from an elevated administrator PowerShell session."
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class WinTapNative {
    public const uint GenericRead = 0x80000000;
    public const uint GenericWrite = 0x40000000;
    public const uint OpenExisting = 3;
    public const uint FileFlagOverlapped = 0x40000000;
    public const uint ErrorIoPending = 997;
    public const uint ErrorOperationAborted = 995;
    public const uint ErrorInvalidParameter = 87;
    public const uint ErrorTimeout = 1460;
    public const uint WaitObject0 = 0;
    public const uint WaitTimeout = 258;

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern IntPtr CreateFile(
        string name, uint access, uint share, IntPtr security, uint creation,
        uint flags, IntPtr template);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool ReadFile(
        IntPtr handle, IntPtr buffer, uint length, out uint transferred,
        IntPtr overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool WriteFile(
        IntPtr handle, IntPtr buffer, uint length, out uint transferred,
        IntPtr overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetOverlappedResult(
        IntPtr handle, IntPtr overlapped, out uint transferred, bool wait);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CancelIoEx(IntPtr handle, IntPtr overlapped);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr CreateEvent(
        IntPtr attributes, bool manualReset, bool initialState, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr handle);

    public static bool ReadFileWithError(
        IntPtr handle, IntPtr buffer, uint length, out uint transferred,
        IntPtr overlapped, out uint error) {
        bool result = ReadFile(handle, buffer, length, out transferred, overlapped);
        error = result ? 0u : unchecked((uint)Marshal.GetLastWin32Error());
        return result;
    }

    public static bool WriteFileWithError(
        IntPtr handle, IntPtr buffer, uint length, out uint transferred,
        IntPtr overlapped, out uint error) {
        bool result = WriteFile(handle, buffer, length, out transferred, overlapped);
        error = result ? 0u : unchecked((uint)Marshal.GetLastWin32Error());
        return result;
    }

    public static bool GetOverlappedResultWithError(
        IntPtr handle, IntPtr overlapped, out uint transferred, bool wait,
        out uint error) {
        bool result = GetOverlappedResult(handle, overlapped, out transferred, wait);
        error = result ? 0u : unchecked((uint)Marshal.GetLastWin32Error());
        return result;
    }

    public static bool CancelIoExWithError(
        IntPtr handle, IntPtr overlapped, out uint error) {
        bool result = CancelIoEx(handle, overlapped);
        error = result ? 0u : unchecked((uint)Marshal.GetLastWin32Error());
        return result;
    }
}
"@

function Get-Win32Error {
    [ComponentModel.Win32Exception]::new(
        [Runtime.InteropServices.Marshal]::GetLastWin32Error()).Message
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Ensure-DiagnosticsDirectory {
    if (-not (Test-Path -LiteralPath $DiagnosticsPath)) {
        New-Item -ItemType Directory -Path $DiagnosticsPath -Force | Out-Null
    }
    $script:DiagnosticsPath = (Resolve-Path -LiteralPath $DiagnosticsPath).Path
}

function Write-Diagnostic([string]$Message) {
    $line = "{0} {1}" -f ([DateTime]::UtcNow.ToString("o")), $Message
    Write-Host $line
    if ($script:DiagnosticsPath) {
        Add-Content -LiteralPath (Join-Path $script:DiagnosticsPath "progress.log") `
            -Value $line -Encoding utf8
    }
}

function Save-Diagnostics([string]$Name, [scriptblock]$Command) {
    try {
        & $Command 2>&1 | Out-File -FilePath (Join-Path $DiagnosticsPath $Name) `
            -Encoding utf8 -Force
    } catch {
        $_ | Out-File -FilePath (Join-Path $DiagnosticsPath "$Name.error") `
            -Encoding utf8 -Force
    }
}

function Save-Packet([string]$Name, [byte[]]$Bytes) {
    if ($DiagnosticsPath -and $null -ne $Bytes) {
        [IO.File]::WriteAllBytes((Join-Path $DiagnosticsPath $Name), $Bytes)
    }
}

function Save-EnvironmentDiagnostics {
    if (-not $DiagnosticsPath) {
        return
    }
    Save-Diagnostics "net-adapters.txt" {
        Get-NetAdapter -IncludeHidden | Format-List *
    }
    Save-Diagnostics "ip-addresses.txt" {
        Get-NetIPAddress -AddressFamily IPv4 | Format-List *
    }
    Save-Diagnostics "routes.txt" {
        Get-NetRoute -AddressFamily IPv4 | Format-List *
    }
    Save-Diagnostics "pnp-devices.txt" {
        Get-PnpDevice -Class Net | Format-List *
    }
    Save-Diagnostics "driver-service.txt" {
        Get-Service -Name $driverService -ErrorAction SilentlyContinue |
            Format-List *
    }
    Save-Diagnostics "driver-events.txt" {
        Get-WinEvent -FilterHashtable @{
            LogName = "System"
            ProviderName = "Service Control Manager"
            StartTime = (Get-Date).AddMinutes(-10)
        } -ErrorAction SilentlyContinue |
            Where-Object Message -Match $driverDescription |
            Format-List *
    }
}

function New-UnmanagedOverlapped {
    $memory = [Runtime.InteropServices.Marshal]::AllocHGlobal(32)
    [Runtime.InteropServices.Marshal]::Copy([byte[]]::new(32), 0, $memory, 32)
    return $memory
}

function Invoke-OverlappedIo(
    [IntPtr]$Handle,
    [byte[]]$Buffer,
    [bool]$Write,
    [int]$TimeoutMilliseconds
) {
    $bufferMemory = [Runtime.InteropServices.Marshal]::AllocHGlobal($Buffer.Length)
    $overlapped = New-UnmanagedOverlapped
    $event = [WinTapNative]::CreateEvent(
        [IntPtr]::Zero, $true, $false, $null)
    Assert-True ($event -ne [IntPtr]::Zero) "CreateEvent failed: $(Get-Win32Error)"
    try {
        # hEvent is the final pointer-sized field in OVERLAPPED.
        [Runtime.InteropServices.Marshal]::WriteIntPtr(
            $overlapped, 24, $event)
        if ($Write) {
            [Runtime.InteropServices.Marshal]::Copy(
                $Buffer, 0, $bufferMemory, $Buffer.Length)
        }

        [uint32]$transferred = 0
        [uint32]$nativeError = 0
        $result = if ($Write) {
            [WinTapNative]::WriteFileWithError(
                $Handle, $bufferMemory, [uint32]$Buffer.Length,
                [ref]$transferred, $overlapped, [ref]$nativeError)
        } else {
            [WinTapNative]::ReadFileWithError(
                $Handle, $bufferMemory, [uint32]$Buffer.Length,
                [ref]$transferred, $overlapped, [ref]$nativeError)
        }

        if (-not $result) {
            if ($nativeError -ne [WinTapNative]::ErrorIoPending) {
                throw "$(if ($Write) { 'Write' } else { 'Read' }) failed: $nativeError (transferred: $transferred)"
            }
            $wait = [WinTapNative]::WaitForSingleObject(
                $event, [uint32]$TimeoutMilliseconds)
            if ($wait -eq [WinTapNative]::WaitTimeout) {
                [uint32]$cancelError = 0
                [WinTapNative]::CancelIoExWithError(
                    $Handle, $overlapped, [ref]$cancelError) | Out-Null
                $cancelWait = [WinTapNative]::WaitForSingleObject($event, 5000)
                Assert-True ($cancelWait -eq [WinTapNative]::WaitObject0) `
                    "Timed-out I/O did not cancel before its buffer was released."
                [uint32]$completionError = 0
                $cancelResult = [WinTapNative]::GetOverlappedResultWithError(
                    $Handle, $overlapped, [ref]$transferred, $false,
                    [ref]$completionError)
                Assert-True (-not $cancelResult -and
                    $completionError -eq [WinTapNative]::ErrorOperationAborted) `
                    "Timed-out I/O returned an unexpected cancellation status."
                return $null
            }
            Assert-True ($wait -eq [WinTapNative]::WaitObject0) `
                "WaitForSingleObject failed: $wait"
            [uint32]$completionError = 0
            $result = [WinTapNative]::GetOverlappedResultWithError(
                $Handle, $overlapped, [ref]$transferred, $false,
                [ref]$completionError)
            if (-not $result) {
                throw "$(if ($Write) { 'Write' } else { 'Read' }) completion failed: $completionError (transferred: $transferred)"
            }
        }

        if (-not $Write) {
            $resultBuffer = [byte[]]::new([int]$transferred)
            [Runtime.InteropServices.Marshal]::Copy(
                $bufferMemory, $resultBuffer, 0, [int]$transferred)
            return $resultBuffer
        }
        return [int]$transferred
    } finally {
        [WinTapNative]::CloseHandle($event) | Out-Null
        [Runtime.InteropServices.Marshal]::FreeHGlobal($overlapped)
        [Runtime.InteropServices.Marshal]::FreeHGlobal($bufferMemory)
    }
}

function Read-Frame(
    [IntPtr]$Handle,
    [int]$TimeoutMilliseconds = 1000,
    [int]$MaximumLength = 1514
) {
    Invoke-OverlappedIo $Handle ([byte[]]::new($MaximumLength)) $false `
        $TimeoutMilliseconds
}

function Write-Frame([IntPtr]$Handle, [byte[]]$Frame, [int]$TimeoutMilliseconds = 5000) {
    $written = Invoke-OverlappedIo $Handle $Frame $true $TimeoutMilliseconds
    Assert-True ($null -ne $written -and $written -eq $Frame.Length) `
        "Frame write completed with $written bytes instead of $($Frame.Length)."
}

function Assert-InvalidFrameWrite([IntPtr]$Handle, [int]$Length) {
    $failure = $null
    $result = $null
    try {
        $result = Invoke-OverlappedIo $Handle ([byte[]]::new($Length)) $true 5000
    } catch {
        $failure = $_
    }
    Assert-True ($null -eq $result) `
        "Invalid $Length-byte frame completed successfully."
    Assert-True ($null -ne $failure) "Invalid $Length-byte frame did not complete with an error."
    Assert-True ($failure.Exception.Message -match "87") `
        "Invalid $Length-byte frame returned unexpected status: $($failure.Exception.Message)"
    Assert-True ($failure.Exception.Message -match "transferred: 0") `
        "Invalid $Length-byte frame transferred data: $($failure.Exception.Message)"
}

function Assert-ZeroLengthWrite([IntPtr]$Handle) {
    Invoke-OverlappedIo $Handle ([byte[]]::new(0)) $true 5000 | Out-Null
}

function Get-BytesHex([byte[]]$Bytes) {
    ($Bytes | ForEach-Object { $_.ToString("X2") }) -join ":"
}

function Get-UInt16BigEndian([byte[]]$Bytes, [int]$Offset) {
    ([uint16]$Bytes[$Offset] -shl 8) -bor $Bytes[$Offset + 1]
}

function Set-UInt16BigEndian([byte[]]$Bytes, [int]$Offset, [uint16]$Value) {
    $Bytes[$Offset] = [byte](($Value -shr 8) -band 0xff)
    $Bytes[$Offset + 1] = [byte]($Value -band 0xff)
}

function Get-IPv4Bytes([string]$Address) {
    [System.Net.IPAddress]::Parse($Address).GetAddressBytes()
}

function Test-BytesEqual([byte[]]$Left, [byte[]]$Right) {
    if ($null -eq $Left -or $null -eq $Right -or $Left.Length -ne $Right.Length) {
        return $false
    }
    for ($i = 0; $i -lt $Left.Length; ++$i) {
        if ($Left[$i] -ne $Right[$i]) {
            return $false
        }
    }
    return $true
}

function Get-InternetChecksum([byte[]]$Bytes, [int]$Offset, [int]$Length) {
    [uint32]$sum = 0
    for ($i = 0; $i -lt $Length; $i += 2) {
        $word = [uint16]$Bytes[$Offset + $i] -shl 8
        if ($i + 1 -lt $Length) {
            $word = $word -bor $Bytes[$Offset + $i + 1]
        }
        $sum += $word
        while ($sum -gt 0xffff) {
            $sum = ($sum -band 0xffff) + ($sum -shr 16)
        }
    }
    return [uint16](($sum -bxor 0xffff) -band 0xffff)
}

function Assert-Checksum([byte[]]$Bytes, [int]$Offset, [int]$Length, [string]$Name) {
    Assert-True ((Get-InternetChecksum $Bytes $Offset $Length) -eq 0) `
        "$Name checksum is invalid."
}

function Get-MacBytes([string]$MacAddress) {
    $parts = $MacAddress -split "[:-]"
    Assert-True ($parts.Count -eq 6) "Invalid adapter MAC address: $MacAddress"
    [byte[]]@($parts | ForEach-Object { [Convert]::ToByte($_, 16) })
}

function Assert-EthernetHeader(
    [byte[]]$Frame,
    [byte[]]$SourceMac,
    [byte[]]$DestinationMac,
    [uint16]$EtherType
) {
    Assert-True ($Frame.Length -ge 14) "Ethernet frame is truncated."
    Assert-True (Test-BytesEqual ([byte[]]$Frame[0..5]) $DestinationMac) `
        "Unexpected Ethernet destination: $(Get-BytesHex $Frame[0..5])"
    Assert-True (Test-BytesEqual ([byte[]]$Frame[6..11]) $SourceMac) `
        "Unexpected Ethernet source: $(Get-BytesHex $Frame[6..11])"
    Assert-True ((Get-UInt16BigEndian $Frame 12) -eq $EtherType) `
        "Unexpected EtherType: 0x$('{0:X4}' -f (Get-UInt16BigEndian $Frame 12))."
}

function New-ArpReply(
    [byte[]]$Request,
    [byte[]]$PeerMac,
    [byte[]]$LocalIp,
    [byte[]]$PeerIp
) {
    $reply = [byte[]]::new(42)
    [Array]::Copy($Request, 6, $reply, 0, 6)
    [Array]::Copy($PeerMac, 0, $reply, 6, 6)
    Set-UInt16BigEndian $reply 12 0x0806
    Set-UInt16BigEndian $reply 14 1
    Set-UInt16BigEndian $reply 16 0x0800
    $reply[18] = 6
    $reply[19] = 4
    Set-UInt16BigEndian $reply 20 2
    [Array]::Copy($PeerMac, 0, $reply, 22, 6)
    [Array]::Copy($PeerIp, 0, $reply, 28, 4)
    [Array]::Copy($Request, 22, $reply, 32, 6)
    [Array]::Copy($LocalIp, 0, $reply, 38, 4)
    return $reply
}

function Test-AndReplyArp(
    [byte[]]$Frame,
    [byte[]]$LocalMac,
    [byte[]]$PeerMac,
    [byte[]]$LocalIp,
    [byte[]]$PeerIp
) {
    if ($Frame.Length -lt 42) {
        throw "ARP frame is truncated."
    }
    Assert-EthernetHeader $Frame $Frame[6..11] ([byte[]](0xff, 0xff, 0xff, 0xff, 0xff, 0xff)) 0x0806
    Assert-True ((Get-UInt16BigEndian $Frame 14) -eq 1) "ARP hardware type is invalid."
    Assert-True ((Get-UInt16BigEndian $Frame 16) -eq 0x0800) "ARP protocol type is invalid."
    Assert-True ($Frame[18] -eq 6 -and $Frame[19] -eq 4) "ARP address lengths are invalid."
    Assert-True ((Get-UInt16BigEndian $Frame 20) -eq 1) "ARP operation is not a request."
    $senderMac = $Frame[22..27]
    $senderIp = $Frame[28..31]
    $targetIp = $Frame[38..41]
    Assert-True (Test-BytesEqual $senderMac $LocalMac) `
        "ARP sender MAC does not match the Ethernet source."
    if ((Test-BytesEqual $targetIp $PeerIp) -and
        (Test-BytesEqual $senderIp $LocalIp)) {
        Assert-True (Test-BytesEqual $Frame[32..37] ([byte[]](0, 0, 0, 0, 0, 0))) `
            "ARP request target MAC is not zero."
        return (New-ArpReply $Frame $PeerMac $LocalIp $PeerIp)
    }
    return $null
}

function New-IcmpEchoReply([byte[]]$Request, [byte[]]$LocalMac, [byte[]]$PeerMac) {
    $reply = [byte[]]$Request.Clone()
    [Array]::Copy($Request, 0, $reply, 6, 6)
    [Array]::Copy($LocalMac, 0, $reply, 0, 6)
    $ipOffset = 14
    $ipHeaderLength = ($Request[$ipOffset] -band 0x0f) * 4
    $icmpOffset = $ipOffset + $ipHeaderLength
    $icmpLength = (Get-UInt16BigEndian $Request ($ipOffset + 2)) - $ipHeaderLength
    $sourceIp = $Request[($ipOffset + 12)..($ipOffset + 15)]
    [Array]::Copy($Request, $ipOffset + 16, $reply, $ipOffset + 12, 4)
    [Array]::Copy($sourceIp, 0, $reply, $ipOffset + 16, 4)
    $reply[$icmpOffset] = 0
    Set-UInt16BigEndian $reply ($ipOffset + 10) 0
    Set-UInt16BigEndian $reply ($ipOffset + 10) `
        (Get-InternetChecksum $reply $ipOffset $ipHeaderLength)
    Set-UInt16BigEndian $reply ($icmpOffset + 2) 0
    Set-UInt16BigEndian $reply ($icmpOffset + 2) `
        (Get-InternetChecksum $reply $icmpOffset $icmpLength)
    return $reply
}

function Assert-IcmpEchoReply(
    [byte[]]$Reply,
    [hashtable]$RequestInfo,
    [byte[]]$LocalMac,
    [byte[]]$PeerMac,
    [byte[]]$LocalIp,
    [byte[]]$PeerIp
) {
    Assert-True ($Reply.Length -ge 42) "ICMP Echo Reply is truncated."
    Assert-True (Test-BytesEqual ([byte[]]$Reply[0..5]) $LocalMac) `
        "Echo Reply Ethernet destination is invalid."
    Assert-True (Test-BytesEqual ([byte[]]$Reply[6..11]) $PeerMac) `
        "Echo Reply Ethernet source is invalid."
    $ipOffset = 14
    Assert-True (($Reply[$ipOffset] -shr 4) -eq 4) `
        "Echo Reply IPv4 version is invalid."
    $headerLength = ($Reply[$ipOffset] -band 0x0f) * 4
    Assert-True ($headerLength -ge 20 -and
        $Reply.Length -ge $ipOffset + $headerLength) `
        "Echo Reply IPv4 header is invalid."
    $totalLength = Get-UInt16BigEndian $Reply ($ipOffset + 2)
    Assert-True ($totalLength -ge $headerLength + 8 -and
        $totalLength -le $Reply.Length) `
        "Echo Reply IPv4 total length is invalid."
    Assert-True ($Reply[$ipOffset + 9] -eq 1) `
        "Echo Reply IPv4 protocol is not ICMP."
    Assert-True ((Get-UInt16BigEndian $Reply ($ipOffset + 6) -band 0x3fff) -eq 0) `
        "Echo Reply is fragmented."
    Assert-True (Test-BytesEqual `
        ([byte[]]$Reply[($ipOffset + 12)..($ipOffset + 15)]) $PeerIp) `
        "Echo Reply IPv4 source is invalid."
    Assert-True (Test-BytesEqual `
        ([byte[]]$Reply[($ipOffset + 16)..($ipOffset + 19)]) $LocalIp) `
        "Echo Reply IPv4 destination is invalid."
    Assert-Checksum $Reply $ipOffset $headerLength "Echo Reply IPv4"
    $icmpOffset = $ipOffset + $headerLength
    Assert-True ($Reply[$icmpOffset] -eq 0 -and $Reply[$icmpOffset + 1] -eq 0) `
        "Echo Reply ICMP type/code is invalid."
    Assert-True ((Get-UInt16BigEndian $Reply ($icmpOffset + 4)) -eq
        $RequestInfo.Identifier -and
        (Get-UInt16BigEndian $Reply ($icmpOffset + 6)) -eq
        $RequestInfo.Sequence) "Echo Reply identifier or sequence changed."
    Assert-Checksum $Reply $icmpOffset ($totalLength - $headerLength) `
        "Echo Reply ICMP"
    $replyPayload = if ($totalLength - $headerLength -gt 8) {
        [byte[]]$Reply[($icmpOffset + 8)..($ipOffset + $totalLength - 1)]
    } else {
        [byte[]]::new(0)
    }
    Assert-True (Test-BytesEqual $replyPayload $RequestInfo.Payload) `
        "Echo Reply payload does not match the request."
}

function Get-ValidIcmpRequest(
    [byte[]]$Frame,
    [byte[]]$LocalMac,
    [byte[]]$PeerMac,
    [byte[]]$LocalIp,
    [byte[]]$PeerIp
) {
    if ($Frame.Length -lt 14) {
        throw "Ethernet frame is truncated."
    }
    if (-not (Test-BytesEqual ([byte[]]$Frame[0..5]) $PeerMac) -or
        -not (Test-BytesEqual ([byte[]]$Frame[6..11]) $LocalMac) -or
        (Get-UInt16BigEndian $Frame 12) -ne 0x0800) {
        return $null
    }
    Assert-True ($Frame.Length -ge 34) "IPv4 frame is truncated."
    $ipOffset = 14
    $version = $Frame[$ipOffset] -shr 4
    $headerLength = ($Frame[$ipOffset] -band 0x0f) * 4
    Assert-True ($version -eq 4 -and $headerLength -ge 20) "IPv4 header is invalid."
    Assert-True ($Frame.Length -ge $ipOffset + $headerLength) "IPv4 header is truncated."
    $totalLength = Get-UInt16BigEndian $Frame ($ipOffset + 2)
    Assert-True ($totalLength -ge $headerLength + 8 -and
        $totalLength -le $Frame.Length) "IPv4 total length is invalid."
    Assert-True ($Frame[$ipOffset + 8] -gt 0) "IPv4 TTL is invalid."
    Assert-True ((Get-UInt16BigEndian $Frame ($ipOffset + 6) -band 0x3fff) -eq 0) `
        "Fragmented IPv4 packet is not accepted."
    Assert-True (Test-BytesEqual `
        ([byte[]]$Frame[($ipOffset + 12)..($ipOffset + 15)]) $LocalIp) `
        "IPv4 source does not match the test address."
    Assert-True (Test-BytesEqual `
        ([byte[]]$Frame[($ipOffset + 16)..($ipOffset + 19)]) $PeerIp) `
        "IPv4 destination does not match the peer."
    Assert-Checksum $Frame $ipOffset $headerLength "IPv4"
    if ($Frame[$ipOffset + 9] -ne 1) {
        return $null
    }
    $icmpOffset = $ipOffset + $headerLength
    $icmpLength = $totalLength - $headerLength
    Assert-True ($Frame[$icmpOffset] -eq 8 -and $Frame[$icmpOffset + 1] -eq 0) `
        "ICMP packet is not an Echo Request."
    Assert-Checksum $Frame $icmpOffset $icmpLength "ICMP"
    return @{
        Frame = $Frame
        Identifier = Get-UInt16BigEndian $Frame ($icmpOffset + 4)
        Sequence = Get-UInt16BigEndian $Frame ($icmpOffset + 6)
        IcmpOffset = $icmpOffset
        IcmpLength = $icmpLength
        Payload = if ($icmpLength -gt 8) {
            [byte[]]$Frame[($icmpOffset + 8)..($icmpOffset + $icmpLength - 1)]
        } else {
            [byte[]]::new(0)
        }
    }
}

function Get-WinTapAdapter {
    $adapters = @(Get-NetAdapter -IncludeHidden -ErrorAction Stop | Where-Object {
        $_.InterfaceDescription -like "*$driverDescription*" -or
        $_.PnPDeviceID -like "$driverHardwareId*"
    })
    if ($adapters.Count -ne 1) {
        throw "Expected exactly one WinTap adapter; found $($adapters.Count)."
    }
    return $adapters[0]
}

function Wait-WinTapAdapter([int]$WaitSeconds = 20) {
    $deadline = [DateTime]::UtcNow.AddSeconds($WaitSeconds)
    do {
        try {
            return Get-WinTapAdapter
        } catch {
            Start-Sleep -Milliseconds 500
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "WinTap adapter was not discovered before the timeout."
}

function Test-WinTapAdapterIdentity($Adapter) {
    $hardwareIds = @(
        Get-PnpDeviceProperty -InstanceId $Adapter.PnPDeviceID `
            -KeyName "DEVPKEY_Device_HardwareIds" -ErrorAction Stop
    ).Data
    $service = (
        Get-PnpDeviceProperty -InstanceId $Adapter.PnPDeviceID `
            -KeyName "DEVPKEY_Device_Service" -ErrorAction Stop
    ).Data
    return (
        ($hardwareIds -contains $driverHardwareId) -and
        $service -eq $driverService
    )
}

function Invoke-DriverInstall {
    Assert-True (-not [string]::IsNullOrWhiteSpace($PackageDirectory)) `
        "-PackageDirectory is required with -InstallDriver."
    $package = (Resolve-Path -LiteralPath $PackageDirectory).Path
    $inf = Join-Path $package $driverInf
    Assert-True (Test-Path -LiteralPath $inf -PathType Leaf) `
        "Driver INF is missing: $inf"
    $installOutput = & pnputil.exe /add-driver $inf /install 2>&1
    $installOutput | Out-File (Join-Path $DiagnosticsPath "install-command.txt") `
        -Encoding utf8 -Force
    if ($LASTEXITCODE -ne 0) {
        throw "pnputil failed with exit code $LASTEXITCODE."
    }
    $service = Get-Service -Name $driverService -ErrorAction SilentlyContinue
    if ($service -and $service.Status -ne "Running") {
        Start-Service -Name $driverService -ErrorAction Stop
        $script:ServiceStartedByHarness = $true
    }
}

function Assert-TestSigning {
    $bcd = (& bcdedit.exe /enum "{current}" 2>&1 | Out-String)
    if ($bcd -notmatch "(?im)^\s*testsigning\s+Yes\s*$") {
        throw "Test signing is not enabled. Enable it with 'bcdedit /set testsigning on' and reboot; hosted runners commonly block this policy."
    }
}

function Add-TestAddress($Adapter) {
    $script:TestAddressCreated = $false
    $existing = @(Get-NetIPAddress -InterfaceIndex $Adapter.ifIndex `
        -AddressFamily IPv4 -ErrorAction SilentlyContinue | Where-Object {
            $_.IPAddress -eq "192.0.2.1"
        })
    if ($existing.Count -gt 0) {
        Assert-True ($existing | Where-Object PrefixLength -eq 30) `
            "192.0.2.1 already exists on the test adapter with the wrong prefix."
    } else {
        $collision = @(Get-NetIPAddress -AddressFamily IPv4 `
            -ErrorAction SilentlyContinue | Where-Object IPAddress -eq "192.0.2.1")
        Assert-True ($collision.Count -eq 0) "192.0.2.1 is already assigned elsewhere."
        New-NetIPAddress -InterfaceIndex $Adapter.ifIndex -IPAddress "192.0.2.1" `
            -PrefixLength 30 -ErrorAction Stop | Out-Null
        $script:TestAddressCreated = $true
    }
    $defaultRoutes = @(Get-NetRoute -InterfaceIndex $Adapter.ifIndex `
        -AddressFamily IPv4 -ErrorAction SilentlyContinue | Where-Object {
            $_.DestinationPrefix -eq "0.0.0.0/0"
        })
    Assert-True ($defaultRoutes.Count -eq 0) `
        "The test adapter has a default route; refusing to alter it."
}

function Remove-TestAddress($Adapter) {
    if ($script:TestAddressCreated -and $Adapter) {
        Get-NetIPAddress -InterfaceIndex $Adapter.ifIndex -AddressFamily IPv4 `
            -ErrorAction SilentlyContinue | Where-Object IPAddress -eq "192.0.2.1" |
            Remove-NetIPAddress -Confirm:$false -ErrorAction SilentlyContinue
    }
}

function Invoke-IntegrationHarness {
    Ensure-DiagnosticsDirectory
    Write-Diagnostic "integration: start"
    if ($RequireTestSigning) {
        Write-Diagnostic "integration: checking test signing"
        Assert-TestSigning
        Write-Diagnostic "integration: test signing enabled"
    }
    if ($InstallDriver) {
        Write-Diagnostic "integration: installing driver"
        try {
            Get-WinTapAdapter | Out-Null
            $script:AdapterExistedBeforeInstall = $true
        } catch {
            $script:AdapterExistedBeforeInstall = $false
        }
        Invoke-DriverInstall
        Write-Diagnostic "integration: driver install completed"
    }

    Write-Diagnostic "integration: waiting for adapter"
    $adapter = Wait-WinTapAdapter
    $script:IntegrationAdapter = $adapter
    Write-Diagnostic "integration: adapter discovered name=$($adapter.Name) status=$($adapter.Status)"
    $script:AdapterWasDisabled = ($adapter.Status -eq "Disabled")
    if ($adapter.Status -eq "Disabled") {
        Write-Diagnostic "integration: enabling adapter"
        Enable-NetAdapter -Name $adapter.Name -Confirm:$false -ErrorAction Stop
        $adapter = Wait-WinTapAdapter
        $script:IntegrationAdapter = $adapter
        Write-Diagnostic "integration: adapter enabled"
    }
    Write-Diagnostic "integration: configuring test address"
    Add-TestAddress $adapter
    Assert-True (Test-WinTapAdapterIdentity $adapter) `
        "The discovered adapter is not backed by the expected driver."
    Write-Host "Using adapter '$($adapter.Name)' ($($adapter.PnPDeviceID)), MAC $($adapter.MacAddress)."

    Write-Diagnostic "integration: opening TAP handle"
    $localMac = Get-MacBytes $adapter.MacAddress
    $peerMac = [byte[]](0x02, 0x57, 0x54, 0x41, 0x50, 0x02)
    $localIp = Get-IPv4Bytes "192.0.2.1"
    $peerIp = Get-IPv4Bytes "192.0.2.2"
    $handle = [WinTapNative]::CreateFile(
        $DevicePath,
        ([WinTapNative]::GenericRead -bor [WinTapNative]::GenericWrite),
        0, [IntPtr]::Zero, [WinTapNative]::OpenExisting,
        [WinTapNative]::FileFlagOverlapped, [IntPtr]::Zero)
    Assert-True ($handle -ne [IntPtr]::new(-1)) "Open failed: $(Get-Win32Error)"

    $ping = $null
    try {
        Write-Diagnostic "integration: starting ping"
        $ping = [System.Net.NetworkInformation.Ping]::new()
        $payload = [Text.Encoding]::ASCII.GetBytes("$driverDescription REQ-008")
        $pingTask = $ping.SendPingAsync(
            "192.0.2.2", $TimeoutSeconds * 1000, $payload)
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        $echoRequest = $null
        $packetIndex = 0
        while ([DateTime]::UtcNow -lt $deadline -and $null -eq $echoRequest) {
            $remaining = [Math]::Max(100, [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds)
            $frame = Read-Frame $handle ([Math]::Min(1000, $remaining))
            if ($null -eq $frame) {
                continue
            }
            $packetIndex++
            Save-Packet "frame-$packetIndex.bin" $frame
            $etherType = Get-UInt16BigEndian $frame 12
            if ($etherType -eq 0x0806) {
                $arpReply = Test-AndReplyArp `
                    $frame $localMac $peerMac $localIp $peerIp
                if ($null -ne $arpReply) {
                    Save-Packet "arp-reply.bin" $arpReply
                    Write-Frame $handle $arpReply
                    Write-Host "ARP request validated and reply written."
                }
                continue
            }
            if ($etherType -eq 0x0800) {
                $echoRequest = Get-ValidIcmpRequest `
                    $frame $localMac $peerMac $localIp $peerIp
                if ($null -ne $echoRequest) {
                    Assert-True (Test-BytesEqual $echoRequest.Payload $payload) `
                        "ICMP Echo Request payload does not match the generated ping."
                    $echoReply = New-IcmpEchoReply $frame $localMac $peerMac
                    Assert-IcmpEchoReply `
                        $echoReply $echoRequest $localMac $peerMac $localIp $peerIp
                    Save-Packet "icmp-echo-reply.bin" $echoReply
                    Write-Frame $handle $echoReply
                    Write-Host "ICMP Echo Request validated and Echo Reply written."
                }
            }
        }
        Assert-True ($null -ne $echoRequest) `
            "Did not receive a valid ICMP Echo Request within $TimeoutSeconds seconds."
        $remaining = [Math]::Max(1, [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds)
        Assert-True $pingTask.Wait($remaining) "Ping did not complete within the timeout."
        $pingReply = $pingTask.Result
        Assert-True ($pingReply.Status -eq `
            [System.Net.NetworkInformation.IPStatus]::Success) `
            "Windows networking stack did not receive a successful Echo Reply: $($pingReply.Status)."
        Assert-True (Test-BytesEqual $pingReply.Buffer $payload) `
            "Windows networking stack received an unexpected Echo Reply payload."
        Write-Host "Windows stack received the matching Echo Reply."
        Write-Diagnostic "integration: ping completed successfully"
    } finally {
        Write-Diagnostic "integration: closing TAP handle"
        if ($ping) {
            $ping.Dispose()
        }
        [uint32]$cancelError = 0
        [WinTapNative]::CancelIoExWithError(
            $handle, [IntPtr]::Zero, [ref]$cancelError) | Out-Null
        [WinTapNative]::CloseHandle($handle) | Out-Null
    }
}

Ensure-DiagnosticsDirectory
if ($Integration) {
    try {
        Invoke-IntegrationHarness
    } catch {
        Write-Diagnostic "integration: failed: $($_.Exception.Message)"
        Save-EnvironmentDiagnostics
        throw
    } finally {
        Write-Diagnostic "integration: cleanup start"
        try {
            if (-not $script:IntegrationAdapter) {
                $script:IntegrationAdapter = Get-WinTapAdapter
            }
            Remove-TestAddress $script:IntegrationAdapter
            if ($script:AdapterWasDisabled) {
                Disable-NetAdapter -Name $script:IntegrationAdapter.Name `
                    -Confirm:$false -ErrorAction SilentlyContinue
            }
            if ($script:ServiceStartedByHarness) {
                Stop-Service -Name $driverService `
                    -ErrorAction SilentlyContinue
            }
            $removeInstalledDevice = $RemoveDevice -or (
                $InstallDriver -and -not $script:AdapterExistedBeforeInstall)
            if ($removeInstalledDevice -and $script:IntegrationAdapter.PnPDeviceID) {
                & pnputil.exe /remove-device $script:IntegrationAdapter.PnPDeviceID 2>&1 |
                    Out-File (Join-Path $DiagnosticsPath "remove-device.txt") `
                    -Encoding utf8 -Force
            }
            Write-Diagnostic "integration: cleanup completed"
        } catch {
            $_ | Out-File (Join-Path $DiagnosticsPath "cleanup.error") `
                -Encoding utf8 -Force
            throw
        } finally {
            Save-EnvironmentDiagnostics
        }
    }
    Write-Host "WinTap REQ-008/REQ-009 integration harness passed."
    exit 0
}

$handle = [WinTapNative]::CreateFile(
    $DevicePath,
    ([WinTapNative]::GenericRead -bor [WinTapNative]::GenericWrite),
    0, [IntPtr]::Zero, [WinTapNative]::OpenExisting,
    [WinTapNative]::FileFlagOverlapped, [IntPtr]::Zero)
Assert-True ($handle -ne [IntPtr]::new(-1)) "Open failed: $(Get-Win32Error)"
try {
    $secondHandle = [WinTapNative]::CreateFile(
        $DevicePath,
        ([WinTapNative]::GenericRead -bor [WinTapNative]::GenericWrite),
        0, [IntPtr]::Zero, [WinTapNative]::OpenExisting,
        [WinTapNative]::FileFlagOverlapped, [IntPtr]::Zero)
    Assert-True ($secondHandle -eq [IntPtr]::new(-1)) `
        "Exclusive device open unexpectedly succeeded twice."

    Assert-ZeroLengthWrite $handle
    foreach ($invalidLength in @(1, 13, 1515)) {
        Assert-InvalidFrameWrite $handle $invalidLength
    }

    Write-Host "TC-040 cancellation check deferred: live adapter traffic prevents an empty read queue."

    $frame = [byte[]]::new(60)
    for ($i = 0; $i -lt $frame.Length; ++$i) {
        $frame[$i] = [byte]$i
    }
    Write-Frame $handle $frame

    if ($Extended) {
        Write-Host "Extended outstanding-read cancellation checks deferred with TC-040."
    }
} finally {
    [WinTapNative]::CloseHandle($handle) | Out-Null
}

Write-Host "WinTap overlapped I/O harness passed."
