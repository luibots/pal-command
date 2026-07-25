[CmdletBinding()]
param(
  [string]$DashboardExe,
  [string]$CompanionExe,
  [int]$PollSeconds = 3,
  [switch]$Once
)

$ErrorActionPreference = 'Stop'
$healthUrl = 'http://127.0.0.1:8765/health'
$logDirectory = Join-Path $env:APPDATA 'com.luibots.palcommand\auto'
$logPath = Join-Path $logDirectory 'palworld-launch.log'

New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null

function Write-WatcherLog([string]$Message) {
  $timestamp = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
  Add-Content -LiteralPath $logPath -Value "[$timestamp] $Message" -Encoding UTF8
}

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

function Find-CompanionExe {
  $candidates = @(
    $CompanionExe,
    (Join-Path $env:LOCALAPPDATA 'PalCompanion\pal-companion.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\PalCompanion\pal-companion.exe'),
    (Join-Path $PSScriptRoot '..\..\palworld-local-llm-companion\.venv\Scripts\pal-companion.exe')
  ) | Where-Object { $_ }

  foreach ($candidate in $candidates) {
    if (Test-Path -LiteralPath $candidate) {
      return (Resolve-Path -LiteralPath $candidate).Path
    }
  }
  return $null
}

function Test-CompanionReady {
  try {
    $response = Invoke-WebRequest -UseBasicParsing -Uri $healthUrl -TimeoutSec 2
    return $response.StatusCode -eq 200
  }
  catch {
    return $false
  }
}

function Start-CompanionApi([string]$Executable) {
  if (Test-CompanionReady) {
    return $true
  }
  if (-not $Executable) {
    Write-WatcherLog 'Companion API is offline and pal-companion.exe was not found.'
    return $false
  }

  $scriptsDirectory = Split-Path -Parent $Executable
  $venvDirectory = Split-Path -Parent $scriptsDirectory
  $workingDirectory = Split-Path -Parent $venvDirectory
  try {
    Start-Process `
      -FilePath $Executable `
      -ArgumentList 'api' `
      -WorkingDirectory $workingDirectory `
      -WindowStyle Hidden
  }
  catch {
    Write-WatcherLog "Could not start companion API: $($_.Exception.Message)"
    return $false
  }
  Write-WatcherLog "Started companion API from $Executable."

  for ($attempt = 0; $attempt -lt 20; $attempt++) {
    Start-Sleep -Milliseconds 500
    if (Test-CompanionReady) {
      Write-WatcherLog 'Companion API health check passed.'
      return $true
    }
  }
  Write-WatcherLog 'Companion API did not become healthy within 10 seconds.'
  return $false
}

$dashboard = Find-DashboardExe
$companion = Find-CompanionExe
$wasRunning = $false
$nextCompanionAttempt = Get-Date

while ($true) {
  $running = Test-PalworldRunning
  if ($running) {
    if (-not (Test-CompanionReady) -and (Get-Date) -ge $nextCompanionAttempt) {
      $null = Start-CompanionApi $companion
      $nextCompanionAttempt = (Get-Date).AddSeconds(30)
    }

    if (-not $wasRunning) {
      if (-not (Get-Process -Name 'pal-command' -ErrorAction SilentlyContinue)) {
        try {
          Start-Process -FilePath $dashboard
          Write-WatcherLog 'Opened PAL COMMAND for a new Palworld session.'
        }
        catch {
          Write-WatcherLog "PAL COMMAND launch failed without stopping the API watcher: $($_.Exception.Message)"
        }
      }
    }
  }
  $wasRunning = $running
  if ($Once) {
    break
  }
  Start-Sleep -Seconds ([Math]::Max(1, $PollSeconds))
}
