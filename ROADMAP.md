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
- [x] Landings classified as Skate 3 does (straight, sketchy with a magnitude, spin landings that keep
  pivoting; nothing judged under 2 m/s); the graphs pick their landing clips by it
- [x] Slope matching on landing: the board's up arrives at the landing surface 3 ticks before touchdown
  (Skate 3's known air), within its per-tick limit
- [x] Manuals and nose manuals (C-stick held lightly, with a balance meter), grabs on the triggers
- [x] Reverts
- [x] Air spin with Skate 3's body spin controller: signed wind-up, a let-go spin carries on, spins about
  the body's up; hippie jumps; held kickflips up to four flips
- [x] Air and landings (phase 5, SK8 air): Skate 3's trajectory selector (7 candidate flights in a cone,
  scored, followed with the start error blended away; long drops resolve over a few ticks), the landing
  assist's automatic spin (lands fakie when that end is nearer), landing speed kept (gaining on
  transitions, no snap to travel: the wheels straighten it), bails only on Skate 3's rules (landing angle
  against speed, into-ground, stairs and rail speeds, landing before the board is caught), wipeouts the
  graph makes (a grab tweaked into the landing), the reset after a bail on the spot or a recent checkpoint
  (A/B to skip once settled), a yaw-servo revert
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
- [x] Grinds from riding (into coping, off a ledge's top, up a bank into a ledge, dropping in), SK8's
  admission and coping hold
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
- [x] Graph runner, air: `ScoringGrabs` (grab variants by C-stick), `TweakProject` (grab tweaks),
  `BodySpin` channels, `ControlAirLegExtension` (`EXTEND`), `SetDeckPitchAndYaw`, the physics spin from the
  graph's `PhysBodySpin`, landing types and `TimeToLand` from the physics' landing prediction
- [x] Graph runner, phase 8: the pump layers (`Pumping`), push-out distance (`PushOut`, from the posed
  feet), powerslide spin and deceleration, the wipeout's lean, twist and stick gesture (`Wipeout`,
  `MatchTwistAndLean`), bumps (`IsBumped`, `SetBumpCoefficients`, from the board's acceleration along the
  ground), the d-pad's character gestures (`CharacterGesture`, its clips converted by the importer),
  body flips (`BodyFlippingSignal`, `IsBodyFlipping`); no tree in the trick tour plays with an unset
  parameter
- [ ] Graph runner, next: the air leg's centre-of-mass height, the push speed from the posed clips'
  trajectory, `PhysicsWantsManualExit` (the manual balances on SK8's PID servo, but held manuals never
  tip out), shoves (nobody to shove), a board posed apart
  from the body (`SetDeckPitchAndYaw`), the wipeout's body from a ragdoll, the player's own gesture picks
- [ ] `fg_import`: lightmaps, RAW-codec animation, skater tints/morphs/outfits
- [x] Draw the imported skater on the Wii: CPU skinning, clip per state with crossfades, textured indexed
  GX draws (checked in the PC preview; not yet run in Dolphin or on hardware)
- [x] GX textures read in place from a `.fgpak` loaded from SD/USB (`sd:/flatground/skater.fgpak`)
- [x] The build converts from the player's disc (only what is missing or out of date) into the app folder's
  `data/`; the game loads from `sd:/apps/flatground/data/`; `build.bat dolphin` runs it with `dist/` as the SD card
- [ ] Load texture packs and other `.fgpak` content from SD
- [x] Imported park collision: triangle ground and walls (`game_core::collision`), loaded from SD
- [x] Locations: the build converts every skate park that fits (six; `data/parks/` and the `parks.txt` index),
  a start menu to choose one, a pause menu (resume, change location, respawn, quit) on START, parks
  switched at runtime with the old one freed (heap back to the same value every time, checked in Dolphin),
  the last location remembered on the SD card; park meshes drawn in place (no per-frame vertex copy)
- [x] The city districts measured cell by cell (`fg_import city`: per-cell sizes and maps; docs/importing.md
  "The city districts' cells")
- [x] The districts and Skate School cut into 100 m cell packs (`fg_import district`, `FG_CITY=1` in the
  build; `game_core::city` index), every cell loading with the park loader
