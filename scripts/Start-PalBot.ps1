<#
  Start-PalBot.ps1 - launches the PAL COMMAND Discord bot.

  Decrypts the DPAPI-stored secrets and hands them to the Python bot as environment
  variables, so the bot token never sits on disk in plaintext and never appears in
  the Python source or in any repo.

  Run -Check to validate setup without starting the bot.
#>
[CmdletBinding()]
param([switch]$Check)

$ErrorActionPreference = 'Stop'
$autoDir = Join-Path $env:APPDATA 'com.luibots.palcommand\auto'
$botPy   = Join-Path $PSScriptRoot 'palcommand-bot.py'

function Read-Sec([string]$name) {
  $p = Join-Path $autoDir "$name.sec"
  if (-not (Test-Path $p)) { return $null }
  $sec = Get-Content $p | ConvertTo-SecureString
  [Runtime.InteropServices.Marshal]::PtrToStringAuto(
    [Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec))
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
Write-Host "Starting PAL COMMAND Discord bot... (log: $log)"
("[{0}] --- bot starting ---" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')) | Add-Content -Path $log -Encoding utf8

# discord.py logs to stderr. Under $ErrorActionPreference='Stop', piping native stderr
# with 2>&1 raises NativeCommandError and kills the launcher - so drop to Continue and
# redirect every stream straight to the log file instead of through a pipeline.
$ErrorActionPreference = 'Continue'
# (*>> writes UTF-16 in PS 5.1, which makes the log unreadable - pipe to Out-File as UTF-8.
#  Safe to use a pipeline here now that ErrorActionPreference is Continue.)
& python $botPy *>&1 | Out-File -FilePath $log -Append -Encoding utf8
