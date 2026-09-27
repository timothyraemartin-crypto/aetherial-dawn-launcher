# The upgrade players will take: the live launcher is installed and running
# with a player's settings and sign-in saved, then its updater runs the new
# installer the way tauri-plugin-updater 2.12.0 does (ShellExecute of
# "setup.exe /P /UPDATE /R /ARGS", then the old launcher exits at once).
# Checks that it ends on the new version with exactly one launcher running,
# the shortcuts intact, and the settings and sign-in kept.
param(
  [Parameter(Mandatory)] [string] $OldSetup, [Parameter(Mandatory)] [string] $OldVersion,
  [Parameter(Mandatory)] [string] $NewSetup, [Parameter(Mandatory)] [string] $NewVersion
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$OldSetup = (Resolve-Path $OldSetup).Path
$NewSetup = (Resolve-Path $NewSetup).Path
$dir = Join-Path $env:LOCALAPPDATA 'Aetherial Dawn'
$exe = Join-Path $dir 'aetherial-dawn-launcher.exe'
$data = Join-Path $env:APPDATA 'gg.aetherialdawn.launcher'
$desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Aetherial Dawn.lnk'
$programs = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$failed = 0
function Check([string] $what, [bool] $ok, [string] $detail = '') {
  $mark = if ($ok) { 'PASS' } else { 'FAIL' }
  Write-Host "$mark  $what$(if ($detail) { "  ($detail)" })"
  if (-not $ok) { $script:failed++ }
}
function Launchers { @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path -ieq $exe }) }
function Entries { @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' | ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.DisplayName -eq 'Aetherial Dawn' }) }
function Target([string] $lnk) { (New-Object -ComObject WScript.Shell).CreateShortcut($lnk).TargetPath }
function Seal([string] $text) { [Security.Cryptography.ProtectedData]::Protect([Text.Encoding]::UTF8.GetBytes($text), $null, 'CurrentUser') }
function Open([string] $path) { [Text.Encoding]::UTF8.GetString([Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($path), $null, 'CurrentUser')) }
function Hash([string] $path) { (Get-FileHash $path -Algorithm SHA256).Hash }
function StartMenu { @(Get-ChildItem $programs -Recurse -Filter 'Aetherial Dawn.lnk' -ErrorAction SilentlyContinue) }

Write-Host "== Old: $(Split-Path $OldSetup -Leaf) ($OldVersion), sha256 $(Hash $OldSetup)"
Write-Host "== New: $(Split-Path $NewSetup -Leaf) ($NewVersion), sha256 $(Hash $NewSetup)"
Check 'nothing installed before the test' (-not (Test-Path $dir) -and -not (Test-Path $data))

Write-Host "== 1. Install the live $OldVersion (silently; it has no one-click) =="
$p = Start-Process -FilePath $OldSetup -ArgumentList '/S' -PassThru -Wait
Check "the $OldVersion installer exits with 0" ($p.ExitCode -eq 0) "exit $($p.ExitCode)"
$e = Entries
Check "Windows lists $OldVersion" ($e.Count -eq 1 -and $e[0].DisplayVersion -eq $OldVersion) (($e | ForEach-Object DisplayVersion) -join ', ')
Check 'desktop shortcut after the old install' (Test-Path $desktop) $desktop
Check 'Start menu shortcut after the old install' ((StartMenu).Count -ge 1)

Write-Host '== 2. A player''s saved settings and sign-in =='
New-Item -ItemType Directory -Force $data | Out-Null
$authOk = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$config = [ordered]@{
  gameDir = $null; baseUrl = 'https://example.invalid/launcher'
  account = [ordered]@{ masterApiId = 4242; discordId = '123456789012345678'; discordUsername = 'upgrade-test'; discordDiscriminator = $null; discordAvatar = $null }
  lastAuthOk = $authOk; closeOnLaunch = $false; backgroundUpdates = $false; shareHealth = $false
  nexusUser = $null; music = $false; onlyServerMods = $false
}
$config | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $data 'config.json')
[IO.File]::WriteAllBytes((Join-Path $data 'session.bin'), (Seal 'upgrade-test-session-token'))
[IO.File]::WriteAllBytes((Join-Path $data 'nexus.bin'), (Seal 'upgrade-test-nexus-key'))
$sessionHash = Hash (Join-Path $data 'session.bin')
$nexusHash = Hash (Join-Path $data 'nexus.bin')
Write-Host "   seeded config.json, session.bin ($sessionHash), nexus.bin ($nexusHash)"

