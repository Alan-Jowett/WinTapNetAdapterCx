#requires -Version 5.1

[CmdletBinding()]
param(
    [string]$VmName = 'test',
    [string]$UserName = 'administrator',
    [System.Management.Automation.PSCredential]$Credential,
    [string]$ConfigureKdNetHostIp,
    [int]$ConfigureKdNetPort = 50000,
    [switch]$RebootAfterConfiguration
)

if (-not $Credential) {
    $Credential = Get-Credential -UserName $UserName -Message "Enter credentials for $VmName"
}

if ($ConfigureKdNetHostIp) {
    Invoke-Command -VMName $VmName -Credential $credential -ScriptBlock {
        param($hostIp, $port, $reboot)

        bcdedit /debug on
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to enable kernel debugging."
        }

        bcdedit /dbgsettings net hostip:$hostIp port:$port
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to configure KDNET."
        }

        bcdedit /dbgsettings
        if ($reboot) {
            shutdown.exe /r /t 5
        }
    } -ArgumentList $ConfigureKdNetHostIp, $ConfigureKdNetPort, $RebootAfterConfiguration
    return
}

Enter-PSSession -VMName $VmName -Credential $credential
