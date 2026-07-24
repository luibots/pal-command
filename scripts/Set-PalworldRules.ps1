[CmdletBinding()]
param(
  [ValidateSet('None', 'Item', 'ItemAndEquipment', 'All')]
  [string]$DeathPenalty = 'None',
  [ValidateRange(0, 240)]
  [double]$EggHatchHours = 0,
  [switch]$Apply
)

$ErrorActionPreference = 'Stop'
$configRoot = Join-Path $env:APPDATA 'com.luibots.palcommand'
$settings = Get-Content (Join-Path $configRoot 'settings.json') -Raw | ConvertFrom-Json
$autoDir = Join-Path $configRoot 'auto'
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$tempDir = Join-Path $tempRoot ('palcmd-rules-' + [guid]::NewGuid().ToString('N'))
$remote = "$($settings.config_dir)/PalWorldSettings.ini"
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$remoteBackup = "$remote.bak-$stamp"
$original = Join-Path $tempDir 'original.ini'
$updated = Join-Path $tempDir 'updated.ini'
$verified = Join-Path $tempDir 'verified.ini'
$batchFile = Join-Path $tempDir 'batch.txt'
$secretPointer = [IntPtr]::Zero

function Invoke-SftpBatch([string]$commands) {
  [IO.File]::WriteAllText($batchFile, $commands, [Text.Encoding]::ASCII)
  $arguments = @(
    '-oBatchMode=no',
    '-oStrictHostKeyChecking=accept-new',
    ('-oUserKnownHostsFile=' + (Join-Path $autoDir 'known_hosts')),
    '-oConnectTimeout=20',
    '-oPubkeyAuthentication=no',
    '-oPreferredAuthentications=password,keyboard-interactive',
    '-P', [string]$settings.sftp_port,
    '-b', $batchFile,
    "$($settings.sftp_user)@$($settings.sftp_host)"
  )
  & 'C:\Windows\System32\OpenSSH\sftp.exe' @arguments 2>$null | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw "SFTP operation failed with exit code $LASTEXITCODE."
  }
}

New-Item -ItemType Directory -Path $tempDir | Out-Null
try {
  $securePassword = Get-Content (Join-Path $autoDir 'sftp.sec') | ConvertTo-SecureString
  $secretPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($securePassword)
  $env:PALCMD_SFTP_PW = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($secretPointer)
  $env:SSH_ASKPASS = Join-Path $autoDir 'askpass.cmd'
  $env:SSH_ASKPASS_REQUIRE = 'force'
  $env:DISPLAY = ':0'

  Invoke-SftpBatch "get $remote $original`nbye`n"
  if (-not (Test-Path $original)) {
    throw 'PalWorldSettings.ini was not downloaded.'
  }

  $text = [IO.File]::ReadAllText($original)
  $deathPattern = '(?<![A-Za-z0-9_])DeathPenalty=([^,\r\n)]*)'
  $eggPattern = '(?<![A-Za-z0-9_])PalEggDefaultHatchingTime=([^,\r\n)]*)'
  $deathMatches = [regex]::Matches($text, $deathPattern)
  $eggMatches = [regex]::Matches($text, $eggPattern)
  if ($deathMatches.Count -ne 1 -or $eggMatches.Count -ne 1) {
    throw 'Expected exactly one death-penalty and one egg-hatching setting.'
  }

  $oldDeathPenalty = $deathMatches[0].Groups[1].Value
  $oldEggHatchHours = $eggMatches[0].Groups[1].Value
  $eggValue = $EggHatchHours.ToString('0.000000', [Globalization.CultureInfo]::InvariantCulture)
  $newText = [regex]::Replace($text, $deathPattern, "DeathPenalty=$DeathPenalty")
  $newText = [regex]::Replace(
    $newText,
    $eggPattern,
    "PalEggDefaultHatchingTime=$eggValue"
  )

  if ([regex]::Matches($newText, "DeathPenalty=$DeathPenalty").Count -ne 1 -or
      [regex]::Matches($newText, "PalEggDefaultHatchingTime=$eggValue").Count -ne 1) {
    throw 'The updated settings failed validation.'
  }
  [IO.File]::WriteAllText($updated, $newText, [Text.UTF8Encoding]::new($false))

  $preview = [pscustomobject]@{
    DeathPenalty = "$oldDeathPenalty -> $DeathPenalty"
    PalEggDefaultHatchingTime = "$oldEggHatchHours -> $eggValue"
    RemoteBackup = $remoteBackup
    Applied = [bool]$Apply
  }
  if (-not $Apply) {
    $preview
    return
  }

  Invoke-SftpBatch "put $original $remoteBackup`nput $updated $remote`nget $remote $verified`nbye`n"
  if (-not (Test-Path $verified)) {
    throw 'The updated server configuration could not be downloaded for verification.'
  }
  if ((Get-FileHash $updated -Algorithm SHA256).Hash -ne
      (Get-FileHash $verified -Algorithm SHA256).Hash) {
    throw 'The uploaded server configuration failed hash verification.'
  }

  $preview
}
finally {
  $env:PALCMD_SFTP_PW = $null
  if ($secretPointer -ne [IntPtr]::Zero) {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($secretPointer)
  }
  $resolvedTemp = [IO.Path]::GetFullPath($tempDir)
  if ($resolvedTemp.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
    Get-ChildItem -LiteralPath $resolvedTemp -File -ErrorAction SilentlyContinue |
      Remove-Item -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $resolvedTemp -Force -ErrorAction SilentlyContinue
  }
}
