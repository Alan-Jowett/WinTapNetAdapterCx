# SPDX-License-Identifier: MIT
# Copyright (c) 2026 WinTapNetAdapterCx contributors
<#
.SYNOPSIS
Creates two disposable WinTap adapters, runs the user-mode switch, and removes
all resources created by this invocation.

.EXAMPLE
.\run-wintap-switch-experiment.ps1 `
    -PackageDirectory C:\Temp\WinTapSwitch\package `
    -SwitchPath C:\Temp\WinTapSwitch\wintap-switch.exe `
    -DevConPath C:\Temp\WinTapSwitch\devcon.exe `
    -DurationSeconds 300 -Stats
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$PackageDirectory,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$SwitchPath,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$DevConPath,

    [ValidateRange(5, 86400)]
    [int]$DurationSeconds = 60,

    [ValidateRange(2, 4096)]
    [int]$ReadDepth = 128,

    [ValidateRange(1, 4096)]
    [int]$WaitOperations = 32,

    [ValidateRange(0, 60000)]
    [int]$CompletionTimeoutMilliseconds = 1,

    [switch]$Stats,

    [string]$IperfPath,

    [ValidateRange(1, 3600)]
    [int]$IperfDurationSeconds = 10,

    [string]$DiagnosticsPath = ".\artifacts\wintap-switch-experiment",

    [ValidateRange(5, 300)]
    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot "wintap-bus-manager.psm1") -Force

$busInf = "wintap_bus_driver.inf"
$childInf = "wintap_netadaptercx_driver.inf"
$busHardwareId = "ROOT\WinTapBus"
$driverService = "WinTapChild"
$childGuids = @([Guid]::NewGuid(), [Guid]::NewGuid())
$createdChildren = @()
$childInterfaces = @{}
$createdAddresses = @()
$createdRoutes = @()
$switchProcess = $null
$iperfProcess = $null
$busInstalled = $false
$adapters = @()

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-Native([string]$FilePath, [string[]]$Arguments) {
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE."
    }
}