- [x] Background file reads on the Wii (`wii/platform/fg_read.c`: a queue worked by a low-priority thread;
  `fg_read_start`/`poll`/`cancel`; `game_core::reader` with an in-memory reader for tests)
- [x] Districts streamed while riding (`game_core::district`: cells read ahead and dropped behind within a
  memory budget, ground and rails across loaded cells, the rider held where cells aren't in yet; districts in
  the location menu with `FG_CITY=1`)
- [x] District start spots searched for in the collision (flat, open, a long run, rails around at its level;
  up to four a district, spread out), each its own location in a scrolling menu
- [x] Cells read in place on the Wii (packed collision with its grid, 32-byte aligned mesh vertices and
  indices used where loaded): Downtown's packs 170 to 139 MB, parsing them adds about 5 MB instead of
  167 MB; districts fade into the sky before the draw distance (fog)
- [x] Districts run in Dolphin: every start of all four, cells streamed for 16 minutes without an error or a
  dropped frame, memory back to the game's own after each (finer collision grids where triangles are dense)
- [ ] More locations: the city districts and Skate School streamed in 100 m cells (packs per cell, a
  background reader on the Wii, collision near the rider and models within the draw distance loaded,
  start spots in the location menu)

## Phase 4: Game
- [x] Grinds: 50-50 and boardslide on rails (imported parks and the demo ledge)
- [x] Grinds as Skate 3 does them (phase 6, docs/graph-runner.md "Grinds"): the board's contact boxes pick
  the family (50-50, 5-0, tip slide, blunt, boardslide, darkslide), rail classes and high sides from the
  park, grind targeting in the landing prediction and the grind air assist, stick pitch and yaw on the
  rail, friction by family and class, lean-off, entry and in-grind wipeouts, pop-outs with energy, every
  grind name the state graphs know (BF_ variants included), the grind trees' twist and height
- [ ] Grind balance as a rigid board (Skate 3's pins and forces; flatground holds the board on the rail),
  the in-grind deck closing-speed wipeout, grind grabs, a light physics fall for bails (Skate 3's ragdoll)
- [x] Camera as Skate 3's (phase 7, docs/graph-runner.md "The camera"): the disc's camera graph run on
  the graph runner choosing the disc's shots (blend trees by speed, slope, steering, air time, grind
  speed and incline), sine-eased shot transitions, the movement compass, heading/elevation/pan/tilt
  trackers, stance mirrors (fakie included, frozen in the air), push and pump lag, small airs not chased,
  three-line collision with the elevation raise, breadcrumb avoidance, the drop predictor, framing yaw
  and dutch roll, the lens's field of view for the TV's aspect; flatground's own shots and choice without
  the disc; the reset fade drawn
- [ ] Camera, next: free look, camera shake, slow motion on long airs, the high camera
- [x] Off the board (docs/graph-runner.md "Off the board"): Y gets off and on, timed by Skate 3's dismount
  and mount clips; walking, running and the sprint burst on Skate 3's biped controller and curves; the
  jump with MatchAirTime; the graphs' `LocoState`, slope and air states; the off-board camera conditions
- [ ] Off the board, next: dropping, throwing and picking up the board; stairs' own walk cycles; jumping
  onto the board from the air; obstacle steering; foot IK
- [x] Body flips and their upside-down bails (phase 8, docs/graph-runner.md "The air"): front and back
  flips from the left stick with a grab, Skate 3's start, speed and grab rules, landing off the flip or
  falling upside down without the grab bails
- [x] Camera after a carve (phase 8): the wind-up from the graph's anticipating flag, SK8's turning
  steering, no one-tick stop at the pop; checked against SK8's frozen-arm shot transitions (a regression
  test: behind within 90 ticks of a hard carve, never past 45 degrees at riding speed)
- [x] Grind rail classes checked against SK8's probes on BlackBoxPark (handrails thin, ledges ledge), the
  far probes' reach from the disc's truck distance
- [ ] Skater animation (skinned, original)
- [ ] Session goals, combo system, replay
- [ ] Sound (ASND): rolling, pop, landing
- [ ] Classic Controller and Wii Remote + Nunchuk control schemes
