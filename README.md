# flatground

An original skateboarding game for the Nintendo Wii, written in Rust. Physics-driven board, flick-stick
tricks on the C-stick, played on a GameCube controller. Runs from the Homebrew Channel.

Status: **skeleton**. One placeholder park (kicker, funbox, ledge, manual pad, bank), or six skate parks
converted from your own disc (chosen in a start menu, changed from the pause menu), and Skate 3-style
riding: goofy or regular, fakie and switch, held pushes and mongo pushes, foot and tail brakes,
powerslides and reverts, kick turns, Skate 3's speed wobble when riding very fast, 30 flick tricks (every flip, shove-it and varial, and their nollie
versions), held double and triple kickflips, air spins that reward a wind-up, grabs, manuals, hippie jumps.
The board follows any surface under its trucks: banks, curved transitions and vert (air out over the
coping, land back in fakie), wallrides across steep faces, pumping, and grinds and slides chosen by how the
board meets the rail (50-50, 5-0, nosegrind, crooked, willy, feeble, smith, boardslide, lipslide, noseslide,
tailslide). Bails come from what the board hits: walls, hard or off-axis landings. Tricks score as in
Skate 3 (its points, repetition penalty, grind, manual and grab curves, air height and spin), and linked
tricks build a line with Skate 3's multiplier and line timer (shown under the score); a bail ends the line.
Session markers (Z + d-pad) take you back to a spot, and a replay plays back the last 30 s. The camera is
Skate 3's: its Low or High camera (pause menu), free look on foot (Z + C-stick), shake on landings and
slow motion through long airs.
With your own Skate 3 disc the build also takes Skate 3's own riding numbers, trick points and
flick shapes from it. Tested on the PC and on an emulated Wii CPU. **Not yet run on a Wii or in Dolphin.**
Plan: [ROADMAP.md](ROADMAP.md).

## Controls (GameCube pad, port 1)
Laid out like Skate 3 on an Xbox 360 pad, by where each button sits (Xbox A, X, B, Y are the GameCube's A, B,
X, Y). The default stance is goofy, as in Skate 3.

