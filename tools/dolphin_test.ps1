# Scripted test run in Dolphin: builds the game (quick "fast" profile), starts it in Dolphin with hands-off
# play (autoplay.txt in the app folder: the trick tour, repeated), lets it run, closes Dolphin cleanly (so
# it writes the virtual SD card, and with it the game's session log, back to dist\) and prints the new log.
#   powershell -ExecutionPolicy Bypass -File tools\dolphin_test.ps1 [-Seconds 60] [-NoBuild] [-Lines 80]
#     -Seconds   how long the game runs (default 60)
#     -NoBuild   use the boot.dol already in dist\apps\flatground
#     -Manual    don't turn on autoplay (play it yourself; Dolphin still closes after -Seconds)
#     -Minimized start Dolphin minimized (no windows popping up; the log still comes back)
# Exit code 0 when a fresh session log came back, 1 otherwise.
param(
    [int]$Seconds = 60,
    [switch]$NoBuild,
    [switch]$Manual,
    [switch]$Minimized,
    [int]$Lines = 80,
    [string]$Repo = (Split-Path -Parent $PSScriptRoot)
)
$ErrorActionPreference = 'Stop'

if (-not $NoBuild) {
    Write-Host '[test] Building (build.bat fast noupdate nopause)...'
    # One job at a time keeps memory use down on small machines; set CARGO_BUILD_JOBS yourself to override.
    if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = '1' }
    & cmd /c "`"$(Join-Path $Repo 'build.bat')`" fast noupdate nopause"
    if ($LASTEXITCODE -ne 0) {
        Write-Host '[test] Build failed.'
        exit 1
    }
}

$app = Join-Path $Repo 'dist\apps\flatground'
$autoplay = Join-Path $app 'autoplay.txt'
$logs = Join-Path $app 'logs'
if (-not $Manual) {
    Set-Content -LiteralPath $autoplay -Value 'trick tour (written by tools\dolphin_test.ps1)' -Encoding ascii
}
$before = @()
if (Test-Path $logs) { $before = @(Get-ChildItem -LiteralPath $logs -Filter 'session_*.txt' | ForEach-Object Name) }

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class FgWin {
    delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr param);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    // Top-level windows (hidden ones too) of a process whose title starts with `prefix`.
    public static List<IntPtr> Find(uint pid, string prefix) {
        var found = new List<IntPtr>();
        EnumWindows((h, p) => {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner == pid) {
                var s = new StringBuilder(256);
                GetWindowText(h, s, 256);
                if (s.ToString().StartsWith(prefix)) found.Add(h);
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

$status = 1
try {
    $process = & (Join-Path $PSScriptRoot 'dolphin.ps1') -Repo $Repo -PassThru -Minimized:$Minimized | Select-Object -Last 1
    if (-not ($process -is [System.Diagnostics.Process])) {
        Write-Host '[test] Dolphin did not start.'
        exit 1
    }
    Write-Host "[test] Running for $Seconds s (Dolphin pid $($process.Id))..."
    Start-Sleep -Seconds $Seconds
    if ($process.HasExited) {
        Write-Host "[test] Dolphin exited by itself (code $($process.ExitCode))."
    } else {
        # The main window is hidden in batch mode (-b); closing it stops emulation the normal way, which
        # writes the virtual SD card back (ConfirmStop is off in the profile, see dolphin.ps1).
        $windows = [FgWin]::Find([uint32]$process.Id, 'Dolphin')
        Write-Host "[test] Closing Dolphin ($($windows.Count) window(s))..."
        foreach ($w in $windows) { [void][FgWin]::PostMessage($w, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
        if (-not $process.WaitForExit(30000)) {
            Write-Host '[test] Dolphin did not close within 30 s; killing it (the SD card is not written back).'
            $process.Kill()
        }
    }
} finally {
    if (Test-Path $autoplay) { Remove-Item -LiteralPath $autoplay -Force }
}

$new = @()
if (Test-Path $logs) {
    $new = @(Get-ChildItem -LiteralPath $logs -Filter 'session_*.txt' | Where-Object { $before -notcontains $_.Name } | Sort-Object Name)
}
& (Join-Path $PSScriptRoot 'get_logs.ps1') -Repo $Repo -Lines 0 | Out-Null
Write-Host "[test] Logs copied to $(Join-Path $Repo 'out\logs') (tools\get_logs.ps1)."
if ($new.Count -gt 0) {
    # get_logs prints the newest log of any drive (an SD card in the PC may hold newer ones): show this run's.
    $log = $new[-1].FullName
    Write-Host ''
    Write-Host "[test] This run's session log: $log"
    Write-Host ''
    Get-Content -LiteralPath $log -TotalCount 20
    Write-Host '...'
    Get-Content -LiteralPath $log -Tail $Lines
    $status = 0
} else {
    Write-Host '[test] No new session log came back from Dolphin.'
}
exit $status
