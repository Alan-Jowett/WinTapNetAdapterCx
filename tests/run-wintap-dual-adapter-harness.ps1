<#
.SYNOPSIS
Runs the REQ-015/REQ-016 routed dual-adapter WinTap relay test.

.DESCRIPTION
This is intentionally separate from run-wintap-harness.ps1.  It provisions
two disposable root devices, owns their independently exclusive TAP handles,
and removes only state recorded as created by this invocation.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$PackageDirectory,

    [ValidateSet("x64", "ARM64")]
    [string]$Architecture = "x64",

    [string]$WdkVersion = "10.0.28000.2526",

    [string]$DevConPath,

    [string]$DiagnosticsPath = ".\artifacts\wintap-dual-adapter-harness",

    [ValidateRange(5, 300)]
    [int]$TimeoutSeconds = 30,

    [ValidateRange(1, 4096)]
    [int]$RelayIterations = 257
)

$ErrorActionPreference = "Stop"
$driverInf = "wintap_netadaptercx_driver.inf"
$driverService = "WinTapRust"
$hardwareIdA = "ROOT\WinTapRust"
$hardwareIdB = "ROOT\WinTapRust2"
$controlPathA = "\\.\WinTapRust"
$controlPathB = "\\.\WinTapRust2"
$macAExpected = "02-57-54-41-50-01"
$macBExpected = "02-57-54-41-50-02"
$ipv4A = "198.51.100.1"
$ipv4B = "198.51.100.2"
$ipv6A = "2001:db8:515:1::1"
$ipv6B = "2001:db8:515:1::2"
$script:RunId = [Guid]::NewGuid().ToString("N")
$script:CreatedAddresses = @()
$script:CreatedNeighbors = @()
$script:CreatedRoutes = @()
$script:CreatedFirewallRules = @()
$script:CreatedPnpInstanceIds = @()
$script:CommandRecords = @()
$script:DriverPackagesBefore = @()
$script:DriverPackagesAfter = @()
$script:DriverPackageSnapshotTaken = $false
$script:AddedPublishedInf = $null
$script:PnpRemovalConfirmed = $false
$script:Relay = $null
$script:Handles = @{}
$script:PacketSequence = 0

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this harness from an elevated administrator PowerShell session."
}
if (-not [Environment]::Is64BitProcess) {
    throw "Run this harness from a 64-bit PowerShell host."
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class WinTapDualNative {
    public const uint GenericRead = 0x80000000;
    public const uint GenericWrite = 0x40000000;
    public const uint OpenExisting = 3;
    public const uint FileFlagOverlapped = 0x40000000;
    public const uint ErrorIoPending = 997;
    public const uint ErrorOperationAborted = 995;
    public const uint ErrorNotFound = 1168;
    public const uint WaitObject0 = 0;
    public const uint WaitTimeout = 258;
    public const uint WaitFailed = 0xffffffff;

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
    public static extern uint WaitForMultipleObjects(
        uint count, IntPtr[] handles, bool waitAll, uint milliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr handle);

    public static IntPtr CreateFileWithError(
        string name, uint access, uint share, IntPtr security, uint creation,
        uint flags, IntPtr template, out uint error) {
        IntPtr result = CreateFile(name, access, share, security, creation, flags, template);
        error = result == new IntPtr(-1) ? unchecked((uint)Marshal.GetLastWin32Error()) : 0u;
        return result;
    }

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

    public static IntPtr CreateEventWithError(
        IntPtr attributes, bool manualReset, bool initialState, string name,
        out uint error) {
        IntPtr result = CreateEvent(attributes, manualReset, initialState, name);
        error = result == IntPtr.Zero ? unchecked((uint)Marshal.GetLastWin32Error()) : 0u;
        return result;
    }

    public static uint WaitForMultipleObjectsWithError(
        IntPtr[] handles, uint milliseconds, out uint error) {
        uint result = WaitForMultipleObjects(
            unchecked((uint)handles.Length), handles, false, milliseconds);
        error = result == WaitFailed ? unchecked((uint)Marshal.GetLastWin32Error()) : 0u;
        return result;
    }
}
"@

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

function Save-Diagnostics([string]$Name, [scriptblock]$Command) {
    try {
        & $Command 2>&1 | Out-File -LiteralPath (Join-Path $script:DiagnosticsPath $Name) `
            -Encoding utf8 -Force
    } catch {
        $_ | Out-File -LiteralPath (Join-Path $script:DiagnosticsPath "$Name.error") `
            -Encoding utf8 -Force
    }
}

function Save-Packet([string]$Direction, [byte[]]$Frame) {
    $script:PacketSequence++
    $name = "{0:D4}-{1}.bin" -f $script:PacketSequence, $Direction
    [IO.File]::WriteAllBytes((Join-Path $script:DiagnosticsPath $name), $Frame)
}

function Save-EnvironmentDiagnostics([string]$Prefix) {
    if (-not $script:DiagnosticsPath) {
        return
    }

    Save-Diagnostics "$Prefix-net-adapters.txt" { Get-NetAdapter -IncludeHidden | Format-List * }
    Save-Diagnostics "$Prefix-ip-addresses.txt" { Get-NetIPAddress | Format-List * }
    Save-Diagnostics "$Prefix-routes.txt" { Get-NetRoute | Format-List * }
    Save-Diagnostics "$Prefix-neighbors.txt" { Get-NetNeighbor | Format-List * }
    Save-Diagnostics "$Prefix-firewall.txt" {
        Get-NetFirewallRule -Name "WinTapDual-$script:RunId-*" -ErrorAction SilentlyContinue |
            Get-NetFirewallAddressFilter | Format-List *
    }
    Save-Diagnostics "$Prefix-pnp-devices.txt" { Get-PnpDevice -Class Net | Format-List * }
    Save-Diagnostics "$Prefix-driver-service.txt" {
        Get-Service -Name $driverService -ErrorAction SilentlyContinue | Format-List *
    }
    Save-Diagnostics "$Prefix-driver-events.txt" {
        Get-WinEvent -FilterHashtable @{
            LogName = "System"
            StartTime = (Get-Date).AddMinutes(-15)
        } -ErrorAction SilentlyContinue |
            Where-Object { $_.ProviderName -eq "Service Control Manager" -and $_.Message -match $driverService } |
            Format-List *
    }
    Save-Diagnostics "$Prefix-command-records.txt" { $script:CommandRecords | Format-List * }
}

function Assert-TestSigning {
    $bcd = (& bcdedit.exe /enum "{current}" 2>&1 | Out-String)
    $bcd | Out-File -LiteralPath (Join-Path $script:DiagnosticsPath "bcdedit.txt") `
        -Encoding utf8 -Force
    if ($bcd -notmatch "(?im)^\s*testsigning\s+Yes\s*$") {
        throw "Test signing is not enabled. Enable it with 'bcdedit /set testsigning on' and reboot."
    }
}

function Invoke-RecordedNative(
    [string]$Name,
    [string]$FilePath,
    [string[]]$Arguments
) {
    $output = & $FilePath @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    $output | Out-File -LiteralPath (Join-Path $script:DiagnosticsPath "$Name.txt") `
        -Encoding utf8 -Force
    $record = [pscustomobject]@{
        Name = $Name
        FilePath = $FilePath
        Arguments = $Arguments -join " "
        ExitCode = $exitCode
    }
    $script:CommandRecords += $record
    if ($exitCode -ne 0) {
        throw "$Name failed with exit code $exitCode. See $Name.txt in $script:DiagnosticsPath."
    }
    return ,$output
}

function Get-NormalizedMac([string]$MacAddress) {
    $value = ($MacAddress -replace "[^0-9A-Fa-f]", "").ToUpperInvariant()
    Assert-True ($value -match "^[0-9A-F]{12}$") "Invalid adapter MAC address: $MacAddress"
    return $value
}

function Get-MacBytes([string]$MacAddress) {
    $normalized = Get-NormalizedMac $MacAddress
    [byte[]]$bytes = [byte[]]::new(6)
    for ($i = 0; $i -lt 6; ++$i) {
        $bytes[$i] = [Convert]::ToByte($normalized.Substring($i * 2, 2), 16)
    }
    return ,$bytes
}

function Get-MacDash([string]$MacAddress) {
    $normalized = Get-NormalizedMac $MacAddress
    $octets = for ($i = 0; $i -lt $normalized.Length; $i += 2) {
        $normalized.Substring($i, 2)
    }
    return ($octets -join "-")
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

function Get-UInt16BigEndian([byte[]]$Bytes, [int]$Offset) {
    return ([uint16]$Bytes[$Offset] -shl 8) -bor [uint16]$Bytes[$Offset + 1]
}

function Set-UInt16BigEndian([byte[]]$Bytes, [int]$Offset, [uint16]$Value) {
    $Bytes[$Offset] = [byte](($Value -shr 8) -band 0xff)
    $Bytes[$Offset + 1] = [byte]($Value -band 0xff)
}

function Get-UInt32BigEndian([byte[]]$Bytes, [int]$Offset) {
    return ([uint32]$Bytes[$Offset] -shl 24) -bor
        ([uint32]$Bytes[$Offset + 1] -shl 16) -bor
        ([uint32]$Bytes[$Offset + 2] -shl 8) -bor
        [uint32]$Bytes[$Offset + 3]
}

function Get-InternetChecksum([byte[]]$Bytes, [int]$Offset, [int]$Length) {
    [uint64]$sum = 0
    for ($i = 0; $i -lt $Length; $i += 2) {
        [uint32]$word = [uint32]$Bytes[$Offset + $i] -shl 8
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

function Get-DriverStoreSnapshot([string]$Name) {
    Invoke-RecordedNative $Name "pnputil.exe" @("/enum-drivers") | Out-Null
    $drivers = @(Get-WindowsDriver -Online -All -ErrorAction Stop)
    $packages = @()
    foreach ($driver in $drivers) {
        $original = [string]$driver.OriginalFileName
        if ($original -ine $driverInf) {
            continue
        }
        $published = if ($driver.PSObject.Properties["PublishedName"]) {
            [string]$driver.PublishedName
        } elseif ($driver.PSObject.Properties["Driver"]) {
            [string]$driver.Driver
        } else {
            $null
        }
        if ([string]::IsNullOrWhiteSpace($published)) {
            throw "Get-WindowsDriver did not expose a published driver package name."
        }
        $packages += [pscustomobject]@{
            PublishedInf = $published.ToLowerInvariant()
            OriginalInf = $original
        }
    }
    return @($packages)
}

function Update-AddedDriverPackage([string]$SnapshotName) {
    $script:DriverPackagesAfter = @(Get-DriverStoreSnapshot $SnapshotName)
    $before = @($script:DriverPackagesBefore | ForEach-Object PublishedInf)
    $added = @(
        $script:DriverPackagesAfter | Where-Object {
            $before -notcontains $_.PublishedInf
        }
    )
    Assert-True ($added.Count -le 1) `
        "More than one new $driverInf driver-store package was detected."
    if ($added.Count -eq 1) {
        $script:AddedPublishedInf = $added[0].PublishedInf
    }
}

function Resolve-DevCon {
    if ($DevConPath) {
        $resolved = (Resolve-Path -LiteralPath $DevConPath -ErrorAction Stop).Path
        Assert-True ((Split-Path -Leaf $resolved) -ieq "devcon.exe") `
            "-DevConPath must name devcon.exe: $resolved"
        return $resolved
    }

    $packageRoot = if ($env:NUGET_PACKAGES) {
        $env:NUGET_PACKAGES
    } else {
        Join-Path $env:USERPROFILE ".nuget\packages"
    }
    $wdkPackage = if ($Architecture -eq "ARM64") {
        "microsoft.windows.wdk.arm64"
    } else {
        "microsoft.windows.wdk.x64"
    }
    $versionRoot = Join-Path (Join-Path $packageRoot $wdkPackage) $WdkVersion
    Assert-True (Test-Path -LiteralPath $versionRoot -PathType Container) `
        "The pinned WDK NuGet package is not restored: $versionRoot"

    $candidates = @(
        Get-ChildItem -LiteralPath $versionRoot -Recurse -File -Filter "devcon.exe" |
            Where-Object { $_.Directory.Name -ieq $Architecture } |
            Select-Object -ExpandProperty FullName
    )
    Assert-True ($candidates.Count -eq 1) `
        "Expected exactly one $Architecture devcon.exe in pinned WDK package $versionRoot; found $($candidates.Count)."
    return $candidates[0]
}

function Get-MatchingPnpDevices {
    $hardwareIds = @($hardwareIdA, $hardwareIdB)
    return @(
        # PnP assigns ROOT\NET instance IDs, so ownership must use the hardware-ID property.
        Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction Stop | Where-Object {
            $deviceHardwareIds = @($_.HardwareID | ForEach-Object { [string]$_ })
            -not [string]::IsNullOrWhiteSpace($_.PNPDeviceID) -and
            @($deviceHardwareIds | Where-Object { $hardwareIds -contains $_ }).Count -gt 0
        } | ForEach-Object {
            [pscustomobject]@{
                InstanceId = [string]$_.PNPDeviceID
                HardwareIds = @($_.HardwareID | ForEach-Object { [string]$_ })
            }
        }
    )
}

function Assert-CleanEnvironment {
    $existing = @(Get-MatchingPnpDevices)
    if ($existing.Count -ne 0) {
        $instances = ($existing | ForEach-Object InstanceId) -join ", "
        throw "Refusing to modify state because matching WinTap Net adapter(s) already exist: $instances"
    }
}

function Invoke-DevConInstall([string]$HardwareId, [string]$Name) {
    # The documented DevCon operation creates the root-enumerated device.
    $output = & $script:ResolvedDevCon install $script:InfPath $HardwareId 2>&1
    $exitCode = $LASTEXITCODE
    $output | Out-File -LiteralPath (Join-Path $script:DiagnosticsPath "$Name.txt") `
        -Encoding utf8 -Force
    $script:CommandRecords += [pscustomobject]@{
        Name = $Name
        FilePath = $script:ResolvedDevCon
        Arguments = "install `"$script:InfPath`" $HardwareId"
        ExitCode = $exitCode
    }
    if ($exitCode -ne 0) {
        throw "devcon install $script:InfPath $HardwareId failed with exit code $exitCode."
    }
}

function Wait-MatchingPnpDeviceCount([int]$ExpectedCount) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $devices = @(Get-MatchingPnpDevices)
        if ($devices.Count -eq $ExpectedCount) {
            return $devices
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Expected $ExpectedCount matching WinTap PnP device(s); found $($devices.Count) before timeout."
}

function Get-WinTapAdapters([string[]]$InstanceIds) {
    return @(
        Get-NetAdapter -IncludeHidden -ErrorAction Stop | Where-Object {
            $InstanceIds -contains $_.PnPDeviceID
        }
    )
}

function Wait-WinTapAdapters([string[]]$InstanceIds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $adapters = @(Get-WinTapAdapters $InstanceIds)
        if ($adapters.Count -eq 2 -and @($adapters | Where-Object Status -ne "Up").Count -eq 0) {
            return $adapters
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Expected exactly two enabled WinTap adapters before timeout; found $($adapters.Count)."
}

function Get-PnpPropertyData([string]$InstanceId, [string]$KeyName) {
    return (Get-PnpDeviceProperty -InstanceId $InstanceId -KeyName $KeyName -ErrorAction Stop).Data
}

function Assert-AdapterIdentity($Adapter, [string]$HardwareId, [string]$ExpectedMac) {
    $hardwareIds = @(Get-PnpPropertyData $Adapter.PnPDeviceID "DEVPKEY_Device_HardwareIds")
    $service = [string](Get-PnpPropertyData $Adapter.PnPDeviceID "DEVPKEY_Device_Service")
    Assert-True (@($hardwareIds | Where-Object { [string]$_ -ieq $HardwareId }).Count -eq 1) `
        "Adapter $($Adapter.PnPDeviceID) does not expose hardware ID $HardwareId."
    Assert-True ($service -eq $driverService) `
        "Adapter $($Adapter.PnPDeviceID) service is '$service', not '$driverService'."

    $expected = Get-NormalizedMac $ExpectedMac
    $current = Get-NormalizedMac $Adapter.MacAddress
    Assert-True ($current -eq $expected) `
        "Adapter $($Adapter.PnPDeviceID) current MAC $($Adapter.MacAddress) is not $ExpectedMac."
    $networkAdapter = @(
        Get-CimInstance -ClassName Win32_NetworkAdapter -ErrorAction Stop |
            Where-Object { $_.InterfaceIndex -eq $Adapter.ifIndex }
    )
    $permanent = @($networkAdapter | Where-Object { -not [string]::IsNullOrWhiteSpace($_.PermanentAddress) } |
        Select-Object -First 1).PermanentAddress
    if (-not [string]::IsNullOrWhiteSpace($permanent)) {
        Assert-True ((Get-NormalizedMac $permanent) -eq $expected) `
            "Adapter $($Adapter.PnPDeviceID) permanent MAC $permanent is not $ExpectedMac."
    }
}

function Map-Adapters([object[]]$Adapters) {
    Assert-True ($Adapters.Count -eq 2) "Expected exactly two adapters to map."
    $a = @($Adapters | Where-Object { (Get-NormalizedMac $_.MacAddress) -eq (Get-NormalizedMac $macAExpected) })
    $b = @($Adapters | Where-Object { (Get-NormalizedMac $_.MacAddress) -eq (Get-NormalizedMac $macBExpected) })
    Assert-True ($a.Count -eq 1 -and $b.Count -eq 1) `
        "MAC-to-control endpoint mapping is missing, duplicate, or ambiguous."
    Assert-AdapterIdentity $a[0] $hardwareIdA $macAExpected
    Assert-AdapterIdentity $b[0] $hardwareIdB $macBExpected
    return @{
        A = $a[0]
        B = $b[0]
    }
}

function Open-ControlHandle([string]$Path) {
    [uint32]$nativeError = 0
    $handle = [WinTapDualNative]::CreateFileWithError(
        $Path,
        ([WinTapDualNative]::GenericRead -bor [WinTapDualNative]::GenericWrite),
        0, [IntPtr]::Zero, [WinTapDualNative]::OpenExisting,
        [WinTapDualNative]::FileFlagOverlapped, [IntPtr]::Zero, [ref]$nativeError)
    Assert-True ($handle -ne [IntPtr]::new(-1)) `
        "Open of $Path failed with Win32 error $nativeError."
    return $handle
}

function Assert-ExclusiveHandle([string]$Path) {
    [uint32]$nativeError = 0
    $second = [WinTapDualNative]::CreateFileWithError(
        $Path,
        ([WinTapDualNative]::GenericRead -bor [WinTapDualNative]::GenericWrite),
        0, [IntPtr]::Zero, [WinTapDualNative]::OpenExisting,
        [WinTapDualNative]::FileFlagOverlapped, [IntPtr]::Zero, [ref]$nativeError)
    if ($second -ne [IntPtr]::new(-1)) {
        [WinTapDualNative]::CloseHandle($second) | Out-Null
        throw "A second exclusive open unexpectedly succeeded for $Path."
    }
    Assert-True ($nativeError -ne 0) "Second open of $Path failed without a Win32 error."
}

function Assert-NoAddressCollisions {
    $testAddresses = @($ipv4A, $ipv4B, $ipv6A, $ipv6B)
    $collisions = @(
        Get-NetIPAddress -ErrorAction Stop | Where-Object { $testAddresses -contains $_.IPAddress }
    )
    Assert-True ($collisions.Count -eq 0) (
        "Test address collision detected before configuration: " +
        (($collisions | ForEach-Object { "$($_.IPAddress) on ifIndex $($_.InterfaceIndex)" }) -join "; "))
}

function Add-TestAddress($Adapter, [string]$Address, [int]$PrefixLength) {
    New-NetIPAddress -InterfaceIndex $Adapter.ifIndex -IPAddress $Address `
        -PrefixLength $PrefixLength -PolicyStore ActiveStore -ErrorAction Stop | Out-Null
    $script:CreatedAddresses += [pscustomobject]@{
        InterfaceIndex = $Adapter.ifIndex
        Address = $Address
        AddressFamily = if ($Address -like "*:*") { "IPv6" } else { "IPv4" }
    }
}

function Wait-TestAddressReady($Adapter, [string]$Address) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $configured = @(
            Get-NetIPAddress -InterfaceIndex $Adapter.ifIndex -ErrorAction Stop |
                Where-Object { $_.IPAddress -eq $Address }
        )
        if ($configured.Count -eq 1) {
            $state = [string]$configured[0].AddressState
            if ($state -eq "Preferred") {
                return
            }
            if ($state -eq "Duplicate") {
                throw "Address $Address was marked duplicate on adapter '$($Adapter.Name)'."
            }
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Address $Address did not become preferred on adapter '$($Adapter.Name)' before timeout."
}

function Add-StaticNeighbor($Adapter, [string]$PeerAddress, [string]$PeerMac) {
    $existing = @(
        Get-NetNeighbor -InterfaceIndex $Adapter.ifIndex -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -eq $PeerAddress }
    )
    Assert-True ($existing.Count -eq 0) `
        "A pre-existing neighbor entry for $PeerAddress prevents safe test setup."
    New-NetNeighbor -InterfaceIndex $Adapter.ifIndex -IPAddress $PeerAddress `
        -LinkLayerAddress (Get-MacDash $PeerMac) -State Permanent -PolicyStore ActiveStore `
        -ErrorAction Stop | Out-Null
    $script:CreatedNeighbors += [pscustomobject]@{
        InterfaceIndex = $Adapter.ifIndex
        IPAddress = $PeerAddress
        AddressFamily = if ($PeerAddress -like "*:*") { "IPv6" } else { "IPv4" }
    }
}

function Add-OnLinkHostRoute($Adapter, [string]$DestinationPrefix, [string]$NextHop) {
    New-NetRoute -InterfaceIndex $Adapter.ifIndex -DestinationPrefix $DestinationPrefix `
        -NextHop $NextHop -RouteMetric 0 -PolicyStore ActiveStore -ErrorAction Stop | Out-Null
    $script:CreatedRoutes += [pscustomobject]@{
        InterfaceIndex = $Adapter.ifIndex
        DestinationPrefix = $DestinationPrefix
        RemoteAddress = $DestinationPrefix -replace "/\d+$", ""
        NextHop = $NextHop
        RouteMetric = 0
        AddressFamily = if ($DestinationPrefix -like "*:*") { "IPv6" } else { "IPv4" }
    }
}

function Add-TestFirewallRule(
    [string]$Suffix,
    [string]$Protocol,
    [string]$LocalAddress,
    [string]$RemoteAddress
) {
    $name = "WinTapDual-$script:RunId-$Suffix"
    New-NetFirewallRule -Name $name -DisplayName $name -Direction Inbound `
        -Action Allow -Enabled True -Protocol $Protocol -LocalAddress $LocalAddress `
        -RemoteAddress $RemoteAddress -Profile Any -PolicyStore ActiveStore `
        -ErrorAction Stop | Out-Null
    $script:CreatedFirewallRules += $name
}

function Assert-NoDefaultRoutes($Adapters) {
    foreach ($adapter in @($Adapters.A, $Adapters.B)) {
        $defaults = @(
            Get-NetRoute -InterfaceIndex $adapter.ifIndex -ErrorAction Stop | Where-Object {
                $_.DestinationPrefix -in @("0.0.0.0/0", "::/0")
            }
        )
        Assert-True ($defaults.Count -eq 0) `
            "The disposable adapter '$($adapter.Name)' has a default route; refusing to continue."
    }
}

function Assert-ExactRoute($Route) {
    $matching = @(
        Get-NetRoute -InterfaceIndex $Route.InterfaceIndex -DestinationPrefix $Route.DestinationPrefix `
            -ErrorAction Stop | Where-Object {
                $_.NextHop -eq $Route.NextHop -and $_.RouteMetric -eq $Route.RouteMetric
            }
    )
    Assert-True ($matching.Count -ge 1) `
        "The exact host route $($Route.DestinationPrefix) through ifIndex $($Route.InterfaceIndex) is missing."
    $findResults = @(Find-NetRoute -RemoteIPAddress $Route.RemoteAddress -ErrorAction Stop)
    $selected = @(
        $findResults | Where-Object {
            $null -ne $_.PSObject.Properties["DestinationPrefix"]
        }
    )
    $diagnosticName = "route-selection-$($Route.RemoteAddress -replace '[:.]', '_').txt"
    Save-Diagnostics $diagnosticName {
        "Expected interface: $($Route.InterfaceIndex)"
        "Expected prefix: $($Route.DestinationPrefix)"
        "Configured route(s):"
        $matching | Format-List *
        "Find-NetRoute result(s):"
        $findResults | Format-List *
        "Selected route result(s):"
        $selected | Format-List *
    }
    Assert-True ($selected.Count -eq 1 -and
        $selected[0].InterfaceIndex -eq $Route.InterfaceIndex -and
        $selected[0].DestinationPrefix -eq $Route.DestinationPrefix) `
        "The host route $($Route.DestinationPrefix) through ifIndex $($Route.InterfaceIndex) is not selected."
}

function Configure-Topology($Adapters) {
    Assert-NoAddressCollisions
    Assert-NoDefaultRoutes $Adapters
    Add-TestAddress $Adapters.A $ipv4A 30
    Add-TestAddress $Adapters.B $ipv4B 30
    Add-TestAddress $Adapters.A $ipv6A 64
    Add-TestAddress $Adapters.B $ipv6B 64
    Wait-TestAddressReady $Adapters.A $ipv4A
    Wait-TestAddressReady $Adapters.B $ipv4B
    Wait-TestAddressReady $Adapters.A $ipv6A
    Wait-TestAddressReady $Adapters.B $ipv6B

    Add-StaticNeighbor $Adapters.A $ipv4B $Adapters.B.MacAddress
    Add-StaticNeighbor $Adapters.B $ipv4A $Adapters.A.MacAddress
    Add-StaticNeighbor $Adapters.A $ipv6B $Adapters.B.MacAddress
    Add-StaticNeighbor $Adapters.B $ipv6A $Adapters.A.MacAddress

    Add-OnLinkHostRoute $Adapters.A "$ipv4B/32" "0.0.0.0"
    Add-OnLinkHostRoute $Adapters.B "$ipv4A/32" "0.0.0.0"
    Add-OnLinkHostRoute $Adapters.A "$ipv6B/128" "::"
    Add-OnLinkHostRoute $Adapters.B "$ipv6A/128" "::"
    foreach ($route in $script:CreatedRoutes) {
        Assert-ExactRoute $route
    }

    Add-TestFirewallRule "A-IPv4" "ICMPv4" $ipv4A $ipv4B
    Add-TestFirewallRule "B-IPv4" "ICMPv4" $ipv4B $ipv4A
    Add-TestFirewallRule "A-IPv6" "ICMPv6" $ipv6A $ipv6B
    Add-TestFirewallRule "B-IPv6" "ICMPv6" $ipv6B $ipv6A
    Assert-NoDefaultRoutes $Adapters
}

function New-UnmanagedOverlapped {
    $memory = [Runtime.InteropServices.Marshal]::AllocHGlobal(32)
    [Runtime.InteropServices.Marshal]::Copy([byte[]]::new(32), 0, $memory, 32)
    return $memory
}

function Start-IoOperation(
    [IntPtr]$Handle,
    [IntPtr]$Buffer,
    [int]$Length,
    [bool]$Write,
    [object]$SourceOperation,
    [string]$Description
) {
    Assert-True ($Length -ge 0 -and $Length -le 1514) "$Description has invalid I/O length $Length."
    $overlapped = New-UnmanagedOverlapped
    [uint32]$eventError = 0
    $event = [WinTapDualNative]::CreateEventWithError(
        [IntPtr]::Zero, $true, $false, $null, [ref]$eventError)
    if ($event -eq [IntPtr]::Zero) {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($overlapped)
        throw "CreateEvent for $Description failed with Win32 error $eventError."
    }
    [Runtime.InteropServices.Marshal]::WriteIntPtr($overlapped, 24, $event)
    [uint32]$transferred = 0
    [uint32]$nativeError = 0
    $success = if ($Write) {
        [WinTapDualNative]::WriteFileWithError(
            $Handle, $Buffer, [uint32]$Length, [ref]$transferred, $overlapped, [ref]$nativeError)
    } else {
        [WinTapDualNative]::ReadFileWithError(
            $Handle, $Buffer, [uint32]$Length, [ref]$transferred, $overlapped, [ref]$nativeError)
    }
    $operation = [pscustomobject]@{
        Handle = $Handle
        Buffer = $Buffer
        Length = $Length
        IsWrite = $Write
        Overlapped = $overlapped
        Event = $event
        Pending = (-not $success -and $nativeError -eq [WinTapDualNative]::ErrorIoPending)
        Terminal = $success
        CompletionObserved = $success
        Succeeded = $success
        Error = $nativeError
        Transferred = [uint32]$transferred
        SourceOperation = $SourceOperation
        Description = $Description
        Disposed = $false
        OwnsBuffer = $false
    }
    if (-not $success -and -not $operation.Pending) {
        [WinTapDualNative]::CloseHandle($event) | Out-Null
        [Runtime.InteropServices.Marshal]::FreeHGlobal($overlapped)
        throw "$Description did not start: Win32 error $nativeError (transferred: $transferred)."
    }
    return $operation
}

function Complete-IoOperation($Operation) {
    if (-not $Operation.Terminal) {
        [uint32]$transferred = 0
        [uint32]$nativeError = 0
        $success = [WinTapDualNative]::GetOverlappedResultWithError(
            $Operation.Handle, $Operation.Overlapped, [ref]$transferred, $false, [ref]$nativeError)
        $Operation.Pending = $false
        $Operation.Terminal = $true
        $Operation.CompletionObserved = $true
        $Operation.Succeeded = $success
        $Operation.Error = $nativeError
        $Operation.Transferred = $transferred
    }
    return @{
        Succeeded = [bool]$Operation.Succeeded
        Error = [uint32]$Operation.Error
        Transferred = [uint32]$Operation.Transferred
    }
}

function Assert-IoSuccess($Operation) {
    $result = Complete-IoOperation $Operation
    Assert-True $result.Succeeded `
        "$($Operation.Description) completed with Win32 error $($result.Error) (transferred: $($result.Transferred))."
    Assert-True ($result.Transferred -eq $Operation.Length) `
        "$($Operation.Description) transferred $($result.Transferred) bytes, expected $($Operation.Length)."
}

function Dispose-IoOperation($Operation) {
    if ($null -eq $Operation -or $Operation.Disposed) {
        return
    }
    Assert-True $Operation.CompletionObserved `
        "Refusing to release $($Operation.Description) before terminal I/O completion."
    [WinTapDualNative]::CloseHandle($Operation.Event) | Out-Null
    [Runtime.InteropServices.Marshal]::FreeHGlobal($Operation.Overlapped)
    $Operation.Disposed = $true
}

function Dispose-BufferedIoOperation($Operation) {
    if ($null -eq $Operation -or $Operation.Disposed) {
        return
    }
    Dispose-IoOperation $Operation
    if ($Operation.OwnsBuffer) {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($Operation.Buffer)
        $Operation.OwnsBuffer = $false
    }
}

function Copy-FrameFromRead($Operation) {
    Assert-True ($Operation.Transferred -ge 14 -and $Operation.Transferred -le 1514) `
        "$($Operation.Description) received out-of-range Ethernet length $($Operation.Transferred)."
    [byte[]]$frame = [byte[]]::new([int]$Operation.Transferred)
    [Runtime.InteropServices.Marshal]::Copy($Operation.Buffer, $frame, 0, $frame.Length)
    return ,$frame
}

function New-RelayRead($Relay, [string]$Direction) {
    $handle = if ($Direction -eq "AtoB") { $Relay.HandleA } else { $Relay.HandleB }
    $buffer = [Runtime.InteropServices.Marshal]::AllocHGlobal(1514)
    try {
        $read = Start-IoOperation $handle $buffer 1514 $false $null "$Direction source read"
        $read.OwnsBuffer = $true
        $read | Add-Member -NotePropertyName Direction -NotePropertyValue $Direction
        return $read
    } catch {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($buffer)
        throw
    }
}

function Assert-EthernetHeader(
    [byte[]]$Frame,
    [byte[]]$SourceMac,
    [byte[]]$DestinationMac,
    [uint16]$EtherType,
    [string]$Name
) {
    Assert-True ($Frame.Length -ge 14) "$Name Ethernet frame is truncated."
    Assert-True (Test-BytesEqual ([byte[]]$Frame[0..5]) $DestinationMac) `
        "$Name Ethernet destination MAC is invalid."
    Assert-True (Test-BytesEqual ([byte[]]$Frame[6..11]) $SourceMac) `
        "$Name Ethernet source MAC is invalid."
    Assert-True ((Get-UInt16BigEndian $Frame 12) -eq $EtherType) `
        "$Name EtherType is invalid."
}

function Assert-ArpFrame([byte[]]$Frame) {
    Assert-True ($Frame.Length -ge 42) "ARP Ethernet frame is truncated."
    Assert-True ((Get-UInt16BigEndian $Frame 12) -eq 0x0806) "ARP EtherType is invalid."
    Assert-True ((Get-UInt16BigEndian $Frame 14) -eq 1) "ARP hardware type is invalid."
    Assert-True ((Get-UInt16BigEndian $Frame 16) -eq 0x0800) "ARP protocol type is invalid."
    Assert-True ($Frame[18] -eq 6 -and $Frame[19] -eq 4) "ARP address lengths are invalid."
    $operation = Get-UInt16BigEndian $Frame 20
    Assert-True ($operation -in @(1, 2)) "ARP operation is invalid."
}

function Assert-IPv4FrameStructure([byte[]]$Frame) {
    $ipOffset = 14
    Assert-True ($Frame.Length -ge 34) "IPv4 Ethernet frame is truncated."
    $headerLength = ($Frame[$ipOffset] -band 0x0f) * 4
    Assert-True (($Frame[$ipOffset] -shr 4) -eq 4 -and $headerLength -ge 20) "IPv4 header is invalid."
    Assert-True ($Frame.Length -ge $ipOffset + $headerLength) "IPv4 header is truncated."
    $totalLength = Get-UInt16BigEndian $Frame ($ipOffset + 2)
    Assert-True ($totalLength -ge $headerLength -and $totalLength -le $Frame.Length - $ipOffset) `
        "IPv4 total length is invalid."
    Assert-Checksum $Frame $ipOffset $headerLength "IPv4"
}

function Assert-IPv6FrameStructure([byte[]]$Frame) {
    $ipOffset = 14
    Assert-True ($Frame.Length -ge $ipOffset + 40) "IPv6 Ethernet frame is truncated."
    Assert-True (($Frame[$ipOffset] -shr 4) -eq 6) "IPv6 version is invalid."
    $payloadLength = Get-UInt16BigEndian $Frame ($ipOffset + 4)
    Assert-True ($ipOffset + 40 + $payloadLength -le $Frame.Length) "IPv6 payload length is invalid."
}

function Get-IPv6PseudoHeader([byte[]]$Source, [byte[]]$Destination, [int]$PayloadLength) {
    [byte[]]$pseudo = [byte[]]::new(40)
    [Array]::Copy($Source, 0, $pseudo, 0, 16)
    [Array]::Copy($Destination, 0, $pseudo, 16, 16)
    $pseudo[32] = [byte](($PayloadLength -shr 24) -band 0xff)
    $pseudo[33] = [byte](($PayloadLength -shr 16) -band 0xff)
    $pseudo[34] = [byte](($PayloadLength -shr 8) -band 0xff)
    $pseudo[35] = [byte]($PayloadLength -band 0xff)
    $pseudo[39] = 58
    return ,$pseudo
}

function Test-IPv6UnspecifiedAddress([byte[]]$Address) {
    Assert-True ($Address.Length -eq 16) "IPv6 address has an invalid length."
    foreach ($octet in $Address) {
        if ($octet -ne 0) {
            return $false
        }
    }
    return $true
}

function Test-SolicitedNodeMulticastAddress([byte[]]$Address) {
    if ($Address.Length -ne 16 -or $Address[0] -ne 0xff -or $Address[1] -ne 0x02 -or
        $Address[11] -ne 0x01 -or $Address[12] -ne 0xff) {
        return $false
    }
    foreach ($offset in 2..10) {
        if ($Address[$offset] -ne 0) {
            return $false
        }
    }
    return $true
}

function Get-SolicitedNodeMulticastAddress([byte[]]$Target) {
    Assert-True ($Target.Length -eq 16) "IPv6 Neighbor Discovery target has an invalid length."
    [byte[]]$address = [byte[]]::new(16)
    $address[0] = 0xff
    $address[1] = 0x02
    $address[11] = 0x01
    $address[12] = 0xff
    [Array]::Copy($Target, 13, $address, 13, 3)
    return ,$address
}

function Assert-Icmpv6Checksum(
    [byte[]]$Frame,
    [int]$IcmpOffset,
    [int]$IcmpLength,
    [byte[]]$Source,
    [byte[]]$Destination
) {
    $pseudo = Get-IPv6PseudoHeader $Source $Destination $IcmpLength
    [byte[]]$checksumBuffer = [byte[]]::new(40 + $IcmpLength)
    [Array]::Copy($pseudo, 0, $checksumBuffer, 0, 40)
    [Array]::Copy($Frame, $IcmpOffset, $checksumBuffer, 40, $IcmpLength)
    Assert-Checksum $checksumBuffer 0 $checksumBuffer.Length "ICMPv6 pseudo-header"
}

function Assert-IPv6Icmpv6Structure([byte[]]$Frame) {
    $ipOffset = 14
    Assert-IPv6FrameStructure $Frame
    if ($Frame[$ipOffset + 6] -ne 58) {
        return
    }

    $payloadLength = Get-UInt16BigEndian $Frame ($ipOffset + 4)
    Assert-True ($payloadLength -ge 4) "ICMPv6 payload is truncated."
    $icmpOffset = $ipOffset + 40
    $payloadEnd = $icmpOffset + $payloadLength
    $source = [byte[]]$Frame[($ipOffset + 8)..($ipOffset + 23)]
    $destination = [byte[]]$Frame[($ipOffset + 24)..($ipOffset + 39)]
    Assert-Icmpv6Checksum $Frame $icmpOffset $payloadLength $source $destination

    $neighborDiscoveryType = $Frame[$icmpOffset]
    $fixedLength = switch ($neighborDiscoveryType) {
        133 { 8 }  # Router Solicitation
        134 { 16 } # Router Advertisement
        135 { 24 } # Neighbor Solicitation
        136 { 24 } # Neighbor Advertisement
        137 { 40 } # Redirect
        default { return }
    }
    Assert-True ($Frame[$icmpOffset + 1] -eq 0) "ICMPv6 neighbor-discovery code is invalid."
    Assert-True ($payloadLength -ge $fixedLength) "ICMPv6 neighbor-discovery message is truncated."
    Assert-True ($Frame[$ipOffset + 7] -eq 255) "ICMPv6 neighbor-discovery hop limit is invalid."

    $hasSourceLinkLayerAddress = $false
    for ($optionOffset = $icmpOffset + $fixedLength; $optionOffset -lt $payloadEnd) {
        Assert-True ($optionOffset + 2 -le $payloadEnd) "ICMPv6 neighbor-discovery option is truncated."
        $optionLength = [int]$Frame[$optionOffset + 1] * 8
        Assert-True ($optionLength -gt 0 -and $optionOffset + $optionLength -le $payloadEnd) `
            "ICMPv6 neighbor-discovery option length is invalid."
        if ($Frame[$optionOffset] -eq 1) {
            Assert-True ($optionLength -eq 8) `
                "ICMPv6 neighbor-discovery source link-layer option has an invalid length."
            $hasSourceLinkLayerAddress = $true
        }
        $optionOffset += $optionLength
    }

    if ($neighborDiscoveryType -eq 135) {
        [byte[]]$source = [byte[]]$Frame[($ipOffset + 8)..($ipOffset + 23)]
        [byte[]]$destination = [byte[]]$Frame[($ipOffset + 24)..($ipOffset + 39)]
        [byte[]]$target = [byte[]]$Frame[($icmpOffset + 8)..($icmpOffset + 23)]
        Assert-True ($target[0] -ne 0xff) "Neighbor Solicitation target is multicast."
        if (Test-IPv6UnspecifiedAddress $source) {
            Assert-True (Test-SolicitedNodeMulticastAddress $destination) `
                "Duplicate Address Detection Neighbor Solicitation destination is not solicited-node multicast."
            Assert-True (-not $hasSourceLinkLayerAddress) `
                "Duplicate Address Detection Neighbor Solicitation includes a source link-layer option."
        } elseif (Test-SolicitedNodeMulticastAddress $destination) {
            Assert-True $hasSourceLinkLayerAddress `
                "Multicast Neighbor Solicitation with a source address lacks a source link-layer option."
        } else {
            Assert-True (Test-BytesEqual $destination $target) `
                "Unicast Neighbor Solicitation destination is not the target address."
        }
    }
}

function Test-IPv6NeighborDiscoveryFrame([byte[]]$Frame) {
    Assert-IPv6FrameStructure $Frame
    $ipOffset = 14
    if ($Frame[$ipOffset + 6] -ne 58) {
        return $false
    }

    $payloadLength = Get-UInt16BigEndian $Frame ($ipOffset + 4)
    Assert-True ($payloadLength -ge 4) "ICMPv6 payload is truncated."
    $icmpOffset = $ipOffset + 40
    if ($Frame[$icmpOffset] -notin @(133, 134, 135, 136, 137)) {
        return $false
    }

    Assert-IPv6Icmpv6Structure $Frame
    return $true
}

function Increment-RelayControlFrame($Relay, [string]$Kind, [string]$Direction) {
    $Relay.ControlFrames[$Kind][$Direction] = [int]$Relay.ControlFrames[$Kind][$Direction] + 1
}

function Assert-NoReflectedInjection($Relay, [string]$Direction, [byte[]]$Frame) {
    foreach ($injectedFrame in $Relay.InjectedFrames[$Direction]) {
        if (Test-BytesEqual $Frame $injectedFrame) {
            throw "The $Direction TAP read returned a byte-identical frame injected into that endpoint."
        }
    }
}

function New-RelayArpControlFrame($Relay, [string]$Direction) {
    $sourceAdapter = if ($Direction -eq "AtoB") { $Relay.Adapters.A } else { $Relay.Adapters.B }
    $destinationAdapter = if ($Direction -eq "AtoB") { $Relay.Adapters.B } else { $Relay.Adapters.A }
    $sourceAddress = if ($Direction -eq "AtoB") { $ipv4A } else { $ipv4B }
    $destinationAddress = if ($Direction -eq "AtoB") { $ipv4B } else { $ipv4A }
    [byte[]]$frame = [byte[]]::new(42)
    [Array]::Copy((Get-MacBytes $destinationAdapter.MacAddress), 0, $frame, 0, 6)
    [Array]::Copy((Get-MacBytes $sourceAdapter.MacAddress), 0, $frame, 6, 6)
    Set-UInt16BigEndian $frame 12 0x0806
    Set-UInt16BigEndian $frame 14 1
    Set-UInt16BigEndian $frame 16 0x0800
    $frame[18] = 6
    $frame[19] = 4
    Set-UInt16BigEndian $frame 20 1
    [Array]::Copy((Get-MacBytes $sourceAdapter.MacAddress), 0, $frame, 22, 6)
    [Array]::Copy(([System.Net.IPAddress]::Parse($sourceAddress).GetAddressBytes()), 0, $frame, 28, 4)
    [Array]::Copy(([System.Net.IPAddress]::Parse($destinationAddress).GetAddressBytes()), 0, $frame, 38, 4)
    return ,$frame
}

function New-RelayNeighborSolicitationFrame($Relay, [string]$Direction) {
    $sourceAdapter = if ($Direction -eq "AtoB") { $Relay.Adapters.A } else { $Relay.Adapters.B }
    $sourceAddress = if ($Direction -eq "AtoB") { $ipv6A } else { $ipv6B }
    $targetAddress = if ($Direction -eq "AtoB") { $ipv6B } else { $ipv6A }
    [byte[]]$source = [System.Net.IPAddress]::Parse($sourceAddress).GetAddressBytes()
    [byte[]]$target = [System.Net.IPAddress]::Parse($targetAddress).GetAddressBytes()
    [byte[]]$destination = Get-SolicitedNodeMulticastAddress $target
    [byte[]]$sourceMac = Get-MacBytes $sourceAdapter.MacAddress
    [byte[]]$frame = [byte[]]::new(86)
    [Array]::Copy([byte[]](0x33, 0x33, 0xff, $target[13], $target[14], $target[15]), 0, $frame, 0, 6)
    [Array]::Copy($sourceMac, 0, $frame, 6, 6)
    Set-UInt16BigEndian $frame 12 0x86dd
    $frame[14] = 0x60
    Set-UInt16BigEndian $frame 18 32
    $frame[20] = 58
    $frame[21] = 255
    [Array]::Copy($source, 0, $frame, 22, 16)
    [Array]::Copy($destination, 0, $frame, 38, 16)
    $frame[54] = 135
    $frame[55] = 0
    [Array]::Copy($target, 0, $frame, 62, 16)
    $frame[78] = 1
    $frame[79] = 1
    [Array]::Copy($sourceMac, 0, $frame, 80, 6)
    [byte[]]$checksumBuffer = [byte[]]::new(72)
    [Array]::Copy((Get-IPv6PseudoHeader $source $destination 32), 0, $checksumBuffer, 0, 40)
    [Array]::Copy($frame, 54, $checksumBuffer, 40, 32)
    Set-UInt16BigEndian $frame 56 (Get-InternetChecksum $checksumBuffer 0 $checksumBuffer.Length)
    return ,$frame
}

function New-RelayUnicastNeighborSolicitationFrame($Relay, [string]$Direction) {
    $sourceAdapter = if ($Direction -eq "AtoB") { $Relay.Adapters.A } else { $Relay.Adapters.B }
    $destinationAdapter = if ($Direction -eq "AtoB") { $Relay.Adapters.B } else { $Relay.Adapters.A }
    $sourceAddress = if ($Direction -eq "AtoB") { $ipv6A } else { $ipv6B }
    $targetAddress = if ($Direction -eq "AtoB") { $ipv6B } else { $ipv6A }
    [byte[]]$source = [System.Net.IPAddress]::Parse($sourceAddress).GetAddressBytes()
    [byte[]]$target = [System.Net.IPAddress]::Parse($targetAddress).GetAddressBytes()
    [byte[]]$frame = [byte[]]::new(78)
    [Array]::Copy((Get-MacBytes $destinationAdapter.MacAddress), 0, $frame, 0, 6)
    [Array]::Copy((Get-MacBytes $sourceAdapter.MacAddress), 0, $frame, 6, 6)
    Set-UInt16BigEndian $frame 12 0x86dd
    $frame[14] = 0x60
    Set-UInt16BigEndian $frame 18 24
    $frame[20] = 58
    $frame[21] = 255
    [Array]::Copy($source, 0, $frame, 22, 16)
    [Array]::Copy($target, 0, $frame, 38, 16)
    $frame[54] = 135
    $frame[55] = 0
    [Array]::Copy($target, 0, $frame, 62, 16)
    [byte[]]$checksumBuffer = [byte[]]::new(64)
    [Array]::Copy((Get-IPv6PseudoHeader $source $target 24), 0, $checksumBuffer, 0, 40)
    [Array]::Copy($frame, 54, $checksumBuffer, 40, 24)
    Set-UInt16BigEndian $frame 56 (Get-InternetChecksum $checksumBuffer 0 $checksumBuffer.Length)
    return ,$frame
}

function New-RelayDuplicateAddressDetectionFrame($Relay, [string]$Direction) {
    $sourceAdapter = if ($Direction -eq "AtoB") { $Relay.Adapters.A } else { $Relay.Adapters.B }
    $targetAddress = if ($Direction -eq "AtoB") { $ipv6B } else { $ipv6A }
    [byte[]]$source = [byte[]]::new(16)
    [byte[]]$target = [System.Net.IPAddress]::Parse($targetAddress).GetAddressBytes()
    [byte[]]$destination = Get-SolicitedNodeMulticastAddress $target
    [byte[]]$frame = [byte[]]::new(78)
    [Array]::Copy([byte[]](0x33, 0x33, 0xff, $target[13], $target[14], $target[15]), 0, $frame, 0, 6)
    [Array]::Copy((Get-MacBytes $sourceAdapter.MacAddress), 0, $frame, 6, 6)
    Set-UInt16BigEndian $frame 12 0x86dd
    $frame[14] = 0x60
    Set-UInt16BigEndian $frame 18 24
    $frame[20] = 58
    $frame[21] = 255
    [Array]::Copy($source, 0, $frame, 22, 16)
    [Array]::Copy($destination, 0, $frame, 38, 16)
    $frame[54] = 135
    $frame[55] = 0
    [Array]::Copy($target, 0, $frame, 62, 16)
    [byte[]]$checksumBuffer = [byte[]]::new(64)
    [Array]::Copy((Get-IPv6PseudoHeader $source $destination 24), 0, $checksumBuffer, 0, 40)
    [Array]::Copy($frame, 54, $checksumBuffer, 40, 24)
    Set-UInt16BigEndian $frame 56 (Get-InternetChecksum $checksumBuffer 0 $checksumBuffer.Length)
    return ,$frame
}

function Assert-ControlFrameSuppression($Relay) {
    foreach ($direction in @("AtoB", "BtoA")) {
        $arpBefore = [int]$Relay.ControlFrames["Arp"][$direction]
        $ndpBefore = [int]$Relay.ControlFrames["Ndp"][$direction]
        $arp = New-RelayArpControlFrame $Relay $direction
        $ndp = New-RelayNeighborSolicitationFrame $Relay $direction
        $unicastNdp = New-RelayUnicastNeighborSolicitationFrame $Relay $direction
        $dad = New-RelayDuplicateAddressDetectionFrame $Relay $direction
        Save-Packet "$direction-control-arp" $arp
        Save-Packet "$direction-control-ndp" $ndp
        Save-Packet "$direction-control-unicast-ndp" $unicastNdp
        Save-Packet "$direction-control-dad" $dad
        Assert-True (-not (Observe-RelayFrame $Relay $direction $arp)) `
            "$direction ARP control frame was not suppressed."
        Assert-True (-not (Observe-RelayFrame $Relay $direction $ndp)) `
            "$direction Neighbor Discovery control frame was not suppressed."
        Assert-True (-not (Observe-RelayFrame $Relay $direction $unicastNdp)) `
            "$direction unicast Neighbor Discovery control frame was not suppressed."
        Assert-True (-not (Observe-RelayFrame $Relay $direction $dad)) `
            "$direction Duplicate Address Detection control frame was not suppressed."
        Assert-True ([int]$Relay.ControlFrames["Arp"][$direction] -eq $arpBefore + 1) `
            "$direction ARP suppression was not counted."
        Assert-True ([int]$Relay.ControlFrames["Ndp"][$direction] -eq $ndpBefore + 3) `
            "$direction Neighbor Discovery, unicast NUD, and Duplicate Address Detection suppression was not counted."
    }
}

function Assert-IPv4IcmpFrame($Relay, [string]$Direction, [byte[]]$Frame) {
    $ipOffset = 14
    Assert-IPv4FrameStructure $Frame
    $headerLength = ($Frame[$ipOffset] -band 0x0f) * 4
    $totalLength = Get-UInt16BigEndian $Frame ($ipOffset + 2)
    Assert-True ($totalLength -ge $headerLength + 8 -and $totalLength -le $Frame.Length - $ipOffset) `
        "IPv4 total length is invalid."
    Assert-True ((Get-UInt16BigEndian $Frame ($ipOffset + 6) -band 0x3fff) -eq 0) `
        "IPv4 test packet is fragmented."
    Assert-Checksum $Frame $ipOffset $headerLength "IPv4"
    Assert-True ($Frame[$ipOffset + 9] -eq 1) "IPv4 test packet is not ICMP."

    $source = [byte[]]$Frame[($ipOffset + 12)..($ipOffset + 15)]
    $destination = [byte[]]$Frame[($ipOffset + 16)..($ipOffset + 19)]
    $aAddress = [System.Net.IPAddress]::Parse($ipv4A).GetAddressBytes()
    $bAddress = [System.Net.IPAddress]::Parse($ipv4B).GetAddressBytes()
    $aMac = Get-MacBytes $Relay.Adapters.A.MacAddress
    $bMac = Get-MacBytes $Relay.Adapters.B.MacAddress
    $icmpOffset = $ipOffset + $headerLength
    $icmpLength = $totalLength - $headerLength
    Assert-Checksum $Frame $icmpOffset $icmpLength "ICMP"

    if ($Direction -eq "AtoB") {
        Assert-EthernetHeader $Frame $aMac $bMac 0x0800 "IPv4 Echo Request"
        Assert-True (Test-BytesEqual $source $aAddress) "IPv4 Echo Request source is not A."
        Assert-True (Test-BytesEqual $destination $bAddress) "IPv4 Echo Request destination is not B."
        Assert-True ($Frame[$icmpOffset] -eq 8 -and $Frame[$icmpOffset + 1] -eq 0) `
            "IPv4 packet is not an ICMP Echo Request."
        $Relay.Proofs.IPv4Request = @{
            Identifier = Get-UInt16BigEndian $Frame ($icmpOffset + 4)
            Sequence = Get-UInt16BigEndian $Frame ($icmpOffset + 6)
            Payload = if ($icmpLength -gt 8) {
                [byte[]]$Frame[($icmpOffset + 8)..($icmpOffset + $icmpLength - 1)]
            } else {
                [byte[]]::new(0)
            }
        }
    } else {
        Assert-EthernetHeader $Frame $bMac $aMac 0x0800 "IPv4 Echo Reply"
        Assert-True (Test-BytesEqual $source $bAddress) "IPv4 Echo Reply source is not B."
        Assert-True (Test-BytesEqual $destination $aAddress) "IPv4 Echo Reply destination is not A."
        Assert-True ($Frame[$icmpOffset] -eq 0 -and $Frame[$icmpOffset + 1] -eq 0) `
            "IPv4 packet is not an ICMP Echo Reply."
        Assert-True ($null -ne $Relay.Proofs.IPv4Request) `
            "IPv4 Echo Reply arrived before the matching request was observed."
        Assert-True ((Get-UInt16BigEndian $Frame ($icmpOffset + 4)) -eq $Relay.Proofs.IPv4Request.Identifier -and
            (Get-UInt16BigEndian $Frame ($icmpOffset + 6)) -eq $Relay.Proofs.IPv4Request.Sequence) `
            "IPv4 Echo Reply identifier or sequence changed."
        $payload = if ($icmpLength -gt 8) {
            [byte[]]$Frame[($icmpOffset + 8)..($icmpOffset + $icmpLength - 1)]
        } else {
            [byte[]]::new(0)
        }
        Assert-True (Test-BytesEqual $payload $Relay.Proofs.IPv4Request.Payload) `
            "IPv4 Echo Reply payload changed."
        $Relay.Proofs.IPv4Reply = $true
    }
}

function Assert-IPv6IcmpFrame($Relay, [string]$Direction, [byte[]]$Frame) {
    $ipOffset = 14
    Assert-IPv6FrameStructure $Frame
    $payloadLength = Get-UInt16BigEndian $Frame ($ipOffset + 4)
    Assert-True ($payloadLength -ge 8 -and $ipOffset + 40 + $payloadLength -le $Frame.Length) `
        "IPv6 payload length is invalid."
    Assert-True ($Frame[$ipOffset + 6] -eq 58) "IPv6 test packet is not ICMPv6."
    $source = [byte[]]$Frame[($ipOffset + 8)..($ipOffset + 23)]
    $destination = [byte[]]$Frame[($ipOffset + 24)..($ipOffset + 39)]
    $aAddress = [System.Net.IPAddress]::Parse($ipv6A).GetAddressBytes()
    $bAddress = [System.Net.IPAddress]::Parse($ipv6B).GetAddressBytes()
    $aMac = Get-MacBytes $Relay.Adapters.A.MacAddress
    $bMac = Get-MacBytes $Relay.Adapters.B.MacAddress
    $icmpOffset = $ipOffset + 40
    Assert-Icmpv6Checksum $Frame $icmpOffset $payloadLength $source $destination

    if ($Direction -eq "AtoB") {
        Assert-EthernetHeader $Frame $aMac $bMac 0x86dd "ICMPv6 Echo Request"
        Assert-True (Test-BytesEqual $source $aAddress) "ICMPv6 Echo Request source is not A."
        Assert-True (Test-BytesEqual $destination $bAddress) "ICMPv6 Echo Request destination is not B."
        Assert-True ($Frame[$icmpOffset] -eq 128 -and $Frame[$icmpOffset + 1] -eq 0) `
            "IPv6 packet is not an ICMPv6 Echo Request."
        $Relay.Proofs.IPv6Request = @{
            Identifier = Get-UInt16BigEndian $Frame ($icmpOffset + 4)
            Sequence = Get-UInt16BigEndian $Frame ($icmpOffset + 6)
            Payload = if ($payloadLength -gt 8) {
                [byte[]]$Frame[($icmpOffset + 8)..($icmpOffset + $payloadLength - 1)]
            } else {
                [byte[]]::new(0)
            }
        }
    } else {
        Assert-EthernetHeader $Frame $bMac $aMac 0x86dd "ICMPv6 Echo Reply"
        Assert-True (Test-BytesEqual $source $bAddress) "ICMPv6 Echo Reply source is not B."
        Assert-True (Test-BytesEqual $destination $aAddress) "ICMPv6 Echo Reply destination is not A."
        Assert-True ($Frame[$icmpOffset] -eq 129 -and $Frame[$icmpOffset + 1] -eq 0) `
            "IPv6 packet is not an ICMPv6 Echo Reply."
        Assert-True ($null -ne $Relay.Proofs.IPv6Request) `
            "ICMPv6 Echo Reply arrived before the matching request was observed."
        Assert-True ((Get-UInt16BigEndian $Frame ($icmpOffset + 4)) -eq $Relay.Proofs.IPv6Request.Identifier -and
            (Get-UInt16BigEndian $Frame ($icmpOffset + 6)) -eq $Relay.Proofs.IPv6Request.Sequence) `
            "ICMPv6 Echo Reply identifier or sequence changed."
        $payload = if ($payloadLength -gt 8) {
            [byte[]]$Frame[($icmpOffset + 8)..($icmpOffset + $payloadLength - 1)]
        } else {
            [byte[]]::new(0)
        }
        Assert-True (Test-BytesEqual $payload $Relay.Proofs.IPv6Request.Payload) `
            "ICMPv6 Echo Reply payload changed."
        $Relay.Proofs.IPv6Reply = $true
    }
}

function Observe-RelayFrame($Relay, [string]$Direction, [byte[]]$Frame) {
    Assert-True ($Frame.Length -ge 14 -and $Frame.Length -le 1514) `
        "Relay received an out-of-range Ethernet frame length $($Frame.Length)."
    Assert-NoReflectedInjection $Relay $Direction $Frame
    $etherType = Get-UInt16BigEndian $Frame 12
    if ($etherType -eq 0x0806) {
        Assert-ArpFrame $Frame
        Increment-RelayControlFrame $Relay "Arp" $Direction
        # Static peer neighbors resolve the test endpoints before relay starts.
        # Relaying ARP/DAD control traffic would reflect it indefinitely between the adapters.
        return $false
    }
    if ($etherType -eq 0x0800) {
        Assert-IPv4FrameStructure $Frame
        $source = [System.Net.IPAddress]::new([byte[]]$Frame[26..29]).ToString()
        $destination = [System.Net.IPAddress]::new([byte[]]$Frame[30..33]).ToString()
        if ($source -eq $ipv4A -and $destination -eq $ipv4B) {
            if ($Direction -ne "AtoB") {
                return $false
            }
            Assert-IPv4IcmpFrame $Relay $Direction $Frame
        } elseif ($source -eq $ipv4B -and $destination -eq $ipv4A) {
            if ($Direction -ne "BtoA") {
                return $false
            }
            Assert-IPv4IcmpFrame $Relay $Direction $Frame
        } else {
            return $false
        }
        return $true
    }
    if ($etherType -eq 0x86dd) {
        if (Test-IPv6NeighborDiscoveryFrame $Frame) {
            Increment-RelayControlFrame $Relay "Ndp" $Direction
            return $false
        }
        Assert-IPv6Icmpv6Structure $Frame
        $source = [System.Net.IPAddress]::new([byte[]]$Frame[22..37]).ToString()
        $destination = [System.Net.IPAddress]::new([byte[]]$Frame[38..53]).ToString()
        if ($source -eq $ipv6A -and $destination -eq $ipv6B) {
            if ($Direction -ne "AtoB") {
                return $false
            }
            Assert-IPv6IcmpFrame $Relay $Direction $Frame
        } elseif ($source -eq $ipv6B -and $destination -eq $ipv6A) {
            if ($Direction -ne "BtoA") {
                return $false
            }
            Assert-IPv6IcmpFrame $Relay $Direction $Frame
        } else {
            return $false
        }
        return $true
    }
    throw "Unsupported EtherType 0x$('{0:X4}' -f $etherType) cannot be relayed."
}

function Start-Relay([IntPtr]$HandleA, [IntPtr]$HandleB, $Adapters) {
    $relay = [pscustomobject]@{
        HandleA = $HandleA
        HandleB = $HandleB
        Adapters = $Adapters
        Reads = @{}
        Writes = @{}
        InjectedFrames = @{
            AtoB = [System.Collections.Generic.List[byte[]]]::new()
            BtoA = [System.Collections.Generic.List[byte[]]]::new()
        }
        ControlFrames = @{
            Arp = @{ AtoB = 0; BtoA = 0 }
            Ndp = @{ AtoB = 0; BtoA = 0 }
        }
        OwnerReopenValidated = $false
        Proofs = @{
            IPv4Request = $null
            IPv4Reply = $false
            IPv6Request = $null
            IPv6Reply = $false
        }
    }
    $relay.Reads["AtoB"] = New-RelayRead $relay "AtoB"
    $relay.Reads["BtoA"] = New-RelayRead $relay "BtoA"
    return $relay
}

function Complete-RelayRead($Relay, [string]$Direction) {
    $read = $Relay.Reads[$Direction]
    $result = Complete-IoOperation $read
    Assert-True $result.Succeeded `
        "$($read.Description) completed with Win32 error $($result.Error)."
    $frame = Copy-FrameFromRead $read
    Save-Packet $Direction $frame
    if (-not (Observe-RelayFrame $Relay $Direction $frame)) {
        Dispose-BufferedIoOperation $read
        $Relay.Reads[$Direction] = New-RelayRead $Relay $Direction
        return
    }
    $destinationHandle = if ($Direction -eq "AtoB") { $Relay.HandleB } else { $Relay.HandleA }
    $writeBuffer = [Runtime.InteropServices.Marshal]::AllocHGlobal([int]$read.Transferred)
    try {
        [Runtime.InteropServices.Marshal]::Copy($frame, 0, $writeBuffer, [int]$read.Transferred)
        $write = Start-IoOperation $destinationHandle $writeBuffer ([int]$read.Transferred) $true $read `
            "$Direction destination write"
        $write.OwnsBuffer = $true
    } catch {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($writeBuffer)
        throw
    }
    $write | Add-Member -NotePropertyName Direction -NotePropertyValue $Direction
    $targetReadDirection = if ($Direction -eq "AtoB") { "BtoA" } else { "AtoB" }
    $Relay.InjectedFrames[$targetReadDirection].Add([byte[]]$frame.Clone())
    $Relay.Reads[$Direction] = $null
    $Relay.Writes[$Direction] = $write
}

function Complete-RelayWrite($Relay, [string]$Direction) {
    $write = $Relay.Writes[$Direction]
    Assert-IoSuccess $write
    # The read buffer is freed only after this paired destination write completed.
    Dispose-BufferedIoOperation $write
    Dispose-BufferedIoOperation $write.SourceOperation
    $Relay.Writes[$Direction] = $null
    $Relay.Reads[$Direction] = New-RelayRead $Relay $Direction
}

function Pump-Relay($Relay, [int]$WaitMilliseconds) {
    foreach ($direction in @("AtoB", "BtoA")) {
        if ($null -ne $Relay.Writes[$direction] -and $Relay.Writes[$direction].Terminal) {
            Complete-RelayWrite $Relay $direction
            return
        }
        if ($null -ne $Relay.Reads[$direction] -and $Relay.Reads[$direction].Terminal) {
            Complete-RelayRead $Relay $direction
            return
        }
    }

    $pending = @(
        foreach ($direction in @("AtoB", "BtoA")) {
            if ($null -ne $Relay.Reads[$direction] -and $Relay.Reads[$direction].Pending) {
                $Relay.Reads[$direction]
            }
            if ($null -ne $Relay.Writes[$direction] -and $Relay.Writes[$direction].Pending) {
                $Relay.Writes[$direction]
            }
        }
    )
    Assert-True ($pending.Count -gt 0) "Relay has no active I/O operation."
    [IntPtr[]]$events = @($pending | ForEach-Object Event)
    [uint32]$nativeError = 0
    $wait = [WinTapDualNative]::WaitForMultipleObjectsWithError(
        $events, [uint32]$WaitMilliseconds, [ref]$nativeError)
    if ($wait -eq [WinTapDualNative]::WaitTimeout) {
        return
    }
    Assert-True ($wait -ne [WinTapDualNative]::WaitFailed) `
        "WaitForMultipleObjects failed with Win32 error $nativeError."
    Assert-True ($wait -ge [WinTapDualNative]::WaitObject0 -and
        $wait -lt [WinTapDualNative]::WaitObject0 + $pending.Count) `
        "WaitForMultipleObjects returned unexpected status $wait."
    $operation = $pending[[int]($wait - [WinTapDualNative]::WaitObject0)]
    $operation.Pending = $false
    Complete-IoOperation $operation | Out-Null
}

function Assert-PingProof($Relay, [string]$Address, [byte[]]$Payload, [string]$Family, $PingTask) {
    Assert-True $PingTask.IsCompleted "Unbound $Family Ping did not complete before the relay timeout."
    $reply = $PingTask.GetAwaiter().GetResult()
    Assert-True ($reply.Status -eq [System.Net.NetworkInformation.IPStatus]::Success) `
        "Unbound $Family Ping to $Address returned $($reply.Status)."
    Assert-True (Test-BytesEqual $reply.Buffer $Payload) `
        "Unbound $Family Ping returned a different payload."
    if ($Family -eq "IPv4") {
        Assert-True ($null -ne $Relay.Proofs.IPv4Request -and $Relay.Proofs.IPv4Reply) `
            "The IPv4 request/reply relay path was not proven."
    } else {
        Assert-True ($null -ne $Relay.Proofs.IPv6Request -and $Relay.Proofs.IPv6Reply) `
            "The IPv6 request/reply relay path was not proven."
    }
}

function Invoke-UnboundPingRelay($Relay, [string]$Address, [string]$Family) {
    $ping = [System.Net.NetworkInformation.Ping]::new()
    [byte[]]$payload = [Text.Encoding]::ASCII.GetBytes("WinTapDual-$script:RunId-$Family")
    try {
        if ($Family -eq "IPv4") {
            $Relay.Proofs.IPv4Request = $null
            $Relay.Proofs.IPv4Reply = $false
        } else {
            $Relay.Proofs.IPv6Request = $null
            $Relay.Proofs.IPv6Reply = $false
        }
        # SendPingAsync has no source-address argument: route selection is deliberately unbound.
        $task = $ping.SendPingAsync($Address, $TimeoutSeconds * 1000, $payload)
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        do {
            Pump-Relay $Relay 250
            $proofComplete = if ($Family -eq "IPv4") {
                $null -ne $Relay.Proofs.IPv4Request -and $Relay.Proofs.IPv4Reply
            } else {
                $null -ne $Relay.Proofs.IPv6Request -and $Relay.Proofs.IPv6Reply
            }
            if ($task.IsCompleted -and $proofComplete) {
                break
            }
        } while ([DateTime]::UtcNow -lt $deadline)
        Assert-PingProof $Relay $Address $payload $Family $task
    } finally {
        $ping.Dispose()
    }
}

function Stop-Relay($Relay) {
    if ($null -eq $Relay) {
        return
    }
    $operations = [System.Collections.Generic.List[object]]::new()
    foreach ($direction in @("AtoB", "BtoA")) {
        foreach ($operation in @($Relay.Reads[$direction], $Relay.Writes[$direction])) {
            if ($null -ne $operation) {
                $operations.Add($operation)
                if ($null -ne $operation.SourceOperation) {
                    $operations.Add($operation.SourceOperation)
                }
            }
        }
    }
    $unique = [System.Collections.Generic.HashSet[object]]::new()
    foreach ($operation in $operations) {
        $wasAdded = $unique.Add($operation)
        if ($wasAdded -and $operation.Pending) {
            [uint32]$cancelError = 0
            $cancelled = [WinTapDualNative]::CancelIoExWithError(
                $operation.Handle, $operation.Overlapped, [ref]$cancelError)
            if (-not $cancelled -and $cancelError -ne [WinTapDualNative]::ErrorNotFound) {
                throw "CancelIoEx for $($operation.Description) failed with Win32 error $cancelError."
            }
        }
    }
    foreach ($operation in $unique) {
        if ($operation.Pending) {
            $wait = [WinTapDualNative]::WaitForSingleObject($operation.Event, 5000)
            Assert-True ($wait -eq [WinTapDualNative]::WaitObject0) `
                "$($operation.Description) did not reach terminal completion before cleanup."
            $operation.Pending = $false
            Complete-IoOperation $operation | Out-Null
        }
    }
    foreach ($operation in $unique) {
        Dispose-BufferedIoOperation $operation
    }
}

function Close-ControlHandles {
    foreach ($handle in $script:Handles.Values) {
        if ($handle -and $handle -ne [IntPtr]::new(-1)) {
            [WinTapDualNative]::CloseHandle($handle) | Out-Null
        }
    }
    $script:Handles = @{}
}

function Assert-OwnerReopen($Adapters) {
    Stop-Relay $script:Relay
    $script:Relay = $null
    Close-ControlHandles

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastError = $null
    $handlesReopened = $false
    do {
        [IntPtr]$handleA = [IntPtr]::new(-1)
        [IntPtr]$handleB = [IntPtr]::new(-1)
        try {
            $handleA = Open-ControlHandle $controlPathA
            Assert-ExclusiveHandle $controlPathA
            $handleB = Open-ControlHandle $controlPathB
            Assert-ExclusiveHandle $controlPathB
            $script:Handles = @{ A = $handleA; B = $handleB }
            $handlesReopened = $true
            break
        } catch {
            $lastError = $_.Exception.Message
            foreach ($handle in @($handleA, $handleB)) {
                if ($handle -and $handle -ne [IntPtr]::new(-1)) {
                    [WinTapDualNative]::CloseHandle($handle) | Out-Null
                }
            }
            $script:Handles = @{}
            Start-Sleep -Milliseconds 250
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    Assert-True $handlesReopened "TAP owner close/reopen did not restore the relay: $lastError"

    $script:Relay = Start-Relay $script:Handles.A $script:Handles.B $Adapters
    Invoke-UnboundPingRelay $script:Relay $ipv4B "IPv4"
    Invoke-UnboundPingRelay $script:Relay $ipv6B "IPv6"
    $script:Relay.OwnerReopenValidated = $true
}

function Invoke-CleanupAction(
    [string]$Name,
    [scriptblock]$Action,
    [System.Collections.Generic.List[string]]$Errors
) {
    try {
        & $Action
    } catch {
        $Errors.Add("${Name}: $($_.Exception.Message)")
    }
}

function Invoke-Cleanup {
    $errors = [System.Collections.Generic.List[string]]::new()
    Invoke-CleanupAction "relay I/O" {
        Stop-Relay $script:Relay
        $script:Relay = $null
    } $errors
    Invoke-CleanupAction "control handles" {
        Close-ControlHandles
    } $errors
    Invoke-CleanupAction "driver-store package tracking" {
        if ($script:DriverPackageSnapshotTaken) {
            Update-AddedDriverPackage "driver-store-cleanup"
        }
    } $errors
    foreach ($ruleName in @($script:CreatedFirewallRules)) {
        Invoke-CleanupAction "firewall rule $ruleName" {
            Remove-NetFirewallRule -Name $ruleName -PolicyStore ActiveStore -ErrorAction Stop
        } $errors
    }
    foreach ($route in @($script:CreatedRoutes)) {
        Invoke-CleanupAction "route $($route.DestinationPrefix)" {
            Get-NetRoute -InterfaceIndex $route.InterfaceIndex -DestinationPrefix $route.DestinationPrefix `
                -ErrorAction Stop | Where-Object { $_.NextHop -eq $route.NextHop } |
                Remove-NetRoute -Confirm:$false -ErrorAction Stop
        } $errors
    }
    foreach ($neighbor in @($script:CreatedNeighbors)) {
        Invoke-CleanupAction "neighbor $($neighbor.IPAddress)" {
            Get-NetNeighbor -InterfaceIndex $neighbor.InterfaceIndex -ErrorAction Stop |
                Where-Object { $_.IPAddress -eq $neighbor.IPAddress } |
                Remove-NetNeighbor -Confirm:$false -ErrorAction Stop
        } $errors
    }
    foreach ($address in @($script:CreatedAddresses)) {
        Invoke-CleanupAction "address $($address.Address)" {
            Get-NetIPAddress -InterfaceIndex $address.InterfaceIndex -AddressFamily $address.AddressFamily `
                -ErrorAction Stop | Where-Object { $_.IPAddress -eq $address.Address } |
                Remove-NetIPAddress -Confirm:$false -ErrorAction Stop
        } $errors
    }
    foreach ($instanceId in @($script:CreatedPnpInstanceIds)) {
        Invoke-CleanupAction "PnP device $instanceId" {
            Invoke-RecordedNative "remove-device-$($instanceId -replace '[\\/:*?`"<>|]', '_')" `
                "pnputil.exe" @("/remove-device", $instanceId) | Out-Null
        } $errors
    }
    Invoke-CleanupAction "PnP removal completion" {
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        do {
            $remaining = @(Get-MatchingPnpDevices)
            if ($remaining.Count -eq 0) {
                break
            }
            Start-Sleep -Milliseconds 250
        } while ([DateTime]::UtcNow -lt $deadline)
        Assert-True ($remaining.Count -eq 0) `
            "Matching WinTap PnP device(s) remain after cleanup: $(($remaining | ForEach-Object InstanceId) -join ', ')"
        $script:PnpRemovalConfirmed = $true
    } $errors
    if ($script:AddedPublishedInf) {
        if ($script:PnpRemovalConfirmed) {
            Invoke-CleanupAction "driver package $script:AddedPublishedInf" {
                Invoke-RecordedNative "remove-driver-$script:AddedPublishedInf" "pnputil.exe" `
                    @("/delete-driver", $script:AddedPublishedInf, "/uninstall", "/force") | Out-Null
            } $errors
        } else {
            $errors.Add(
                "driver package $script:AddedPublishedInf was retained because created PnP removal was not confirmed.")
        }
    }
    return $errors
}

function Invoke-DualAdapterHarness {
    Ensure-DiagnosticsDirectory
    Assert-TestSigning
    $script:InfPath = Join-Path (Resolve-Path -LiteralPath $PackageDirectory -ErrorAction Stop).Path $driverInf
    Assert-True (Test-Path -LiteralPath $script:InfPath -PathType Leaf) `
        "Driver INF is missing: $script:InfPath"
    $script:ResolvedDevCon = Resolve-DevCon

    # All following preflight checks run before provisioning or network mutation.
    Assert-CleanEnvironment
    $script:DriverPackagesBefore = @(Get-DriverStoreSnapshot "driver-store-before")
    $script:DriverPackageSnapshotTaken = $true
    Invoke-DevConInstall $hardwareIdA "devcon-install-wintaprust"
    $firstDevice = @(Wait-MatchingPnpDeviceCount 1)
    $script:CreatedPnpInstanceIds = @($firstDevice | ForEach-Object InstanceId)
    Invoke-DevConInstall $hardwareIdB "devcon-install-wintaprust2"
    $devices = @(Wait-MatchingPnpDeviceCount 2)
    $script:CreatedPnpInstanceIds = @($devices | ForEach-Object InstanceId)
    $script:CreatedPnpInstanceIds | Out-File `
        -LiteralPath (Join-Path $script:DiagnosticsPath "created-pnp-instance-ids.txt") -Encoding utf8 -Force

    Update-AddedDriverPackage "driver-store-after"
    $adapterInstances = @($devices | ForEach-Object InstanceId)
    $adapters = Map-Adapters (Wait-WinTapAdapters $adapterInstances)
    Assert-True ($firstDevice.Count -eq 1) "First DevCon install did not create exactly one device."

    $script:Handles.A = Open-ControlHandle $controlPathA
    Assert-ExclusiveHandle $controlPathA
    $script:Handles.B = Open-ControlHandle $controlPathB
    Assert-ExclusiveHandle $controlPathB
    Configure-Topology $adapters

    $script:Relay = Start-Relay $script:Handles.A $script:Handles.B $adapters
    Assert-ControlFrameSuppression $script:Relay
    Assert-OwnerReopen $adapters
    Assert-ControlFrameSuppression $script:Relay
    for ($iteration = 0; $iteration -lt $RelayIterations; ++$iteration) {
        Invoke-UnboundPingRelay $script:Relay $ipv4B "IPv4"
        Invoke-UnboundPingRelay $script:Relay $ipv6B "IPv6"
    }
}

Ensure-DiagnosticsDirectory
$primaryFailure = $null
$cleanupErrors = $null
try {
    Invoke-DualAdapterHarness
} catch {
    $primaryFailure = $_
    Save-Diagnostics "primary-failure.txt" { $primaryFailure | Format-List * -Force }
} finally {
    if ($script:Relay) {
        Save-Diagnostics "relay-control-frames.txt" {
            [pscustomobject]@{
                ControlFrames = $script:Relay.ControlFrames
                OwnerReopenValidated = $script:Relay.OwnerReopenValidated
                InjectionFrameCounts = @{
                    AtoB = $script:Relay.InjectedFrames["AtoB"].Count
                    BtoA = $script:Relay.InjectedFrames["BtoA"].Count
                }
            } | ConvertTo-Json -Depth 5
        }
    }
    Save-EnvironmentDiagnostics "before-cleanup"
    $cleanupErrors = Invoke-Cleanup
    Save-EnvironmentDiagnostics "after-cleanup"
    if ($cleanupErrors.Count -gt 0) {
        $cleanupErrors | Out-File -LiteralPath (Join-Path $script:DiagnosticsPath "cleanup-errors.txt") `
            -Encoding utf8 -Force
    }
}

if ($primaryFailure) {
    if ($cleanupErrors.Count -gt 0) {
        Write-Error ("Cleanup also failed: " + ($cleanupErrors -join "; "))
    }
    throw $primaryFailure
}
if ($cleanupErrors.Count -gt 0) {
    throw ("Harness cleanup failed: " + ($cleanupErrors -join "; "))
}

Write-Host "WinTap REQ-015 dual-adapter relay harness passed."
