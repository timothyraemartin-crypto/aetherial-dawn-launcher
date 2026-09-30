<#
D18: read-only snapshot of every file under the given folders: path, size,
SHA-256 and read-only flag, one line each, sorted, so two snapshots can be
compared with: Compare-Object (Get-Content A.txt) (Get-Content B.txt)
It never opens the sign-in token's contents (it hashes files, prints no data).
#>
# -Paths takes folders separated by ";" (works the same with -File or -Command).
param([Parameter(Mandatory)][string]$Paths, [Parameter(Mandatory)][string]$Out)
$ErrorActionPreference = "SilentlyContinue"
$rows = foreach ($p in ($Paths -split ";" | Where-Object { $_ })) {
  if (-not (Test-Path $p)) { "MISSING`t$p"; continue }
  Get-ChildItem -LiteralPath $p -Recurse -Force -File | ForEach-Object {
    "{0}`t{1}`t{2}`tro={3}" -f $_.FullName, $_.Length, (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash, $_.IsReadOnly
  }
}
$rows | Sort-Object | Set-Content -Path $Out -Encoding UTF8
Write-Host "Saved: $Out ($(@($rows).Count) lines)"
