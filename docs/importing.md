# Importing from your own Skate 3 disc

flatground ships only original content. Players who own Skate 3 (Xbox 360) can convert assets from their
own disc image with `fg_import` (`tools/import`) and load them into the game. Nothing converted ever goes
into this repository (CLAUDE.md rule 1, enforced by `tools/check_no_game_data.sh` in CI).

## With the game's build (the usual way)
Make an `.iso` of your disc (a full XGD image or a plain XDVDFS image both work) and put it next to the repo
folder, or one folder down. `build.bat` (or `make -C wii package`) then runs `fg_import build`, which:

- finds the disc image: `FG_ISO`, else `iso=<path>` in a git-ignored `fg_local.txt` at the repo root, else the
  one `.iso` in the repo folder, the folder holding it, or a folder directly inside either (two or more:
  it asks you to pick one with `FG_ISO` or `fg_local.txt`). None: the game builds with its placeholder art.
- converts each pack the game loads into `imported/build/` (kept across `make clean`), and only when it is
  missing or out of date. Each pack has a `.stamp` next to it recording the converter's source hash, the disc
  image's size and modification time, and the conversion options; a different stamp means convert again. So
  pulling a converter fix re-converts once, and an unchanged build skips straight past.
- copies each pack into `dist/apps/flatground/data/` unless the copy there is already the same version.

Copy `dist/apps/flatground` to the SD card's `apps` folder and the game finds its packs at
`sd:/apps/flatground/data/` (or `usb:/`). `FG_NO_ASSETS=1` skips all of this. The Docker route only sees files
inside the repo folder, so put the image there (`*.iso` is git-ignored) or point `FG_ISO` at a path inside it.

`build.bat dolphin` then starts the game in Dolphin (the regular build from dolphin-emu.org) with its own
profile in `out/dolphin` and `dist/` as the virtual SD card, so it loads the same files a Wii would.

The game shows on start-up whether the imported skater loaded, and its CPU time per frame (average and the
worst frame of the last second; the budget at 60 Hz is 16.7 ms) in the top left corner.

## By hand
The converters can also be run one at a time:

```bash
cargo run --release -p fg_import -- textures path/to/skate3.iso --from data/content/parkassets.big --png
```

The output goes to `imported/` (git-ignored):

| File | What |
|---|---|
| `textures.fgpak` | Wii pack: textures already in GX formats, read in place by `game_core::pack` |
| `textures.txt` | Manifest: source asset → Xbox format, size → GX format, size |
| `png/` | With `--png`: PC previews to check the conversion |

Options: `--from` (repeatable) picks a disc directory, a loose `.rx2`, or a `.big` archive. The default is
`data/content/global_locators`. `--max` sets the largest texture edge (default 256). The Wii has 88 MB of RAM
in all, so keep packs small. `--rgba8` keeps alpha textures at 8 bits per channel.

### Animations
```bash
cargo run --release -p fg_import -- anims path/to/skate3.iso --clips OLLIE --clips KICKFLIP
```
Writes `imported/anims.fgpak` and `anims.txt`. The pack holds the skeleton (`anim/<bank>/skeleton`), the
clips (`anim/<bank>/<clip>`) and every reference pose (`anim/<bank>/pose/<name>`). The default bank is
`OnBoard.abin` from `data/big/miscload.big`: 36 bones and 2,672 clips. Converted whole it takes 47 MB, far
more than the Wii has, so pick clips with `--clips` (ollies and kickflips: about 2 MB). `--from` takes another
`.abin`, or a `.big` to take every bank in it.

Layouts: `game_core/src/anim.rs`. Rotations are stored as Q15 quaternions and translations as 16-bit
values scaled to each clip's range. Scale is stored only when a clip uses it.

Clips on the disc store motion relative to the bank's `RIG_TPOSE` pose, the game's bind pose. The importer
composes that pose in, the same arithmetic as the game's `AddBindPose` (rotation = bind ⊗ clip,
translation = bind.rotate(clip.t) + bind.t). Each exported sample is then a plain local transform, and
`world = local × parent world`. `--additive` keeps the raw deltas instead.

### The skater
```bash
cargo run --release -p fg_import -- skater path/to/skate3.iso --lod 2
```
Builds the default skater from the disc's fallback recipe (`data/cacrecipes/SavedRecipeFallbackMale.bin`).
That is 10 parts: arms, feet, hair, eyes/teeth, shirt, pants, head, deck, trucks and wheels. Writes
`imported/skater.fgpak` (about 1.1 MiB at `--lod 2`):

