# flatground

An original skateboarding game for the Nintendo Wii, written in Rust. Physics-driven board, flick-stick
tricks on the C-stick, played on a GameCube controller. Runs from the Homebrew Channel.

Status: **skeleton**. One placeholder park (kicker, funbox, ledge, manual pad, bank), or six skate parks
converted from your own disc (chosen in a start menu, changed from the pause menu), and Skate 3-style
riding: goofy or regular, fakie and switch, held pushes and mongo pushes, foot and tail brakes,
powerslides and reverts, kick turns, 30 flick tricks (every flip, shove-it and varial, and their nollie
versions), held double and triple kickflips, air spins that reward a wind-up, grabs, manuals, hippie jumps.
The board follows any surface under its trucks: banks, curved transitions and vert (air out over the
coping, land back in fakie), wallrides across steep faces, pumping, and grinds and slides chosen by how the
board meets the rail (50-50, 5-0, nosegrind, crooked, willy, feeble, smith, boardslide, lipslide, noseslide,
tailslide). Bails come from what the board hits: walls, hard or off-axis landings. With your own Skate 3 disc the build also takes Skate 3's own riding numbers and
flick shapes from it. Tested on the PC and on an emulated Wii CPU. **Not yet run on a Wii or in Dolphin.**
Plan: [ROADMAP.md](ROADMAP.md).

## Controls (GameCube pad, port 1)
Laid out like Skate 3 on an Xbox 360 pad, by where each button sits (Xbox A, X, B, Y are the GameCube's A, B,
X, Y). The default stance is goofy, as in Skate 3.

| Input | Action |
|---|---|
| A (hold for a stronger push; keep holding to keep pushing) | Push |
| B | Mongo push |
| A + B, then let go | Hippie jump (hold longer to jump higher) |
| X | Foot brake |
| Control stick | Steer (pushed far to the side and a little back: hard turn); spin in the air (a quick wind-up just before the pop spins faster) |
| Control stick fully back and to one side | Powerslide; just after landing, a revert |
| Control stick hard to the side, nearly stopped | Kick turn |
| C-stick: pull back, then flick forward | Ollie (a quicker flick pops higher) |
| ... flick forward and to one side | Kickflip / heelflip (hold the flick at the end: double, triple) |
| ... roll round the back edge to the front | Pop shove-it / FS shove-it (further round: 360 shove-its) |
| ... roll a quarter round, then flick across | Varial kickflip / heelflip, hardflip, inward heelflip |
| ... roll three quarters round, then flick across | 360 flip, laser flip, 360 hardflip, 360 inward heelflip |
| C-stick: push forward, then flick back | Nollie, and the nollie version of each trick |
| C-stick held lightly back / forward while rolling | Manual / nose manual (keep the balance marker centred); fully over drags the tail or nose to brake |
| L / R in the air | Left-hand / right-hand grab (indy, melon; both: mute) |
| Come down onto a rail | The grind or slide comes from how the board meets it: along it a 50-50 (C-stick held back: 5-0, forward: nosegrind), angled a crooked, willy, feeble or smith, across it a boardslide, lipslide, noseslide or tailslide. Flick to pop off |
| C-stick held back on a transition, then let go | Pump (gain speed) |
| Rolling tail first (fakie), push | The rider turns round and pushes switch |
| Y | Get off the board (on the ground), and back on it |

Off the board (Y), with the board in hand:

| Input | Action |
|---|---|
| Control stick | Walk and run where the stick points, seen from the camera (pushed fully: run). Pulled back: turn round |
| A (hold) | Sprint: a burst that fades while held |
| B | Jump |
| Y | Back on the board, rolling the way the rider faces at his speed |
| START | Pause menu (see Menus) |
| HOME (Wii Remote) or START + Z | Quit to the Homebrew Channel at once |

The flick directions are for a goofy rider; regular and switch mirror them (a goofy kickflip flick is a
heelflip for a regular rider), as in Skate 3.

