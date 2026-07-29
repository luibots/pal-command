[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$OriginalLevelPath,
  [Parameter(Mandatory)][string]$ModifiedLevelPath,
  [Parameter(Mandatory)][string]$BackupTag
)

$ErrorActionPreference = 'Stop'
$cfgDir = Join-Path $env:APPDATA 'com.luibots.palcommand'
$autoDir = Join-Path $cfgDir 'auto'
$cfg = Get-Content (Join-Path $cfgDir 'settings.json') -Raw | ConvertFrom-Json
$sftpExe = 'C:\Windows\System32\OpenSSH\sftp.exe'

function Read-LocalSecret([string]$Name) {
  $secure = Get-Content -LiteralPath (Join-Path $autoDir "$Name.sec") |
    ConvertTo-SecureString
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

if ($cfg.rest_url) {
  try {
    Invoke-WebRequest -UseBasicParsing `
      -Uri ($cfg.rest_url.TrimEnd('/') + '/v1/api/info') `
      -TimeoutSec 4 -ErrorAction Stop | Out-Null
    throw 'REST is reachable; upload refused.'
  } catch {
    if ($_.Exception.Message -like 'REST is reachable*') { throw }
  }
}
if (Test-TcpPort $cfg.rcon_host ([int]$cfg.rcon_port)) {
  throw 'RCON is reachable; upload refused.'
}

$original = (Resolve-Path -LiteralPath $OriginalLevelPath).Path
$modified = (Resolve-Path -LiteralPath $ModifiedLevelPath).Path
$verifyDir = Join-Path $env:TEMP "palcmd-level-verify-$BackupTag"
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
$batchPath = Join-Path $autoDir "level-upload-$BackupTag.batch"
$batch = @(
  "put `"$($original.Replace('\', '/'))`" $world/Level.sav.bak-palcmd-$BackupTag",
  "put `"$($modified.Replace('\', '/'))`" $world/Level.sav",
  "get $world/Level.sav `"$($verifyDir.Replace('\', '/'))/Level.sav`"",
  'bye'
) -join "`n"
[IO.File]::WriteAllText(
  $batchPath,
  ($batch + "`n"),
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
  $localHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $modified).Hash
  $remoteHash = (
    Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $verifyDir 'Level.sav')
  ).Hash
  if ($localHash -ne $remoteHash) {
    throw 'Remote Level.sav hash verification failed'
  }
  Write-Output 'LEVEL_UPLOAD_AND_REMOTE_HASH_VERIFY_OK'
} finally {
  $env:PALCMD_SFTP_PW = $null
  $password = $null
  Remove-Item -LiteralPath $batchPath -Force -ErrorAction SilentlyContinue
}
