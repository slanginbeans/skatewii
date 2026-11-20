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

The game shows on start-up whether the imported skater, park and rules loaded (`RULES: DISC` or
`BUILT-IN`), and its CPU time per frame (average and the
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
`imported/skater.fgpak` (about 2 MiB at `--lod 2`):

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
| `crouch`, `crouch_nollie` | `r_antic_crouch_ollie_n_0_into`, `r_antic_crouch_nollie_n_0_into` | flick stick held back / forward (frame follows the crouch amount) |
| `push/into`, `push/cyc1`, `push/cyc2` | `r_pushlsp_hstr_n_0_*` | A: the first push of a run, then alternating cycles |
| `mongo/into`, `mongo/cyc1`, `mongo/cyc2` | `r_pushlsp_hstr_mongo_0_*` | B, or A when riding switch (as Skate 3 does) |
| `brake/into`, `brake/cyc`, `brake/out` | `r_brake_n_n_0_*` | X held: the foot goes down, drags, comes back up |
| `tailbrake`, `nosebrake` | `m_brake_n_0_cyc`, `m_nosebrake_n_0_cyc` | C-stick pushed past the manual zone |
| `slide/fs`, `slide/bs` | `r_turn_180slide_fs_n_0_n`, `..._bs_...` | powerslides (held at the sideways point) and reverts (whole) |
| `kickturn/fs`, `kickturn/bs` | `r_kickturn_stat_fs`, `..._bs` | stick hard to the side when nearly stopped |
| `fakie` | `fakie_head_channel_cyc` | rolling tail first: its neck and head (and part of the upper back) replace the riding clip's, so the rider looks where the board is going |
| `hippy/antic`, `hippy/into`, `hippy/air` | `t_hippyjump_low_n_0_*` | A + B held, then the jump |
| `lean/left`, `lean/right` | `r_idle_hcom_n100`, `r_idle_hcom_p100` | single poses blended over the riding clips by how hard you steer (`lean/crouch_*`: `r_idle_lcom_*`, while crouched) |
| `land/clean`, `land/heavy`, `land/sketchy_fs`, `land/sketchy_bs` | `l_hcom_limp`, `l_hcom_himp`, `l_sketch_fs_hcom_limp`, `l_sketch_bs_hcom_limp` | after touching down: a normal landing, a big drop, a crooked catch turned frontside / backside |
| `manual`, `nosemanual` | `m_idle_n_0_cyc`, `m_noseidle_n_0_cyc` | C-stick held lightly back / forward while rolling |
| `grab/fs`, `grab/bs`, `grab/mute` | `gr_grab_n_fs_0_cyc`, `gr_grab_n_bs_0_cyc`, `gr_mutegrab_n_0_cyc` | rear hand, front hand, or both (L and R are the left and right hand; which is which depends on the stance) |
| `bail/fall`, `bail/getup` | `w_brace_lean090twist000_n_0_n`, `w_recovery_onfront_n_0_n` | after a bail, one after the other |
| `grind/5050`, `grind/bslide` | `g_5050_fs_low_0_cyc`, `g_bslide_fs_low_0_cyc` | on a rail, board along it / across it |
| `<trick>/pop`, `/air`, `/flip`, `/out` | e.g. `kickflip_in_low_g`, `kickflip_in_low_a`, `t_low_kick_cyc1`, `t_low_kick_out4` | in the air, back to back |

Tricks are `ollie`, `nollie`, `kickflip`, `heelflip`, `popshuvit`, `fspopshuvit`, `360popshuvit`,
`fs360popshuvit`, `varialkickflip`, `varialheelflip`, `hardflip`, `inwardheelflip`, `360flip`, `laserflip`,
`360hardflip`, `360inwardheelflip`, and each with an `n_` prefix for its nollie version (the plain nollie is
`nollie`). Most have a `pop` and an `air` clip (`<name>_low_g`, `<name>_low_a`); kickflips and heelflips also
`flip` (played once per flip of a held double or triple) and `out`. `--anim role=clip`
replaces a default, and `--anim role=` drops one. The pop plays at its own speed; the stages after it are
sped up or slowed (0.6× to 2.5×) to end as the board lands, predicted from the jump. Many trick clips on the
disc expect the game to move the board itself (the `uflip`/`dcatch` catch clips end with the deck upside
down), so the defaults are chains whose deck ends grip up. Clips cross-fade over 0.12 s.

The game pins the trajectory root bone to the origin: clips carry forward travel in it, and the game moves the
skater itself. The character is drawn in board space with Skate's axes as they are (Y up, travel along +Z).
The clips are authored for a goofy rider (right foot towards the nose). A regular rider, or a goofy one riding
switch, is drawn as their mirror image across the board's long axis: each bone drives its left/right
partner's vertices (the skeleton's mirror pairs), reflected.

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

