# Starts the packaged game (dist\apps\flatground\boot.dol) in Dolphin, with dist\ as the virtual SD card, so
# the game finds its converted packs at sd:/apps/flatground/data/ exactly as on a real Wii.
# Run by `build.bat dolphin` after a build. Uses its own Dolphin profile (out\dolphin) and leaves the
# player's normal Dolphin settings alone.
#   FG_DOLPHIN=C:\path\to\Dolphin.exe   if Dolphin isn't found by itself
#   -PassThru   return Dolphin's process (tools\dolphin_test.ps1 uses it to close Dolphin again)
param([string]$Repo = (Split-Path -Parent $PSScriptRoot), [switch]$PassThru)
$ErrorActionPreference = 'Stop'

function Find-Dolphin {
    if ($env:FG_DOLPHIN -and (Test-Path $env:FG_DOLPHIN)) { return $env:FG_DOLPHIN }
    # The repo's local settings (git-ignored): dolphin=<path to Dolphin.exe>
    $local = Join-Path $Repo 'fg_local.txt'
    if (Test-Path $local) {
        foreach ($line in Get-Content -LiteralPath $local) {
            if ($line -match '^\s*dolphin\s*=\s*(.+?)\s*$' -and (Test-Path $Matches[1])) { return $Matches[1] }
        }
    }
    $candidates = @(
        "$env:ProgramFiles\Dolphin\Dolphin.exe",
        "$env:ProgramFiles\Dolphin-x64\Dolphin.exe",
        "$env:ProgramFiles\Dolphin Emulator\Dolphin.exe",
        "${env:ProgramFiles(x86)}\Dolphin\Dolphin.exe",
        "$env:LOCALAPPDATA\Programs\Dolphin\Dolphin.exe",
        "$env:USERPROFILE\Downloads\Dolphin-x64\Dolphin.exe",
        "$env:USERPROFILE\Desktop\Dolphin-x64\Dolphin.exe"
    )
    foreach ($c in $candidates) { if ($c -and (Test-Path $c)) { return $c } }
    # On the PATH?
    $cmd = Get-Command Dolphin.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return $null
}

# Sets key=value under [section] of an ini file, keeping everything else.
function Set-IniValue([string]$Path, [string]$Section, [string]$Key, [string]$Value) {
    $lines = [System.Collections.Generic.List[string]]::new()
    if (Test-Path $Path) { $lines.AddRange([string[]](Get-Content -LiteralPath $Path)) }
    $start = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i].Trim() -ieq "[$Section]") { $start = $i; break }
    }
    if ($start -lt 0) {
        $lines.Add("[$Section]")
        $lines.Add("$Key = $Value")
    } else {
        $end = $lines.Count
        for ($i = $start + 1; $i -lt $lines.Count; $i++) {
            if ($lines[$i].Trim().StartsWith('[')) { $end = $i; break }
        }
        $found = $false
        for ($i = $start + 1; $i -lt $end; $i++) {
            if ($lines[$i] -match "^\s*$([regex]::Escape($Key))\s*=") { $lines[$i] = "$Key = $Value"; $found = $true; break }
        }
        if (-not $found) { $lines.Insert($end, "$Key = $Value") }
    }
    Set-Content -LiteralPath $Path -Value $lines -Encoding ascii
}

$dol = Join-Path $Repo 'dist\apps\flatground\boot.dol'
if (-not (Test-Path $dol)) {
    Write-Host "[dolphin] $dol isn't there; the build didn't finish."
    exit 1
}
$dolphin = Find-Dolphin
if (-not $dolphin) {
    Write-Host '[dolphin] Dolphin not found. Install it from https://dolphin-emu.org (the normal build, not'
    Write-Host '          Slippi), or set FG_DOLPHIN to the full path of Dolphin.exe, then run'
    Write-Host '          build.bat dolphin again.'
    exit 1
}

$user = Join-Path $Repo 'out\dolphin'
$sdFolder = Join-Path $Repo 'dist'
New-Item -ItemType Directory -Force -Path (Join-Path $user 'Config'), (Join-Path $user 'Wii') | Out-Null
$ini = Join-Path $user 'Config\Dolphin.ini'
# Virtual SD card: insert it, build it from dist\ at every start (Dolphin's "Sync SD Folder").
Set-IniValue $ini 'Core' 'WiiSDCard' 'True'
Set-IniValue $ini 'Core' 'WiiSDCardEnableFolderSync' 'True'
Set-IniValue $ini 'General' 'WiiSDCardSyncFolder' ($sdFolder + '\')
Set-IniValue $ini 'General' 'WiiSDCardPath' (Join-Path $user 'Wii\sd.raw')
# GameCube pad in port 1 (the game's controls); 6 = standard controller.
Set-IniValue $ini 'Core' 'SIDevice0' '6'
# Stop without a confirmation box: Dolphin writes the virtual SD card back to dist\ (the game's logs) when
# emulation stops, so closing the window cleanly is what brings the logs out.
Set-IniValue $ini 'Interface' 'ConfirmStop' 'False'
# Show the frame rate on screen.
Set-IniValue (Join-Path $user 'Config\GFX.ini') 'Settings' 'ShowFPS' 'True'

Write-Host "[dolphin] $dolphin"
Write-Host "[dolphin] Profile: $user (controller mapping: Controllers in Dolphin's menu)"
Write-Host "[dolphin] SD card: $sdFolder"
$process = Start-Process -FilePath $dolphin -ArgumentList @('-u', "`"$user`"", '-b', '-e', "`"$dol`"") -PassThru
if ($PassThru) { $process }