function Get-AdapterForChild([Guid]$Guid) {
    $hardwareId = "WINTAPBUS\{$($Guid.ToString().ToUpperInvariant())}"
    $matches = @(
        Get-NetAdapter -IncludeHidden -ErrorAction Stop | Where-Object {
            if ([string]::IsNullOrWhiteSpace([string]$_.PnPDeviceID)) {
                return $false
            }
            $ids = @(
                Get-PnpDeviceProperty -InstanceId $_.PnPDeviceID `
                    -KeyName "DEVPKEY_Device_HardwareIds" -ErrorAction Stop
            ).Data | ForEach-Object { [string]$_ }
            $ids -contains $hardwareId
        }
    )
    Assert-Condition ($matches.Count -eq 1) `
        "Expected exactly one network adapter for child $Guid; found $($matches.Count)."
    $service = [string](Get-PnpDeviceProperty -InstanceId $matches[0].PnPDeviceID `
        -KeyName "DEVPKEY_Device_Service" -ErrorAction Stop).Data
    Assert-Condition ($service -eq $driverService) `
        "Adapter $($matches[0].Name) uses service '$service', not '$driverService'."
    return $matches[0]
}

function Add-PointToPointAddress($Adapter, [string]$Address, [string]$PeerAddress) {
    Set-NetIPInterface -InterfaceIndex $Adapter.ifIndex -AddressFamily IPv4 `
        -DadTransmits 0 -ErrorAction Stop | Out-Null
    New-NetIPAddress -InterfaceIndex $Adapter.ifIndex -IPAddress $Address `
        -PrefixLength 32 -PolicyStore ActiveStore -ErrorAction Stop | Out-Null
    $script:createdAddresses += [pscustomobject]@{
        InterfaceIndex = $Adapter.ifIndex
        Address = $Address
    }
    New-NetRoute -InterfaceIndex $Adapter.ifIndex -DestinationPrefix "$PeerAddress/32" `
        -NextHop "0.0.0.0" -RouteMetric 0 -PolicyStore ActiveStore `
        -ErrorAction Stop | Out-Null
    $script:createdRoutes += [pscustomobject]@{
        InterfaceIndex = $Adapter.ifIndex
        DestinationPrefix = "$PeerAddress/32"
    }
}

function Remove-SwitchProcess {
    if ($null -eq $script:switchProcess) {
        return
    }
    if (-not $script:switchProcess.HasExited) {
        Stop-Process -Id $script:switchProcess.Id -Force -ErrorAction SilentlyContinue
        $script:switchProcess.WaitForExit(5000)
    }
    $script:switchProcess.Dispose()
    $script:switchProcess = $null
}

function Remove-IperfProcess {
    if ($null -eq $script:iperfProcess) {
        return
    }
    if (-not $script:iperfProcess.HasExited) {
        Stop-Process -Id $script:iperfProcess.Id -Force -ErrorAction SilentlyContinue
        $script:iperfProcess.WaitForExit(5000)
    }
    $script:iperfProcess.Dispose()
    $script:iperfProcess = $null
}

function Remove-ExperimentResources {
    Remove-IperfProcess
    Remove-SwitchProcess

    foreach ($route in @($script:createdRoutes)) {
        Remove-NetRoute -InterfaceIndex $route.InterfaceIndex `
            -DestinationPrefix $route.DestinationPrefix -Confirm:$false `
            -ErrorAction SilentlyContinue
    }
    foreach ($address in @($script:createdAddresses)) {
        Get-NetIPAddress -InterfaceIndex $address.InterfaceIndex `
            -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object IPAddress -eq $address.Address |
            Remove-NetIPAddress -Confirm:$false -ErrorAction SilentlyContinue
    }
    foreach ($guid in @($script:createdChildren)) {
        Remove-WinTapBusChild $guid $TimeoutSeconds | Out-Null
    }
    if ($script:busInstalled) {
        Invoke-Native $script:DevConPath @("remove", $busHardwareId)
        $script:busInstalled = $false
    }
}

Assert-Condition (
    [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) `
    "Run this script from an elevated administrator PowerShell session."
Assert-Condition ([Environment]::Is64BitProcess) `
    "Run this script from a 64-bit PowerShell host."
Assert-Condition ($ReadDepth % 2 -eq 0) "Read depth must be a positive even value."
Assert-Condition ($WaitOperations -le $ReadDepth) `
    "Wait operations must not exceed read depth."

$package = (Resolve-Path -LiteralPath $PackageDirectory -ErrorAction Stop).Path
$switch = (Resolve-Path -LiteralPath $SwitchPath -ErrorAction Stop).Path
$devcon = (Resolve-Path -LiteralPath $DevConPath -ErrorAction Stop).Path
$busInfPath = Join-Path $package $busInf
$childInfPath = Join-Path $package $childInf
Assert-Condition (Test-Path -LiteralPath $busInfPath -PathType Leaf) `
    "Bus INF is missing: $busInfPath"
Assert-Condition (Test-Path -LiteralPath $childInfPath -PathType Leaf) `
    "Child INF is missing: $childInfPath"

if (-not (Test-Path -LiteralPath $DiagnosticsPath)) {
    New-Item -ItemType Directory -Path $DiagnosticsPath -Force | Out-Null
}
$DiagnosticsPath = (Resolve-Path -LiteralPath $DiagnosticsPath).Path

try {
    Invoke-Native "pnputil.exe" @("/add-driver", $childInfPath, "/install")
    Invoke-Native $devcon @("install", $busInfPath, $busHardwareId)
    $busInstalled = $true

    foreach ($guid in $childGuids) {
        $createdChildren += $guid
        $child = New-WinTapBusChild $guid $TimeoutSeconds
        $childInterfaces[$guid.ToString()] = $child.InterfacePath
    }

    $adapterA = Get-AdapterForChild -Guid $childGuids[0]
    $adapterB = Get-AdapterForChild -Guid $childGuids[1]
    $adapters = @($adapterA, $adapterB)
    Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$adapters[0].InterfaceGuid)) `
        "Adapter $($adapters[0].Name) did not expose an interface GUID."
    Assert-Condition (-not [string]::IsNullOrWhiteSpace([string]$adapters[1].InterfaceGuid)) `
        "Adapter $($adapters[1].Name) did not expose an interface GUID."
    Add-PointToPointAddress $adapters[0] "198.51.100.1" "198.51.100.2"
    Add-PointToPointAddress $adapters[1] "198.51.100.2" "198.51.100.1"

    $interfaceGuidA = ([Guid]$adapters[0].InterfaceGuid).ToString("D")
    $interfaceGuidB = ([Guid]$adapters[1].InterfaceGuid).ToString("D")
    $arguments = @(
        "--endpoint", "$interfaceGuidA=$($childInterfaces[$childGuids[0].ToString()])",
        "--endpoint", "$interfaceGuidB=$($childInterfaces[$childGuids[1].ToString()])",
        "--read-depth", $ReadDepth,
        "--wait-operations", $WaitOperations,
        "--completion-timeout-ms", $CompletionTimeoutMilliseconds
    )
    if ($Stats) {
        $arguments += "--stats"
    }

    $stdoutPath = Join-Path $DiagnosticsPath "switch-stdout.txt"
    $stderrPath = Join-Path $DiagnosticsPath "switch-stderr.txt"
    $switchProcess = Start-Process -FilePath $switch -ArgumentList $arguments `
        -WorkingDirectory (Split-Path -Parent $switch) -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath -PassThru
    Write-Host "Started wintap-switch.exe (PID $($switchProcess.Id)) for $DurationSeconds seconds."
    if (-not [string]::IsNullOrWhiteSpace($IperfPath)) {
        $iperf = (Resolve-Path -LiteralPath $IperfPath -ErrorAction Stop).Path
        $iperfServerOutput = Join-Path $DiagnosticsPath "iperf-server.txt"
        $iperfServerError = Join-Path $DiagnosticsPath "iperf-server-error.txt"
        $iperfClientOutput = Join-Path $DiagnosticsPath "iperf-client.txt"
        $iperfProcess = Start-Process -FilePath $iperf -ArgumentList "-s" `
            -RedirectStandardOutput $iperfServerOutput -RedirectStandardError $iperfServerError `
            -PassThru
        Start-Sleep -Seconds 1
        Get-NetIPAddress -InterfaceIndex $adapters[0].ifIndex, $adapters[1].ifIndex `
            -AddressFamily IPv4 | Format-List * | Out-File (Join-Path $DiagnosticsPath "tap-addresses.txt")
        Get-NetRoute -InterfaceIndex $adapters[0].ifIndex, $adapters[1].ifIndex `
            -AddressFamily IPv4 | Format-List * | Out-File (Join-Path $DiagnosticsPath "tap-routes.txt")
        & $iperf "-c" "198.51.100.2" "-B" "198.51.100.1" "-t" $IperfDurationSeconds `
            *> $iperfClientOutput
        if ($LASTEXITCODE -ne 0) {
            throw "iperf3 failed with exit code $LASTEXITCODE."
        }
        Write-Host "iperf3 completed successfully for $IperfDurationSeconds seconds."
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($DurationSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($switchProcess.WaitForExit(100)) {
            throw "wintap-switch.exe exited early with code $($switchProcess.ExitCode)."
        }
    }
    Assert-Condition (-not $switchProcess.HasExited) `
        "wintap-switch.exe exited early with code $($switchProcess.ExitCode)."
} finally {
    Remove-ExperimentResources
}
