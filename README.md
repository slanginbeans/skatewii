# flatground

An original skateboarding game for the Nintendo Wii, written in Rust. Physics-driven board, flick-stick
tricks on the C-stick, played on a GameCube controller. Runs from the Homebrew Channel.

Status: **skeleton**. One placeholder park (kicker, funbox, ledge, manual pad, bank), pushing, carving,
ollies, nollies, kickflips, heelflips, shove-its, air spins, fakie landings and bails. Tested on the PC
and on an emulated Wii CPU. **Not yet run on a Wii or in Dolphin.** Plan: [ROADMAP.md](ROADMAP.md).

## Controls (GameCube pad, port 1)
| Input | Action |
|---|---|
| A | Push |
| B | Brake |
| Control stick | Steer; spin in the air |
| C-stick: pull back, then flick up | Ollie |
| ... flick up-left / up-right | Kickflip / heelflip |
| ... roll around the left / right edge up | Pop shove-it / FS shove-it |
| C-stick: push forward, then flick down | Nollie |
| HOME (Wii Remote) or START + Z | Quit to the Homebrew Channel |

## Layout
```
game_core/       the game: no_std + alloc, unit-tested on the host and under qemu (PowerPC 750CL)
  flick.rs         flick-stick trick recogniser
  skater.rs        board physics, landing rules, trick scoring
  park.rs          height-field park made of blocks and ramps (placeholder geometry in code)
  game.rs          update + render loop; draws only through gfx::Gfx
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

The game ships with original placeholder art only. If you own Skate 3 for Xbox 360, `fg_import` can convert
assets from your own disc image for your own use: see [docs/importing.md](docs/importing.md). No game data
is ever included in this repository.

## Building on Windows
Double-click **`build.bat`**. It updates from GitHub, builds the game and puts it in
**`dist\apps\flatground`**. Copy that folder into the SD card's `apps` folder.

It needs either devkitPro with "Wii Development" ticked
([installer](https://github.com/devkitPro/installer/releases/latest), installed to `X:\devkitPro` on any
drive; `build.bat` offers to install Rust for you), or Docker Desktop.

| Command | Does |
|---|---|
| `build.bat` | update, build, package |
| `build.bat noupdate` | build and package without updating |
| `build.bat clean` | delete the build output |
| `build.bat nopause` | don't wait for a key at the end |

Switches combine. Optional settings: `set FG_SD=E:` also copies the app to `E:\apps\flatground`;
`FG_DEVKITPRO` points at devkitPro if it's somewhere unusual; `FG_ASSUME_YES=1` installs Rust without asking.

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

To try it in Dolphin, open `wii/build/flatground.dol` and map a GameCube controller to port 1.