| Entry | Kind |
|---|---|
| `skater/skeleton` | the 36-bone animation skeleton (same as `anims`) |
| `skater/<slot>` | skinned mesh (`game_core/src/skinned.rs`): f32 positions, packed normals and UVs, 4 bone weights |
| `skater/<slot>/diffuse` | GX texture (CMPR; RGB5A3 for the alpha-tested hair) |
| `skater/clip/<role>` | the clips the game plays (below), bind pose composed in |

`--lod 0` (default, about 8.8k vertices) or `--lod 2` (about 2.3k vertices; use this for the Wii, which
skins on its CPU every frame). `--max` sets the texture size. `--png` renders front and side previews: bind
pose, plus frames of `--clip` (`--frame N` for one frame).

#### Playing it on the Wii
The build puts it in the app folder for you (above). By hand: copy `imported/skater.fgpak` to
`sd:/apps/flatground/data/skater.fgpak` (the older `sd:/flatground/skater.fgpak` works too; `usb:/` as well).
The game loads it at start-up and draws it instead of the box skater; without it, or if it doesn't parse, the
placeholder stays and a line on the start screen says why. To look at it on the PC first, run the game's preview with
`cargo run --release -p fg_preview -- out/preview --skater imported/skater.fgpak --every 6`.

The clip roles and their defaults from `OnBoard.abin` (the importer prints each clip and how upright the
deck is at its start and end):

| Role | Default clip | When |
|---|---|---|
| `idle` | `r_idle_ride_n_0_cyc` | rolling, and the fallback for anything missing |
| `crouch` | `r_antic_crouch_ollie_n_0_into` | flick stick held back (frame follows the crouch amount) |
| `push` | `r_pushlsp_lstr_mongo_left_cyc2` | after A, while the clip lasts |
| `lean/left`, `lean/right` | `r_idle_hcom_n100`, `r_idle_hcom_p100` | single poses blended over the riding clips by how hard you steer (`lean/crouch_*`: `r_idle_lcom_*`, while crouched) |
| `land/clean`, `land/heavy`, `land/sketchy` | `l_hcom_limp`, `l_hcom_himp`, `l_sketch_fs_hcom_limp` | after touching down: a normal landing, a big drop, a crooked catch |
| `manual`, `nosemanual` | `m_idle_n_0_cyc`, `m_noseidle_n_0_cyc` | main stick held down / up while rolling |
| `grab/fs`, `grab/bs`, `grab/mute` | `gr_grab_n_fs_0_cyc`, `gr_grab_n_bs_0_cyc`, `gr_mutegrab_n_0_cyc` | R, L, or both triggers held in the air |
| `bail/fall`, `bail/getup` | `w_brace_lean090twist000_n_0_n`, `w_recovery_onfront_n_0_n` | after a bail, one after the other |
| `<trick>/pop`, `/air`, `/flip`, `/out` | e.g. `kickflip_in_low_g`, `kickflip_in_low_a`, `t_low_kick_cyc1`, `t_low_kick_out4` | in the air, back to back |

Tricks are `ollie`, `nollie`, `kickflip`, `heelflip`, `popshuvit` and `fsshuvit`. `--anim role=clip`
replaces a default, and `--anim role=` drops one. The pop plays at its own speed; the stages after it are
sped up or slowed (0.6× to 2.5×) to end as the board lands, predicted from the jump. Many trick clips on the
disc expect the game to move the board itself (the `uflip`/`dcatch` catch clips end with the deck upside
down), so the defaults are chains whose deck ends grip up. Clips cross-fade over 0.12 s.

The game pins the trajectory root bone to the origin: clips carry forward travel in it, and the game moves the
skater itself. The character is drawn in board space with Skate's axes as they are (Y up, travel along +Z).

How the skinning fits together, all checked against the previews:
- Mesh bones are matched to the animation skeleton by name. Finger, twist and face bones follow their nearest
  animated ancestor, and so do eight `*Hlp` helper bones that have none (they bind to the root). Each
  palette entry carries the bind matrix of the bone that drives it, so skin matrix = bind × bone world.
- Bones bind to the bank's `RIG_TPOSE` (world matrices), not to the mesh's own bind matrices. The two agree in
  position to the millimetre, but each body bone's roll about its own axis differs (by up to 90°), so binding
  to the mesh frames twists the skin as soon as a clip plays (hunched shoulders, a bulging torso). The mesh
  binds, with a quarter turn about X for body bones, are only a fallback when a rest bone doesn't line up.
