# Session logs

Every run of the game on a Wii (or in Dolphin) writes a log to the SD card (or USB stick), next to the game:

```
apps/flatground/logs/session_YYYYMMDD_HHMMSS.txt
```

The newest 10 sessions are kept. Lines are buffered in memory and written out (and synced to the card, so a crash,
power-off or emulator stop keeps them) every 2 seconds. Errors, bails and crashes are written at once, and a crash also shows the last log lines on the
crash screen.

## Reading them
- **From the Wii:** put its SD card in the PC and run
  `powershell -ExecutionPolicy Bypass -File tools\get_logs.ps1`. It copies every session log it finds on any
  drive (and from Dolphin's virtual SD card, `dist\apps\flatground\logs`) into `out\logs\` and prints the newest
  one's last lines.
- **From Dolphin:** `build.bat dolphin` runs the game with `dist\` as its SD card. Dolphin copies the card back
  into `dist\` when emulation stops, so the logs appear in `dist\apps\flatground\logs` after you close it (the
  game's Dolphin profile stops without asking). If Dolphin isn't found, put `dolphin=<path to Dolphin.exe>` in
  `fg_local.txt` at the repo root (git-ignored).
- **Scripted test run in Dolphin:** `powershell -ExecutionPolicy Bypass -File tools\dolphin_test.ps1 [-Seconds 60]
  [-NoBuild] [-Manual] [-Lines 80]`. It builds (`build.bat fast noupdate nopause`, one Cargo job at a time unless
  `CARGO_BUILD_JOBS` says otherwise; `-NoBuild` skips it), puts `autoplay.txt` in the app folder, starts the
  game in Dolphin (as `build.bat dolphin` does), waits, then closes Dolphin by posting `WM_CLOSE` to its
  hidden main window (the profile has `ConfirmStop = False`, so emulation stops at once and the SD card is
  written back), collects the logs (`get_logs.ps1`) and prints this run's log. It fails (exit code 1) when no
  new log came back.
- **Hands-off play (autoplay):** with a file named `autoplay.txt` in the app folder (`sd:/apps/flatground/`
  or `usb:/apps/flatground/`, any contents) the game ignores the pad and plays `game_core::autoplay::Autoplay`:
  the trick tour (pushes, carves, a manual, ollies, a grab, a 360 flip, a half cab, a powerslide, a hippie
  jump, a bail) over and over, each tour starting once the rider is up again (`autoplay: tour N done` in the
  log). In an imported park the script doesn't know the layout, so a phase that hasn't finished after 20 s
  ends the tour. Delete the file to play normally again; `dolphin_test.ps1` removes it after its run.
- **On the PC without a Wii:** `cargo run --release -p fg_preview -- out/preview --tour --log` prints the same game
  lines as the preview plays.

## What is in them
At the start:
- the build (version and git commit), where the game was started from, the video mode (NTSC/PAL50/PAL60,
  progressive or interlaced, 4:3 or 16:9), free memory;
- each pack (rules, skater, park, motion): the file, its size and its build stamp (which converter version and disc image
  made it), and what it holds (for the motion library: clips, clips with poses, trees, attribute records, timed
  events/windows); the main tuning values in use; whether autoplay is on; the line shown on the start screen.

While playing:
- `frame:` once a second: the CPU's own work per frame (average and worst, split into physics, skinning and
  drawing), the time waiting for the GPU to finish, the idle time until the TV's next refresh, frames that missed
  their refresh, park meshes and vertices drawn, the vertex arena, heap and free memory. The budget at 60 Hz is
  16.7 ms for CPU + GPU wait; idle is what is left.
- `slow frame:` whenever a frame missed its refresh, with that frame's breakdown.
- `status:` once a second: the skater's mode, position, heading, speed, stance (switch, fakie), the camera's
  distance and how often a wall pulled it in, and the range each stick axis and trigger reached (for tuning the
  C-stick, whose range differs between pads).
- `flick:` every flick: the shape recognised, its strength (how fast), and the trick it became (after mirroring
  for the stance and the manual's nose/tail rules).
- `pop:`, `landed:` (with how crooked and how hard), `BAIL:` (with the reason and the numbers it was judged on),
  `stance:`, `manual:`, `grind:`, `powerslide`, `hippie jump`, `respawned`.

The on-screen readout (top left) shows the same frame numbers: `CPU avg PEAK worst  GPU wait  DROP missed`.

## How it fits together
- `wii/platform/fg_log.c`: the buffered file log (plain C, also tested on the PC: `wii/platform/tests/log_test.c`,
  run by CI on x86 and big-endian PowerPC).
- `wii/platform/fg_platform.c`: opens the log, measures the GPU and refresh waits and missed refreshes in
  `fg_end_frame`, reports memory, puts the log tail on the crash screen.
- `game_core::devlog`: the game's own lines, collected in `Game::log`; `Game::timer` lets the platform give the
  game a clock so it can time its sections.
- `wii/game/src/lib.rs`: writes the start-up lines and the `frame:` line, and passes the game's lines on.
