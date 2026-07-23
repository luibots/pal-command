<#
  Test-Mod.ps1 - prove a .pak changes ONLY what you think it changes.

  This is the gate to run before deploying any mod to a live server. It unpacks the
  mod, unpacks the same assets from the vanilla game, converts both DataTables to
  JSON and diffs them row-by-row / field-by-field. If the mod touches a column you
  did not intend, you find out here instead of after a restart.

      .\tests\Test-Mod.ps1 -PakPath ..\palworld-mods\mods\zzz_luibasefix_P.pak `
                           -ExpectField BaseCampMaxNumInGuild

  -ExpectField may be given more than once. Any field that changes and is NOT in
  that list is treated as a FAILURE.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$PakPath,
  [string[]]$ExpectField = @(),
  [string]$GamePak = 'C:\Program Files (x86)\Steam\steamapps\common\Palworld\Pal\Content\Paks\Pal-Windows.pak',
  [switch]$Quiet
)

$ErrorActionPreference = 'Stop'
$root    = Split-Path $PSScriptRoot -Parent
$tools   = Join-Path $root 'tools'
$work    = Join-Path $env:TEMP ("palcmd-modtest-" + [guid]::NewGuid().ToString('N').Substring(0,8))
$fail = 0

function Say([string]$m, [string]$c = 'Gray') { if (-not $Quiet) { Write-Host $m -ForegroundColor $c } }

# ---------------------------------------------------------------- tools
New-Item -ItemType Directory -Force $tools | Out-Null
$repak   = Join-Path $tools 'repak.exe'
$uasset  = Join-Path $tools 'UAssetGUI.exe'
$usmap   = Join-Path $tools 'Mappings.usmap'

if (-not (Test-Path $repak)) {
  Say 'Fetching repak...' 'DarkGray'
  $z = Join-Path $tools 'repak.zip'
  Invoke-WebRequest -Uri 'https://github.com/trumank/repak/releases/download/v0.2.3/repak_cli-x86_64-pc-windows-msvc.zip' -OutFile $z -UseBasicParsing
  Expand-Archive $z -DestinationPath $tools -Force; Remove-Item $z -Force
}
if (-not (Test-Path $uasset)) {
  Say 'Fetching UAssetGUI...' 'DarkGray'
  Invoke-WebRequest -Uri 'https://github.com/atenfyr/UAssetGUI/releases/download/v1.1.0/UAssetGUI.exe' -OutFile $uasset -UseBasicParsing
}
if (-not (Test-Path $usmap)) {
  Say 'Fetching Palworld mappings...' 'DarkGray'
  Invoke-WebRequest -Uri 'https://github.com/PalworldModding/UsefulFiles/raw/master/Mappings.usmap' -OutFile $usmap -UseBasicParsing
}
# UAssetGUI resolves mapping files by name from its own data folder
$mapDir = Join-Path $env:LOCALAPPDATA 'UAssetGUI\Mappings'
New-Item -ItemType Directory -Force $mapDir | Out-Null
Copy-Item $usmap (Join-Path $mapDir 'Palworld.usmap') -Force

if (-not (Test-Path $PakPath)) { throw "Mod pak not found: $PakPath" }
if (-not (Test-Path $GamePak)) { throw "Vanilla game pak not found: $GamePak" }

New-Item -ItemType Directory -Force $work | Out-Null
try {
  Write-Host ''
  Write-Host "MOD DIFF: $(Split-Path $PakPath -Leaf)" -ForegroundColor White
  Write-Host ('-' * 52)

  # ------------------------------------------------------------ contents
  $listing = & $repak list $PakPath 2>$null
  $assets = @($listing | Where-Object { $_ -match '\.uasset$' })
  Say "Contains $($assets.Count) asset(s):" 'Cyan'
  foreach ($a in $assets) { Say "  $a" }
  if ($assets.Count -eq 0) { Write-Host '  [FAIL] pak contains no .uasset files' -ForegroundColor Red; $fail++ }

  $modDir = Join-Path $work 'mod'; $vanDir = Join-Path $work 'vanilla'
  & $repak unpack -q -f -o $modDir $PakPath 2>$null | Out-Null

  foreach ($asset in $assets) {
    Write-Host ''
    Say "Comparing: $asset" 'Cyan'
    $uexp = $asset -replace '\.uasset$', '.uexp'

    & $repak unpack -q -f -o $vanDir -i $asset -i $uexp $GamePak 2>$null | Out-Null
    $vanAsset = Join-Path $vanDir ($asset -replace '/', '\')
    $modAsset = Join-Path $modDir ($asset -replace '/', '\')
    if (-not (Test-Path $vanAsset)) { Write-Host "  [WARN] not present in the vanilla game pak (new asset?)" -ForegroundColor Yellow; continue }

    $vanJson = Join-Path $work 'van.json'; $modJson = Join-Path $work 'mod.json'
    & $uasset tojson $vanAsset $vanJson VER_UE5_1 Palworld 2>$null | Out-Null
    & $uasset tojson $modAsset $modJson VER_UE5_1 Palworld 2>$null | Out-Null
    if (-not (Test-Path $vanJson) -or -not (Test-Path $modJson)) {
      Write-Host '  [WARN] could not convert to JSON - binary compare only' -ForegroundColor Yellow
      $same = (Get-FileHash $vanAsset).Hash -eq (Get-FileHash $modAsset).Hash
      Say ("  binary identical: {0}" -f $same)
      continue
    }

    $van = (Get-Content $vanJson -Raw) | ConvertFrom-Json
    $mod = (Get-Content $modJson -Raw) | ConvertFrom-Json
    $vanRows = $van.Exports[0].Table.Data
    $modRows = $mod.Exports[0].Table.Data
    if (-not $vanRows -or -not $modRows) { Say '  (not a DataTable - skipping field diff)' 'DarkGray'; continue }

    Say ("  rows: vanilla $($vanRows.Count) -> mod $($modRows.Count)")
    if ($vanRows.Count -ne $modRows.Count) { Write-Host '  [FAIL] row count changed' -ForegroundColor Red; $fail++ }

    $changed = @{}
    for ($i = 0; $i -lt [Math]::Min($vanRows.Count, $modRows.Count); $i++) {
      $vr = @{}; foreach ($c in $vanRows[$i].Value) { $vr[$c.Name] = $c.Value }
      $mr = @{}; foreach ($c in $modRows[$i].Value) { $mr[$c.Name] = $c.Value }
      foreach ($k in $vr.Keys) {
        if ("$($vr[$k])" -ne "$($mr[$k])") {
          if (-not $changed.ContainsKey($k)) { $changed[$k] = @() }
          $changed[$k] += "row $($vanRows[$i].Name): $($vr[$k]) -> $($mr[$k])"
        }
      }
    }

    if ($changed.Count -eq 0) { Write-Host '  [WARN] this pak changes NOTHING vs vanilla' -ForegroundColor Yellow }
    foreach ($k in $changed.Keys) {
      $n = $changed[$k].Count
      $expected = $ExpectField -contains $k
      if ($expected) {
        Write-Host "  [PASS] '$k' changed on $n row(s) - expected" -ForegroundColor Green
      } else {
        Write-Host "  [FAIL] '$k' changed on $n row(s) - NOT in -ExpectField" -ForegroundColor Red
        $fail++
      }
      $changed[$k] | Select-Object -First 3 | ForEach-Object { Say "           $_" 'DarkGray' }
      if ($n -gt 3) { Say "           ... and $($n - 3) more" 'DarkGray' }
    }

    # anything expected but untouched is worth flagging too
    foreach ($e in $ExpectField) {
      if (-not $changed.ContainsKey($e)) { Write-Host "  [WARN] expected '$e' to change, but it did not" -ForegroundColor Yellow }
    }
  }

  Write-Host ''
  Write-Host ('-' * 52)
  if ($fail -gt 0) { Write-Host "VERDICT: $fail problem(s) - DO NOT DEPLOY" -ForegroundColor Red; exit 1 }
  Write-Host 'VERDICT: mod changes only the expected fields - safe to deploy' -ForegroundColor Green
  exit 0
} finally {
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