- Character positions are int16 × 2⁻¹⁴ with +0.8 m on Y (fixed by the game's shader). Normals are
  cross(tangent, binormal).
- Only the hair's alpha is coverage. The other diffuse textures keep masks in alpha, which is dropped.

### Water and menu tables (from the executable)
Two tables exist only inside the game's code: the ocean PCA water animation and the front-end menu tables.
**fg_import never decrypts executables and contains no console keys.** It reads them from an image you
have already decrypted yourself:
```bash
cargo run --release -p fg_import -- xex-tables path/to/decrypted.xex
```
It accepts an unencrypted XEX2 (stored raw or LZX-compressed) or the mapped PE image (`MZ`). For a PE image,
`--base` gives its load address (default `82000000`). The disc's own `default.xex` is still encrypted and is
refused with an explanation. Output: `imported/xex.fgpak` with `xex/ocean_pca` (30 frames × 7 rows × 4 f32,
played at 30 Hz) and `xex/menu_tables` (tab-separated text, format in `tools/import/src/xex/menu_tables.rs`).
Each table is optional; whatever is not found is reported and skipped.

Commands for exploring the disc: `ls`, `tree`, `big` (list an archive), `rx2` (list an arena's sections),
and `cat` (copy one asset out, using `archive.big::entry` for files inside a BIG).

## What it reads and how
Only plain disc files. `default.xex` is never decrypted or read.

| Layer | Module | Notes |
|---|---|---|
| Disc file system | `xdvdfs.rs` | XDVDFS/GDFX; finds the game partition of XGD1/2/3 images |
| `EB` BIG v3 archives | `big.rs` | Big-endian index. Entries are raw, RefPack, or `chunkref` (RefPack/zlib chunks). Read lazily from the ISO. |
| RefPack | `refpack.rs` | EA's LZ77 variant (`10 FB` / `90 FB` headers) |
| RW4 arenas (`.rx2`) | `rw4.rs` | 24-byte section records. The payload pointer sits in the record *before* the typed one. |
| Xenos textures | `texture.rs` | Untiles Xbox 360 tiling, including the packed mip tail when the fetch constant sets `packed_mips`. Decodes DXT1/3/5, ATI2, A8R8G8B8, B5G6R5 and A8. |
| GX encoding | `gx.rs` | DXT1 → CMPR transcode without loss (8×8 tiles, big-endian colours, flipped index order). Other formats are re-encoded to CMPR, RGB5A3 or RGBA8. |
| Pack | `pack.rs` / `game_core::pack` | `.fgpak` v1, layout documented in `game_core/src/pack.rs` |

Format knowledge comes from the public research in
[SK8-ENGINE/skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine) (GPL-3.0), which
builds on dumbad's reverse engineering. `fg_import` is GPL-3.0-only.

Also: `anim/abin` reads `.abin` banks (base-38 names, clip/pose/hierarchy records), `anim/vbr` decodes the
VBR codec (8-frame DCT blocks), and `xex/` covers unencrypted XEX2, LZX and the two table scanners. All are
ported from SK8. The console-exact `cos` and dot product are replaced with standard float math, which is
plenty for an offline converter.

## Not done yet
- Animation: RAW-codec records (faces, vehicles, gestures: 1,030 in `miscload.big`) and banks whose clips
  carry only some skeleton parts (the pedestrian bank: 462). SK8 does not decode these either.
- Skater: recipe colour tints (most parts look right untinted; hair and skin tones are baked into their
  textures here), morph targets (body shape and face sliders), other create-a-character outfits, and
  normal and specular maps.
- Game side: the imported skater is skinned on the CPU (GX transforms each vertex by one matrix, so it
  can't blend bone weights). It is checked in the PC preview only, not yet in Dolphin or on hardware, and
  its frame cost on the Wii is unmeasured (the HUD now shows it). Bails are clip-based: Skate 3 plays its
  wipeout clips over a physics ragdoll, which is not ported, so a bail is a step-off and a get-up.
- World geometry: district streams (`ATOC`/`SFIL` inside `worldDIST_*.big`) and RW4 meshes (vertex
  descriptors, index buffers) to Wii display lists.
- Collision (RW ClusteredMesh) and grind rails.
- The executable tables are tested on synthetic images only, until run on a real decrypted image.
- Audio: EA-XMA has no free decoder, so this would need a separate PCM/ADPCM conversion step.
- Game side: only `skater.fgpak` is loaded and drawn. Texture packs, animation packs and the executable
  tables are parsed and validated, but nothing uses them yet.
