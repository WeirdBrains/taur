<#
.SYNOPSIS
  Stop and remove the TAUR Windows service (data in C:\ProgramData\TAUR is kept).
#>
param([string]$InstallDir = "C:\Program Files\TAUR")
$ErrorActionPreference = "Stop"
$svc = "$InstallDir\taur-service.exe"
if (-not (Test-Path $svc)) { throw "$svc not found" }
& $svc stop
& $svc uninstall
Remove-Item "$InstallDir\taur-service.exe", "$InstallDir\taur-service.xml" -ErrorAction SilentlyContinue
Write-Host "TAUR service removed."
