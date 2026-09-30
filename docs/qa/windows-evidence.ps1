<#
Aetherial Dawn launcher QA: read-only evidence for D13 and D14.

It only READS. It changes no file, setting, registry key or process, and it
never opens the sign-in token, skymp5-client-settings.txt or any key file.
It writes one text file to your Desktop and prints where it is.

Run it from PowerShell:
  powershell -ExecutionPolicy Bypass -File .\windows-evidence.ps1 -Label before
Labels used by the run sheet: before, during, after, held.
#>
param(
  [string]$Label = "check",
  # Only for testing the script elsewhere; leave these out on the real PC.
  [string]$SteamPath = "",
  [string]$OutDir = ""
)

$ErrorActionPreference = "SilentlyContinue"
$stamp = Get-Date -Format "yyyy-MM-dd_HH-mm-ss"
if (-not $OutDir) { $OutDir = [Environment]::GetFolderPath("Desktop") }
$out = Join-Path $OutDir "AD-evidence-$Label-$stamp.txt"
$lines = New-Object System.Collections.Generic.List[string]
function Say($text) { $lines.Add([string]$text) }
# Log lines can carry a Steam account name (DepotDownloader's -username) or
# other sign-in values; they are hidden before anything is written.
function Redact([string]$line) {
  $line = $line -replace '(?i)(-{1,2}(username|user|password|pass|passwd|token|apikey|key)[\s=]+)("[^"]*"|\S+)', '$1<hidden>'
  $line -replace '(?i)((account|user(name)?|login)\s*[:=]\s*)("[^"]*"|\S+)', '$1<hidden>'
}

function Section($name) { Say ""; Say "== $name" }

Say "Aetherial Dawn launcher QA evidence (read-only)"
Say "label: $Label"
Say "local time: $(Get-Date -Format o)"
Say "windows: $([Environment]::OSVersion.VersionString)"

Section "Steam"
$steam = if ($SteamPath) { $SteamPath } else { (Get-ItemProperty "HKCU:\Software\Valve\Steam").SteamPath }
if (-not $steam) { $steam = "C:\Program Files (x86)\Steam" }
$steam = $steam -replace "/", "\"
Say "steam folder: $steam"

# Find the Steam library holding Skyrim Special Edition (app 489830).
$libs = @($steam)
$vdf = Join-Path $steam "steamapps\libraryfolders.vdf"
if (Test-Path $vdf) {
  foreach ($m in [regex]::Matches((Get-Content $vdf -Raw), '"path"\s+"([^"]+)"')) { $libs += ($m.Groups[1].Value -replace "\\\\", "\") }
}
$acf = $null
foreach ($lib in ($libs | Select-Object -Unique)) {
  $p = Join-Path $lib "steamapps\appmanifest_489830.acf"
  if (Test-Path $p) { $acf = $p; break }
}

Section "appmanifest_489830.acf (D14)"
if (-not $acf) { Say "not found in: $($libs -join '; ')" } else {
  $item = Get-Item $acf
  Say "path: $acf"
  Say "read-only: $($item.IsReadOnly)"
  Say "last written: $($item.LastWriteTime.ToString('o'))"
  $text = Get-Content $acf -Raw
  foreach ($key in "buildid", "TargetBuildID", "StateFlags", "AutoUpdateBehavior", "UpdateResult", "BytesToDownload", "BytesDownloaded", "BytesToStage", "BytesStaged", "LastUpdated", "ScheduledAutoUpdate") {
    $m = [regex]::Match($text, '"' + $key + '"\s+"([^"]*)"')
    Say ("{0}: {1}" -f $key, $(if ($m.Success) { $m.Groups[1].Value } else { "(absent)" }))
  }
  $flags = [regex]::Match($text, '"StateFlags"\s+"(\d+)"')
  if ($flags.Success) {
    $f = [int]$flags.Groups[1].Value
    $names = @{1="Invalid";2="Uninstalled";4="FullyInstalled";8="Encrypted";16="Locked";32="FilesMissing";64="AppRunning";128="FilesCorrupt";256="UpdateRunning";512="UpdatePaused";1024="UpdateStarted";2048="Uninstalling";4096="BackupRunning";65536="Reconfiguring";131072="Validating";262144="AddingFiles";524288="Preallocating";1048576="Downloading";2097152="Staging";4194304="Committing";8388608="UpdateStopping"}
    Say ("StateFlags means: " + (($names.Keys | Sort-Object | Where-Object { $f -band $_ } | ForEach-Object { $names[$_] }) -join ", "))
  }
  $depots = [regex]::Matches($text, '"(\d{6,})"\s*\{\s*"manifest"\s+"(\d+)"')
  foreach ($d in $depots) { Say "depot $($d.Groups[1].Value) manifest $($d.Groups[2].Value)" }
}

Section "Game files"
$game = if ($acf) { Join-Path (Split-Path $acf) "common\Skyrim Special Edition" } else { $null }
Say "game folder: $game"
foreach ($name in "SkyrimSE.exe", "skse64_loader.exe") {
  $p = if ($game) { Join-Path $game $name } else { $null }
  if ($p -and (Test-Path $p)) {
    $i = Get-Item $p
    Say ("{0}: version {1}, {2} bytes, sha256 {3}, written {4}" -f $name, $i.VersionInfo.FileVersion, $i.Length, (Get-FileHash $p -Algorithm SHA256).Hash, $i.LastWriteTime.ToString('o'))
  } else { Say "${name}: missing" }
}
$marker = if ($game) { Join-Path $game ".aetherial-dawn\game.json" } else { $null }
if ($marker -and (Test-Path $marker)) { Say "launcher version marker: $((Get-Content $marker -Raw) -replace '\s+', ' ')" } else { Say "launcher version marker: none" }

Section "Running programs"
foreach ($name in "steam", "SkyrimSE", "skse64_loader", "aetherial-dawn-launcher", "Aetherial Dawn") {
  $procs = Get-Process -Name $name
  Say ("{0}: {1}" -f $name, $(if ($procs) { "running (started " + (($procs | ForEach-Object { $_.StartTime.ToString('HH:mm:ss') }) -join ", ") + ")" } else { "not running" }))
}

Section "Steam content log, lines about Skyrim SE (D13)"
$clog = Join-Path $steam "logs\content_log.txt"
if (Test-Path $clog) { Get-Content $clog -Tail 4000 | Select-String "489830" | Select-Object -Last 60 | ForEach-Object { Say (Redact $_.Line) } } else { Say "no content_log.txt" }

Section "Launcher log, patch and Steam lines (D13)"
$llog = Join-Path $env:LOCALAPPDATA "gg.aetherialdawn.launcher\logs\launcher.log"
if (Test-Path $llog) { Get-Content $llog -Tail 4000 | Select-String -Pattern "patch:|steam|version|verify" | Select-Object -Last 80 | ForEach-Object { Say (Redact $_.Line) } } else { Say "no launcher.log" }

$lines | Set-Content -Path $out -Encoding UTF8
Write-Host "Saved: $out"
