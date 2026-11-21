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
- [x] Fixed 60 Hz simulation (Skate 3's tick) on every TV: PAL50 runs one or two ticks a frame and draws
  between the last two; flicks are read at 60 Hz
- [x] SK8-style board core (`skater/board.rs`): a full velocity with sideways slip, truck-curvature
  steering with grip, downhill heading and steep-face turn torques, natural air off crests and lips,
  ground-normal smoothing, wall slides with a collision yaw, runouts, wheel-height curbs, push and brake
  timing from the imported clips, SK8 powerslides, vert pops re-aimed up, SK8 wallrides (needs a floor),
  pumping by curvature; every curve from the disc through the rules pack
- [ ] Calibrate the flatground-own scales (brake, powerslide grip, side grip) against SK8 runs
- [ ] Grinds from riding (into coping, onto curbs and ledges): the lock-on hook is ready (`try_grind`)
- [x] Scripted Dolphin runs from `game_core::autoplay` (`autoplay.txt`, `tools/dolphin_test.ps1`)
- [ ] Screenshots from scripted runs in CI-like checks
- [x] Late flips, finger flips, under flips, dark catches: recognised and selected by the state graphs (Z is
  the dark catch); the board does what the clips do (the imported board is skinned to the clips'
  board bone), landings bail only inside the clips' danger zone, dark catches ride dark

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
- [x] `fg_import motion`: every clip's attribute tracks and events, every animation tree, and the poses of
  every clip the on-board motion graph plays (`motion.fgpak`, loaded at start-up); tree playback in
  `game_core::motion::tree` (SK8's semantics)
- [x] Run Skate 3's on-board action and motion graphs from the player's disc (`motion/graph`,
  `game_core::graph`, docs/graph-runner.md): SK8's state selection, behaviours, intents, crossfades; the
  rider plays the graph's trees (mirrored per tree), and the graph decides pushes (push contact), foot
  brakes (brake contact), pops (the takeoff's jump, SK8 trick height), manuals (no fail, pitch servo),
  kick turns, reverts, turning round out of fakie; every gesture file recognised in parallel
- [x] Graph runner, phase 4: held manuals last as long as the stick is held; SK8's push behaviours (push
  strength, the push trees' coefficients, repush deadline) drive both the push clips and the board;
  channel blends by bone weight and the fakie head and torso channel; air tricks through to the board
  (late/finger/under flips, dark catches, multi-flips up to 4, landings from the clips' danger zone);
  operations typed at import (graph load 217 ms -> 13 ms on the Wii); simultaneous gestures in SK8's
  hash-bucket order; the trick tour does all of it in Dolphin
- [x] Graph runner, riding animation: SK8's SetTurning on the physics' animation turn conditioner (carve
  lean, direction, quickness, holding, speed tuck), body tilt (`TILT_X`), Crouching and the centre-of-mass
  height (`DISTTOCOG`, trigger crouch, auto pump), landing data, manual angle, all with disc tuning through
  the rules pack; the rider stands upright riding straight and leans into carves (it sat in the full carve
  lean, its blend parameters never set); trees played with unset parameters are logged and placed at the
  neutral point; the no-op operations audited (docs/graph-runner.md); a GameCube trigger dead zone;
  `TrucksOrDeckInContact` from the physics (grabs no longer wiped out the graph while the physics landed)
- [ ] Graph runner, next: the pose parameters still unset (pump channel, air leg extension `EXTEND`, late
  shove-it deck pitch/yaw, push out distance, powerslide spin/decel, grab tweaks, wipeout lean/twist, grind
  crouch/twist, bumps), CalcLandingQuality and landing types from the landing behaviours, the push speed
  from the posed clips' trajectory, SK8's manual PID (PhysicsWantsManualExit), character gestures (d-pad)
  and shoves, grind trick-outs by grind type (needs the grind physics of phase 6), camera shots from the
  graph (phase 7)
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
