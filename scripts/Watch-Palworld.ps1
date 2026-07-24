[CmdletBinding()]
param(
  [string]$DashboardExe
)

$ErrorActionPreference = 'Stop'

function Find-DashboardExe {
  $candidates = @(
    $DashboardExe,
    (Join-Path $env:LOCALAPPDATA 'PalCommand\pal-command.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\PalCommand\pal-command.exe'),
    (Join-Path $PSScriptRoot '..\src-tauri\target\release\pal-command.exe')
  ) | Where-Object { $_ }

  foreach ($candidate in $candidates) {
    if (Test-Path -LiteralPath $candidate) {
      return (Resolve-Path -LiteralPath $candidate).Path
    }
  }
  throw 'PAL COMMAND executable was not found. Reinstall PAL COMMAND or rerun Install-PalworldAutoLaunch.ps1.'
}

function Test-PalworldRunning {
  $null -ne (Get-Process -Name 'Palworld-Win64-Shipping','Palworld' -ErrorAction SilentlyContinue)
}

$dashboard = Find-DashboardExe
$wasRunning = $false

while ($true) {
  $running = Test-PalworldRunning
  if ($running -and -not $wasRunning) {
    if (-not (Get-Process -Name 'pal-command' -ErrorAction SilentlyContinue)) {
      Start-Process -FilePath $dashboard
    }
  }
  $wasRunning = $running
  Start-Sleep -Seconds 3
}
