# Feature audit: flatground against Skate 3

What Skate 3 lets a player do, whether flatground does it, and what to build next. Skate 3's feature set
is taken from SK8-ENGINE/skate-3-rust-engine (GPL-3.0, "SK8" below, checked out at `_ref/`) and from how
Skate 3 plays. Paths are `game_core/src/` unless they say otherwise. This audit is from reading the code
only. Nothing was built or run for it.

**Status:** **done** = plays like Skate 3 (allowing for the approximations noted). **partial** = there
but incomplete or simplified. **missing** = not there. **Size:** S = a day or two, M = about a week,
L = several weeks.

## How to read the graph-side evidence

flatground runs Skate 3's own action and motion graphs (`graph/`), so a feature can be missing in two
ways:

- **Unfed intents.** The pad never sets an intent the graph checks, so the states that need it are never
  entered. `graph/input.rs` (`names!`, `Intentions`) feeds riding, push, brake, slides, grabs, dark
  catch, wind-up, manual, tweak, board adjust, gestures, the off-board toggle, jump, sprint, move and
  look. It never sets these intents that SK8's input layer sets (`skate-core/src/input/*.rs`):
  `OB_AirBodyTweakX/Y`, `OB_DoAirBodyTweak`,
  `HandPlantTweakX/Y`, `Dismount`/`NewDismount`, `WipeOutRequest`, `WipeOutPushOff`, `WipeOutRecover`
  (the physics recovers on its own instead), `WipeoutControlX/Y` and `WipeoutGestureX/Y`.
- **Fixed conditions and stubbed behaviours.** `graph/ops.rs` holds `FIXED_FALSE` (conditions pinned
  so the graph stays on the board), the no-op behaviour list and `UNSUPPORTED`. Pinned false today:
  plants (`IsFootPlanting`, `IsHandPlanting`, `HasNewHandPlantPos`, `ShouldPrepareOneFootAirForFootplant`),
  skitching (`IsEnteringSkitch`, `IsSkitching`, `IsSkitchShimmying`, `IsSkitchingWithAbsorb`),
  `IsDroppingIn`, `IsHippyHurdling`, `IsLandingIntoGrind`, `IsGrindApproach` and the wipeout timers.
  Stubbed behaviours: `FootPlantAbsorb`, `ScoringHandPlants`, `Shove`,
  `AirDismounting`, `SetScoreAugmentation`. `UNSUPPORTED` holds the three skitching behaviours, and
  `GestureType` (the wipeout body tweaks) is also logged as unsupported.
- **Board adjust** (fixed). `Beh::BoardAdjust` (`graph/host.rs` `run`, `AdjustSeam`) now publishes its
  motion-graph intents every update as SK8 does (`angleFilter`, `negateOnMirror`, the +-pi seam;
  `skate-game/src/graph_host/action_board_adjust.rs`), so the `BoardAdjusts/Left/Right/Up/Down` states'
  clips pose the deck, and `SetDeckPitchAndYaw` hands its trees the posed deck's yaw and pitch
  (`deck_angles`, SK8 `publish_deck_angles`). The physics' deck follows (see the Air row).

## Riding

| Feature | Status | Evidence | SK8 | Size |
|---|---|---|---|---|
| Push (held, tapped, repush timing) | done | `skater.rs` `update_push`, `start_push`, `graph_push`; `graph/push.rs` | `riding/push*.rs` | – |
| Mongo push | done | `update_push` (B); `MongoPushFootToFar` pinned false (the wind-up shortcut) | same | S (the condition) |
| Foot, tail and nose brake | done | `update_brake`; the tail brake from `update_manual` | `riding/braking.rs` | – |
| Powerslide (start, spin, decel, revert out) | done | `update_slide`; `PowerSlideSpin`/`Decel` in the graph; `PowerSlideManualAtt` is a no-op | `input/power_sliding.rs` | S |
| Carving and turn lean | done | `skater/board.rs` `ride`; `graph/riding.rs` SetTurning | `riding/steering.rs` | – |
| Kick turns | done | `kickturn_rate`, the graph's `KickTurn` | `animation/kickturn.rs` | – |
| Pumping | done | `board.rs` `update_pumping`; the graph's `Pumping` | `riding/pumping.rs` | – |
| Speed wobble | done | `skater/wobble.rs` (SK8's `calculate`, Skate 3's `physics_speed_wobble` and Normal onset 15 m/s, strength 0.5 as built-in defaults; `tuning/wobble.rs`), applied to the steering tilt in `board.rs` `ride`; its amplitude feeds the turn conditioner (`World::wobble`); tests `skater::wobble`, `tests/manual_wobble.rs` | `riding/speed_wobble.rs` | – |
| Fakie, switch, turning round | done (getting on is buggy) | `update_fakie`, `Event::Switched`; switch stance when getting on is a known bug | `animation/riding_fakie.rs` | – |
| Runouts and wall hits | done | `run_out`, `board.rs` `hit_wall` | `riding/collision_response.rs` | – |
| Difficulty and tuning presets (Easy/Normal/Hardcore, truck tightness) | missing | Skate 3's tuning is fixed through the rules pack | `skate-game/src/difficulty.rs`, `custom_difficulty.rs` | M |