### A park
```bash
cargo run --release -p fg_import -- park path/to/skate3.iso --park BlackBoxPark
```
Writes `imported/park.fgpak` (about 500 KB): `park/collision` (every collision triangle, format in
`game_core/src/collision.rs`), `park/spawn` and `park/rails` (grind rails, `game_core/src/rails.rs`). The build converts `BlackBoxPark` unless `FG_PARK` names
another. Parks small enough for the Wii: `StartPark` (the opening warehouse: an empty floor), `MegaPark`,
`BlackBoxPark`, `IndustrialSkatePark`, `DownTownSkatePark`, `SkateSchool` and `MaloofMoneyCup`. The city
districts are hundreds of MB each.

A district's assets sit in streams inside `worldDIST_<park>.big`: an `ATOC` table
(`DIST_<park>_<stream>.xst`) and `SFIL` stream files (`c<stream>_*.xsf`, RefPack-compressed sections). The
`Sim` stream holds RW4 arenas with RenderWare ClusteredMesh collision (`tools/import/src/world.rs`), already in
world space: Y up, metres. 16-bit compressed vertex offsets are unsigned, as the game reads them (SK8's
correction; signed, big clusters wrap by 65.5 m). The start is the broad flat face nearest the district
origin on the main floor (the lowest height band holding at least 30% of the flat area), facing the middle.

In the game, walkable triangles (normal at least 0.35 up, about 70°) are ground and steeper ones are walls. The
board stands on the highest walkable triangle under it no higher than a step above it, so ledges and
overhangs work at any level. Transitions steeper than about 70° are walls until vert riding exists.

The park is drawn with its own models from the `Pres` stream (`tools/import/src/parkmodel.rs`): RW4 arenas of
several meshes, already in world space, with float positions and half-float UVs. A mesh's texture comes
through its material handle, the external-reference table (GUID to handle), the material whose `Name`
carries that GUID, its `diffuse` name and the texture asset of that name (SK8's chain). Textures are
shrunk to `--max` (default 128) and stored as CMPR, or RGB5A3 with alpha testing for cut-outs; lighting from
the sun is baked into the vertex colours. BlackBoxPark: 132 meshes, 15,670 vertices, 20 textures, about
1 MB in all. Meshes within 90 m of the camera are drawn. `--max 0` leaves the models out, and the collision
is drawn instead (flat shaded, coloured by surface).

Grind rails are the game's spline sections (type `0x00EB0004`) in the same `Sim` assets: chains of cubic
segments `D + C t + B t² + A t³`, sampled at 4 points per segment (SK8's `retail_grind_splines.py`).
BlackBoxPark has 120 rails (about 1.35 km); MegaPark has none. Coming down within 0.4 m of a rail, with any
flip finished and no grab held, locks on: a 50-50 with the board within 30° of the rail, a boardslide within
30° of across it. On the rail gravity and a little friction act along it; the end of the rail, running out of
speed or a flick takes you off, and that scores the grind.

### Riding rules
```bash
cargo run --release -p fg_import -- rules path/to/skate3.iso
```
Writes `imported/rules.fgpak` (about 16 KB) with what makes the skating feel like Skate 3:

- `rules/tuning`: the riding numbers flatground's skating reads (`game_core::tuning::PATHS`), taken from
  the game's tuning database (`data/big/db.big`: `data/db/skaterschema` and `skatercollections`, EA's
  AttribSys vault format, read by `attrib.rs`). Push strength and top push speed, rolling resistance, foot
  and tail brakes, steering against stick and speed, jump heights, the air spin curves, the landing limits,
  the flick recogniser's tolerance and the low camera's shots ("normal" difficulty, the game's default).
  Values are found by hashing their names, as the game does; inherited values are followed up the parent
  chain.
- `rules/gestures/<file>`: the flick-stick shapes from `data/joystick/*.pat` in `miscload.big`
  (`skater.pat` drives the ground flick tricks; the rest are kept for later: board sideways, late flips,
  finger flips, the left stick).

Without it the game uses its own built-in values and shapes, which only approximate Skate 3's.

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
| Tuning database | `attrib.rs` | AttribSys vault pairs (`.vlt` + `.bin`): pointer fix-ups, exports, class field definitions, collections with parent inheritance; 64-bit lookup8 name hashes |
| Flick shapes | `rules.rs` | `.pat` text: `global_tolerance_dist`, `pattern`, `tolerance_dist`, `coord` |

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
- Park visuals: lightmaps (the second UV set), display lists instead of copying vertices every frame, and
  the city districts' streaming cells.
- The executable tables are tested on synthetic images only, until run on a real decrypted image.
- Audio: EA-XMA has no free decoder, so this would need a separate PCM/ADPCM conversion step.
- Riding: Skate 3 turns, pumps and wipes out through rigid-body truck and ragdoll physics; flatground keeps
  a simpler board model and uses Skate 3's numbers where they carry over (see `game_core::tuning`). The
  board-sideways, late-flip and finger-flip shapes are converted but not used yet, nor dark catches.
- Game side: `skater.fgpak`, `park.fgpak` and `rules.fgpak` are loaded. Texture packs, animation packs and the executable
  tables are parsed and validated, but nothing uses them yet.
