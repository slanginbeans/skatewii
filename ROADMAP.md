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
- [ ] Tune flick thresholds on a real C-stick (its range and gate differ from the main stick)
- [ ] Pushing animation and timing, carve lean, speed wobble
- [ ] Better landings: slope matching, sketchy landings instead of only clean/bail
- [ ] Manuals (tip the flick stick), reverts, grabs on the triggers
- [ ] Ramps and quarter pipes you can go vertical on (needs a real surface model, not only a height field)
- [ ] Scripted Dolphin runs from `game_core::autoplay`, with screenshots in CI-like checks

## Phase 2: Renderer
- [ ] Display lists for static geometry (built once, `GX_CallDispList`)
- [ ] GX hardware lighting with vertex normals instead of pre-lit colours
- [ ] Textures (GX formats, CMPR) and a simple shadow blob under the skater
- [ ] Widescreen setup (VI width) and a proper HUD font texture
- [ ] Frame timing and memory readout in a debug overlay

## Phase 3: Content pipeline
- [ ] PC-side converter: your own glTF/Blender levels and models to a big-endian Wii pack read in place
- [ ] Collision mesh + grind rails authored in the level files
- [ ] Load parks from SD

## Phase 4: Game
- [ ] Grinds and slides on ledges and rails
- [ ] Skater animation (skinned, original)
- [ ] Session goals, combo system, replay
- [ ] Sound (ASND): rolling, pop, landing
- [ ] Classic Controller and Wii Remote + Nunchuk control schemes
