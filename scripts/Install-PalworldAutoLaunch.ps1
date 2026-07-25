[CmdletBinding()]
param(
  [string]$DashboardExe,
  [string]$CompanionExe,
  [switch]$Remove
)

$ErrorActionPreference = 'Stop'
$taskName = 'PAL COMMAND - Open with Palworld'

if ($Remove) {
  Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
  Write-Host 'PAL COMMAND Palworld auto-launch removed.'
  return
}

$watcher = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Watch-Palworld.ps1')).Path
$candidates = @(
  $DashboardExe,
  (Join-Path $env:LOCALAPPDATA 'PalCommand\pal-command.exe'),
  (Join-Path $env:LOCALAPPDATA 'Programs\PalCommand\pal-command.exe'),
  (Join-Path $PSScriptRoot '..\src-tauri\target\release\pal-command.exe')
) | Where-Object { $_ }

$dashboard = $candidates |
  Where-Object { Test-Path -LiteralPath $_ } |
  Select-Object -First 1
if (-not $dashboard) {
  throw 'PAL COMMAND executable was not found. Build or install the dashboard first.'
}
$dashboard = (Resolve-Path -LiteralPath $dashboard).Path
$companionCandidates = @(
  $CompanionExe,
  (Join-Path $env:LOCALAPPDATA 'PalCompanion\pal-companion.exe'),
  (Join-Path $env:LOCALAPPDATA 'Programs\PalCompanion\pal-companion.exe'),
  (Join-Path $PSScriptRoot '..\..\palworld-local-llm-companion\.venv\Scripts\pal-companion.exe')
) | Where-Object { $_ }
$companion = $companionCandidates |
  Where-Object { Test-Path -LiteralPath $_ } |
  Select-Object -First 1
if ($companion) {
  $companion = (Resolve-Path -LiteralPath $companion).Path
}

Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
$arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden " +
  "-File `"$watcher`" -DashboardExe `"$dashboard`""
if ($companion) {
  $arguments += " -CompanionExe `"$companion`""
}
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $arguments
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet `
  -AllowStartIfOnBatteries `
  -DontStopIfGoingOnBatteries `
  -MultipleInstances IgnoreNew `
  -RestartCount 3 `
  -RestartInterval (New-TimeSpan -Minutes 1) `
  -ExecutionTimeLimit (New-TimeSpan -Days 3650)

Register-ScheduledTask `
  -TaskName $taskName `
  -Action $action `
  -Trigger $trigger `
  -Principal $principal `
  -Settings $settings `
  -Description 'Open PAL COMMAND and keep the local Pal Companion API healthy while Palworld runs.' `
  -Force | Out-Null
Start-ScheduledTask -TaskName $taskName

Write-Host "PAL COMMAND will open with Palworld for Windows user $env:USERNAME."
Write-Host "Dashboard: $dashboard"
if ($companion) {
  Write-Host "Companion API: $companion"
}
else {
  Write-Warning 'pal-companion.exe was not found. The watcher will keep looking in standard locations.'
}
