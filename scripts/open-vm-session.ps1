#requires -Version 5.1

[CmdletBinding()]
param(
    [string]$VmName = 'test',
    [string]$UserName = 'administrator',
    [string]$CredentialFile = (Join-Path $PSScriptRoot '..\creds.txt'),
    [string]$ConfigureKdNetHostIp,
    [int]$ConfigureKdNetPort = 50000,
    [switch]$RebootAfterConfiguration
)

$password = (Get-Content -LiteralPath $CredentialFile -Raw -ErrorAction Stop).Trim()
if ([string]::IsNullOrWhiteSpace($password)) {
    throw "Credential file '$CredentialFile' is empty."
}

$securePassword = ConvertTo-SecureString -String $password -AsPlainText -Force
$credential = [System.Management.Automation.PSCredential]::new($UserName, $securePassword)

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
