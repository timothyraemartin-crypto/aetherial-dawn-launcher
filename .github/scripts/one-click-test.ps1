# Installs the built launcher the way a new player does (a plain run of the
# installer, no flags), a passive same-version reinstall (/P /UPDATE /R), a
# silent install (/S), and a silent uninstall. This is not an old-to-new
# signed self-update test. Any failed check stops release publication.
param([Parameter(Mandatory)] [string] $Setup, [Parameter(Mandatory)] [string] $Version)
$ErrorActionPreference = 'Stop'
$Setup = (Resolve-Path $Setup).Path
$setupName = Split-Path $Setup -Leaf
$dir = Join-Path $env:LOCALAPPDATA 'Aetherial Dawn'
$exe = Join-Path $dir 'aetherial-dawn-launcher.exe'
$uninstaller = Join-Path $dir 'uninstall.exe'
$desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Aetherial Dawn.lnk'
$programs = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$failed = 0
function Check([string] $what, [bool] $ok, [string] $detail = '') {
  $mark = if ($ok) { 'PASS' } else { 'FAIL' }
  Write-Host "$mark  $what$(if ($detail) { "  ($detail)" })"
  if (-not $ok) { $script:failed++ }
}
function Show($code) { if ($null -eq $code) { 'none, still running' } else { $code } }
function Launchers { @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -ieq $exe }) }
function StopLaunchers { Launchers | Stop-Process -Force; Start-Sleep -Seconds 2 }
# Every installer process seen while $Until is false, with its command line,
# and its exit code once it is gone. Polled every 100 ms.
function Watch-Setups([scriptblock] $Until, [int] $Seconds) {
  $seen = @{}
  $deadline = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $deadline) {
    foreach ($w in @(Get-CimInstance Win32_Process -Filter "Name = '$setupName'")) {
      if (-not $seen.ContainsKey($w.ProcessId)) {
        $p = Get-Process -Id $w.ProcessId -ErrorAction SilentlyContinue
        if ($p) { $null = $p.Handle }
        $seen[$w.ProcessId] = [pscustomobject]@{ Id = $w.ProcessId; Parent = $w.ParentProcessId; CommandLine = $w.CommandLine; Process = $p }
      }
    }
    if (& $Until) { break }
    Start-Sleep -Milliseconds 100
  }
  foreach ($s in $seen.Values) {
    if ($s.Process) { $null = $s.Process.WaitForExit(30000) }
    $code = if ($s.Process -and $s.Process.HasExited) { $s.Process.ExitCode } else { $null }
    $s | Add-Member ExitCode $code
  }
  @($seen.Values)
}
# The exit code, or $null while it is still running.
function Wait-Exit($p, [int] $Seconds) { if (-not $p.WaitForExit($Seconds * 1000)) { return $null }; $p.ExitCode }

Write-Host "== Installer $setupName, version $Version =="
Check 'nothing installed before the test' (-not (Test-Path $dir)) $dir

Write-Host '== 1. A plain run, as a new player double-clicks it =='
$parent = Start-Process -FilePath $Setup -PassThru
$null = $parent.Handle
$setups = Watch-Setups { (Launchers).Count -gt 0 -and -not (Get-CimInstance Win32_Process -Filter "Name = '$setupName'") } 180
$parentCode = Wait-Exit $parent 30
Write-Host "   parent pid $($parent.Id) exit code $(Show $parentCode)"
foreach ($s in $setups) { Write-Host "   setup pid $($s.Id) (parent $($s.Parent)) exit $(Show $s.ExitCode): $($s.CommandLine)" }
Check '(a) the plain run exits by itself with 0' ($parentCode -eq 0) "exit code $(Show $parentCode)"
$children = @($setups | Where-Object { $_.Id -ne $parent.Id })
Check '(b) exactly one child installer' ($children.Count -eq 1) "$($children.Count) children"
if ($children.Count -ge 1) {
  $c = $children[0]
  Check '(b) the child was started by the plain run' ($c.Parent -eq $parent.Id) "parent $($c.Parent)"
  Check '(b) the child runs with /P /R' ($c.CommandLine -match '(^|\s)/P(\s|$)' -and $c.CommandLine -match '(^|\s)/R(\s|$)') $c.CommandLine
  Check '(b) the child exits with 0' ($c.ExitCode -eq 0) "exit $(Show $c.ExitCode)"
}
Check '(b) no third installer process' ($setups.Count -le 2) "$($setups.Count) installer processes in all"
$left = @(Get-CimInstance Win32_Process -Filter "Name = '$setupName'")
Check '(c) no installer left open on a page' ($left.Count -eq 0) "$($left.Count) still open"
Check '(d) the launcher is installed' (Test-Path $exe) $exe
Check '(d) the uninstaller is installed' (Test-Path $uninstaller) $uninstaller
$key = Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' | Where-Object { (Get-ItemProperty $_.PSPath).DisplayName -eq 'Aetherial Dawn' } | Select-Object -First 1
$shown = if ($key) { (Get-ItemProperty $key.PSPath).DisplayVersion } else { 'no uninstall entry' }
Check "(d) Windows lists it as version $Version" ($shown -eq $Version) "HKCU uninstall DisplayVersion $shown"
Check '(e) desktop shortcut' (Test-Path $desktop) $desktop
$start = @(Get-ChildItem $programs -Recurse -Filter 'Aetherial Dawn.lnk' -ErrorAction SilentlyContinue)
Check '(e) Start menu shortcut' ($start.Count -ge 1) ($start.FullName -join ', ')
Start-Sleep -Seconds 5
$n = (Launchers).Count
Check '(f) exactly one launcher running' ($n -eq 1) "$n running"
# The window starts hidden and the page shows it once its first screen is
# drawn; the launcher shows it by itself after 3 s only if the page didn't.
$shownBy = Get-Content (Join-Path $env:LOCALAPPDATA 'gg.aetherialdawn.launcher\logs\launcher.log') -ErrorAction SilentlyContinue | Select-String 'window: shown by the fallback'
$handle = @(Launchers | ForEach-Object { $_.MainWindowHandle } | Where-Object { $_ -ne 0 }).Count
Check '(h) the window is on screen' ($handle -ge 1) "$handle visible windows"
Check '(h) the page drew and showed the window itself' (-not $shownBy) "$shownBy"
StopLaunchers

