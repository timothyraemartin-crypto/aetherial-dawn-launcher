<#
D18: builds the fake Steam library and player files that the uninstall test
starts from (PLAN-D18-UNINSTALL.md section 2). It WRITES files, so it runs
only in a throwaway Windows: it refuses unless the user is Windows Sandbox's
WDAGUtilityAccount or -Disposable is given (for a throwaway VM).
No real game files, keys or Vortex data are used or touched.
#>
param(
  [string]$Root = "C:\ADTest",
  [string]$AppData = $env:LOCALAPPDATA,
  [switch]$Disposable
)
$ErrorActionPreference = "Stop"
if ($env:USERNAME -ne "WDAGUtilityAccount" -and -not $Disposable) {
  throw "Refusing: this only runs in Windows Sandbox or with -Disposable in a throwaway VM."
}
if (Test-Path $Root) { throw "Refusing: $Root already exists." }

$lib = Join-Path $Root "SteamLibrary"
$game = Join-Path $lib "steamapps\common\Skyrim Special Edition"
$mark = Join-Path $game ".aetherial-dawn"
$save = Join-Path $AppData "Skyrim Special Edition"
foreach ($d in $game, (Join-Path $game "Data"), (Join-Path $mark "mods"), (Join-Path $mark "disabled\1727000000-strays\Data"), $save) { New-Item -ItemType Directory -Force -Path $d | Out-Null }

$acf = Join-Path $lib "steamapps\appmanifest_489830.acf"
Set-Content -Path $acf -Encoding ASCII -Value @'
"AppState"
{
	"appid"		"489830"
	"StateFlags"		"4"
	"buildid"		"13900225"
	"AutoUpdateBehavior"		"1"
}
'@
(Get-Item $acf).IsReadOnly = $true                                    # U1

Set-Content (Join-Path $game "SkyrimSE.exe") "dummy"                  # U2 stand-in
Set-Content (Join-Path $mark "game.json") '{"build":"1.6.1170.0","manual":false}'  # U3
Set-Content (Join-Path $mark "mods\feed-mod.json") '{"id":"feed-mod","files":["Data/FeedMod.esp"]}'
Set-Content (Join-Path $mark "disabled\1727000000-strays\Data\PlayersOwn.esp") "dummy"  # strays.rs DISABLED_DIR
Set-Content (Join-Path $game "Data\FeedMod.esp") "dummy"              # U4
Set-Content (Join-Path $game "Data\VortexOwned.esp") "dummy"
Set-Content (Join-Path $game "Skyrim.ccc.aetherial-dawn-backup") "ccc" # U6: serverorder.rs CCC_BACKUP
Set-Content (Join-Path $game "Skyrim.ccc") ""
Set-Content (Join-Path $save "plugins.txt") "*FeedMod.esp"            # U5 (server order)
Set-Content (Join-Path $save "plugins.txt.aetherial-dawn-backup") "*PlayersOwn.esp`r`n*VortexOwned.esp"
Write-Host "Fixture ready under $Root and $save"