| Input | Action |
|---|---|
| A (hold for a stronger push; keep holding to keep pushing) | Push |
| B | Mongo push |
| A + B, then let go | Hippie jump: the rider jumps off the board and lands back on it as it rolls on (hold longer to jump higher) |
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
| C-stick held lightly back / forward while rolling | Manual / nose manual (keep the balance marker centred; with Skate 3's motion graphs a manual lasts as long as the stick is held, as in Skate 3); fully over drags the tail or nose to brake |
| L or R fully in on the ground (a ground grab), then A / B | Boneless (A) / fastplant (B): the rider plants a foot, the board comes up with the other and he jumps off the planted foot (with the goofy stance L is frontside, R backside; regular the other way round) |
| A or B, then a flick on the C-stick while the pushing foot is still coming down | No-comply (a flick once the foot is down, or without pushing, is the flick's trick) |
| L / R in the air | Left-hand / right-hand grab (indy, melon; both: mute) |
| L / R in the air, then A / B (held) | One-foot grab (a foot off the board); about to meet a ledge, the coping or a wall, the free foot plants on it and the rider springs off it (foot plant, wallplant) |
| C-stick held in the air | Board adjust: turns the board across (left / right) or tips its nose down / up (forward / back) to line it up: across a rail it slides (boardslide) instead of grinding, and the landing goes by where the board points |
| Come down onto a rail | The grind or slide comes from how the board meets it: along it a 50-50 (C-stick held back: 5-0, forward: nosegrind), angled a crooked, willy, feeble or smith, across it a boardslide, lipslide, noseslide or tailslide. Flick to pop off |
| Control stick (and C-stick) left / right on a rail | Balance: pushes a slide across the rail (on a ledge, off it), leans a nose or tail slide in or out (too far out and it falls off the rail); fully over, a 50-50 drops onto one truck and turns across (crooked). Coming down hard on a rail with the board tilted against it is a wipeout |
| L / R fully in on a rail | Grind grab (the grabs Skate 3 has for that grind: FS / BS grab, nosegrab, tailgrab), scored with the grind |
| C-stick held back on a transition, then let go | Pump (gain speed) |
| Come down onto the coping and stay still on it | A coping stall, back into the ramp when it stops: on both trucks an axle stall, the board's middle on it a rock to fakie (control stick held to the side as it falls back in: rock and roll; the nose back over the ramp: disaster), a tip a nose / tail stall, over the deck a blunt stall. Skate 3 plays and scores them as the grind they are |
| Z held riding up a ramp to its coping | Hand plant (invert): the rider plants a hand on the coping, goes upside down over it and comes back down the ramp riding forwards. In it: C-stick tweaks it (up / down / left / right), A or B (held) takes one foot off, X both |
| Rolling tail first (fakie), push | The rider turns round and pushes switch |
| Y | Get off the board (on the ground), and back on it |
| Z in the air (with Skate 3's motion graphs) | Dark catch |
| Z + d-pad up | Put a session marker down where the rider is: riding on the ground with the board upright, or standing on foot, on ground a checkpoint could go on (Skate 3's rules; one marker, a new one replaces it) |
| Z + d-pad down, held | Back to the marker: after Skate 3's hold (0.2 s up to 100 m away, up to 1 s from 1 km; the screen washes out as it fills) the rider is put there at rest, facing the way he was, on the board or on foot as he was, as a respawn puts him back: the line ends. Not mid-bail or in the air (the hold waits) |
| Z + d-pad left | Replay of the last 30 s (see below) |
| Z + C-stick | Free look where Skate 3 has it: on foot (see below). On the board it does nothing (no flick, no dark catch); not in a hand plant (Z found the coping, the C-stick tweaks it) |
| D-pad | Character gestures (with Skate 3's motion graphs) |

Off the board (Y), with the board in hand:

| Input | Action |
|---|---|
| Control stick | Walk and run where the stick points, seen from the camera (pushed fully: run). Pulled back: turn round |
| A (hold) | Sprint: a burst that fades while held |
| B | Jump |
| Y | Back on the board, rolling the way the rider faces at his speed, in the stance he got off in. In the air (after B): he puts the board under his feet and lands on it |
| Y held, standing within 1.5 m of a ramp's edge facing it | Drop in: he steps up to the edge and stands on the board with its tail on the coping while Y is held; let go of Y to drop in, or pull the control stick back to step away |
| Z + C-stick up / down | Free look: the camera rises to look down on him (up) or drops to look up (down), up to Skate 3's 30 degrees; let go and it eases back |
| L (fully in) | Drop the board beside him (Skate 3's LT off the board) |
| R (fully in) | Throw the board down ahead of him, rolling (Skate 3's RT) |

With the board out of his hands (dropped or thrown):

| Input | Action |
|---|---|
| L or R (fully in) | Pick it back up: it flies back to his hands, from 30 m at most (further away it is out of sight) |
| Y | Get on: next to it, onto it where it lies (with Skate 3's graphs it is fetched into his hands for the mount); further away it is fetched first |
| B over it (running alongside it) | Jump onto it and ride away (the caveman): Skate 3's landing assist brings him down on its deck |
| START | Pause menu (see Menus) |
| HOME (Wii Remote) or START + Z | Quit to the Homebrew Channel at once |

**Z with the d-pad.** A d-pad press while Z is held, with the rider on the ground or on foot (not in the
air), is a chord: it goes to the marker or the replay, not to the gestures, and the Z of the chord then
counts as let go for the riding until you really let it go, so it can't dark-catch a flip off the next
pop. In the air the chord isn't taken: Z is the dark catch there and the d-pad its gestures. Z alone, and
the d-pad without Z, work as before.

**Z with the C-stick: free look.** The C-stick pushed off centre while Z is held, not in the air, is the
look stick: it never reaches the riding (no flick, manual or board adjust), and its Z counts as let go
until you let go of it (no dark catch). The camera only moves where Skate 3 lets its look stick move it:
in the shots with `OptionUseFreeCamStick`, which are the on-foot shot (`offboard_ground`) and the drop-in.
On the board the chord just does nothing. As in Skate 3 it tilts the camera up and down (Skate 3's
`FreeCamElevationOffset`, 30 degrees, eased in at 2 a second); its heading offset is 0 on the disc, so
left / right doesn't turn it. Let go of Z (or centre the stick) and the camera eases back to the shot; a
stick still off centre stays the look's until centred. A C-stick already pushed when Z goes down stays the
riding's (a wind-up isn't cut). In the air Z is the dark catch and the C-stick flicks, as before.

**The camera.** Besides the shots, Skate 3's camera shakes and slows time:
- Shake: a hand-held sway that grows with speed, and a jolt on every landing that dies away over a few
  seconds (Skate 3's `camera_shake` numbers; its close low camera sways, the high camera's longer arm
  hardly does). A bail fades the shake out.
- Slow motion: an air on the board with over 2 s of predicted flight (not off a grind) slows the game to
  as little as 35 ticks a second (0.58x) a fifth of the way through the flight, back to full speed by half
  way, as Skate 3's low camera does (its high camera never slows). Only the clock slows: the game runs
  the same 1/60 s steps, fewer of them a frame, drawn in between; the replay plays it back slowed.

**Replay** (Z + d-pad left): the last 30 s as they were drawn, opened paused on the latest moment; the
riding waits meanwhile and goes on exactly where it stopped when you leave.

| Input | Action |
|---|---|
| A | Play / pause (at the end: play from the start) |
| L / R (analog) | Scrub back / forward (up to four times real speed) |
| D-pad left / right | One frame back / on |
| D-pad down / up | Slower / faster: full speed, 1/2, 1/4 |
| Y | Next camera: the gameplay camera, a tripod (stays where the camera was, turns to follow the rider), an orbit round the rider (C-stick turns it) |
| X (Skate 3's B), START or Z + d-pad left | Back to the game |

The flick directions are for a goofy rider; regular and switch mirror them (a goofy kickflip flick is a
heelflip for a regular rider), as in Skate 3.

## Menus and locations
With parks converted from your own disc, the game starts in a **start menu** listing the locations (the
skate parks the build converted, then the built-in demo park), with the last one you chose highlighted
(`settings.txt` in the app folder remembers it; the first time, the build's default). Up/down on the d-pad or
the control stick moves, A (or START) rides. **START** while riding opens the **pause menu**: Resume, Change
location (the same list; B goes back), Draw distance, Camera, Respawn (back to the park's start), Quit to
loader. B or START resumes. **Draw distance** (left/right, 40 to 120 m in steps of 10, 90 by default) sets how
far a city district is drawn and read from the card, with the fog following it: shorter runs faster on a
real Wii. **Camera** (left/right or A) is Skate 3's Camera Angle: **Low** (the default, as Skate 3's camera
preferences start; close behind the board and nearly level with it) or **High** (Skate 3's high camera,
about 3 to 4.4 m back and 17 to 30 degrees up). `settings.txt` remembers both (a file from an older build
still loads, with the default camera).
While a menu is open the riding waits and the pad only drives the menu (the d-pad's gestures don't fire, and
the A that chose an item is not a push until you let go of it). Changing location shows a loading screen
while the park is read from the SD card (Dolphin reads the biggest, 4.2 MB, in well under a second; a real
SD card is slower). The list scrolls when it is long.

The menu also lists the **city districts** (Downtown, University, Industrial) and
**Skate School**, each up to four times, once per start spot (Downtown, Downtown 2, ...). A district is far
bigger than the Wii's memory, so it streams: 100 m cells are read from the SD card in the background as you
ride, nearest first, and dropped behind you; everything beyond 50 m fades into the sky, and if you outrun the
card the rider waits where the next cells aren't in yet. It runs in Dolphin; it hasn't been tried on a real
Wii yet. Details: [docs/importing.md](docs/importing.md), "Riding a district".

Without converted parks there is no start menu: the demo park is ridden straight away.

## Layout
```
game_core/       the game: no_std + alloc, unit-tested on the host and under qemu (PowerPC 750CL)
  flick.rs         the flick stick: gestures, crouch, manual hold
  gesture.rs       Skate 3-style stick-shape recogniser, the 30 flick tricks and their stance mirroring
  tuning.rs        every riding number (built in, or Skate 3's from the player's disc)
  skater.rs        board physics, stance, pushing, braking, landing rules, what each landed trick was
  scoring/         Skate 3's trick scoring: its trick catalogue, rewards, combo multiplier and lines
  marker.rs        session markers and the Z + d-pad chord
  replay.rs        the 30 s replay: the rolling recording of what was drawn, and its player
  camera/          Skate 3's camera: shots and blends, the camera graph's choice, rig, collision, framing,
                   free look, shake, slow motion
  park.rs          height-field park made of blocks and ramps (placeholder geometry in code)
  game.rs          update + render loop; draws only through gfx::Gfx
  menu.rs          locations (the index the importer writes), start and pause menus
  collision.rs     imported parks' collision: triangles in a grid (plain, or packed and read in place)
  world.rs         imported parks: models, textures, drawing within the draw distance
  city.rs          a district's cell index (cells, start spots)
  district.rs      a district streamed in cells while riding: what to read, keep and drop
  reader.rs        background file reads (the Wii's reader thread; an in-memory one for tests)
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
`FG_ISO` picks your disc image; `FG_PARK` the default location (default BlackBoxPark); `FG_CITY=0` leaves
out the city districts and Skate School (streamed locations, about 330 MB); `FG_NO_ASSETS=1`
skips converting;
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
