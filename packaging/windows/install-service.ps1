<#
.SYNOPSIS
  Install TAUR as a Windows service using WinSW. Run in an elevated PowerShell.
.DESCRIPTION
  taur.exe has no native Windows service support, so WinSW wraps it. This script
  copies WinSW next to taur.exe, generates taur.xml from taur-service.xml plus the
  TAUR_* settings in the env file, then installs and starts the service.
.EXAMPLE
  .\install-service.ps1 -WinSW C:\Downloads\WinSW-x64.exe -EnvFile C:\ProgramData\TAUR\taur.env
#>
param(
  [Parameter(Mandatory = $true)][string]$WinSW,
  [string]$EnvFile = "C:\ProgramData\TAUR\taur.env",
  [string]$InstallDir = "C:\Program Files\TAUR"
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path "$InstallDir\taur.exe")) { throw "taur.exe not found in $InstallDir" }
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { throw "Git for Windows must be installed and on PATH" }
New-Item -ItemType Directory -Force -Path "C:\ProgramData\TAUR\data", "C:\ProgramData\TAUR\logs" | Out-Null

$entries = @()
if (Test-Path $EnvFile) {
  foreach ($line in Get-Content $EnvFile) {
    if ($line -match '^\s*(TAUR_[A-Z_]+)\s*=\s*(.*)$') {
      $value = $Matches[2].Trim().Trim('"')
      $esc = [System.Security.SecurityElement]::Escape($value)
      $entries += "  <env name=`"$($Matches[1])`" value=`"$esc`"/>"
    }
  }
} else {
  Write-Warning "$EnvFile not found; using defaults"
}
$template = Get-Content (Join-Path $PSScriptRoot "taur-service.xml") -Raw
if ($entries | Where-Object { $_ -match 'name="TAUR_HOME"' }) {
  $template = $template -replace '  <env name="TAUR_HOME"[^\n]*\n', ''
}
$xml = $template.Replace("  <!--ENV-->", ($entries -join "`r`n"))
$xml = $xml.Replace("C:\Program Files\TAUR\taur.exe", "$InstallDir\taur.exe")
Copy-Item $WinSW "$InstallDir\taur-service.exe" -Force
# The XML may contain the SMTP password: restrict it to Administrators and SYSTEM.
Set-Content -Path "$InstallDir\taur-service.xml" -Value $xml -Encoding UTF8
icacls "$InstallDir\taur-service.xml" /inheritance:r /grant:r "Administrators:F" "SYSTEM:F" | Out-Null
& "$InstallDir\taur-service.exe" install
& "$InstallDir\taur-service.exe" start
Write-Host "TAUR service installed. Logs: C:\ProgramData\TAUR\logs"
