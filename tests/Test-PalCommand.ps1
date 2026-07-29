<#
  Test-PalCommand.ps1 - safety checks for the PAL COMMAND stack.

  The point of this suite is one thing: prove that if something goes wrong on the
  server, the guild's progress is recoverable. An untested backup is a guess.

  Run before deploying any mod, and any time you want reassurance:
      .\tests\Test-PalCommand.ps1
      .\tests\Test-PalCommand.ps1 -Deep     # also pulls the live server file list

  Exit code 0 = all good, 1 = something failed.
#>
[CmdletBinding()]
param([switch]$Deep, [switch]$Quiet)

$cfgDir  = Join-Path $env:APPDATA 'com.luibots.palcommand'
$autoDir = Join-Path $cfgDir 'auto'
$script:pass = 0; $script:fail = 0; $script:warn = 0

function Group([string]$n) { if (-not $Quiet) { Write-Host ''; Write-Host "== $n ==" -ForegroundColor Cyan } }
function Ok  ([string]$m) { $script:pass++; if (-not $Quiet) { Write-Host "  [PASS] $m" -ForegroundColor Green } }
function Bad ([string]$m) { $script:fail++; Write-Host "  [FAIL] $m" -ForegroundColor Red }
function Warn([string]$m) { $script:warn++; if (-not $Quiet) { Write-Host "  [WARN] $m" -ForegroundColor Yellow } }
function Info([string]$m) { if (-not $Quiet) { Write-Host "         $m" -ForegroundColor DarkGray } }

Write-Host 'PAL COMMAND - SAFETY TEST SUITE' -ForegroundColor White

# --------------------------------------------------------------- config
Group 'Configuration'
$settingsPath = Join-Path $cfgDir 'settings.json'
if (-not (Test-Path $settingsPath)) { Bad 'settings.json missing - nothing else can be checked'; exit 1 }
try {
  $cfg = (Get-Content $settingsPath -Raw -Encoding UTF8) | ConvertFrom-Json
  Ok 'settings.json parses'
} catch { Bad "settings.json is corrupt: $_"; exit 1 }

foreach ($k in @('sftp_host','sftp_user','save_games_path','repo_local_path')) {
  if ($cfg.$k) { Ok "$k is set" } else { Bad "$k is EMPTY - backups cannot work" }
}
if (Test-Path (Join-Path $autoDir 'sftp.sec'))  { Ok 'SFTP password stored (DPAPI)' }  else { Bad 'SFTP password NOT stored - unattended backup will fail' }
if (Test-Path (Join-Path $autoDir 'admin.sec')) { Ok 'Admin password stored (DPAPI)' } else { Warn 'Admin password not stored - no forced save before backup' }

# --------------------------------------------------------------- backups exist
Group 'Backup snapshots'
$repo = $cfg.repo_local_path
$saves = Join-Path $repo 'saves'
if (-not (Test-Path $saves)) { Bad "no saves/ folder at $saves"; }
$snaps = @(Get-ChildItem $saves -Filter '*.tar.gz' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending)
if ($snaps.Count -eq 0) { Bad 'NO SNAPSHOTS AT ALL - there is nothing to restore from' }
else {
  Ok "$($snaps.Count) snapshot(s) present"
  $newest = $snaps[0]
  $ageH = [math]::Round(((Get-Date) - $newest.LastWriteTime).TotalHours, 1)
  Info "newest: $($newest.Name)  ($ageH h old, $([math]::Round($newest.Length/1MB,2)) MB)"
  if ($ageH -gt 12) { Warn "newest backup is $ageH hours old - scheduled task may not be running" }
  else { Ok "newest backup is recent ($ageH h)" }
  if ($newest.Length -lt 100KB) { Bad "newest snapshot is suspiciously small ($($newest.Length) bytes)" }
  else { Ok 'newest snapshot is a plausible size' }
}