## Flip tricks, air and catching

| Feature | Status | Evidence | SK8 | Size |
|---|---|---|---|---|
| Ollie, nollie, fakie and switch pops (flick speed sets height) | done | `pop`, `take_off`, `pop_speed`; `flick.rs` | `input/trick_intentions.rs` | – |
| Kickflips, heelflips and multi-flips (held, up to 4) | done | `graph/input.rs` `HOLDABLE`; `Skater::follow_graph_trick` | same | – |
| Shuvs and varials | done | graph gesture sets (`RIGHT_SETS`) | same | – |
| Body spins (wind-up, carry-on) | done | `spin_input`, `skater/air.rs` `physics_air_spin`, `auto_spin` | `air/body_spin.rs` | – |
| Late flips, finger flips, underflips | done | `input.rs` gates (`late_flips_blocked`, `finger_flips_blocked`) | same | – |
| Body flips (front and back flips) | done | `update_body_flip`, `skater/flip.rs` | `air/body_flip.rs` | – |
| Dark catch | done | Z → `DarkCatch` | same | – |
| Catching the board (danger zone; landing before the catch bails) | done | `GraphControl` danger zone; `land` | `animation/landing_quality.rs` | – |
| Landing classification and slope match | done | `skater/landing.rs` `classify`; `land` | `physics/skeleton_landing*.rs` | – |
| Trajectory selector and landing assist | done | `skater/air.rs` (`launch`, `score`, `grind_target`) | `air/trajectory/` | – |
| Board adjust in the air (right stick lines the board up for a grind or landing) | done | `Beh::BoardAdjust` publishes the C-stick (`graph/host.rs` `AdjustSeam`) and the graph plays `BoardAdjusts/*`; `SetDeckPitchAndYaw` from the posed deck (`deck_angles`). The physics' deck (`skater/adjust.rs` `DeckAdjust`) follows the posed board while the graph adjusts, else the C-stick at Skate 3's measured angles (`tuning/adjust.rs`: 80/88 degrees across, 20/24 nose down/up); `Skater::frame` turns the grind boxes and the grind air assist's points with it, and a rail or the ground bakes it into the board (`try_grind`, `land`). Tests: `tests/board_adjust.rs` (across a rail: boardslide; lined up: lands straight; imported: the graph's board-adjust states), `graph/tests.rs` `board_adjust`. `GrindAdjustMaxAngle` is the flight's steering limit in Skate 3 too, not the deck's | `action_board_adjust.rs`; `pose_output.rs` `publish_deck_angles` | – |
| Hippie jump | done (animation buggy) | `update_hippy`, `hippie_air`; the animation is a known bug | `player/offboard/hippy_jump.rs` | – |
| Hippie hurdle | missing | `IsHippyHurdling` pinned false | selector conditions | S |

## Grabs, grinds, manuals, plants

