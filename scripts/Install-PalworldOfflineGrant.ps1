[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$OriginalSaveDir,
  [Parameter(Mandatory)][string]$ModifiedSaveDir,
  [Parameter(Mandatory)][string]$PlayerSaveName,
  [Parameter(Mandatory)][string]$BackupTag
)

$ErrorActionPreference = 'Stop'
$cfgDir = Join-Path $env:APPDATA 'com.luibots.palcommand'
$autoDir = Join-Path $cfgDir 'auto'
$cfg = Get-Content (Join-Path $cfgDir 'settings.json') -Raw | ConvertFrom-Json
$sftpExe = 'C:\Windows\System32\OpenSSH\sftp.exe'

function Read-LocalSecret([string]$Name) {
  $secretPath = Join-Path $autoDir "$Name.sec"
  if (-not (Test-Path -LiteralPath $secretPath)) {
    throw "Missing local credential: $Name"
  }
  $secure = Get-Content -LiteralPath $secretPath | ConvertTo-SecureString
  $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
  try {
    [Runtime.InteropServices.Marshal]::PtrToStringAuto($pointer)
  } finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer)
  }
}

function Test-TcpPort([string]$HostName, [int]$Port) {
  if (-not $HostName -or -not $Port) { return $false }
  $client = [Net.Sockets.TcpClient]::new()
  try {
    $pending = $client.BeginConnect($HostName, $Port, $null, $null)
    if (-not $pending.AsyncWaitHandle.WaitOne(3000)) { return $false }
    $client.EndConnect($pending)
    return $true
  } catch {
    return $false
  } finally {
    $client.Dispose()
  }
}

# Refuse to write while either Palworld administration channel is accepting connections.
if ($cfg.rest_url) {
  try {
    Invoke-WebRequest -UseBasicParsing -Uri ($cfg.rest_url.TrimEnd('/') + '/v1/api/info') `
      -TimeoutSec 4 -ErrorAction Stop | Out-Null
    throw 'REST is reachable; the server may still be running. Upload refused.'
  } catch {
    if ($_.Exception.Message -like 'REST is reachable*') { throw }
  }
}
if (Test-TcpPort $cfg.rcon_host ([int]$cfg.rcon_port)) {
  throw 'RCON is reachable; the server may still be running. Upload refused.'
}

$originalRoot = (Resolve-Path -LiteralPath $OriginalSaveDir).Path
$modifiedRoot = (Resolve-Path -LiteralPath $ModifiedSaveDir).Path
$originalLevel = Join-Path $originalRoot 'Level.sav'
$modifiedLevel = Join-Path $modifiedRoot 'Level.sav'
$originalPlayer = Join-Path (Join-Path $originalRoot 'Players') $PlayerSaveName
$modifiedPlayer = Join-Path (Join-Path $modifiedRoot 'Players') $PlayerSaveName
foreach ($path in @($originalLevel, $modifiedLevel, $originalPlayer, $modifiedPlayer)) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Required save file is missing: $path"
  }
}

$verifyDir = Join-Path $env:TEMP "palcmd-remote-verify-$BackupTag"
if (Test-Path -LiteralPath $verifyDir) {
  throw "Verification directory already exists: $verifyDir"
}
New-Item -ItemType Directory -Path $verifyDir | Out-Null

$password = Read-LocalSecret 'sftp'
$askpass = Join-Path $autoDir 'askpass.cmd'
$env:SSH_ASKPASS = $askpass
$env:SSH_ASKPASS_REQUIRE = 'force'
$env:DISPLAY = ':0'
$env:PALCMD_SFTP_PW = $password

$world = "$($cfg.save_games_path)/SaveGame"
$remotePlayer = "$world/Players/$PlayerSaveName"
$batchLines = @(
  "put `"$($originalLevel.Replace('\', '/'))`" $world/Level.sav.bak-palcmd-$BackupTag",
  "put `"$($originalPlayer.Replace('\', '/'))`" $remotePlayer.bak-palcmd-$BackupTag",
  "put `"$($modifiedPlayer.Replace('\', '/'))`" $remotePlayer",
  "put `"$($modifiedLevel.Replace('\', '/'))`" $world/Level.sav",
  "get $remotePlayer `"$($verifyDir.Replace('\', '/'))/$PlayerSaveName`"",
  "get $world/Level.sav `"$($verifyDir.Replace('\', '/'))/Level.sav`"",
  'bye'
)
$batchPath = Join-Path $autoDir "grant-upload-$BackupTag.batch"
[IO.File]::WriteAllText(
  $batchPath,
  (($batchLines -join "`n") + "`n"),
  [Text.ASCIIEncoding]::new()
)

try {
  $options = @(
    '-oBatchMode=no',
    '-oStrictHostKeyChecking=accept-new',
    ('-oUserKnownHostsFile=' + (Join-Path $autoDir 'known_hosts')),
    '-oConnectTimeout=20',
    '-oPubkeyAuthentication=no',
    '-P', "$($cfg.sftp_port)",
    '-b', $batchPath,
    "$($cfg.sftp_user)@$($cfg.sftp_host)"
  )
  & $sftpExe @options 2>$null | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw "SFTP upload failed with exit code $LASTEXITCODE"
  }

  $checks = @(
    @($modifiedLevel, (Join-Path $verifyDir 'Level.sav')),
    @($modifiedPlayer, (Join-Path $verifyDir $PlayerSaveName))
  )
  foreach ($pair in $checks) {
    $localHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $pair[0]).Hash
    $remoteHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $pair[1]).Hash
    if ($localHash -ne $remoteHash) {
      throw "Remote hash verification failed for $($pair[1])"
    }
  }
  Write-Output 'UPLOAD_AND_REMOTE_HASH_VERIFY_OK'
} finally {
  $env:PALCMD_SFTP_PW = $null
  $password = $null
  Remove-Item -LiteralPath $batchPath -Force -ErrorAction SilentlyContinue
}
