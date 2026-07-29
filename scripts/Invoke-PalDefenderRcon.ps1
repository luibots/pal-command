[CmdletBinding()]
param(
  [Parameter(Mandatory)]
  [ValidateSet('Version', 'GrantItem', 'GrantTechnologyPoints', 'GrantAncientTechnologyPoints')]
  [string]$Action,
  [string]$ItemId = '',
  [ValidateRange(1, 999999)]
  [int]$Amount = 1
)

$ErrorActionPreference = 'Stop'
$configRoot = Join-Path $env:APPDATA 'com.luibots.palcommand'
$settings = Get-Content (Join-Path $configRoot 'settings.json') -Raw | ConvertFrom-Json
$companionEnv = Join-Path $PSScriptRoot '..\..\palworld-local-llm-companion\.env'
$secretPointer = [IntPtr]::Zero

function Read-EnvValue([string]$Name) {
  $prefix = "$Name="
  $line = Get-Content -LiteralPath $companionEnv |
    Where-Object { $_.StartsWith($prefix, [StringComparison]::Ordinal) } |
    Select-Object -First 1
  if (-not $line) {
    return ''
  }
  return $line.Substring($prefix.Length)
}

if (-not $settings.rcon_enabled -or -not $settings.rcon_host) {
  throw 'PAL COMMAND RCON is not configured.'
}

$playerId = Read-EnvValue 'ADMIN_SUPPLY_PLAYER_ID'
switch ($Action) {
  'Version' {
    $command = 'version'
  }
  'GrantItem' {
    if (-not $playerId) {
      throw 'The private admin player is not configured.'
    }
    if ($ItemId -notmatch '^[A-Za-z0-9_]+$') {
      throw 'The item ID contains unsupported characters.'
    }
    $command = "give $playerId $ItemId $Amount"
  }
  'GrantTechnologyPoints' {
    if (-not $playerId) {
      throw 'The private admin player is not configured.'
    }
    $command = "givetechpoints $playerId $Amount"
  }
  'GrantAncientTechnologyPoints' {
    if (-not $playerId) {
      throw 'The private admin player is not configured.'
    }
    $command = "givebosstechpoints $playerId $Amount"
  }
}

try {
  $securePassword = Get-Content (Join-Path $configRoot 'auto\admin.sec') |
    ConvertTo-SecureString
  $secretPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($securePassword)
  $password = [Runtime.InteropServices.Marshal]::PtrToStringAuto($secretPointer)

  $env:PALCMD_RCON_PASSWORD = $password
  $clientPath = Join-Path $PSScriptRoot 'paldefender_rcon.py'
  $rawResult = & python $clientPath `
    --host ([string]$settings.rcon_host) `
    --port ([string]$settings.rcon_port) `
    --command $command
  if ($LASTEXITCODE -ne 0) {
    throw 'The private RCON client failed.'
  }
  $response = ($rawResult | ConvertFrom-Json).response
  $success = $true
  try {
    $parsed = $response | ConvertFrom-Json
    if ($null -ne $parsed.Success) {
      $success = [bool]$parsed.Success
    }
    elseif ($parsed.Error) {
      $success = $false
    }
  }
  catch {
    if ($response -match '(?i)\b(error|failed|invalid|not found|offline)\b') {
      $success = $false
    }
  }

  [pscustomobject]@{
    Success = $success
    Action = $Action
    Response = $response
  } | ConvertTo-Json -Compress
  if (-not $success) {
    exit 2
  }
}
finally {
  $env:PALCMD_RCON_PASSWORD = $null
  if ($secretPointer -ne [IntPtr]::Zero) {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($secretPointer)
  }
}
