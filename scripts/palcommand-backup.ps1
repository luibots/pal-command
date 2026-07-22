<#
  palcommand-backup.ps1 - unattended Palworld server backup.

  Runs the same pipeline as the PAL COMMAND app, standalone, for Windows Task Scheduler:
    REST force-save -> SFTP pull (Level.sav + players + configs) -> integrity check
    -> redact secrets from configs -> tar.gz snapshot -> retention prune -> git commit + push.

  Config is read from the app's settings.json (single source of truth).
  Secrets are read from DPAPI-encrypted files (only this user on this machine can decrypt).
  No passwords live in this script.

  Set up secrets once with:  palcommand-backup.ps1 -SetupSecrets -SftpPassword x -AdminPassword y
#>
[CmdletBinding()]
param(
  [switch]$SetupSecrets,
  [string]$SftpPassword,
  [string]$AdminPassword,
  [switch]$SetupDiscord,
  [string]$DiscordToken,
  [string]$DiscordChannelId,
  [switch]$TestDiscord,
  [switch]$Quiet
)

$ErrorActionPreference = 'Stop'
$cfgDir       = Join-Path $env:APPDATA 'com.luibots.palcommand'
$autoDir      = Join-Path $cfgDir 'auto'
$settingsPath = Join-Path $cfgDir 'settings.json'
$logPath      = Join-Path $autoDir 'backup.log'
$sftpExe      = 'C:\Windows\System32\OpenSSH\sftp.exe'

New-Item -ItemType Directory -Force $autoDir | Out-Null

