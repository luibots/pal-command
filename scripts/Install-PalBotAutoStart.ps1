[CmdletBinding()]
param([switch]$Remove)

$ErrorActionPreference = 'Stop'
$taskName = 'PAL COMMAND - Discord Bot'

if ($Remove) {
  Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
  Write-Host 'PAL COMMAND Discord bot auto-start removed.'
  return
}

$launcher = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-PalBot.ps1')).Path
$arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden " +
  "-File `"$launcher`""
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $arguments
$triggers = @(
  (New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME),
  # A dead supervisor can exhaust Task Scheduler's finite restart allowance.
  # This heartbeat wakes it again; IgnoreNew prevents duplicate live instances.
  (New-ScheduledTaskTrigger `
    -Once `
    -At (Get-Date).AddMinutes(1) `
    -RepetitionInterval (New-TimeSpan -Minutes 5))
)
$principal = New-ScheduledTaskPrincipal `
  -UserId $env:USERNAME `
  -LogonType Interactive `
  -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet `
  -AllowStartIfOnBatteries `
  -DontStopIfGoingOnBatteries `
  -MultipleInstances IgnoreNew `
  -RestartCount 10 `
  -RestartInterval (New-TimeSpan -Minutes 1) `
  -ExecutionTimeLimit (New-TimeSpan -Days 3650)

Register-ScheduledTask `
  -TaskName $taskName `
  -Action $action `
  -Trigger $triggers `
  -Principal $principal `
  -Settings $settings `
  -Description 'Keep the PAL COMMAND Discord bot online and restore it after an unexpected exit.' `
  -Force | Out-Null
Start-ScheduledTask -TaskName $taskName

Write-Host "PAL COMMAND Discord bot supervision enabled for $env:USERNAME."
