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
wii/
  game/            Rust staticlib for the Wii: Gfx over the C layer, allocator, panic screen
  platform/        C layer over libogc (video/GX, pads, panic console) + main()
  Makefile         links flatground.dol with devkitPPC
tools/preview/     PC tool: runs the game with scripted input through a software rasteriser to PNGs
tools/check_symbols.sh  guards what the Wii staticlib may import
```
How the Rust/Wii build works and why: [docs/wii-build.md](docs/wii-build.md).

## Building
Host checks (any OS with Rust):
```bash
cargo test
```
```bash
cargo run --release -p fg_preview -- out/preview
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
