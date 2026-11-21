# flatground: project rules

An original skateboarding game for the Nintendo Wii, written in Rust (see README.md, ROADMAP.md).

## Hard rules
1. **The repo holds original content only; game data comes from the player's own disc.** Nothing from a
   commercial game is ever committed or shipped: no extracted or converted files, textures, meshes, level
   layouts, audio, UI, logos, `.xex` code, and no excerpts of them in tests, fixtures, docs or CI artifacts.
   Players supply data by running `tools/import` (`fg_import`) on a Skate 3 (Xbox 360) disc image they own.
   Its output goes outside the repo or to git-ignored `/imported`, and `tools/check_no_game_data.sh` (CI) must
   pass. Importer tests use synthetic data built in the test. Format knowledge from public research (notably
   SK8-ENGINE/skate-3-rust-engine, GPL-3.0) is fine, with credit. `fg_import` is GPL-3.0-only. The game must
   still boot and play with its built-in placeholder art when no imported data is present.
   Simple values are fine: tuning numbers, curve points, thresholds and timings known from the game (or SK8)
   may be used directly as built-in defaults and constants. The rule is about assets and data files, not
   individual numbers.
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
- The simulation steps at a fixed 1/60 s (`game::TICK`) whatever the display rate; on PAL50 a frame runs one
  or two ticks and drawing blends between the last two. Keep simulation deterministic.
- Colours are `0xRRGGBBAA` (`gfx::rgb`).
