<#
D18: compares snapshot A (before uninstall) with snapshot B (after) and marks
each row U1-U7 of PLAN-D18-UNINSTALL.md PASS or FAIL against the
"Settings only" default. Read-only: it reads the two snapshot files only.
Any change no row explains is listed as UNEXPECTED.
Exit 0: compared (PASS and FAIL are both evidence). Exit 2: a snapshot is
missing, not snapshot/1, or incomplete, so nothing is judged.
#>
param([Parameter(Mandatory)][string]$Before, [Parameter(Mandatory)][string]$After)
$ErrorActionPreference = "Stop"

function Load($path) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Write-Host "INCOMPLETE: $path is missing."; exit 2 }
  $all = @(Get-Content -LiteralPath $path)
  if ($all.Count -lt 3 -or $all[0] -ne "snapshot/1") { Write-Host "INCOMPLETE: $path is not a snapshot/1 file."; exit 2 }
  if ($all[2] -ne "complete=true") { Write-Host "INCOMPLETE: $path says $($all[2]); take it again."; exit 2 }
  $s = @{ Files = @{}; Fields = @{}; Roots = @{} }
  foreach ($l in $all[3..($all.Count - 1)]) {
    $c = $l -split "`t"
    switch ($c[0]) {
      "root" { $s.Roots[$c[1]] = $c[2] }
      "file" { $s.Files[$c[1]] = ($c[2..($c.Count - 1)] -join " ") }
      "field" { $s.Fields[$c[1]] = $c[2] }
    }
  }
  return $s
}
$A = Load $Before
$B = Load $After

$G = "FIXTURE/SteamLibrary/steamapps/common/Skyrim Special Edition"
$ACF = "FIXTURE/SteamLibrary/steamapps/appmanifest_489830.acf"
$STRAY = "$G/.aetherial-dawn/disabled/1727000000-strays/Data/PlayersOwn.esp"
function Hash($s, $p) { if ($s.Files.ContainsKey($p)) { ($s.Files[$p] -split " " | Where-Object { $_ -like "sha256=*" }) } else { "absent" } }
function Ro($s, $p) { if ($s.Files.ContainsKey($p)) { ($s.Files[$p] -split " " | Where-Object { $_ -like "ro=*" }) } else { "absent" } }
function Row($id, $ok, $why) { "{0}`t{1}`t{2}" -f $id, $(if ($ok) { "PASS" } else { "FAIL" }), $why }
$claimed = New-Object System.Collections.Generic.HashSet[string]

# U1: read-only cleared and auto-update back on (0 = always keep updated).
$claimed.Add($ACF) | Out-Null
$au = $B.Fields[$ACF]; $ro = Ro $B $ACF
Row "U1" (($ro -eq "ro=False") -and ($au -eq "AutoUpdateBehavior=0")) "after: $ro, $au"

# U2: game files stay on the server build.
$claimed.Add("$G/SkyrimSE.exe") | Out-Null
Row "U2" ((Hash $A "$G/SkyrimSE.exe") -eq (Hash $B "$G/SkyrimSE.exe") -and (Hash $B "$G/SkyrimSE.exe") -ne "absent") "SkyrimSE.exe unchanged"

# U3: set-aside file back in Data with the same bytes, disabled folder empty.
$claimed.Add($STRAY) | Out-Null; $claimed.Add("$G/Data/PlayersOwn.esp") | Out-Null
$left = @($B.Files.Keys | Where-Object { $_.StartsWith("$G/.aetherial-dawn/disabled/") })
$back = (Hash $B "$G/Data/PlayersOwn.esp") -eq (Hash $A $STRAY) -and (Hash $A $STRAY) -ne "absent"
Row "U3" ($back -and $left.Count -eq 0) "PlayersOwn.esp back in Data: $back; files still set aside: $($left.Count)"

# U4: launcher's mod and Vortex's plugin both left alone.
$claimed.Add("$G/Data/FeedMod.esp") | Out-Null; $claimed.Add("$G/Data/VortexOwned.esp") | Out-Null
$u4 = ((Hash $A "$G/Data/FeedMod.esp") -eq (Hash $B "$G/Data/FeedMod.esp")) -and ((Hash $A "$G/Data/VortexOwned.esp") -eq (Hash $B "$G/Data/VortexOwned.esp")) -and ((Hash $B "$G/Data/FeedMod.esp") -ne "absent")
Row "U4" $u4 "FeedMod.esp and VortexOwned.esp unchanged"

# U5 and U6: each file has the backup's bytes and the backup is gone.
foreach ($x in @(@("U5", "SAVES/plugins.txt"), @("U5", "SAVES/loadorder.txt"), @("U6", "$G/Skyrim.ccc"))) {
  $file = $x[1]; $bak = "$file.aetherial-dawn-backup"
  $claimed.Add($file) | Out-Null; $claimed.Add($bak) | Out-Null
  $restored = (Hash $A $bak) -ne "absent" -and (Hash $B $file) -eq (Hash $A $bak)
  $gone = -not $B.Files.ContainsKey($bak)
  Row $x[0] ($restored -and $gone) "$(Split-Path -Leaf $file): has the backup's bytes: $restored; backup removed: $gone"
}

# U7: launcher app data removed (config in ROAMING, the rest in LOCAL).
$u7 = @("ROAMING", "LOCAL" | Where-Object { $B.Roots[$_] -ne "missing" })
Row "U7" ($u7.Count -eq 0) "still present: $(if ($u7.Count) { $u7 -join ', ' } else { 'none' }) (the installer's 'delete app data' box: note if it was ticked)"

# Anything else that changed under FIXTURE or SAVES.
$keys = @($A.Files.Keys) + @($B.Files.Keys) | Where-Object { $_ -match "^(FIXTURE|SAVES)/" } | Sort-Object -Unique
foreach ($k in $keys) {
  if ($claimed.Contains($k)) { continue }
  if (-not $A.Files.ContainsKey($k)) { "UNEXPECTED`tadded`t$k" }
  elseif (-not $B.Files.ContainsKey($k)) { "UNEXPECTED`tremoved`t$k" }
  elseif ($A.Files[$k] -ne $B.Files[$k]) { "UNEXPECTED`tchanged`t$k" }
}
"INFO`tINSTALL`t$($B.Roots['INSTALL'])"