Write-Host "== 3. The player has $OldVersion open =="
Start-Process -FilePath $exe | Out-Null
$deadline = (Get-Date).AddSeconds(60)
while ((Launchers).Count -eq 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
Start-Sleep -Seconds 10
$old = Launchers
Check "exactly one $OldVersion launcher running" ($old.Count -eq 1) "$($old.Count) running"
Check "the running launcher is $OldVersion" ($old.Count -ge 1 -and $old[0].MainModule.FileVersionInfo.ProductVersion -like "$OldVersion*") (($old | ForEach-Object { $_.MainModule.FileVersionInfo.ProductVersion }) -join ', ')

Write-Host "== 4. The updater installs $NewVersion: $(Split-Path $NewSetup -Leaf) /P /UPDATE /R /ARGS, then the old launcher exits =="
$setupName = Split-Path $NewSetup -Leaf
$inst = Start-Process -FilePath $NewSetup -ArgumentList '/P', '/UPDATE', '/R', '/ARGS' -PassThru
$null = $inst.Handle
$old | Stop-Process -Force
if (-not $inst.WaitForExit(180000)) { Check 'the update installer finishes within 3 minutes' $false } else { Check 'the update installer exits with 0' ($inst.ExitCode -eq 0) "exit $($inst.ExitCode)" }
$deadline = (Get-Date).AddSeconds(60)
while ((Launchers).Count -eq 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
Start-Sleep -Seconds 15
$left = @(Get-CimInstance Win32_Process -Filter "Name = '$setupName'")
Check 'no installer left open' ($left.Count -eq 0) "$($left.Count) open"

Write-Host "== 5. After the update =="
$e = Entries
Check "Windows lists exactly one Aetherial Dawn, version $NewVersion" ($e.Count -eq 1 -and $e[0].DisplayVersion -eq $NewVersion) (($e | ForEach-Object DisplayVersion) -join ', ')
$fv = (Get-Item $exe).VersionInfo.ProductVersion
Check "the installed launcher is $NewVersion" ($fv -like "$NewVersion*") "file version $fv"
$now = Launchers
Check 'exactly one launcher running' ($now.Count -eq 1) "$($now.Count) running"
Check "the running launcher is $NewVersion" ($now.Count -ge 1 -and $now[0].MainModule.FileVersionInfo.ProductVersion -like "$NewVersion*") (($now | ForEach-Object { $_.MainModule.FileVersionInfo.ProductVersion }) -join ', ')
Check 'desktop shortcut still there and points at the launcher' ((Test-Path $desktop) -and ((Target $desktop) -ieq $exe)) $(if (Test-Path $desktop) { Target $desktop })
$sm = StartMenu
Check 'Start menu shortcut still there and points at the launcher' ($sm.Count -eq 1 -and ((Target $sm[0].FullName) -ieq $exe)) (($sm | ForEach-Object { "$($_.FullName) -> $(Target $_.FullName)" }) -join '; ')
$c = Get-Content -Raw (Join-Path $data 'config.json') | ConvertFrom-Json
Check 'settings kept: the signed-in account' ($c.account.discordId -eq '123456789012345678' -and $c.account.discordUsername -eq 'upgrade-test') "$($c.account.discordUsername) $($c.account.discordId)"
Check 'settings kept: last sign-in check time' ($c.lastAuthOk -eq $authOk) "$($c.lastAuthOk)"
Check 'settings kept: the player''s choices' ($c.closeOnLaunch -eq $false -and $c.shareHealth -eq $false -and $c.onlyServerMods -eq $false -and $c.music -eq $false -and $c.backgroundUpdates -eq $false) "closeOnLaunch $($c.closeOnLaunch), shareHealth $($c.shareHealth), onlyServerMods $($c.onlyServerMods), music $($c.music), backgroundUpdates $($c.backgroundUpdates)"
$s = Join-Path $data 'session.bin'
Check 'sign-in kept: session.bin unchanged and still opens' ((Test-Path $s) -and (Hash $s) -eq $sessionHash -and (Open $s) -eq 'upgrade-test-session-token')
$n = Join-Path $data 'nexus.bin'
Check 'Nexus sign-in kept: nexus.bin unchanged and still opens' ((Test-Path $n) -and (Hash $n) -eq $nexusHash -and (Open $n) -eq 'upgrade-test-nexus-key')
$log = Join-Path $env:LOCALAPPDATA 'gg.aetherialdawn.launcher\logs\launcher.log'
if (Test-Path $log) { Write-Host '   launcher.log (last 25 lines):'; Get-Content $log -Tail 25 | ForEach-Object { Write-Host "   | $_" } }
Launchers | Stop-Process -Force

if ($failed) { throw "$failed check(s) failed" }
Write-Host "upgrade $OldVersion -> $NewVersion passed: every check"
