# Collects the game's session logs into out\logs: from every drive that has apps\flatground\logs (an SD card or USB
# stick from the Wii) and from dist\apps\flatground\logs (Dolphin's virtual SD card). Prints the newest log's path
# and its last lines.
#   powershell -ExecutionPolicy Bypass -File tools\get_logs.ps1 [-Lines 40]
param([int]$Lines = 40, [string]$Repo = (Split-Path -Parent $PSScriptRoot))
$ErrorActionPreference = 'Stop'

$dest = Join-Path $Repo 'out\logs'
New-Item -ItemType Directory -Force -Path $dest | Out-Null

$sources = @()
foreach ($d in Get-PSDrive -PSProvider FileSystem) {
    $p = Join-Path $d.Root 'apps\flatground\logs'
    if (Test-Path $p) { $sources += [pscustomobject]@{ Name = $d.Name; Path = $p } }
}
$dolphin = Join-Path $Repo 'dist\apps\flatground\logs'
if (Test-Path $dolphin) { $sources += [pscustomobject]@{ Name = 'dolphin'; Path = $dolphin } }
$sources = $sources | Sort-Object Path -Unique

if (-not $sources) {
    Write-Host '[logs] No logs found. Put the Wii''s SD card (or USB stick) in this PC, or run the game in Dolphin'
    Write-Host '       (build.bat dolphin), then run this again.'
    exit 1
}

$copied = @()
foreach ($s in $sources) {
    $to = Join-Path $dest $s.Name
    New-Item -ItemType Directory -Force -Path $to | Out-Null
    foreach ($f in Get-ChildItem -LiteralPath $s.Path -Filter 'session_*.txt') {
        Copy-Item -LiteralPath $f.FullName -Destination $to -Force
        $copied += Get-Item -LiteralPath (Join-Path $to $f.Name)
    }
    Write-Host "[logs] $($s.Path) -> $to"
}
$newest = $copied | Sort-Object Name | Select-Object -Last 1
if ($newest) {
    Write-Host "[logs] newest: $($newest.FullName)"
    Write-Host ''
    Get-Content -LiteralPath $newest.FullName -Tail $Lines
}
