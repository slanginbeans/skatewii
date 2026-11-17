# How Rust gets onto the Wii

## Decision
- Rust Wii code compiles as a `no_std` **staticlib** for the stock target `powerpc-unknown-linux-gnu` on
  **stable** Rust, with `-C target-cpu=750 -C relocation-model=static` (`.cargo/config.toml`).
- devkitPPC's `powerpc-eabi-gcc` links it with libogc into an ELF; `elf2dol` makes the `.dol`.
- libogc is reached through a small hand-written C layer (`wii/platform/fg_platform.h`), not bindgen.

The same approach is used by other Rust-on-Wii projects (for example modernwiifare, whose ADR 0006
describes it); this project's code is written independently.

## Why
- The prebuilt `core`/`alloc` for that target are big-endian 32-bit PowerPC with hard float: the same
  machine model as the Wii's Broadway (a PowerPC 750CL). No nightly, no `-Z build-std`, no custom target JSON.
- Many GX calls are `static inline` FIFO writes in libogc's headers, which bindgen can't bind usefully. A C
  layer is simpler and gives Rust a narrow, stable API.

## Known differences and how they're handled
| Issue | Handling |
|---|---|
| The prebuilt `core` references `_Unwind_Resume` / `rust_eh_personality` | `panic = "abort"`; weak stubs in `fg_platform.c` satisfy the linker and are never reached. |
| SysV (16-byte) vs EABI (8-byte) stack alignment | Rust keeps 16-byte alignment internally; calls into C are compatible. Verify on hardware. |
| No PIC/GOT in a fixed-address `.dol` | `relocation-model=static`. |
| Heap | `GlobalAlloc` over newlib `malloc`/`memalign` (MEM1 arena). Big assets will go to MEM2 later. |
| Float math | `libm` crate in Rust (no C math needed beyond what `check_symbols.sh` allows). |

## Verified so far
- `game_core` tests pass on x86-64 and on qemu emulating a 750CL (big-endian, same float behaviour).
- The staticlib's only external symbols are newlib (`malloc`, `free`, `realloc`, `memcpy`, `memset`),
  the `fg_*` C functions and the two weak unwind stubs (`tools/check_symbols.sh`).
- `fg_platform.c` type-checks against the current libogc headers with warnings on.
- **Not yet verified:** the devkitPPC link (CI `wii` job), Dolphin, hardware.

## Fallback
If the link or ABI proves unsound: a custom `powerpc-unknown-eabi` target JSON with nightly and
`-Z build-std=core,alloc`. The C API stays the same.
