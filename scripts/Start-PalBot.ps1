<#
  Start-PalBot.ps1 - launches the PAL COMMAND Discord bot.

  Decrypts the DPAPI-stored secrets and hands them to the Python bot as environment
  variables, so the bot token never sits on disk in plaintext and never appears in
  the Python source or in any repo.

  Run -Check to validate setup without starting the bot.
#>
[CmdletBinding()]
param(
  [switch]$Check,
  [switch]$Once,
  [int]$RestartDelaySeconds = 10
)

$ErrorActionPreference = 'Stop'
$autoDir = Join-Path $env:APPDATA 'com.luibots.palcommand\auto'
$botPy   = Join-Path $PSScriptRoot 'palcommand-bot.py'

function Read-Sec([string]$name) {
  $p = Join-Path $autoDir "$name.sec"
  if (-not (Test-Path $p)) { return $null }
  $sec = Get-Content $p | ConvertTo-SecureString
  $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec)
  try {
    [Runtime.InteropServices.Marshal]::PtrToStringAuto($bstr)
  }
  finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
  }
}

$token = Read-Sec 'discord_token'
$chanFile = Join-Path $autoDir 'discord_channel.txt'
$chan = if (Test-Path $chanFile) { (Get-Content $chanFile -Raw).Trim() } else { '' }
$adminPw = Read-Sec 'admin'

if ($Check) {
  Write-Host '=== PAL BOT - SETUP CHECK ==='
  Write-Host ("  bot script     : {0}" -f $(if (Test-Path $botPy) { 'found' } else { 'MISSING' }))
  Write-Host ("  discord token  : {0}" -f $(if ($token)   { 'stored' } else { 'NOT SET - run palcommand-backup.ps1 -SetupDiscord' }))
  Write-Host ("  alert channel  : {0}" -f $(if ($chan)    { $chan }   else { 'NOT SET' }))
  Write-Host ("  admin password : {0}" -f $(if ($adminPw) { 'stored' } else { 'NOT SET' }))
  $py = (Get-Command python -ErrorAction SilentlyContinue)
  Write-Host ("  python         : {0}" -f $(if ($py) { $py.Source } else { 'NOT FOUND' }))
  if ($py) {
    $v = & python -c "import discord; print(discord.__version__)" 2>$null
    Write-Host ("  discord.py     : {0}" -f $(if ($v) { $v } else { 'NOT INSTALLED - run: python -m pip install discord.py' }))
  }
  return
}

if (-not $token) { throw 'No Discord token stored. Run: palcommand-backup.ps1 -SetupDiscord -DiscordToken <token> -DiscordChannelId <id>' }
if (-not (Test-Path $botPy)) { throw "Bot script not found at $botPy" }

$env:PALCMD_DISCORD_TOKEN   = $token
$env:PALCMD_DISCORD_CHANNEL = $chan
$env:PALCMD_ADMIN_PW        = $adminPw
$env:PALCMD_MODS_REPO       = 'C:\Users\llllllllllllllllllll\projects\palworld-mods'

$log = Join-Path $autoDir 'bot.log'
$mutex = [Threading.Mutex]::new($false, 'Local\PalCommandDiscordBot')
$ownsMutex = $false
try {
  try {
    $ownsMutex = $mutex.WaitOne(0, $false)
  }
  catch [Threading.AbandonedMutexException] {
    $ownsMutex = $true
  }
  if (-not $ownsMutex) {
    Write-Host 'PAL COMMAND Discord bot is already supervised by another process.'
    return
  }

  Write-Host "Starting PAL COMMAND Discord bot supervisor... (log: $log)"
  do {
    ("[{0}] --- bot starting ---" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')) |
      Add-Content -Path $log -Encoding utf8

    # discord.py logs to stderr. Native stderr becomes a non-terminating
    # NativeCommandError in Windows PowerShell, so keep Continue while the child runs.
    $ErrorActionPreference = 'Continue'
    & python $botPy *>&1 | Out-File -FilePath $log -Append -Encoding utf8
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'

    ("[{0}] --- bot exited ({1}); {2} ---" -f (
      Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
    ), $exitCode, $(if ($Once) { 'supervisor stopping' } else { 'restarting' })) |
      Add-Content -Path $log -Encoding utf8

    if (-not $Once) {
      Start-Sleep -Seconds ([Math]::Max(2, $RestartDelaySeconds))
    }
  } while (-not $Once)
}
finally {
  if ($ownsMutex) {
    $mutex.ReleaseMutex()
  }
  $mutex.Dispose()
}