# --------------------------------------------------------------- RESTORE DRY RUN
Group 'Restore dry-run (the one that matters)'
if ($snaps.Count -gt 0) {
  $tmp = Join-Path $env:TEMP ("palcmd-restoretest-" + [guid]::NewGuid().ToString('N').Substring(0,8))
  New-Item -ItemType Directory -Force $tmp | Out-Null
  try {
    & tar.exe -xzf $snaps[0].FullName -C $tmp 2>$null
    if ($LASTEXITCODE -ne 0) { Bad 'snapshot could NOT be extracted - it is not a usable backup' }
    else {
      Ok 'snapshot extracts cleanly'

      $level = Join-Path $tmp 'Level.sav'
      if (-not (Test-Path $level)) { Bad 'Level.sav MISSING from snapshot - world is not recoverable' }
      else {
        $len = (Get-Item $level).Length
        $fs = [IO.File]::OpenRead($level); $buf = New-Object byte[] 12; [void]$fs.Read($buf,0,12); $fs.Close()
        $magic = [Text.Encoding]::ASCII.GetString($buf,8,3)
        if ($magic -match 'Pl') { Ok "Level.sav present and valid (magic '$magic', $([math]::Round($len/1MB,2)) MB)" }
        else { Bad "Level.sav has BAD magic '$magic' - snapshot is corrupt" }
        if ($len -lt 100KB) { Bad "Level.sav is only $len bytes - almost certainly truncated" }
      }

      if (Test-Path (Join-Path $tmp 'LevelMeta.sav')) { Ok 'LevelMeta.sav present' } else { Warn 'LevelMeta.sav missing' }

      $players = @(Get-ChildItem (Join-Path $tmp 'Players') -Filter '*.sav' -ErrorAction SilentlyContinue)
      if ($players.Count -eq 0) { Bad 'NO player saves in snapshot - everyone would lose their character' }
      else {
        Ok "$($players.Count) player save(s) in snapshot"
        $empty = @($players | Where-Object { $_.Length -lt 100 })
        if ($empty.Count -gt 0) { Bad "$($empty.Count) player save(s) are empty/truncated" }
        else { Ok 'all player saves are non-trivial in size' }
      }
    }
  } finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
} else { Bad 'skipped - no snapshot to test' }

# --------------------------------------------------------------- off-site
Group 'Off-site copy'
if (-not $cfg.repo_remote) { Warn 'no git remote configured - backups are LOCAL ONLY (a dead drive loses everything)' }
else {
  Ok "remote configured: $($cfg.repo_remote)"
  Push-Location $repo
  try {
    $ErrorActionPreference = 'Continue'
    $local  = (& git rev-parse HEAD 2>$null)
    & git fetch origin 2>$null | Out-Null
    $remote = (& git rev-parse "origin/$($cfg.git_branch)" 2>$null)
    if ($local -and $remote -and $local -eq $remote) { Ok 'local backups are fully pushed off-site' }
    elseif ($local -and $remote) { Warn 'local repo is AHEAD of remote - newest snapshots are not off-site yet' }
    else { Warn 'could not compare local vs remote' }
    # secrets must never be committed
    $leak = (& git grep -I -l -E 'AdminPassword="[^<]' -- 'config/*' 2>$null)
    if ($leak) { Bad "a real AdminPassword appears in tracked file(s): $leak" }
    else { Ok 'no plaintext AdminPassword in tracked files' }
  } finally { Pop-Location }
}

