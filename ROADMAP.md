# flatground roadmap

**Goal:** a skateboarding game that feels great on a GameCube pad at 60 fps on a real Wii. Feel first,
content second.

## Phase 0: Skeleton (done in code, awaiting first Wii run)
- [x] Workspace: `game_core` (no_std, host-tested), Wii staticlib, C platform layer, Makefile, CI
- [x] Rust on the Wii via the stock `powerpc-unknown-linux-gnu` target on stable (docs/wii-build.md)
- [x] Core tests pass on an emulated PowerPC 750CL (qemu) as well as on x86-64
- [x] Staticlib imports only newlib + our C layer (`tools/check_symbols.sh`, in CI)
- [x] C layer type-checks against libogc headers; CI links the `.dol` in the devkitPPC container
- [x] Flick-stick recogniser, board physics, landing rules, chase camera, HUD text, placeholder park
- [x] PC preview tool rendering scripted runs to PNG
- [ ] **First run in Dolphin**, then on hardware: boots, 60 fps, pad works, HOME exits
- [ ] Panic screen checked on hardware (force one from a debug button)

## Phase 1: Feel
- [x] Skate 3's flick recognition (stick shapes, tolerance, misses, flick speed sets the pop) with its own
  shapes from the player's disc; flatground's own shapes without it
- [ ] Tune flick thresholds on a real C-stick (its range and gate differ from the main stick)
- [x] Pushing animation, carve lean (imported skater)
- [x] Skate 3 riding: goofy/regular, fakie (detected from motion) and switch (pushing out of fakie turns the
  rider round), held and mongo pushes, foot/tail/nose brakes, powerslides, kick turns, deck-tilt steering,
  rolling resistance, all from Skate 3's tuning when the player's disc supplies it
- [x] Controls laid out like Skate 3's Xbox 360 pad by button position
- [ ] Speed wobble
- [x] Sketchy and heavy landings besides clean/bail
- [ ] Slope matching on landing
- [x] Manuals and nose manuals (C-stick held lightly, with a balance meter), grabs on the triggers
- [x] Reverts
- [x] Air spin with Skate 3's wind-up; hippie jumps; held double/triple kickflips
- [x] Ramps and quarter pipes you can go vertical on: the board follows any surface under its trucks (ray
  probes against the park's triangles), airs out over coping, wallrides, pumping, bails from impacts
- [ ] Scripted Dolphin runs from `game_core::autoplay`, with screenshots in CI-like checks
- [ ] Late flips, finger flips, dark catches, board-sideways flicks (shapes are already converted)

## Phase 2: Renderer
- [ ] Display lists for static geometry (built once, `GX_CallDispList`)
- [ ] GX hardware lighting with vertex normals instead of pre-lit colours
- [ ] Textures (GX formats, CMPR) and a simple shadow blob under the skater
- [ ] Widescreen setup (VI width) and a proper HUD font texture
- [x] CPU time per frame on the HUD (average + worst of the last second)
- [ ] Memory readout in a debug overlay

## Phase 3: Content pipeline
- [ ] PC-side converter: your own glTF/Blender levels and models to a big-endian Wii pack read in place
- [x] `fg_import`: disc image → BIG/RefPack → RW4 textures → GX `.fgpak` (player-supplied data, docs/importing.md)
- [x] `fg_import`: skeleton, clips and poses from VBR `.abin` banks; water and menu tables from a player-decrypted executable
- [x] `fg_import`: default skater as skinned meshes + textures, clips composed onto the bind pose (PNG previews)
- [x] `fg_import park`: district streams (ATOC/SFIL) and ClusteredMesh collision, spawn point
- [x] `fg_import park`: grind rails from the spline sections
- [x] `fg_import park`: the park's textured models (Pres stream, material -> texture binding)
- [x] `fg_import rules`: Skate 3's riding tuning (AttribSys database) and flick shapes (`.pat`)
- [ ] `fg_import`: lightmaps, RAW-codec animation, skater tints/morphs/outfits
- [x] Draw the imported skater on the Wii: CPU skinning, clip per state with crossfades, textured indexed
  GX draws (checked in the PC preview; not yet run in Dolphin or on hardware)
- [x] GX textures read in place from a `.fgpak` loaded from SD/USB (`sd:/flatground/skater.fgpak`)
- [x] The build converts from the player's disc (only what is missing or out of date) into the app folder's
  `data/`; the game loads from `sd:/apps/flatground/data/`; `build.bat dolphin` runs it with `dist/` as the SD card
- [ ] Load texture packs and other `.fgpak` content from SD
- [x] Imported park collision: triangle ground and walls (`game_core::collision`), loaded from SD

## Phase 4: Game
- [x] Grinds: 50-50 and boardslide on rails (imported parks and the demo ledge)
- [x] Grinds and slides chosen by how the board meets the rail (11 kinds, frontside/backside)
- [ ] Grind balance; blunts, darkslides; a light physics fall for bails (Skate 3's ragdoll)
- [ ] Skater animation (skinned, original)
- [ ] Session goals, combo system, replay
- [ ] Sound (ASND): rolling, pop, landing
- [ ] Classic Controller and Wii Remote + Nunchuk control schemes