## Menus and locations
With parks converted from your own disc, the game starts in a **start menu** listing the locations (the
skate parks the build converted, then the built-in demo park), with the last one you chose highlighted
(`settings.txt` in the app folder remembers it; the first time, the build's default). Up/down on the d-pad or
the control stick moves, A (or START) rides. **START** while riding opens the **pause menu**: Resume, Change
location (the same list; B goes back), Respawn (back to the park's start), Quit to loader. B or START resumes.
While a menu is open the riding waits and the pad only drives the menu (the d-pad's gestures don't fire, and
the A that chose an item is not a push until you let go of it). Changing location shows a loading screen
while the park is read from the SD card (Dolphin reads the biggest, 4.2 MB, in well under a second; a real
SD card is slower).

Without converted parks there is no start menu: the demo park is ridden straight away.

## Layout
```
game_core/       the game: no_std + alloc, unit-tested on the host and under qemu (PowerPC 750CL)
  flick.rs         the flick stick: gestures, crouch, manual hold
  gesture.rs       Skate 3-style stick-shape recogniser, the 30 flick tricks and their stance mirroring
  tuning.rs        every riding number (built in, or Skate 3's from the player's disc)
  skater.rs        board physics, stance, pushing, braking, landing rules, trick scoring
  camera/          Skate 3's camera: shots and blends, the camera graph's choice, rig, collision, framing
  park.rs          height-field park made of blocks and ramps (placeholder geometry in code)
  game.rs          update + render loop; draws only through gfx::Gfx
  menu.rs          locations (the index the importer writes), start and pause menus
  character.rs     imported skinned character: pack parsing, clip sampling, CPU skinning
  rider.rs         picks and crossfades the imported skater's clips from the game state
wii/
  game/            Rust staticlib for the Wii: Gfx over the C layer, allocator, panic screen
  platform/        C layer over libogc (video/GX, pads, panic console) + main()
  Makefile         links flatground.dol with devkitPPC
tools/preview/     PC tool: runs the game with scripted input through a software rasteriser to PNGs
tools/import/      PC tool: converts assets from your own Skate 3 disc image into Wii packs (docs/importing.md)
tools/check_symbols.sh  guards what the Wii staticlib may import
```
How the Rust/Wii build works and why: [docs/wii-build.md](docs/wii-build.md).

Every session writes a log to the SD card (`apps/flatground/logs/`): frame timing, memory, every flick, landing
and bail with its numbers, and the stick ranges seen. `tools\get_logs.ps1` fetches them from the card. See
[docs/logging.md](docs/logging.md).

The game ships with original placeholder art only. If you own Skate 3 for Xbox 360, `fg_import` can convert
assets from your own disc image for your own use: see [docs/importing.md](docs/importing.md). No game data
is ever included in this repository.

## Building on Windows
Double-click **`build.bat`**. It updates from GitHub, builds the game and puts it in
**`dist\apps\flatground`**. Copy that folder into the SD card's `apps` folder.

If you own Skate 3 for Xbox 360 and keep an `.iso` of your disc next to this folder (or one folder down), the
build also converts what the game uses from it (the skater, and every skate park the Wii can hold as a
location; `FG_PARK` picks the default one) into `dist\apps\flatground\data` (about 37 MB). It only
converts what is missing or out of date, so later builds take seconds. Details:
[docs/importing.md](docs/importing.md).

It needs either devkitPro with "Wii Development" ticked
([installer](https://github.com/devkitPro/installer/releases/latest), installed to `X:\devkitPro` on any
drive; `build.bat` offers to install Rust for you), or Docker Desktop.

| Command | Does |
|---|---|
| `build.bat` | update, build, package |
| `build.bat noupdate` | build and package without updating |
| `build.bat clean` | delete the build output |
| `build.bat nopause` | don't wait for a key at the end |
| `build.bat dolphin` | build, then start the game in Dolphin (`dist` as its SD card) |
| `build.bat fast` | quicker build for testing (no link-time optimisation) |

Switches combine. Optional settings: `set FG_SD=E:` also copies the app to `E:\apps\flatground`;
`FG_DEVKITPRO` points at devkitPro if it's somewhere unusual; `FG_ASSUME_YES=1` installs Rust without asking;
`FG_ISO` picks your disc image; `FG_PARK` the default location (default BlackBoxPark); `FG_NO_ASSETS=1` skips converting;
`FG_DOLPHIN` points at `Dolphin.exe`.

On Linux/macOS with Docker: `./build.sh`. No installs at all: download `flatground-sd-card` from the newest
**ci** run under the repo's Actions tab.

## Building by hand
Host checks (any OS with Rust):
```bash
cargo test
```
```bash
cargo run --release -p fg_preview -- out/preview
```
With a skater pack you converted from your own disc (`--every N` adds close-ups every N frames):
```bash
cargo run --release -p fg_preview -- out/preview --skater imported/skater.fgpak
```
Game logic on an emulated Wii CPU (Debian/Ubuntu: `qemu-user gcc-powerpc-linux-gnu libc6-dev-powerpc-cross`):
```bash
cargo test -p game_core --target powerpc-unknown-linux-gnu
```
The Wii `.dol` needs [devkitPro](https://devkitpro.org/wiki/Getting_Started) with the `wii-dev` group:
```bash
make -C wii package
```
Then copy `dist/apps/flatground` to `SD:/apps/`. No local devkitPro? Every CI run builds the `.dol`: download
the `flatground-sd-card` artifact from the run's page.

To try it in Dolphin: `build.bat dolphin` (Windows), or open `dist/apps/flatground/boot.dol` with Dolphin's
SD card set to sync from the `dist` folder (Config, Wii, SD Card). Map a GameCube controller to port 1.
A scripted test run (build, play hands-off for a minute, close Dolphin, print the session log):
`powershell -ExecutionPolicy Bypass -File tools\dolphin_test.ps1` (docs/logging.md).