| Feature | Status | Evidence | SK8 | Size |
|---|---|---|---|---|
| Grabs (L/R, C-stick variants) | done | `update_grab`, `select_grab`, `grab_variant`; `ScoringGrabs` | same | – |
| Grab tweaks (tweaking into a landing bails) | done | `TweakProject`, `GRAB_TWEAK_BAIL` | same | – |
| Grind and slide entry (families, admission, air assist) | done | `grind.rs` `investigate`, `admit`, `try_grind`, `engage` | `physics/grind_contact/` | – |
| Grind balance | partial | the board is held on the rail kinematically (`slide_rail`), not Skate 3's rigid board with pins; no closing-speed wipeout | `physics/grind_forces/`, `grind_contact/balance` | L |
| Grind exits (end of rail, lean-off, pop-out with energy) | done | `launch`, `ExitLeanAngleVsTime` | `grind_forces/launch.rs`, `release` | – |
| Tricks out of grinds (flip out) | done | `Gates.grinding` limits the flicks to the grind trick-outs | `physics/grind_trick.rs` | – |
| Grind grabs | missing | ROADMAP Phase 4 | `grind_trick.rs` | M |
| Darkslides | done | the `Family` upside-down box (`grind.rs`) | `grind_contact/families.rs` `darkslide` | – |
| Primo | not a Skate 3 grind family | not in SK8's grind families or flatground's grind names | – | – |
| Lip tricks and coping stalls (rock to fakie, axle stall, blunt) | partial | grinding into coping and the coping hold (`TimeOfGravityRelief...`), blunt from the tip box; no separate stall states | grind admission into coping | M |
| Inverts and handplants | missing | `IsHandPlanting` and others pinned false; `ScoringHandPlants` and `SetHandPlantAnticLength` no-ops; `HandPlantTweakX/Y` unfed | `physics/handplant.rs` (+ contact, ik, trajectory) | L |
| Manuals and nose manuals (balance meter) | done | `update_manual` (no graphs: flatground's balance meter tips out), `graph_manual` publishes `PhysicsWantsManualExit` (`Skater::manual_exit`) as SK8's `apply_correction` does. Skate 3's graphs end a manual only on the stick, leaving the ground or that flag, which fires only while dragging with the mode's corrective force off (Easy); Normal has it on, so a Normal manual drags and never tips out, as on the disc. Tests: `graph::tests::manual_exit`, `tests/manual_wobble.rs` (Skate 3's own graphs). The corrective drag and no-contact torque bleed are not modelled | `physics/manual/`, `input/manual_intentions.rs` | – |
| Manual tricks (flicks off the manual) | done | `flick_group` (`Group::Nose`/`Tail`) | same | – |
| Reverts | done | `update_revert`, `SlideFs180`/`SlideBs180` | `physics/revert_state.rs` | – |
| Foot plants: boneless, no-comply, fastplant | missing | `IsFootPlanting` and `ShouldPrepareOneFootAirForFootplant` pinned false; `FootPlantAbsorb` no-op; no FootJump state | `physics/boneless.rs` (state 602), `physics/footplant/`, `air/known/footplant.rs` | M |
| Wallrides | done | `board.rs` `update_wallride`, `start_wallride`; `try_air_wallride` | `riding/ground_contact_response.rs` | – |
| Wallies, wallplants | missing (minor) | wallplants come with the foot plants; no wallie code here or in SK8 | footplant against walls | S (after foot plants) |

## Off the board

| Feature | Status | Evidence | SK8 | Size |
|---|---|---|---|---|
| Getting off and on (Y, clip-timed) | done (buggy) | `toggle_board`, `step_off`, `step_on`; switch stance on getting on is a known bug | `player/offboard/toggle_board.rs` | – |
| Walk, run, sprint | done (animation freezes) | `skater/biped.rs` `Biped::step`, `walk`; the walk and run animation freezing is a known bug | `player/offboard/ground_*`, `cadence.rs` | – |
| Jump on foot | done | `biped.rs` jump, `MatchAirTime` | `offboard/air_launch/` | – |
| Dropping and throwing the board down (stand on it or get on rolling) | done | L / R fully in feed `OB_DropBoard` / `OB_ThrowBoard` / `OB_RetrieveBoard` (`graph/input.rs` `board_triggers`); ToggleBoard runs (`graph/toggle.rs`, `graph/host.rs` `toggle_board`: the `OFB_THROW_*` and `OFB_RETRIEVE_HIGH_*` clips on the `RetrieveBoard` channel, which the importer now converts); `IsHoldingSkateboard`, `IsDeckFree`, `IsDroppingSkateboard`, `IsRetrievingSkateboard` from the possession (`skater/possession.rs`: held, free body rolling and stopped by walls, hidden past 30 m, flying back; Skate 3's `physics_skatecontroller` values in `tuning/possession.rs`); Y next to the board gets on it where it lies (with the graphs the mount fetches it, `OB_Mounting`), further away fetches it. Tests: `skater/possession.rs`, `graph/toggle.rs` unit tests; `tests/drop_board.rs` (drop, throw, walk away and back and get on, retrieve, hidden, and with imported data the graphs' drop/throw/retrieve and mount) | `input/offboard_intentions.rs` (L / R), `graph_host/motion_offboard/toggle_board.rs`, `player/offboard/toggle_board.rs`, `board_possession` | – |
| Caveman and mid-air get-on (landing on the board from the air) | done | mid-air get-on (`Mount/Air`, `Skater::mount_in_air`; with the board free Y fetches it first); caveman: Skate 3's landing assist onto the free board (`Possession::landing_flight`, `landing_assist_limit`), `CanLandOnBoard` from it, the physics' landing on deck started by the motion graph's `DoLandOnBoard` (`Mount/Air/IntoLandingOnBoard`, `OnBoard/LandingOnBoard`), `IsLandingOnBoard` real, `LandOnBoard` asks for the board's tail ahead (FlipBoardBackwards); missing the deck lands on foot. Tests: `tests/drop_board.rs` `caveman_running_with_it_dropped_a_jump_lands_on_it_riding`, `imported_caveman_lands_on_the_board`, `y_in_the_air_with_the_board_in_hand_still_gets_on_there` | `physics/landing_on_deck`, `player/offboard/landing_deck.rs` | – |
| Dropping in | partial | rolling off a deck into a transition drops in (`grind.rs` drop-in admission, test `rolling_slowly_off_a_deck_drops_in...`); the on-foot drop-in at the coping is missing (`IsDroppingIn` pinned false) | `graph_host/action_conditions.rs` drop-in | M |
| Climbing ledges | partial | steps up to 0.45 m (`biped::STEP_UP`); no climb | `physics/climbing/` (SK8's own extension, not stock Skate 3) | M (optional) |
| Stairs' own walk cycles, obstacle steering, foot IK | missing | docs/graph-runner.md "Not done" | `offboard/movement_intent.rs`, `physics/foot_ik.rs` | M |
| Off-board air body tweaks | missing | `OB_AirBodyTweakX/Y` unfed; `OffboardBodyTweakBlend` no-op | `input/offboard_intentions.rs` | S |

## Bails, skitching, others

| Feature | Status | Evidence | SK8 | Size |
|---|---|---|---|---|
| Bails on Skate 3's rules | done | `bail`, `land`, the graph's `WipeOut` | `player/wipeout/` | – |
| Ragdoll body in a bail | missing | the bail is the drawn clip only; `WipeoutTimeToLand`/`SinceContact` pinned | `physics/wipeout_states/ragdoll*` | L |
| Bail steering and body tweaks (Hall of Meat moves) | partial | `Wipeout` lean and twist and `EnableWipeoutGestures` run; `GestureType` unsupported; `WipeoutControl*` unfed | `input/wipeout_intentions.rs` | M (after the ragdoll) |
| Hall of Meat scoring (bones broken, damage) | missing | no injury model | audio and scoring hooks only | M |
| Reset after a bail (spot or checkpoint) | done | `skater/respawn.rs` `Recovery`, `Checkpoints` | `player/respawn.rs` | – |
| Skitching | missing | `UNSUPPORTED`: `EnterSkitchingBehaviour`, `SkitchingBehaviour`, `SkitchShimmyingBehaviour`; no vehicles | selector conditions and `motion_shove.rs` (`SkitchAntic`); no traffic either | L (needs moving cars) |
| Shoving pedestrians | missing | `Shove`/`MaintainShove` no-ops; no pedestrians | `graph_host/motion_shove.rs` | L |
| Object Dropper | out of scope | – | – | – |

## Camera, scoring, session, replay

| Feature | Status | Evidence | SK8 | Size |
|---|---|---|---|---|
| Skate 3 camera graph (shots, trackers, avoidance) | done | `camera/` (`select.rs` runs `Default_cameragraph`) | `skate-core/src/camera/` | – |
| High and Low camera setting | partial | the low camera only; no option | `docs/hails-additions/21-low-camera.md` (`camera_type`) | S |
| Free look | missing | ROADMAP "Camera, next" | `camera/manual_cam.rs`, `look_input.rs` | S |
| Camera shake | missing | – | `camera/shake*.rs` | S |
| Slow motion on long airs | missing | camera conditions mention it only | `camera/slow_motion.rs` | S |
| Trick scoring | partial | `Event::Landed { points }` added to `Game::score` (`game.rs`); grinds score once a name holds 12 ticks; wallrides a flat 250 | `skate-core/src/scoring/catalog.rs`, `skate-data/src/scoring.rs` | M |
| Combos and lines (multiplier, line timer, a bail costs the line) | missing | no combo state; `SetScoreAugmentation` no-op | `scoring/session.rs` (combo levels, line capacity, bail factor), `timer.rs`; `scoring_hud.rs` | M |
| Session markers (place one and go back to it) | missing | only the automatic checkpoints (`respawn.rs`); the d-pad is taken by character gestures | `skate-game/src/session_marker/` | S |
| Replay (the last ~30 s, scrubbing, camera) | missing | none (`autoplay.rs` records scripted input only) | `skate-game/src/replay.rs`: a 30 s rolling presentation recording | M (replaying pad input on the deterministic 60 Hz sim is cheap on the Wii) |
| Sound | missing | ROADMAP Phase 4 | `skate-audio` | L |

## Known bugs (fixed on the `wii-feel` branch)

- Hippie jump animation: the graphs now see both pushes and play Skate 3's own `OnBoard/HippyJump` states
  (wind-up, jump, landing on the board); the physics reports the flight as off the board (LandingOnDeck,
  `OBTimeToLand` read) and the board is drawn rolling on along the ground.
- Mid-air get-on: Y in the air off the board plays `Mount/Air` and lands on the board.
- Switch stance when getting on the board: kept.
- Walk and run animation freezing: the phase-controlled cycles now play at the walker's step cadence.

## Recommended build order

Feel of core skating comes first, then the two features the user named, then the rest.

| # | Work | Why here | Size |
|---|---|---|---|
| 1 | Finish the known bugs (hippie jump, mid-air get-on, switch on mount, walk freeze) | everything below builds on mounting and walking | – |
| 2 | ~~Manual tip-out under the graph (`PhysicsWantsManualExit`)~~ done: published as Skate 3 does (it fires only without the corrective force, so Normal manuals never tip out, as in Skate 3) | a manual that never fails breaks the feel of lines | S |
| 3 | ~~Speed wobble (`TurnInput.wobble`, `riding/speed_wobble.rs`)~~ done | core riding feel at speed | S |
| 4 | ~~**Board adjust in the air**: publish `BoardAdjust`'s MG intents (seam handling and mirroring as SK8 does), drive `SKATEPITCH`/`SKATEYAW`, and feed the adjusted board yaw and pitch to grind contact and landing~~ done | user-named; lines up grinds and landings; the S part (publishing) unblocks the clips at once | M |
| 5 | ~~**Drop and throw the board** (`OB_DropBoard`/`OB_ThrowBoard`/`OB_RetrieveBoard`, a free board body, `IsDeckFree`/`IsDroppingSkateboard`, `ToggleBoard`)~~ done | user-named; also needed for caveman | L |
| 6 | ~~Caveman and landing on the board from the air (`Mount/Air`, `LandOnBoard`, `IsLandingOnBoard`)~~ done | follows from 1 and 5 | M |
| 7 | Grind balance as a rigid board, then grind grabs | grinds are half of street skating | L, then M |
| 8 | Foot plants (boneless, no-comply, fastplant, wallplant) | common Skate 3 street moves; the graph states are already there | M |
| 9 | Combos and lines, plus fuller trick scoring | gives the tricks a goal | M |
| 10 | Session markers | cheap and a big help for practising spots | S |
| 11 | Replay (input recording) | the Skate 3 loop of landing a trick and watching it | M |
| 12 | Camera: free look, High/Low setting, shake, slow motion | polish | S each |
| 13 | On-foot drop-in, lip stalls, handplants and inverts | transition skating | M, M, L |
| 14 | Ragdoll bails, bail steering and body tweaks, Hall of Meat | spectacle; large | L |
| 15 | Difficulty presets; off-board extras (stairs cycles, foot IK, air body tweaks, hippie hurdle) | polish | S–M |
| 16 | Skitching and shoving (need moving cars and pedestrians), sound | whole new systems | L |