Write-Host '== 2. Passive same-version reinstall: /P /UPDATE /R =='
$p = Start-Process -FilePath $Setup -ArgumentList '/P', '/UPDATE', '/R' -PassThru
$null = $p.Handle
$setups = Watch-Setups { $p.HasExited } 180
$code = Wait-Exit $p 30
Check '(g) the update run exits with 0' ($code -eq 0) "exit $(Show $code)"
Check '(g) the update run starts no second installer' ($setups.Count -le 1) "$($setups.Count) installer processes"
$deadline = (Get-Date).AddSeconds(30)
while ((Launchers).Count -eq 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
Start-Sleep -Seconds 5
$n = (Launchers).Count
Check '(g) the update starts exactly one launcher' ($n -eq 1) "$n running"
StopLaunchers

Write-Host '== 3. A silent install: /S =='
$p = Start-Process -FilePath $Setup -ArgumentList '/S' -PassThru
$null = $p.Handle
$setups = Watch-Setups { $p.HasExited } 180
$code = Wait-Exit $p 30
Check '(g) the silent run exits with 0' ($code -eq 0) "exit $(Show $code)"
Check '(g) the silent run starts no second installer' ($setups.Count -le 1) "$($setups.Count) installer processes"
Start-Sleep -Seconds 10
$n = (Launchers).Count
Check '(g) the silent run starts no launcher' ($n -eq 0) "$n running"
StopLaunchers

Write-Host '== 4. Silent uninstall: /S =='
if (Test-Path $uninstaller) {
  $p = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru
  $null = $p.Handle
  $code = Wait-Exit $p 120
  Check 'the uninstaller exits with 0' ($code -eq 0) "exit $(Show $code)"
  # NSIS may finish removing shortcuts and the registry entry in a child after
  # the process started above has exited. Wait for the whole uninstall result,
  # not merely the first executable deletion, with a fixed upper bound.
  $deadline = (Get-Date).AddSeconds(60)
  do {
    $remaining = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue |
      Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DisplayName -eq 'Aetherial Dawn' })
    $start = @(Get-ChildItem $programs -Recurse -Filter 'Aetherial Dawn.lnk' -ErrorAction SilentlyContinue)
    $exeLeft = Test-Path $exe
    $desktopLeft = Test-Path $desktop
    if (-not $exeLeft -and $remaining.Count -eq 0 -and -not $desktopLeft -and $start.Count -eq 0) { break }
    if ((Get-Date) -ge $deadline) { break }
    Start-Sleep -Milliseconds 500
  } while ($true)
  Check 'the launcher executable is removed' (-not $exeLeft) $exe
  Check 'the uninstall registry entry is removed' ($remaining.Count -eq 0)
  Check 'the desktop shortcut is removed' (-not $desktopLeft) $desktop
  Check 'the Start menu shortcut is removed' ($start.Count -eq 0) ($start.FullName -join ', ')
  Check 'no launcher is running after uninstall' ((Launchers).Count -eq 0)
}

if ($failed) { throw "$failed check(s) failed" }
Write-Host 'installer and uninstaller checks passed: every check'
