<#
D18: builds the fake Steam library and player files that the uninstall test
starts from (PLAN-D18-UNINSTALL.md section 2). It WRITES files, so it runs
only in a throwaway Windows: it refuses unless the user is Windows Sandbox's
WDAGUtilityAccount or -Disposable is given (for a throwaway VM).
No real game files, keys or Vortex data are used or touched.

- Same bytes every run: fixed contents (UTF-8, CRLF) and fixed file times.
- Built in "<Root>.partial" and renamed to <Root> only when complete, so an
  interrupted run never leaves a folder that looks finished. Running it again
  rebuilds a leftover ".partial" (only one this script made), and does nothing
  when <Root> already matches.
- Player files are written only where none exist or they are already the
  fixture's own bytes. It never overwrites someone's real plugins.txt.
Exit 0: fixture ready. Anything else: refused or failed, and nothing is half-made.
#>
param(
  [string]$Root = "C:\ADTest",
  [string]$Saves = $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "Skyrim Special Edition" } else { "" }),
  [switch]$Disposable
)
$ErrorActionPreference = "Stop"
if ($env:USERNAME -ne "WDAGUtilityAccount" -and -not $Disposable) {
  throw "Refusing: this only runs in Windows Sandbox or with -Disposable in a throwaway VM."
}
if (-not $Saves) { throw "Refusing: no -Saves folder (LOCALAPPDATA is not set)." }

$Stamp = "adtest-fixture/1"
$Time = [DateTime]::new(2024, 9, 22, 10, 13, 20, [DateTimeKind]::Utc)   # 1727000000
$G = "SteamLibrary/steamapps/common/Skyrim Special Edition"
# Base "R" = under Root, "S" = under Saves. Names match the source:
# strays.rs DISABLED_DIR, serverorder.rs CCC_BACKUP, loadorder.rs "txt.aetherial-dawn-backup".
$Files = @(
  @{ B = "R"; P = ".adtest-fixture"; T = $Stamp }
  @{ B = "R"; P = "SteamLibrary/steamapps/appmanifest_489830.acf"; T = "`"AppState`"`r`n{`r`n`t`"appid`"`t`t`"489830`"`r`n`t`"StateFlags`"`t`t`"4`"`r`n`t`"buildid`"`t`t`"13900225`"`r`n`t`"AutoUpdateBehavior`"`t`t`"1`"`r`n}"; RO = $true }  # U1
  @{ B = "R"; P = "$G/SkyrimSE.exe"; T = "dummy SkyrimSE.exe" }                                  # U2 stand-in
  @{ B = "R"; P = "$G/.aetherial-dawn/game.json"; T = '{"build":"1.6.1170.0","manual":false}' }  # U3
  @{ B = "R"; P = "$G/.aetherial-dawn/mods/feed-mod.json"; T = '{"id":"feed-mod","files":["Data/FeedMod.esp"]}' }
  @{ B = "R"; P = "$G/.aetherial-dawn/disabled/1727000000-strays/Data/PlayersOwn.esp"; T = "dummy PlayersOwn.esp" }
  @{ B = "R"; P = "$G/Data/FeedMod.esp"; T = "dummy FeedMod.esp" }                               # U4
  @{ B = "R"; P = "$G/Data/VortexOwned.esp"; T = "dummy VortexOwned.esp" }
  @{ B = "R"; P = "$G/Skyrim.ccc"; T = "" }                                                      # U6
  @{ B = "R"; P = "$G/Skyrim.ccc.aetherial-dawn-backup"; T = "ccBGSSSE001-Fish.esm" }
  @{ B = "S"; P = "plugins.txt"; T = "*FeedMod.esp" }                                            # U5 (server order)
  @{ B = "S"; P = "plugins.txt.aetherial-dawn-backup"; T = "*PlayersOwn.esp`r`n*VortexOwned.esp" }
  @{ B = "S"; P = "loadorder.txt"; T = "Skyrim.esm`r`nFeedMod.esp" }
  @{ B = "S"; P = "loadorder.txt.aetherial-dawn-backup"; T = "Skyrim.esm`r`nPlayersOwn.esp`r`nVortexOwned.esp" }
)
$Utf8 = New-Object System.Text.UTF8Encoding($false)
function Full($base, $rel) { Join-Path $base ($rel -replace "/", [IO.Path]::DirectorySeparatorChar) }
function Same($path, $text) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $false }
  # Base64 compares whole byte arrays, empty ones too, on PowerShell 5.1 and 7.
  return [Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -eq [Convert]::ToBase64String($Utf8.GetBytes($text))
}
function IsFixture($rootDir) {
  foreach ($f in $Files) {
    $base = if ($f.B -eq "R") { $rootDir } else { $Saves }
    if (-not (Same (Full $base $f.P) $f.T)) { return $false }
  }
  return $true
}

if (Test-Path -LiteralPath $Root) {
  if (IsFixture $Root) { Write-Host "Fixture already built under $Root; nothing changed."; exit 0 }
  throw "Refusing: $Root exists and is not this fixture. Use a fresh Sandbox or another -Root."
}
$Partial = "$Root.partial"
if (Test-Path -LiteralPath $Partial) {
  if (-not (Same (Full $Partial ".adtest-fixture") $Stamp)) { throw "Refusing: $Partial exists and this script did not make it." }
  $ro = Full $Partial "SteamLibrary/steamapps/appmanifest_489830.acf"
  if (Test-Path -LiteralPath $ro) { (Get-Item -LiteralPath $ro).IsReadOnly = $false }
  Remove-Item -LiteralPath $Partial -Recurse -Force
  Write-Host "Removed an interrupted earlier run ($Partial)."
}
foreach ($f in $Files | Where-Object { $_.B -eq "S" }) {
  $p = Full $Saves $f.P
  if ((Test-Path -LiteralPath $p) -and -not (Same $p $f.T)) { throw "Refusing: $p already exists with other contents (a real player file?)." }
}

# The stamp goes first, so a leftover .partial is always recognisable as ours.
$ordered = @($Files | Where-Object { $_.P -eq ".adtest-fixture" }) + @($Files | Where-Object { $_.P -ne ".adtest-fixture" })
foreach ($f in $ordered) {
  $base = if ($f.B -eq "R") { $Partial } else { $Saves }
  $p = Full $base $f.P
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $p) | Out-Null
  if (-not (Same $p $f.T)) { [IO.File]::WriteAllBytes($p, $Utf8.GetBytes($f.T)) }
  [IO.File]::SetLastWriteTimeUtc($p, $Time)
  if ($f.RO) { (Get-Item -LiteralPath $p).IsReadOnly = $true }
}
Rename-Item -LiteralPath $Partial -NewName (Split-Path -Leaf $Root)
if (-not (IsFixture $Root)) { throw "The fixture did not verify after building." }
Write-Host "Fixture ready under $Root and $Saves ($($Files.Count) files)."