# --------------------------------------------------------------- mod distribution
Group 'Mod distribution'
$modsRepo = Join-Path (Split-Path $repo -Parent) 'projects\palworld-mods'
if (-not (Test-Path $modsRepo)) { $modsRepo = 'C:\Users\llllllllllllllllllll\projects\palworld-mods' }
$manifestPath = Join-Path $modsRepo 'mods.json'
if (-not (Test-Path $manifestPath)) { Warn 'mods repo not found locally - skipping manifest checks' }
else {
  $bytes = [IO.File]::ReadAllBytes($manifestPath)
  if ($bytes.Length -ge 3 -and $bytes[0] -eq 239 -and $bytes[1] -eq 187 -and $bytes[2] -eq 191) {
    Bad 'mods.json has a UTF-8 BOM - this BREAKS the Mod Manager for every user'
  } else { Ok 'mods.json has no BOM' }
  try {
    $mf = (Get-Content $manifestPath -Raw -Encoding UTF8) | ConvertFrom-Json
    Ok "mods.json parses ($($mf.mods.Count) mod(s))"
    foreach ($m in $mf.mods) {
      if ($m.file) {
        $pak = Join-Path $modsRepo ($m.file -replace '/', '\')
        if (-not (Test-Path $pak)) { Bad "mod '$($m.id)': file missing ($($m.file))"; continue }
        $sha = (Get-FileHash $pak -Algorithm SHA256).Hash.ToLower()
        if ($sha -eq ([string]$m.sha256).ToLower()) { Ok "mod '$($m.id)': sha256 matches manifest" }
        else { Bad "mod '$($m.id)': SHA MISMATCH - the manager will refuse to install it" }
        continue
      }

      if ($m.sourceDir -and $m.files) {
        $badFiles = 0
        foreach ($entry in $m.files) {
          $relative = (Join-Path $m.sourceDir $entry.path) -replace '/', '\'
          $file = Join-Path $modsRepo $relative
          if (-not (Test-Path $file)) {
            Bad "mod '$($m.id)': file missing ($relative)"
            $badFiles++
            continue
          }
          $sha = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
          if ($sha -ne ([string]$entry.sha256).ToLower()) {
            Bad "mod '$($m.id)': SHA MISMATCH ($relative)"
            $badFiles++
          }
        }
        if ($badFiles -eq 0) {
          Ok "mod '$($m.id)': $($m.files.Count) file hash(es) match manifest"
        }
        continue
      }

      Bad "mod '$($m.id)': unsupported manifest entry"
    }
  } catch { Bad "mods.json is invalid JSON: $_" }
}

# live download check
try {
  $r = Invoke-WebRequest -Uri 'https://github.com/luibots/palworld-mods/releases/latest/download/Palworld.Mod.Manager.bat' -UseBasicParsing -TimeoutSec 20 -MaximumRedirection 5
  if ($r.StatusCode -eq 200 -and $r.RawContentLength -gt 100) { Ok 'guild download link is live' }
  else { Warn "download link returned $($r.StatusCode)" }
} catch { Bad "guild download link is BROKEN: $($_.Exception.Message)" }

# --------------------------------------------------------------- live server (optional)
if ($Deep) {
  Group 'Live server'
  try {
    $adminSec = Get-Content (Join-Path $autoDir 'admin.sec') | ConvertTo-SecureString
    $adminPw = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($adminSec))
    $b64 = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("admin:$adminPw"))
    $mx = Invoke-RestMethod -Uri ("{0}/v1/api/metrics" -f $cfg.rest_url) -Headers @{ Authorization = "Basic $b64" } -TimeoutSec 15
    Ok "server reachable - day $($mx.days), $($mx.currentplayernum)/$($mx.maxplayernum) online, $($mx.serverfps) FPS"
    if ($mx.currentplayernum -gt 0) { Warn "$($mx.currentplayernum) player(s) ONLINE - do not restart/deploy right now" }
    else { Ok 'nobody online - safe window for a deploy' }
  } catch { Warn "could not reach the server: $($_.Exception.Message)" }
}

# --------------------------------------------------------------- verdict
Write-Host ''
Write-Host ('=' * 52)
Write-Host ("RESULT: {0} passed, {1} failed, {2} warnings" -f $script:pass, $script:fail, $script:warn) -ForegroundColor White
if ($script:fail -gt 0) {
  Write-Host 'DO NOT DEPLOY - fix the failures above first.' -ForegroundColor Red
  exit 1
}
Write-Host 'Safe: your world is backed up and recoverable.' -ForegroundColor Green
exit 0