function Log([string]$m) {
  $line = ('[{0}] {1}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $m)
  Add-Content -Path $logPath -Value $line -Encoding utf8
  if (-not $Quiet) { Write-Host $line }
}

# One-time secret setup (DPAPI)
if ($SetupSecrets) {
  if (-not $SftpPassword -or -not $AdminPassword) { throw 'Provide -SftpPassword and -AdminPassword.' }
  ($SftpPassword  | ConvertTo-SecureString -AsPlainText -Force | ConvertFrom-SecureString) | Out-File (Join-Path $autoDir 'sftp.sec')  -Encoding ascii
  ($AdminPassword | ConvertTo-SecureString -AsPlainText -Force | ConvertFrom-SecureString) | Out-File (Join-Path $autoDir 'admin.sec') -Encoding ascii
  Log 'Secrets stored (DPAPI, user+machine bound).'
  return
}

function Read-Secret([string]$name) {
  $p = Join-Path $autoDir "$name.sec"
  if (-not (Test-Path $p)) { throw "Missing secret '$name'. Run with -SetupSecrets first." }
  $sec = Get-Content $p | ConvertTo-SecureString
  [Runtime.InteropServices.Marshal]::PtrToStringAuto(
    [Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec))
}

# One-time Discord setup (DPAPI). The bot token is a credential - same treatment as the rest.
if ($SetupDiscord) {
  if (-not $DiscordToken -or -not $DiscordChannelId) { throw 'Provide -DiscordToken and -DiscordChannelId.' }
  ($DiscordToken | ConvertTo-SecureString -AsPlainText -Force | ConvertFrom-SecureString) | Out-File (Join-Path $autoDir 'discord_token.sec') -Encoding ascii
  $DiscordChannelId.Trim() | Out-File (Join-Path $autoDir 'discord_channel.txt') -Encoding ascii
  Log 'Discord token stored (DPAPI) and alert channel set.'
  return
}

# Post an embed to the alert channel. Works whether or not the bot process is running.
function Send-DiscordAlert([string]$title, [string]$desc, [int]$colour, $fields) {
  $tokenFile = Join-Path $autoDir 'discord_token.sec'
  $chanFile  = Join-Path $autoDir 'discord_channel.txt'
  if (-not (Test-Path $tokenFile) -or -not (Test-Path $chanFile)) { return }
  try {
    $sec = Get-Content $tokenFile | ConvertTo-SecureString
    $tok = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
             [Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec))
    $chan = (Get-Content $chanFile -Raw).Trim()
    $embed = @{
      title       = $title
      description = $desc
      color       = $colour
      timestamp   = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
      footer      = @{ text = 'PAL COMMAND' }
    }
    if ($fields) { $embed['fields'] = @($fields) }
    $json  = (@{ embeds = @($embed) } | ConvertTo-Json -Depth 8 -Compress)
    $bytes = [Text.Encoding]::UTF8.GetBytes($json)
    # Discord REJECTS bot calls to guild/channel endpoints unless a proper DiscordBot
    # User-Agent is sent, and reports it as the very misleading 40333 "internal network error".
    $headers = @{
      Authorization = "Bot $tok"
      'User-Agent'  = 'DiscordBot (https://github.com/luibots/pal-command, 0.1)'
    }
    for ($try = 1; $try -le 3; $try++) {
      try {
        Invoke-RestMethod -Uri "https://discord.com/api/v10/channels/$chan/messages" -Method Post `
          -Headers $headers -ContentType 'application/json' -Body $bytes -TimeoutSec 15 | Out-Null
        return $true
      } catch {
        if ($try -eq 3) { Log "Discord alert failed after 3 tries: $_"; return $false }
        Start-Sleep -Seconds ($try * 2)
      }
    }
  } catch { Log "Discord alert failed: $_"; return $false }
  return $false
}

if ($TestDiscord) {
  $tokenFile = Join-Path $autoDir 'discord_token.sec'
  $chanFile  = Join-Path $autoDir 'discord_channel.txt'
  if (-not (Test-Path $tokenFile) -or -not (Test-Path $chanFile)) {
    Log 'Discord is NOT set up yet - nothing was sent. Run -SetupDiscord with your bot token and channel id first.'
    return
  }
  $sent = Send-DiscordAlert 'PAL COMMAND connected' 'Alerts are wired up. You will get a message here when a backup runs, when the server goes down, and when a new mod is published.' 16098596 $null
  if ($sent) { Log 'Test alert SENT - check your Discord channel.' }
  else       { Log 'Test alert FAILED - see the error above. Nothing was posted.' }
  return
}

try {
  # Load config + secrets
  $cfg = (Get-Content $settingsPath -Raw -Encoding UTF8) | ConvertFrom-Json
  $sftpPw  = Read-Secret 'sftp'
  $adminPw = Read-Secret 'admin'
  $repo    = $cfg.repo_local_path
  if (-not (Test-Path $repo)) { throw "Backup repo folder not found: $repo" }
  $ts = Get-Date -Format 'yyyyMMdd-HHmmss'
  Log "=== backup start ($ts) ==="

  # 1. Force a clean save via REST
  if ($cfg.rest_url) {
    try {
      $b64 = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("admin:$adminPw"))
      Invoke-RestMethod -Uri ("{0}/v1/api/save" -f $cfg.rest_url) -Method Post -Headers @{ Authorization = "Basic $b64" } -Body '{}' -ContentType 'application/json' -TimeoutSec 15 | Out-Null
      Log 'REST save ok; waiting 8s for flush'
      Start-Sleep -Seconds 8
    } catch { Log "REST save failed ($_) - pulling anyway" }
  }

  # 2. SFTP pull
  $askpass = Join-Path $autoDir 'askpass.cmd'
  "@echo off`r`necho %PALCMD_SFTP_PW%" | Out-File $askpass -Encoding ascii
  $env:SSH_ASKPASS = $askpass; $env:SSH_ASKPASS_REQUIRE = 'force'; $env:DISPLAY = ':0'; $env:PALCMD_SFTP_PW = $sftpPw
  $sftpOpts = @(
    '-oBatchMode=no','-oStrictHostKeyChecking=accept-new',
    ('-oUserKnownHostsFile=' + (Join-Path $autoDir 'known_hosts')),
    '-oConnectTimeout=20','-oPubkeyAuthentication=no',
    '-P', "$($cfg.sftp_port)"
  )
  $sftpTarget = "$($cfg.sftp_user)@$($cfg.sftp_host)"
  # Pass the batch as a FILE, not stdin: PowerShell prepends a BOM to native-command
  # stdin, which sftp reads as part of the first command and rejects ("Invalid command.").
  function Invoke-Sftp([string]$batch) {
    $bf = Join-Path $autoDir ('batch-' + [guid]::NewGuid().ToString('N') + '.txt')
    [IO.File]::WriteAllText($bf, $batch, (New-Object System.Text.ASCIIEncoding))
    try { & $sftpExe @sftpOpts '-b' $bf $sftpTarget 2>$null }
    finally { Remove-Item $bf -Force -ErrorAction SilentlyContinue }
  }

  $stage = Join-Path $env:TEMP ("palcmd-stage-" + $ts)
  New-Item -ItemType Directory -Force (Join-Path $stage 'Players') | Out-Null

  # find the world folder (single folder without a dot)
  $sgPath = $cfg.save_games_path
  $lsWorld = Invoke-Sftp "ls -1 $sgPath`nbye`n"
  # Real listing entries are printed as full paths; sftp's echoed "sftp> ls ..." lines are not.
  $wPrefix = "$sgPath/"
  $world = ($lsWorld -split "`n" | ForEach-Object { $_.Trim() } |
            Where-Object { $_.StartsWith($wPrefix) } |
            ForEach-Object { $_.Substring($wPrefix.Length) } |
            Where-Object { $_ -and $_ -notmatch '\.' } | Select-Object -First 1)
  if (-not $world) { $world = 'SaveGame' }
  $wbase = "$sgPath/$world"
  Log "world folder: $world"

  # enumerate player saves
  $lsPlayers = Invoke-Sftp "ls -1 $wbase/Players`nbye`n"
  $pPrefix = "$wbase/Players/"
  $players = @($lsPlayers -split "`n" | ForEach-Object { $_.Trim() } |
               Where-Object { $_.StartsWith($pPrefix) } |
               ForEach-Object { $_.Substring($pPrefix.Length) } |
               Where-Object { $_ -match '\.sav$' })

  # Enumerate the top-level world .sav files. WorldOption.sav is absent on some servers,
  # and a single missing file aborts the whole sftp batch - so only request what exists.
  $lsWorldFiles = Invoke-Sftp "ls -1 $wbase`nbye`n"
  $fPrefix = "$wbase/"
  $worldFiles = @($lsWorldFiles -split "`n" | ForEach-Object { $_.Trim() } |
                  Where-Object { $_.StartsWith($fPrefix) } |
                  ForEach-Object { $_.Substring($fPrefix.Length) } |
                  Where-Object { $_ -match '\.sav$' })
  Log ("world files: " + ($worldFiles -join ', '))

  # pull everything in one batch
  $b = New-Object System.Text.StringBuilder
  foreach ($f in $worldFiles) { [void]$b.AppendLine("get $wbase/$f $stage/$f") }
  foreach ($p in $players) { [void]$b.AppendLine("get $wbase/Players/$p $stage/Players/$p") }
  [void]$b.AppendLine("get $($cfg.config_dir)/PalWorldSettings.ini $stage/PalWorldSettings.ini")
  [void]$b.AppendLine("get $($cfg.config_dir)/GameUserSettings.ini $stage/GameUserSettings.ini")
  [void]$b.AppendLine("bye")
  Invoke-Sftp ($b.ToString()) | Out-Null
  $env:PALCMD_SFTP_PW = $null

  if (-not (Test-Path "$stage/Level.sav")) { throw 'Level.sav not pulled - aborting (nothing overwritten).' }

  # 3. Integrity check (Palworld magic near offset 8 contains "Pl")
  $fsr = [IO.File]::OpenRead("$stage/Level.sav"); $buf = New-Object byte[] 12; [void]$fsr.Read($buf,0,12); $fsr.Close()
  $magic = [Text.Encoding]::ASCII.GetString($buf,8,3)
  if ($magic -notmatch 'Pl') { throw "Level.sav failed integrity check (magic '$magic') - refusing to store a torn save." }
  Log ("Level.sav ok ({0} MB, magic {1})" -f [math]::Round((Get-Item "$stage/Level.sav").Length/1MB,2), $magic)

  # 4. Redact secrets from configs
  $configOut = Join-Path $repo 'config'; New-Item -ItemType Directory -Force $configOut | Out-Null
  $sidecar = @('# PAL COMMAND real secrets - git-ignored, needed for restore','[PalWorldSettings.ini]')
  foreach ($ini in @('PalWorldSettings.ini','GameUserSettings.ini')) {
    $src = Join-Path $stage $ini
    if (-not (Test-Path $src)) { continue }
    $text = Get-Content $src -Raw
    if ($ini -eq 'PalWorldSettings.ini') {
      foreach ($key in @('AdminPassword','ServerPassword')) {
        $m = [regex]::Match($text, ($key + '="([^"]*)"'))
        if ($m.Success -and $m.Groups[1].Value -and $m.Groups[1].Value -ne '<REDACTED>') {
          $sidecar += ('{0}={1}' -f $key, $m.Groups[1].Value)
          $text = $text -replace ($key + '="[^"]*"'), ($key + '="<REDACTED>"')
        }
      }
    }
    Set-Content -Path (Join-Path $configOut $ini) -Value $text -Encoding utf8
  }
  Set-Content -Path (Join-Path $configOut 'secrets.local.ini') -Value ($sidecar -join "`n") -Encoding utf8

  # 5. tar.gz snapshot (matches the app's saves/ format)
  $savesOut = Join-Path $repo 'saves'; New-Item -ItemType Directory -Force $savesOut | Out-Null
  $short = if ($world.Length -ge 8) { $world.Substring(0,8) } else { $world }
  $archive = Join-Path $savesOut ("{0}_{1}.tar.gz" -f $ts, $short)
  & tar.exe -czf $archive -C $stage --exclude='PalWorldSettings.ini' --exclude='GameUserSettings.ini' . 2>$null
  if (-not (Test-Path $archive)) { throw 'tar failed - no archive produced.' }
  Log ("snapshot {0} ({1} MB)" -f (Split-Path $archive -Leaf), [math]::Round((Get-Item $archive).Length/1MB,2))
  Remove-Item -Recurse -Force $stage

  # 6. Retention prune
  $keep = [int]$cfg.backup_retention; if ($keep -lt 1) { $keep = 20 }
  Get-ChildItem $savesOut -Filter '*.tar.gz' | Sort-Object LastWriteTime -Descending |
    Select-Object -Skip $keep | ForEach-Object { Remove-Item $_.FullName -Force; Log "pruned $($_.Name)" }

  # 7. git commit + push
  $gi = Join-Path $repo '.gitignore'
  if (-not (Test-Path $gi)) { "*.tmp`nsecrets.local.ini`n*.local.ini`nbackup/`n" | Out-File $gi -Encoding ascii }
  Push-Location $repo
  & git add -A 2>$null | Out-Null
  $day = ''
  try {
    $b64 = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("admin:$adminPw"))
    $mx = Invoke-RestMethod -Uri ("{0}/v1/api/metrics" -f $cfg.rest_url) -Headers @{ Authorization = "Basic $b64" } -TimeoutSec 10
    $day = " day-$($mx.days)"
  } catch {}
  & git -c user.email='palcommand@local' -c user.name='PAL COMMAND (auto)' commit -q -m "auto backup ${ts}${day} - $($players.Count) players" 2>$null | Out-Null
  $committed = $LASTEXITCODE -eq 0
  $pushed = $false
  if ($committed -and $cfg.repo_remote) {
    & git push -q origin $cfg.git_branch 2>$null | Out-Null
    $pushed = $LASTEXITCODE -eq 0
  }
  Pop-Location
  Log ("git: committed={0} pushed={1}" -f $committed, $pushed)

  $sizeMb = [math]::Round((Get-Item $archive).Length / 1MB, 2)
  $null = Send-DiscordAlert 'Backup complete' `
    ("World saved and{0} stored." -f $(if ($pushed) { ' pushed off-site' } else { ' committed locally' })) `
    2278750 `
    @(
      @{ name = 'When';      value = (Get-Date -Format 'yyyy-MM-dd HH:mm'); inline = $true },
      @{ name = 'World';     value = ("{0}{1}" -f $world, $day);            inline = $true },
      @{ name = 'Size';      value = "$sizeMb MB";                          inline = $true },
      @{ name = 'Players';   value = "$($players.Count) saves";             inline = $true },
      @{ name = 'Off-site';  value = $(if ($pushed) { 'Yes' } else { 'No' }); inline = $true }
    )
  Log '=== backup done ==='
}
catch {
  Log "ERROR: $_"
  $null = Send-DiscordAlert 'BACKUP FAILED' `
    ("The scheduled Palworld backup did not complete.`n``````$_``````") `
    15680580 `
    @(
      @{ name = 'When'; value = (Get-Date -Format 'yyyy-MM-dd HH:mm'); inline = $true },
      @{ name = 'What to do'; value = 'Check the log at %APPDATA%\com.luibots.palcommand\auto\backup.log'; inline = $false }
    )
  exit 1
}
