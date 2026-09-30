<#
D18: read-only snapshot of everything the uninstall test looks at
(PLAN-D18-UNINSTALL.md section 1, rows U1-U10). It only lists and hashes; it
never changes, moves or deletes a file.

Roots, each printed by a short name so no user name or drive path appears:
  FIXTURE  the fake library from sandbox-fixture.ps1   (U1-U6)
  SAVES    %LOCALAPPDATA%\Skyrim Special Edition       (U5)
  DOCS     Documents\My Games\Skyrim Special Edition   (U9, the game's ini files)
  ROAMING  %APPDATA%\gg.aetherialdawn.launcher         (U7, config)
  LOCAL    %LOCALAPPDATA%\gg.aetherialdawn.launcher    (U7, logs, cache, sign-in)
  INSTALL  %LOCALAPPDATA%\Aetherial Dawn               (the installed program)
ROAMING, LOCAL and INSTALL are private: only each file's name and size are
recorded, never its contents or hash, so the sign-in file is never opened.
Also recorded: Windows' graphics-card preference for the fake SkyrimSE.exe
(U10, a registry value read only).

Output: "snapshot/1", the label, "complete=true|false", then sorted lines.
Same files in, same bytes out. Written to "<Out>.partial" and renamed at the
end, so an interrupted run leaves no finished-looking file. It refuses to
overwrite an existing -Out.
Exit 0: complete. Exit 2: written but incomplete (the "error" lines say why).
#>
param(
  [Parameter(Mandatory)][string]$Out,
  [string]$Label = "",
  [string]$Root = "C:\ADTest",
  [string]$Saves = $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "Skyrim Special Edition" } else { "" }),
  [string]$Docs = $(try { Join-Path ([Environment]::GetFolderPath("MyDocuments")) "My Games\Skyrim Special Edition" } catch { "" }),
  [string]$Roaming = $(if ($env:APPDATA) { Join-Path $env:APPDATA "gg.aetherialdawn.launcher" } else { "" }),
  [string]$Local = $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "gg.aetherialdawn.launcher" } else { "" }),
  [string]$Install = $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "Aetherial Dawn" } else { "" })
)
$ErrorActionPreference = "Stop"
if (Test-Path -LiteralPath $Out) { throw "Refusing: $Out already exists. Pick a new -Out." }
if ($Label -match "[`r`n]") { throw "Refusing: -Label must be one line." }

$roots = @(
  @{ N = "FIXTURE"; P = $Root; Private = $false }
  @{ N = "SAVES"; P = $Saves; Private = $false }
  @{ N = "DOCS"; P = $Docs; Private = $false }
  @{ N = "ROAMING"; P = $Roaming; Private = $true }
  @{ N = "LOCAL"; P = $Local; Private = $true }
  @{ N = "INSTALL"; P = $Install; Private = $true }
)
$lines = New-Object System.Collections.Generic.List[string]
$errors = 0
function Err($what) { $script:errors++; $lines.Add("error`t$what") }

foreach ($r in $roots) {
  if (-not $r.P) { Err "$($r.N)`tno path (environment variable not set)"; continue }
  $base = [IO.Path]::GetFullPath($r.P).TrimEnd([char]'\', [char]'/')
  if (-not (Test-Path -LiteralPath $base -PathType Container)) { $lines.Add("root`t$($r.N)`tmissing"); continue }
  $lines.Add("root`t$($r.N)`tpresent")
  $ev = $null
  $items = Get-ChildItem -LiteralPath $base -Recurse -Force -File -ErrorAction SilentlyContinue -ErrorVariable ev
  foreach ($e in @($ev)) { if ($e) { Err "$($r.N)`tcould not list: $($e.CategoryInfo.Category)" } }
  foreach ($f in $items) {
    $rel = "$($r.N)/" + ($f.FullName.Substring($base.Length).TrimStart([char]'\', [char]'/') -replace "\\", "/")
    if ($r.Private) { $lines.Add("file`t$rel`tsize=$($f.Length)"); continue }
    try { $h = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash }
    catch { Err "$rel`tcould not hash"; continue }
    $lines.Add("file`t$rel`tsize=$($f.Length)`tsha256=$h`tro=$($f.IsReadOnly)")
    if ($f.Name -eq "appmanifest_489830.acf") {
      try {
        $m = [regex]::Match((Get-Content -LiteralPath $f.FullName -Raw), '"AutoUpdateBehavior"\s+"(\d+)"')
        $v = if ($m.Success) { $m.Groups[1].Value } else { "absent" }
        $lines.Add("field`t$rel`tAutoUpdateBehavior=$v")
      } catch { Err "$rel`tcould not read AutoUpdateBehavior" }
    }
  }
}

# U10: the value Windows keeps under the game exe's full path. Only on Windows.
if ($env:OS -eq "Windows_NT") {
  $exe = Join-Path $Root "SteamLibrary\steamapps\common\Skyrim Special Edition\SkyrimSE.exe"
  $key = "HKCU:\Software\Microsoft\DirectX\UserGpuPreferences"
  try {
    $v = (Get-ItemProperty -LiteralPath $key -Name $exe -ErrorAction Stop).$exe
    $lines.Add("field`tREG/GpuPreference`tvalue=$v")
  } catch [System.Management.Automation.PSArgumentException] { $lines.Add("field`tREG/GpuPreference`tvalue=absent") }
    catch [System.Management.Automation.ItemNotFoundException] { $lines.Add("field`tREG/GpuPreference`tvalue=absent") }
    catch { Err "REG/GpuPreference`tcould not read" }
} else { $lines.Add("field`tREG/GpuPreference`tvalue=not-windows") }

$body = $lines.ToArray()
[Array]::Sort($body, [StringComparer]::Ordinal)
$complete = if ($errors -eq 0) { "true" } else { "false" }
$text = (@("snapshot/1", "label=$Label", "complete=$complete") + $body) -join "`r`n"
$partial = "$Out.partial"
[IO.File]::WriteAllBytes($partial, (New-Object System.Text.UTF8Encoding($false)).GetBytes($text + "`r`n"))
Move-Item -LiteralPath $partial -Destination $Out
Write-Host "Saved: $Out ($($body.Count) lines, complete=$complete)"
if ($errors) { exit 2 }
