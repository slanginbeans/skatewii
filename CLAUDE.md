# flatground: project rules

An original skateboarding game for the Nintendo Wii, written in Rust (see README.md, ROADMAP.md).

## Hard rules
1. **Original content only.** No code, data, assets, level layouts, characters, names, logos, UI or audio from
   any commercial game, and nothing extracted from game discs or ports. Generic skateboarding (trick names, park
   features, flick-stick controls as a genre mechanic) is fine. Placeholder art is built in code or authored
   for this project.
2. **`game_core` stays `no_std` + `alloc`** and platform-free: it draws only through `gfx::Gfx` and reads
   only `input::PadRaw`. Use `libm`, not float intrinsics that need a C math library.
3. **The C/Rust boundary is `wii/platform/fg_platform.h`.** Change it, `wii/game/src/ffi.rs` and the
   `repr(C)` types in `game_core` (`Vertex`, `PadRaw`) together; the layout tests must still pass.
4. **The Wii staticlib may only import what `tools/check_symbols.sh` allows.** If you need something new
   from C, add it to the platform layer and the allow-list on purpose.
5. Gameplay changes come with tests (unit tests, or a scripted run in `game_core/tests/`).

## Build and test
- Host: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Wii CPU: `cargo test -p game_core --target powerpc-unknown-linux-gnu` (needs qemu-user + powerpc cross gcc).
- Wii staticlib: `cargo build --release -p fg_game --target powerpc-unknown-linux-gnu`, then
  `tools/check_symbols.sh target/powerpc-unknown-linux-gnu/release/libfg_game.a`.
- `.dol`: `make -C wii package` (needs devkitPro), or `./build.sh` (Docker). Without either, CI's `wii` job builds it.
- Windows users run `build.bat` (finds devkitPro's msys2 on any drive and runs `tools/winbuild.sh` in it, else
  Docker via `tools/docker-build.sh`). Keep `.bat` files ASCII with CRLF endings (`.gitattributes`); CI's
  `windows-bat` job checks its switches and the no-tools message.
- Look at frames: `cargo run --release -p fg_preview -- out/preview` (PNG output, git-ignored).

## Conventions
- Y up, metres, seconds. Yaw 0 faces +Z, positive yaw turns towards +X.
- Fixed timestep from the platform (1/60, or 1/50 on PAL50). Keep simulation deterministic.
- Colours are `0xRRGGBBAA` (`gfx::rgb`).
