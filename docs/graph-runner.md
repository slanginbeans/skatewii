# The state-graph runner

Skate 3 decides what the rider is doing (riding, pushing, winding up, which trick, manual, landing,
grinding ...) and what animation plays with two state graphs from its disc: `ActionGraph_OnBoard`
(turns controller intents into motion intents) and `MotionGraph_OnBoard` (states, animation trees,
values for the physics). flatground runs the player's own copies of them.

Skate 3's camera graph (`Default_cameragraph`) runs on the same runner and chooses the camera's shots
("The camera" below).

Nothing of the graphs is in this repository. `fg_import motion` compiles both from the player's disc into
the `motion/graph` entry of `motion.fgpak`, and types every operation once (`motion/graph/ops`, names
already resolved against the graph's strings and the motion library); without them the game uses its own
hand-written riding logic.

How the graphs are read and run follows SK8-ENGINE/skate-3-rust-engine (GPL-3.0), a Rust port of Skate 3
that runs the disc's graphs: crates/skate-data state_graph (binding), crates/skate-core graph (selection,
activation, controller) and crates/skate-game graph_host (what each behaviour and condition does).

## Code

| File | What |
|---|---|
| `game_core/src/graph/mod.rs` | the compiled format and its reader |
| `game_core/src/graph/write.rs` | the compiler (importer and tests): ownership, `enabled` inheritance, transition targets and interrupt ancestors by name, state-bound conditions |
| `game_core/src/graph/select.rs` | next-state selection and the controller (behaviour begin/update/end, hooks, state clocks) |
| `game_core/src/graph/ops.rs` | every behaviour, condition and hook parsed into a typed operation |
| `game_core/src/graph/opcode.rs` | the typed operations stored by the importer and read back at load |
| `game_core/src/graph/host.rs` | the runner: both graphs, intents, what each operation does |
| `game_core/src/graph/playback.rs` | what the motion graph plays: trees joined by crossfades, channel blends and sequences, persistent channels, the pose program the rider runs |
| `game_core/src/graph/push.rs` | SK8's push behaviours: push strength, the push trees' coefficients, the repush deadline |
| `game_core/src/graph/riding.rs` | the riding animation's behaviours: the physics' animation turn conditioner, SetTurning, SettingBodyTilt, Crouching, SetManualAngle, the landing data |
| `game_core/src/graph/input.rs` | the pad as action-graph intents, gesture recognizers |
| `game_core/src/graph/drive.rs` | the game's side: building the runner, the physics state it reads, timing |

## Selection

Each tick, for each graph:

1. From the current leaf state up to the root, look for a transition whose condition holds, `urgent`
   first, then `high`, `med`, default. A transition also needs its target (and the target's new
   ancestors) to hold their preconditions; the target is then entered at its first active leaf.
2. Without one: a state with `interruptable="false"` stays while it and its parents hold their sustain
   conditions; an interruptable one is re-resolved from the root (children in authored order, the first
   whose conditions hold); `interruptable="<Ancestor>"` re-resolves below that ancestor first.
3. Conditions carry a mask: `precond` (checked when entering), `sustain` (checked while staying on the
   current path), `always` (both). States with `active="false"` are only reached by transitions.
4. Behaviours of the states left end (newest first), the transition's hooks run, behaviours of the states
   entered begin, then every active behaviour updates.

## What the graphs drive

With `motion/graph` loaded (`Game::graph`), each tick:

1. The pad becomes action-graph intents (`graph/input.rs`, below).
2. Both graphs update; the motion graph plays animation trees (`graph/playback.rs`).
3. `Driver::control` turns the motion graph's state, its playing clips and its packet values into a
   `skater::GraphControl` for the physics:
   - pushes drive only from the graph's push contact (the `PUSHCONTACT` window or the `PushContact`
     state), for the push sets' contact time, towards the speed the push trees' coefficients end at
     (`graph/push.rs`, SK8 push_behaviors.rs): `ComputeFirstPushStrength` and `PushCycle` turn how long
     the button is held into a push strength (`button_time_to_dv` over `button_time_max`, at most
     `max_holding_acc` while held, a re-press queues the next push), `ComputeTargetCoefsFromSpeedAndStrength`
     picks `HSTR_VEL_B` / `LSTR_VEL_B` / `VEL_E` from the speed and the strength (the four push sets'
     lengths and speeds from the `_CYC1` clips), `SetPushCoefs` eases the trees to them (at the
     `blend_*_over_frames` rates) and the physics chases where those same coefficients end;
     `ComputeRepushDeadline` sets the out factor and `BreakOutOfPush` is the tree past out factor x its
     length with no held or re-pressed push;
   - the foot brake slows the board only while a brake clip's foot is down (`BRAKE_CONTACT`);
   - pops fire when the takeoff's air clip starts (its `JUMP` attribute), as the trick the graph scores
     (`ScoringTrick`), at SK8's trick height: min(wind-up strength, flick strength) from
     `SetTrickHeight`, finalised (at or under `jump_clamp_low` the minimum hop, at or over
     `jump_clamp_high` full height; `anim_motion/jumping/clamp_low_inclusive` and `clamp_high_inclusive`
     come from the disc through the rules pack) and raised to the `JumpHeightOverride` packet value
     (flick strength x 0.2; 0.27 out of a manual, 1.0 out of a grind);
   - manuals engage and end where the graph's Manual states are (`ManualEngageTime` over 0.2 s, brake,
     out), as Skate 3's do: a held stick holds a manual as long as it is held; the deck pitch servoes
     (SK8's PID) to (0.25 + 0.75 lean) of the most tilt, or to the brake angle. `PhysicsWantsManualExit`
     is the physics' `Skater::manual_exit` (SK8 manual/controller.rs `apply_correction`): set while
     dragging the tail or nose with the board rolling against the manual's balance direction (tail -1,
     nose +1, against the speed along the board) and the physics mode's corrective force
     (`physics_mode/normal/Hash_F8CBC0F5FEF2240E`) off. Skate 3's Normal and Hardcore have it on (Easy
     off), so with the stock rules a Normal manual drags its tail and never tips out; with it off the
     graph's `Brake` and `Cycle` states let go of the manual (scripted in `tests/manual_wobble.rs`);
   - in the air the board does what the clips do: a late flip, finger flip, under flip or dark catch
     becomes the trick (its look and flips from the graph's `ScoringTrick`, `Kickflip2`..`4` for held
     multi-flips; the graph's `TimeToLand` thresholds read the physics' landing prediction, below), a
     dark catch rides dark (`SetDark`, the board grip down), and landing while the playing clips are in
     their `DANGERZONE` (the board not caught; within its last 0.1 s counts as caught) bails when the
     board comes down faster than Skate 3's danger-zone closing speed (`Wipeout_AirYTrick`), as SK8's
     wipeout checks do, instead of after a fixed trick time;
   - the body spin's input is the graph's `PhysBodySpin` packet (the main stick's x as the graph hands
     it over), and the landing assist only turns the body while the graph's `BodySpin` is 0;
   - grabs are scored by the graph's `ScoringGrabs` name (the variant the C-stick picks; a variant once
     picked stays) and their tweak is the filtered `TweakX`/`TweakY`; when the motion graph goes to
     `WipeOut` on its own (its grab `Cyc` states do when the board touches down while `HasTweak` holds),
     the physics bails with it. The imported rider's board is a skinned
     part on the clips' `SKATEBOARD_ROOT`, so it flips exactly as the clip does; the built-in board
     follows the graph's trick and its progress;
   - kick turns turn the board while `KickTurnSteering` runs (at its spin scale; the action graph gates
     them on the stick, the speed (lower riding fakie) and `PhysicsSpeedAndSlopeCompare`, which is
     Skate 3's heading adjust factor from the turn torque curves, SK8 speed_and_slope.rs), reverts start on
     entering `Revert/Fs|Bs`, and `Switch` and `PushFromFakie` turn the rider round (fakie to
     switch) when their clip (`B_SWITCH`: `R_SWITCH_RIDE_*`) crosses its `MIRRORED` / `SWITCH` events,
     just after it starts, as Skate 3 toggles its mirror and switch flags on them (SK8
     skater_animation/state.rs `apply_stance_events`; a library without them turns round on entering
     the states). See "The board and turning round" below;
   - powerslides stay in the physics (flatground's Skate 3 rules); `CanEnterSlide` and
     `ShouldLeaveSlide` read them;
   - the physics' grinds reach `IsGrinding` by Skate 3's names (below, "Grinds"; `BF_` variants
     included), `IsGrindBluntingBackslash` is a blunt (the backslash family), `GrindTrickOutTypeAllowed`
     reads a darkslide's way out, and the graph pops out of a grind on its takeoff clip's jump, read with
     the grind's gesture group;
   - wipeouts have one source, the physics: the graph's wipeout conditions (`PhysFilteredState
     wipeout`, `IsPhysicsWiping`, `PhysicsWantsWipeOut`, `PhysicsWantsRunout`) read the physics'
     bail and runout (the graph's own `WipeOut` transitions, above, take the physics with them), and
     `TrucksOrDeckInContact` is true only while grinding (SK8 reads the truck
     and deck contacts, not the wheels). It used to be true always, so every grab state's
     "board not in the air or trucks touching" transition sent the graph to `WipeOut/Air/Flailing`
     while the physics landed clean (the graph then started over and grabbed again, three times a
     jump on a pad whose triggers rest above 0).
4. The riding behaviours set the riding trees' parameters (`graph/riding.rs`; disc tuning
   `anim_carving`, `physics_animation`, `anim_motion/crouching`, `auto_pump`, `body_tilt` and `manual`
   through the rules pack, else flatground's own values):
   - the physics' animation turn conditioner (SK8 turn_conditioner.rs) runs once a tick on the
     physics' turn (`Skater::steer`, positive to the left as Skate 3's) and its lean into the turn
     (the angle balancing the turn's centripetal pull): three filters, quickness from how much the
     stick moves, holding from how still the lean stays;
   - `SetTurning` (SK8 set_turning.rs, turn_remap.rs): `angle` (the lean, through the quick and slow
     remaps mixed by quickness, eased in by `blend_lean_in`, at most `clamp_curr_lean` an update,
     fully to the side in a powerslide), `dir`, `quickness`, `holding`, `speedtuck` past
     `speed_tuck_start`; the `Turn` and `Slide` packet values;
   - `ApplyingBodyTilt` / `SettingBodyTilt` (SK8 body_tilt.rs): `tilt_x` eases towards world up along
     the board's side (0 on flat ground, about 1 riding a wall), less while the body spins,
     rate-limited per update;
   - `Crouching` (SK8 crouching.rs) and `SetDistComToBoard`: `disttocog` between
     `crouching_min_height` and the speed's `crouching_max_height`, from the crouch intents (the
     triggers, `max(L, R)` on the ground as SK8's riding intentions; hard turns; a manual's back
     stick in a slide), the auto pump and the restoring force, rate-limited; the rider's height
     over the board is the playing clips' own `DISTTOCOG` (`IsCrouchedEnoughForBlendToGrabCycle`
     is that under 0.6 m);
   - `StoreLandingData` / `SetLandingData`: the landing trees' `AvgVelY` (fastest descent since
     leaving the ground), `Spin` (the landing classifier's signed magnitude) and `disttocog`;
     `HasLandingType` reads the physics' landing type (Skate 3's `CalcLandingQuality`: 0 straight, 1
     sketchy, 2 spin, 3 slightly off, which the graph lands straight) and `ChooseRandomLanding` picks the
     variant; `SetManualAngle` (SK8
     manual.rs): `manual_angle` from the C-stick;
   - the manual out timer starts when a manual ends and runs down only off the manual (SK8), so a
     trick just after a manual still pops from it.

   A tree played with a parameter that no behaviour has set yet is logged once per parameter
   (`graph: <TREE> plays with unset parameter <NAME>`). SK8 (and Skate 3) leave such a phase blend on
   its authored first child; flatground places it once at 0 when the children's range holds 0 (no
   lean, no tilt) and keeps SK8's choice otherwise, so a gap shows as a neutral pose, not the first
   child's extreme one (unset `ANGLE` used to hold the rider in the full carve lean).
5. The rider (`Rider::update_graph`) runs the motion graph's pose program (`playback::PoseOp`): each
   tree's clips blended as one pose, sampled and mirrored for the stance the tree started in
   (`anim::mirror_pose`, mode 1) before blending so stance changes crossfade, then the transitions'
   blends. A `channelblend` transition (29 in the on-board graph, 28 of them `useChannelFromWeights`)
   blends each bone by the weight times its channel weight (SK8 ACSChannelBlend; from the older pose
   with `useChannelFromWeights`, else the newer): clips that are channel animations (525 of the
   bank's, kept by the importer as a weight per bone) take only their bones; other clips weigh 1
   everywhere. Persistent channels are blended over the result the same way. Clips that don't fit the
   skeleton fall back to the clip roles. A tree started before the board's frame was turned end for
   end ("The board and turning round", below) is turned half round about the rider's up before blending.

Channels the on-board graph uses: `channelblend` transitions (landings into riding, pushes, slides,
grind exits, anticipation); `FakieHeadChannel`, the one persistent channel (`fakie`, tree
`B_FAKIE_CHANNEL`: fades in over 0.3 s when the rider starts riding fakie, its `torso` parameter
snapped on entry to 1 in a manual, 0 in a powerslide, else 0.5, then eased at 0.01 a tick; fades out
when riding fakie ends); the pump layers (`Pumping`, below) and the d-pad gesture channel
(`CharacterGesture`, below). Not run: the shove channel (`Shove` / `MaintainShove`: shoving the
people next to the rider; flatground's parks have nobody to shove).

Phase 8 operations (SK8 graph_host, every number from the disc through the rules pack, `tuning::moves`,
else flatground's own):

- `CharacterGesture` (SK8 motion_character_gesture.rs): the d-pad (the Xbox pad's d-pad is in the same
  place on the GameCube pad) makes `GestureLeftStart` / `RightStart` / `UpStart` / `DownStart` and
  `...Held` action intents, which the action graph hands over; a start plays that direction's gesture on
  the gesture channel (`<gesture>_BOTH_INTO` over 0.2 s, its `CYC` while the d-pad is held, then
  `OUT`); another direction takes over, `ForceBrake` blocks a start. Skate 3 lets the player pick each
  direction's gesture from a catalogue of 37; flatground's choice is left shrug, right peace, up shaka,
  down surf's up (`ops::GESTURES`), and the importer converts those clips. Both hands always: on the
  board flatground's hands are never busy (no hand services).
- `Pumping` (SK8 pumping_channel.rs): the physics' pump (the speed the pump added this tick) over
  `max_phys_pump`, filtered by `pump_prop_blend` and amplified by `pump_amplify`, starts a `B_PUMP`
  layer on a free channel (five at most) each time it rises past `new_pump_thresh`, and sets that
  layer's weight while it keeps rising (`pump_blendin` / `pump_blendout` fades).
- `IsBumped`, `SetBumpCoefficients` (SK8 ground_acceleration.rs, bump.rs): the board's acceleration with
  the deck's and the ground's up taken out, in the board's frame (x across, z along); bumped past
  `min_bump_mag` (x scaled by `scale_x_acc`); the bump trees get its direction weighted from
  `min_bump_blend_value` at `min_bump_mag` to full at `max_bump_mag`, mirrored with the stance.
- `PowerSlideSpin` (`slidespin` = minus `slide_phys_to_anim_spin` of the deck's travel along its nose, on
  begin), `PowerSlideDecel` (`decel` = `slide_speed_to_lean` of the speed along the ground, smoothed by
  `slide_smooth`).
- `PushOut` (SK8 PushFootFrame `out_distance`): the pushing foot's position against the board (along its
  nose and up) from the rider's last pose, as `OUTDISTANCEX` / `OUTDISTANCEY`, on begin.
- `MatchTwistAndLean`, `Wipeout`, `EnableWipeoutGestures` (SK8 twist_lean.rs, motion_wipeout_execute.rs):
  the bail trees' `Twist` and `Lean` follow the body's (rate-limited by `anim_wipeout`'s clamps and
  blends), and once the stick has been let go to the middle (or after 80 ticks) the main stick steers
  `WipeoutGestureX` / `Y`. flatground has no ragdoll: the body's lean is the drawn bail's tip and its
  twist the board's heading against the travel.
- `BodyFlippingSignal`, `IsBodyFlipping` (below, "Body flips").

The graphs start over when the physics puts the rider back on the board after a wipeout or runout.
Without the entry everything above is flatground's hand-written logic, unchanged.

## The board and turning round (`skater/board.rs`, `rider.rs`, `game.rs`)

- **Height**: the clips' board (`SKATEBOARD_ROOT`, the character's origin) is the deck's centre, which
  Skate 3 puts on its physical deck (SK8 skeleton_root.rs `animation_to_board`). The physical deck's
  centre is `DECK_HEIGHT` (0.0875 m: SK8 `RETAIL_LEVEL_GROUND_DECK_CENTER_HEIGHT`, the wheel radius
  0.031 plus the trucks' 0.0565) over the board's contact point. The imported rider is drawn with its
  board's lowest wheel (as posed: a manual pitches the board about the wheels left down) on the
  contact point, along the chord between the two trucks' contacts (so over a ramp's kink both wheels
  touch), and lifted out of the ground under any wheel (at most 4 cm: on a transition ridden across,
  the smoothed roll would dip a side's wheels in). The board used to be drawn with its deck centre
  on the contact point, its wheels 8 cm in the ground.
- **Steering fakie**: Skate 3's action graph hands the physics `FakieTurn` (`Turn` with
  `fakieFilter="negate"`, the `Turn` packet SetTurning publishes, SK8 set_turning.rs), so riding fakie
  the turn is negated before the trucks, whose yaw already reverses rolling tail first: the stick turns
  the travel its own way, as rolling forwards. flatground used to leave it as it was, so steering fakie
  felt inverted.
- **Turning round** (fakie to switch, the auto switch and pushing from fakie): Skate 3's physical board
  never turns; its mirror flag toggles on the switch clip's event and the "effective" board frame
  (SK8 board_motion_output.rs, `board_flipped`) turns end for end with it. flatground turns the
  board's frame (`fwd`, the speed, `Skater::frame_turned`), and every animation tree remembers the
  frame it started in (`playback::Tree::turned`): trees from before (the fakie riding and `B_SWITCH`
  itself, whose `R_SWITCH_RIDE_*` clips turn the shoulders from fakie to switch) are drawn turned back
  half round, so the rider and board stay where they are in the world and the blend into the switch
  trees turns only what the clips do. PAL50's blend between ticks turns its older frame with it. The
  board used to be drawn turned half round for the rest of the switch clip and then spun through
  180 degrees as the next tree faded in, 0.7 s after the switch.

## The air (physics: `skater/air.rs`, `skater/landing.rs`, `skater/respawn.rs`)

After SK8's `air/` (body_spin.rs, known/, state/, reckoning/, trajectory/), `animation/landing_quality.rs`,
`player/wipeout/air.rs`, `player/wipeout_state/recovery.rs`, `player/respawn.rs` and
`physics/revert_state.rs`; every number from the disc through the rules pack (`physics_bodyspin`,
`physics_airstates`, `physics_trajectory`, `physics_reckoning`, `physics_wipeout`, `physics_animation`,
the revert and checkpoint classes), else flatground's own.

- **Landing prediction** (the trajectory selector): at take-off, and every 7 ticks until a landing is
  known, the launch velocity and six more round a cone (`ConeAngleX`/`Z`, the latter scaled by
  `ConeAngleZVsSpeed`, at a cone speed clamped to `SpeedFactorMin`/`Max`, none faster than the launch)
  are followed through the park up to `TrajectoryMaxTime` / `TrajectoryMaxDrop`, with chord rays four
  ticks long, at most 28 a tick (Skate 3 queries them asynchronously too: an ollie resolves in two or
  three ticks, a long drop in more). Scores: `LandingTimeBonus` against the flight time, the landing
  force and direction terms (scaled by `LandingForceScalarVsDPVelNorm`), the middle candidate's bonus,
  wallride candidates (a floor well below: height x `ScoreWallrideHeight`, the highest +2000, the flight
  lifted by `WallRideBoost` within `MaxTrajectoryAdjust`), unrideable and don't-align surfaces (the
  collision surface id's category, bits 7..11: 6, 7, 8), the vert transition term after a vert pop. Under
  `MinTimeForTrajectoryToBeValid` it isn't a known landing (only its normal is kept); otherwise the board
  flies it ("known air"), its start error blended away over `TrajErrorBlendAwayTime`; a second, narrower
  pass (`ConeAngle*SecondPass`) runs when the landing surface is more than `MinNormalDeltaForSecondPass`
  off the ground left. A wall hit drops it and asks again. The cone itself bounds the nudge: an ollie's
  landing moves by a few tenths of a metre, onto a bank or ledge just beside it, not across a gap. The launch's own flight
  may be steered onto a rail (`ScoreGrind`, "Grinds" below); then the grind air assist turns the board,
  not the landing assist. The graphs' `TimeToLand` and the built-in rider's trick
  timing read this prediction.
- **Body spin** (SK8 `BodySpin::Update`): the stick's spin `PropBodySpinVsTime`, weighted by the
  strongest signed wind-up around take-off (`DerivativeBodySpinVsTime`, floor `MinDerivativeScalar`; a
  wind-up the other way is lost); let go, a low-passed copy of the stick (0.8/0.2, also on the ground, so
  a turn ridden into a pop carries into the air) decays by 0.96 a tick and keeps the spin going; speeding
  up by `MaxDeltaVsTime` a tick, slowing by no more than `MaxDeltaOppositeDirection`. The body (board and
  rider) turns about its own up, not world up.
- **Landing assist** (SK8 KnownAir `CalculateBodySpinSpeed`): with a known landing whose direction is
  faster than `MinTargetHeadingVel` and the spin intent at 0, the body turns its leading end (the nose,
  the tail after a fakie take-off) towards the landing direction, or the other way round (landing fakie
  or switch) past `MaxHeadingAdjustVsUpY`; aiming to finish 1.2x as fast as the time left asks (never in
  under 0.032 s), at most `MaxAutoBodySpinSpeed`, halved under `MinAutoBodySpeed`, x0.005 against the
  current spin and x0.75 slowing it, through the spin controller at `MaxDeltaForAutoVsTime` (at most
  2 rad/s). Without a known landing (PhysicsAir) the stick's spin goes through the same path
  (`BodySpinInputFilter` x `MaxSpinSpeed`). In Skate 3 a flag the external-physics path sets
  (`flags_2472` bit 28) hands the stick straight to the spin; it is never set in play, so flatground
  doesn't have it.
- **Attitude**: the board's up turns towards the landing surface to arrive 3 ticks before touchdown,
  never more than `Air_MaxUpVectAngleDelta` a tick (read at a 0.8 m centre of mass over the deck,
  flatground's); without a known landing `SpeedToAlignToGround_PhysAir` of the angle a tick, none past
  `DontAlignAnglePhysicsAir`.
- **Landing**: bails only on Skate 3's rules: the landing angle (board against the travel along the
  surface, from the nearer end) over the angle limit against that speed (lenient enough that riding
  speeds never bail on it), into the ground faster than `Wipeout_AirMaxSpeedIntoGround` (onto stairs,
  surface category 8, `Wipeout_AirMaxSpeedIntoStairs`; onto a rail, `..IntoCollisionNearGrind`), the board
  not caught (above), or (without the graphs, flatground's own rule) still tweaking a grab. The speed
  along the surface is kept, times `LandingSpeedScalarVsGroundNormalY` (transitions gain); the board keeps
  its heading and the wheels' grip and straightening take out the sideways part. The landing type
  (`CalcLandingQuality`) uses `LandingSketchySideSpeed` and `LandingSketchyTwistSpin`; a spin landing
  keeps pivoting 0.15 s (flatground's).
- **After a bail** (SK8 recovery and checkpoints): put back automatically once the body has been under
  `WipeoutOverSpeed` for `TeleportMinTimeAfterSettlingForAutoReset` after `TeleportMinTimeForAutoReset`,
  or after `TeleportMaxTime`, after a `TeleportAutoResetFadeOutTime` fade; A or B (Skate 3's push
  buttons, the `WipeOutRecover` request) put the rider back early once settled or after 1.6 s. Where: the
  spot the body came to rest on when it is safe (ground under it within reach, facing up enough, clear
  above, no rail at it), else the best of the last 32 checkpoints (recorded every 20 ticks of riding at
  least 1.5 m apart, skipping the newest; the one used is taken out), else the park's spawn; facing the
  way it was travelling, in the same stance.
- **Reverts**: a yaw servo (the revert class's speed against the angle left, gain, tolerance against the
  slope, longest time) turns the board half round on its wheels with the travel kept.
- **Body flips** (`skater/flip.rs`; SK8 known/body.rs, known/trajectory.rs, body_flip.rs,
  physics/known_air/flip.rs, known/post_physics.rs, input/body_flip_signal.rs): the left stick's
  `FrontFlip` / `BackFlip` shape (`skaterls`), counted only with a grab trigger in and no push button
  down, becomes the action graph's flip intent while the gesture is recent
  (`anim_motion/body_flip/extend_bodyflip_gesture`) and the rider has just left the ground
  (`extend_takeoff_point`); the motion graph attaches it for the physics. With a grab held, the flip
  starts once the known flight is longer than the start curve for the landing surface's up y
  (`physics_airstates` hashed field), about the board's nose axis so the head goes over the toe side
  (front) or the heels (back); its speed eases (`FlipSpeedSmoothingFactor`) towards `FlipScalar` turns
  over the flight, at most `FlipMaxSpeed`, while the grab is held or under
  `BodyFlipMinGrabTimeFraction` seconds in; let go, it slows to a stop. The body spin runs at
  `FlipBodySpinScalar` meanwhile. Upside down without the grab (up y under `Wipeout_AirFallingMinUpY`
  and against the landing surface under `Wipeout_AirFallingMaxAngle`) is a bail; the landing bakes the
  turn into the board, which must come down within 53 degrees of the surface (ours) or it bails. The
  graph's `IsBodyFlipping` reads it (its multi-flip and runout transitions). Drawn about a point 0.85 m
  over the board (ours); scored `BACKFLIP` / `FRONTFLIP` (`2X ...` for two turns). Skate 3's
  `PerfectBodyFlips` assist mode and the flip axis adjustment by the body spin are not run.
- **Board adjust** (`skater/adjust.rs`, `tuning/adjust.rs`; SK8 action_board_adjust.rs, pose_output.rs):
  the action graph's `BoardAdjust` hands the C-stick to the motion graph, whose `BoardAdjusts/Left`,
  `Right`, `Up` and `Down` clips (`BLEND_ADJUST_*`) swing or tip the deck against the body, and Skate
  3's physical board follows its posed one. flatground keeps the deck's yaw and pitch against the body
  (`DeckAdjust`): while the graph adjusts (its intents published and the motion graph in
  `/BoardAdjusts/`) they follow the imported rider's posed board (its nose turned round for a mirrored
  rider, SK8's `board_flipped`; only the way the stick asks, so the take-off clip's pitch fading under
  the adjust doesn't count), else the C-stick at the angles measured off Skate 3's clips (full stick:
  80 degrees right, 88 left for the goofy clips, the other way round mirrored; nose 19.7 down pushed,
  23.9 up pulled; half the rest of the way a tick, at most 13 degrees), with no trick turning the
  board, no grab and no body flip. `Skater::frame` turns and tips the deck about its middle by them,
  so the rail investigation's boxes (the family: across a rail a boardslide) and the grind air
  assist's points and turn are the adjusted deck's; meeting a rail or the ground bakes them into the
  board and the body rides it, so the landing's angle is the deck's. Skate 3 has no tuning attribute
  for it (the angles are its clips'); `GrindAdjustMaxAngle` is the flight's steering limit, not the
  deck's. With the imported rider, the clips' adjust may show doubled for the few ticks the landing
  clip takes to blend it out after the bake.
- Not done: the air leg's centre-of-mass height.

## Grinds (physics: `skater/grind.rs`, `rails.rs`, grind targeting in `skater/air.rs`)

After SK8's `physics/grind_contact/*`, `physics/grind_forces/*`, `physics/grind_air/*`,
`air/trajectory/grind*`, `air/trajectory/grind_surface/*` and skate-game's `physics/grind/contact.rs` and
`physics/grind_chromosome*`; every tuning number from the disc through the rules pack (`physics_grinds`,
`physics_grinds_air`, the grind fields of `physics_trajectory` and `physics_wipeout`,
`physics_mode/normal/GrindLockDist`, `physicsdeck`, `physicstrucks`, `anim_motion/grind_twist` and
`grind_height`; `game_core::tuning::grind`), whose built-in values are Skate 3's own (read off the disc). SK8's
code constants (box sizes, the family strengths, the launch and energy figures, the 17/24/60-degree windows,
the balance forces) are used as SK8 documents them.

- **Rails** carry a class per segment (thin, fat, ledge, curb, blocked) and a high side, found at import by
  probing beside them (docs/importing.md); a version-1 park pack loads with every segment thin. The
  probes are SK8 grind_surface.rs's (near probes 0.09 m out and 0.04 m up and down, far probes out by
  the disc's `DeckCenterToTruck` and 1.06 times that up and down, a ledge when a far probe meets a top
  within 0.65 of its reach, the 0.09 m blocking probe, the curb test). Checked on BlackBoxPark
  (`fg_preview --park ... --rails` prints every segment's probes): its handrails and flat bars come out
  thin (four segments each, 1.2 to 1.5 m over the ground) or fat (two bars whose collision sits beside
  the spline), and its 98 ledge rails are boxes, ledges and deck edges with a top level with the rail
  beside them; coping arcs are cut into many short segments, which is why ledge segments (1,642) far
  outnumber the rest (110 thin, 8 fat, 226 curb, 194 blocked). Lean-off applies only to ledges and curbs
  (and tip slides off thin rails), as in SK8: every grind of the Phase 6 tour was on a ledge or coping,
  so each leaned off; a thin rail's grind goes to its end (`grind: off (off the end)`).
- **Contact**: the rail must cross one of the board's boxes, around the deck's centre 0.0875 m over the
  board's contact point (`DECK_HEIGHT`, SK8's level-ground deck height: wheel radius 0.031 plus the
  trucks' 0.0565 drop): under each truck (`TruckToWheel` each side, 0.02 to 0.22 m under
  the deck), under the deck between the trucks (`TestDepthEpsilon` over to `TestDepth` under, the
  deck-centre-to-truck distance each way), past each truck under a tip (0.215 m long), over an upside-down
  deck (after a dark catch). The family, the board's current one tried first: both trucks a 50-50 (the
  sticks' x under 0.9), one truck a 5-0 (front or back by which truck; both: the C-stick's pitch), a tip a
  tip slide (a blunt when the deck lies over a ledge's high side), the deck a boardslide (rail up within 49
  degrees of the deck's, trucks clear of it), the upside-down deck a darkslide. Skate 3's tighter depth
  windows (its rigid deck comes to rest on the rail) are left to the boxes.
- **Admission and entry** (SK8 admission.rs, entry.rs): from the air any contact (after 0.08 s in the air
  and 10 ticks since the last pop out of a grind, any rail); riding: from above within 17 degrees of the
  rail, from below or into coping (the approach's slope sine past 0.25 / 0.96) under a cross-rail speed
  of (2.8 + 2.4 x the along-rail share) x `DVEntryThreshScalarVsSinSlope`, dropping in under 1.45 m/s on a
  tip or a truck (moving towards it, ours). Riding in, `VertEngagementHelpVsVelY` of the cross-rail
  velocity turns along the rail; from the air 0.4 (grinds), 0.2 (board and dark slides) or 0.8 (tips and
  blunts) of it is taken out; the speed along the rail is the full 3-D component, no minimum. A rail more
  than 60 degrees off the deck from the air, a velocity change over
  `Wipeout_AirMaxSpeedIntoCollisionNearGrind`, or more than 7 m/s across a thin rail or towards a ledge's
  high side wipes out; on fat rails and ledges more than 3.5 m/s towards the low side doesn't lock on.
  Riding steeply into a rail starts the coping hold (`TimeOfGravityReliefVsVerticalSpeed` x
  `GravityReliefTimeVsLinearSpeed`): a stall on coping stays on the rail while it lasts.
- **Grind targeting** (SK8 trajectory grind.rs): the landing prediction's own flight is steered onto a rail
  it comes down on within the lock distance (`GrindLockDist`, scaled on ledges by `GrindLockLedge*` by
  where the flight and the body are against the high side), not too fast across (`GrindMaxSpeedSqrInto*`)
  or down onto it (`GrindMaxSpeedDownOntoGrind`), turned no more than `GrindAdjustMaxAngle`, nudged within
  `MaxTrajectoryAdjust`; it scores `ScoreGrind` and lands on the rail's up leaned by
  `GrindLandingVelScalar` within `GrindLandingMaxAngle`. Other flights landing near a rail lose
  `GrindPenaltyVsDistToGrind`. The landing assist leaves the spin alone on such a flight.
- **Grind air assist** (SK8 grind_air): on a flight steered onto a rail, the board's five lock-on points
  (deck middle, nose and tail tips, both trucks) are followed `NumFramesToTest` ticks ahead (with
  `StompAccelerationAdjust`) to where they cross the rail's plane; the kind whose point comes nearest
  within its reach (`MaxDist*` x the deck-centre-to-truck distance) and whose angle is within its window
  (`AngleRange*`) is aimed for: the board moves towards the rail by at most `MaxOffsetDeltaPerFrame` a tick
  (`MaxOffsetDist` in all) and turns towards along or across it by at most `MaxAngleDeltaPerFrame` a tick
  (x `AngleYAssist*`). Skate 3 moves its animated board this way; flatground moves the board itself.
- **On the rail** the board slides along the rail and its frame eases (0.1 a tick) to the family's: 50-50 along the rail on its up, 5-0 tipped by
  the pitch and turned by the yaw, slides across it, tip slides leaned 20 degrees, blunts 42, darkslides
  upside down; the trucks' hangers (0.064 m under the deck's centre, ours) rest on the rail in 50-50s
  and 5-0s, the deck's underside (its grip in a darkslide) in slides. Gravity acts along the rail; friction = 0.48 x the rail's up y x strength[family][class]
  (grinds 50/40/40 thin/fat/ledge, slides 55/55/60, tips 60, blunts 70/80/80) x `FrictionVsTime` x 1.3,
  over an effective mass of 25 kg (ours: a thin-rail 50-50 slows about 0.8 m/s^2 at entry, flatground's
  grind friction before). The C-stick's y (cubed, smoothed 0.91) tips a 50-50 past 0.12 into a nosegrind
  (up) or a 5-0 (down); in a 5-0 the free end is held up with the stick let go, lifts no more than 0.26
  rad and comes back up below -0.25; both sticks' x (dead zone 0.25, cubed) turn it across the rail;
  level again with the other truck over the rail it is a 50-50, and both sticks' x at 0.9 or more drop a
  50-50 onto one truck (SK8's 50-50 needs them under 0.9). A deck more than
  `Wipeout_GrindMaxAngularDeckError` from where the rail holds it wipes out.
- **Balance** (`skater/balance.rs`; SK8 `grind_forces{,/slide,/support,/post}.rs`, skate-game
  `physics/grind/contact.rs`): Skate 3's board on a rail is a rigid body its family's forces move;
  flatground gives the board that freedom across the rail: how far the family's reference point (the
  deck's middle for 50-50s and slides, the truck on the rail for 5-0s, the tip for blunts) is off the rail
  and how fast it moves, under Skate 3's forces over the effective mass. Pins pull it back (50-50 800 N/m,
  5-0 3000 at the truck, blunt 3000 at 0.39 m along the board), damped by 0.133 of their strength and
  scaled by `PinVsSlope` against the grind normal's up (leaning off weakens them; the coping hold keeps
  them full). Board and dark slides: the sticks' x push 25 N a unit (darkslides 40 N, fading out 0.16 m
  off the middle), damped 20 N per m/s inside 0.16 m, pulled back 10 N and stopped past it; on a ledge only
  the push acts; a wound-up pop pulls them in (201 N past 0.12 m). Tip slides: the main stick's x pushes
  37 N a unit (pulled in 39 N for 8 ticks after dropping in faster than 1.8 m/s). The exit lean pushes to
  the low side (`ExitAssistVsLeanAngle` x 65 N) and friction damps the speed across with the speed along.
  The sticks push to the rider's right of the travel. The grind is lost when the rail leaves its box (the
  truck's width `TruckToWheel`, the deck between the trucks for a boardslide, 0.3 m for a darkslide, the
  tip): the board falls off with its speed (`fell off`); a tip slide pushed in past the truck is a
  boardslide. The speed across a new segment after a bend stays the board's; the speed into the rail it
  meets (coming onto it, or a dip) is its closing speed, and more than `Wipeout_GrindXZDeck` (6 m/s) of it
  across the rider's ground plane wipes out (SK8 post.rs; square to the rail it is all along the rider's
  up). A board or dark slide that stalls on a rail (not a ledge) hangs up and wipes out (SK8
  involuntary.rs, wipeout 10).
- **Grind grabs** (Skate 3's `OnBoard/Grinding/<grind>/<grind>_<grab>` states): a trigger fully in is the
  action graph's `LeftGroundGrab` / `RightGroundGrab`, which its `Grinding` state turns into
  `WantsLeft/RightGrindGrab` and its `LeftGrab` / `RightGrab` / `DoubleGrab` states into `GrindFSGrab` /
  `GrindBSGrab` (the other way round mirrored) / `GrindDoubleGrab`; the motion graph's grind-grab states
  (`Into`, `GrabCycle`, `Out`; `IsCrouchedEnoughForBlendToGrabCycle`, `CurrentGrabType`, `HandBusy`) play
  them, score `<grind>_<grab>` in the cycle (`ScoringTrick`), and publish `PhysGrindGrabMinHeight` (0.2),
  which holds a 5-0's free end up (SK8 control.rs). The physics reads the grab from the scored name and the
  height from the packet (`GraphControl::grind_grab`, `grind_grab_min_height`). Without the graphs, the
  same table: 50-50s FS/BS grab, 5-0s and salads FS grab/nosegrab, nosegrinds and crooks tailgrab/BS
  grab, nose slides and noseblunts tailgrab (frontside hand), blunts tailgrab and tailslides nosegrab
  (backside hand). A grab held 0.2 s or more is scored with the grind (`FS 5-0 NOSEGRAB`, plus 200 points
  and 150 a second, as air grabs).
- **Lean-off** (SK8 balance.rs): on ledges (and tip slides on anything but a thin rail) a timer starts on
  entry; after 2 s (0.8 from below, 0.1 into coping) `ExitLeanAngleVsTime` rolls the board off the low
  side, the exit lean pushing it there against the weakening pins; past 28 degrees, or past 15 with ground
  under the low side (or once the rail leaves its box while leaning), it is off, at least 0.6 m/s off the
  low side (ours).
- **Leaving**: off the rail's end (grinds lift 0.4 m/s, ours: SK8's release strength comes from its caller),
  leaning off, stopped (under 0.2 m/s for 0.4 s outside the coping hold, ours), or popped. The pop (SK8
  launch.rs): the speed along the rail kept, the jump x 1.03 along the launch normal (tip slides leaned
  0.291 outwards, blunts 0.109 inwards, others halfway between the rail's up and the board's), sideways the
  main stick x 1.8 less the high side's push (0.5, slides 0.9 on ledges), capped at 1.8 m/s; each pop costs
  0.6 energy, back at 0.0035 a tick; under 0.31 the pop goes off to a side and grind targeting waits 10
  ticks. A rail left without a pop can't be taken again for 0.5 s (ours). Without the graphs the flick
  pops with the grind's group (Nose for the front truck's grinds and nose slides, Tail for the back's).
- **Names** (SK8 grind_chromosome.rs, pose.rs, grind_names): FS or BS from the rider's chest against the
  side of the rail the rider came from (the pose at take-off, or 30 ticks of riding before); the truck or
  tip on the rail; straight (the deck within 20 degrees of the rail); low (the free end under the rail's
  plane); turned round from the approach (the `BF_` variants of 50-50s and 5-0s); facing along the travel;
  the family. They give `FS_`/`BS_` + `50_50`, `5_O`, `NOSEGRIND`, `CROOK`, `OVERCROOK`, `WILLY`,
  `OVERWILLY`, `SALAD`, `SMITH`, `FEEBLE`, `BOARD`, `LIP`, `NOSESLIDE`, `TAILSLIDE`, `NOSEBLUNT`, `BLUNT`,
  `DARKSLIDE`, and `BF_` + the 50-50 and 5-0 family's: every name the motion graph's `IsGrinding` tests. The
  graph sees a new name from the next tick, the score once it has held 12.
- **Graph**: `IsGrinding` reads the name, `IsGrindBluntingBackslash` the blunt family,
  `GrindTrickOutTypeAllowed` a darkslide's way out, `TrucksOrDeckInContact` the grind; the grind trees'
  `GrindControlFade` (twist), `ControlGrindCrouch` (height) and `CreateGrindAttributes` run (above).
  Wipeouts stay with the physics.
- **The camera** (phase 7, "The camera" below) reads the rail's direction and incline, the ledge's
  high side and the speed along it.
- Not done: the board's orientation is still eased kinematically and only its motion across the rail is
  physical (Skate 3 drives the whole rigid body, whose collision with the rail's mesh also supports it:
  flatground's rails have no mesh, so the support is a constraint, the coping's pin/lift forces and the
  5-0's truck compensation have nothing to act on, and a tip slide has no restoring force but the stick's);
  the involuntary exit's release forces (flatground pushes off at once) and the 35-tick wipeout; the
  orientation noise; the grind normal's tilt on ledges from the far probes (the rail's up is used); the
  5-0 hang rule on ledges; the trajectory candidates' height preference (`RequiredAngleVsHeight`).


## Off the board (physics: `skater/biped.rs`; `skater.rs` `toggle_board`)

Skate 3 has no separate off-board graph: the `OffBoard`, `Dismount` and `Mount` states are part of
`ActionGraph_OnBoard` and `MotionGraph_OnBoard`, and their clips (`BR_*` with the board in hand, `NB_*`
without) are in `OffBoard.abin`, which the importer merges into the motion library.

- **Getting off.** Y on the ground (not on ground whose up is below 0.64: SK8's `DisableDismount`) sends
  `NewToggleOffBoardState`; the action graph goes to `TransToOffBoard` and the motion graph to
  `Dismount/Ground/UserDismount` (into a stand under 1 m/s, a run under 8, a fast run under 15, else a
  runout: `ComVelCompare`). The board keeps rolling until the dismount clip's `OB_TRAJ` window starts
  (SK8: `OB_Traj` sets the physics' go-to-biped flag), then the walker (`Mode::Walk`) takes over, facing the
  board's travel at its speed. The motion graph then goes to `OffBoard/OBGround/Locomotion`.
- **Walking** (Skate 3's biped, as SK8 ports it: `ground_input.rs`, `movement_intent.rs`,
  `movement_velocity.rs`, `cadence.rs`; the curves are `physics_biped` and `physics_state_offboard`, with
  Skate 3's values built in). The stick, turned into the world by the camera's yaw, is read against the
  facing: its angle from straight back (0..1 of pi) gives the move amount (`Hash_DF759B46440F16E9`) and
  turn (`TurnVsStickAngle`), times how far it is pushed. Speed is `SpeedVsInput` of the move amount (6 m/s
  at full), blended towards the sprint's 10 m/s by `Hash_6B93C51256A30FB4` of the time sprinting (a burst
  that fades by about 3.8 s; a release keeps counting 0.3 s), scaled by the slope (`Hash_31309236050A8F09`).
  The rider always moves along his facing: the velocity changes by at most 0.2 m/s a tick and slows by at
  most 2 m/s^2 while still walking; the turn rate chases `TurnVsSpeed` x turn by at most
  `TurnDeltaVsSpeed` a tick. `LocoState` is stand / walk / run / sprint by the speed against the halfway
  points between the walk, run and sprint cycles' own speeds (1.68, 5.37, 9.10 m/s from their
  `ANIMTRANSZ`): 3.53 and 7.24 m/s. The ground is flatground's own: the highest surface no more than
  0.45 m above the feet, a wall above that, a fall below a 0.35 m drop.
- **Jumping**: B (`OB_Jump`). Up at sqrt(2 g `JumpHeight`) (0.9 m), ahead at the ground speed's forward part
  x `JumpSpeedScalar` (0.8). In the air the motion graph plays its jump selection (`OBAir/InAir/FromRun`,
  `FromStand`), and `MatchAirTime` seeks the clip to the flight's progress (1 - time left / whole time) and
  sets its `AnimTime` and `AnimTrans*` (the travel, at most 2 m), so the clip ends on landing and the
  landing states pick their clip from it. flatground predicts the landing as a fall straight down to the
  ground under the feet.
- **Getting on**: Y on foot. The motion graph's `Mount` states play a mount clip from the stand, walk, run or
  sprint; when its `OB_TRAJ` window ends the rider is on the board, its nose the way he faced, rolling at
  his forward speed, and the graph goes on to `OnBoard`. Without the state graphs Y gets off and on at once.
- **What the graphs read** (`drive::walking`): `PhysFilteredState` offboard / offboardair, `LocoState`,
  `IsHoldingSkateboard` (the board in the hands: `skater::possession`), `IsDeckFree` (free, hidden or
  flying back), `IsDroppingSkateboard` / `IsRetrievingSkateboard` (below), `IsInBipedAir`,
  `CanBipedLand` (in the air), `CanLandOnBoard` (on foot only once the landing assist has taken a flight
  onto the free board, below), `IsLandingOnBoard` (coming down onto the board from the air: the hippie
  jump, the air mount and the caveman; SK8's physical state 503), `GroundSlopeType`'s ramp classes (5 and
  25 degrees; flatground doesn't tell stairs apart yet), `ComVelCompare` in the facing's frame.
- **Drawn**: the off-board clips' origin is the ground under the feet (the riding clips' is the deck's
  centre); the drawing moves between the two over 0.2 s while getting off or on.
- **Dropping, throwing and picking up the board** (`skater/possession.rs`, Skate 3's SkateboardController
  possession as SK8 ports it: skate-core player/offboard/board_possession, skate-game
  physics/offboard/board_manager; the animation side is `graph/toggle.rs`, SK8's ToggleBoard). Off the
  board, L fully in is `OB_DropBoard` and R fully in `OB_ThrowBoard` (SK8 offboard_intentions.rs: LT / RT
  reaching 1.0, by position on the pad), either also `OB_RetrieveBoard`; the action graph's `OffBoard`
  passes them to the motion graph (`CreateMGIntentFromAGIntent`), whose `OffBoard` ToggleBoard plays, with
  the board in hand, `OFB_THROW_90` (drop) or `OFB_THROW_0` (throw) on the `RetrieveBoard` channel
  (`IsDroppingSkateboard` while it plays, which keeps the `Mount` states out), and without it the
  `OFB_RETRIEVE_HIGH_FRONT/BACK_INTO`, `_CYC`, `_OUT` trees aimed (`yaw`, at most 5 degrees an update) at
  the board (`IsRetrievingSkateboard`). The physics lets go at the throw clips' `OB_RELEASEBOARD` /
  `OB_THROWBOARD` (about 0.4 s in) and flies the board back at each cycle's `OB_RetrieveBoard`. The board
  then is its own body: a drop keeps the hands' velocity, a throw goes 18 degrees up from the facing
  (`ThrownSkateboardLaunchPitch`) at the rider's speed plus `ThrownSkateboardVelocity` of it (3 m/s
  standing), turned for 8 ticks towards `ThrownSkateboardPitchTarget` and its heading (the yaw, pitch and
  roll scalars); it falls, lands on its wheels (restitution 0, friction 0.36: the released board's
  material, `physics_wipeout/SkateboardFriction`), rolls along its nose with the riding board's rolling
  resistance without input and the slope, slides sideways against that friction, and stops at walls.
  Further than `MaxDistance` (30 m) from the rider it is hidden (drawn out of sight). Picked up, it flies
  from where it is (or from `DistanceForBoardReturn`, 30 m, away) to the hands over `RetrievalTimeVsDist`
  of the distance, weighted by `RetrievalSpeedCurve`. Getting on (`OB_Mounting`, the mount states'
  attribute from `OB_Mount`) fetches a free board the same way from at most `DistanceForMountedReturn` (5
  m) in at most `MinTimeToRetrieveBoardWhileMounting` (0.1 s), so the mount clips (board in hand) play as
  in Skate 3; without the graphs Y next to the board (within 5 m) steps onto it where it lies, at its
  rolling speed, its nose or tail ahead, further away fetches it first. The board off the feet is drawn
  where its body is (the imported board's bones placed there, blended back into the clips' board as it
  reaches the hands; the placeholder board likewise). The physics' possession values are Skate 3's
  (`physics_skatecontroller`, `tuning/possession.rs`), with the rules pack's taking over.
- **The caveman** (landing on the board from the air off it; SK8 physics/landing_on_deck, skate-core
  player/offboard/landing_deck). In the air off the board with the board free (dropped running, rolling
  alongside), each tick Skate 3's landing assist looks for a flight from the feet down onto its deck (the
  later time the rider meets it, against gravity, or half gravity while the board is in the air too; that
  time stretched to 0.75..1.3 of itself towards the free fall's), and takes it when it is at least 0.4 s
  and within `landing_assist_limit` of the jump's own (3.5 m/s at take-off, 0.5 m/s from 0.19 s: biped_air
  orientation.rs) and the board is upright (`DeckMinUprightness`). That is `CanLandOnBoard`: the motion
  graph goes from `OffBoard/OBAir/InAir` into `Mount/Air/IntoLandingOnBoard`, whose `DoLandOnBoard` starts
  the landing on deck in the physics (at once without the graphs), then `OnBoard/LandingOnBoard`
  (LandOnBoard: coming down onto the board the wrong way round, it asks for its tail to be taken ahead,
  Skate 3's FlipBoardBackwards; flatground takes the board's frame that way rather than turning the
  rider). Touching down with the feet within 0.55 m of the deck rides on it from where it is, keeping the
  speed along it; missing it (`!CanLandOnBoard`: `Dismount/Air/HippyRunout`) he lands on foot; faster than
  `MaxSpeedLandingOnBoard` (11.5 m/s) a bail. Y in the air with the board in hand is still the air mount
  (`Skater::mount_in_air`); with it free, Y fetches it (`OB_Mounting`) and mounts once it is in hand.
- **Not done**: moving objects, standing on cars, hippie hurdles, stairs' own clips, the obstacle steering
  of `movement_intent.rs`, foot IK; the free board has no rigid rotation of its own (it keeps its nose,
  pitched only by the throw, and lands on its wheels whatever way it falls) and doesn't collide with the
  rider; the retrieve hands (SK8's hand contact that catches the board early) aren't modelled.
- **Camera**: walking is `IsOffboard` and the walker's air `IsAirOffboard` (both read by Skate 3's camera
  graph).

## Controls (GameCube pad)

Buttons map by position on the pad: Xbox A to GC A, X to B, B to X, Y to Y, RB to Z, LT/RT to L/R.

| GC | Intents | Does |
|---|---|---|
| Y | `ToggleOffBoardState` (held), `NewToggleOffBoardState` (pressed) | off the board and back on (below) |
| L / R fully in, off the board | `OB_DropBoard` / `OB_ThrowBoard`, either `OB_RetrieveBoard` | drop the board, throw it down ahead; out of the hands, pick it back up (above, "Off the board") |
| A | `RightPush`, `Pushing`, `NewPush` | push (a press is remembered for 0.3 s); after a bail, back on the board once settled (Skate 3's `WipeOutRecover`) |
| B | `LeftPush`, `Pushing`, `NewPush` | mongo push; after a bail, as A |
| A and B | both pushes | the hippie jump: the action graph takes the pair as its wind-up (`OnBoard/HippyJump`), as SK8's riding intentions pass both on |
| X | `Brake` | foot brake |
| Z | `DarkCatch`, `NewDarkCatch`, `GrabWorld` | dark catch. SK8 also reads Xbox B for it; on the GameCube that is X, the foot brake, so only Z |
| d-pad | `GestureLeft/Right/Up/DownStart`, `...Held` | character gestures (shrug, peace, shaka, surf's up) |
| L / R | `LeftAirGrab` / `RightAirGrab`, fully in `Left/RightGroundGrab`, `Crouch` | grabs, crouch; with the left stick's flip shape just after take-off, body flips. A GameCube trigger at rest reads 1..7 of 255 and SK8 counts any trigger above 0, so the pad has a dead zone (`input::TRIGGER_DEAD_ZONE`, 0.1 of full, then rescaled); the physics grabs on the same reading |
| main stick | `Turn`, `HardTurn`, `HardTurnCrouch`, `KickTurn`, `BodySpin`, `Left/RightSlide`, `Left/RightSlideStart` | steering, kick turns, powerslides, spins |
| C-stick | `AnticMag`, `AnticAngle`, `Manual`, `ManualBrake`, `TweakX/Y`, `BoardAdjustAngle/Mag` | wind-up (past 0.9), manuals (held below full for 0.2 s), grab variants (past half way) and tweaks; in the air the board adjust (`OnBoard/InAir/BoardAdjusts/*`); on a rail its y tips a 50-50 into a nosegrind (up) or a 5-0 (down) and its x (with the main stick's) turns a 5-0 across the rail (Skate 3's `PhysGrindUpDown`, `PhysGrindTranslation`) |
| off the board | `OB_Jump` (B pressed: Skate 3's X), `OB_Sprint` (A without X), `OB_Mag`, `OB_Steer` (the walker's move amount and turn), `OB_BipedWorldX/Z`, `OB_BipedStickMag` (the stick; on the board the raw stick, which the dismount reads), `OB_LookAtX/Y` (C-stick) | walking, below |
| gestures | `Trick`, the gesture's name, `GestureSpeed`, `HoldPattern` | right stick: `skater`, `skater90`, `skaterN90`, `skater_air`, `skater_fingerflip`; left stick: `skaterls` (SlideFs180/SlideBs180 reverts), `skaterstep`; all recognised in parallel at 60 Hz |

Gesture groups come from the graphs (`HasGestureIntent`, `CreateTrickIntentFromGesture`): Square takes
every flick as it is; Nose and Tail groups turn the other end's flicks into nollies or ollies (shove-its keep
their direction); the 90 groups also take their `90_`/`N90_` flicks. So manuals use Square and grind
trick-outs use Tail, Nose or the 90 groups by grind, as in Skate 3. When several gestures complete on the
same tick the one in the lowest hash bucket wins, as in SK8's `select`: the bucket is the sum of the
gesture name's words in the engine's text encoding, modulo 43 (`host::gesture_bucket`, computed from the
names). Within a bucket flatground keeps string order (Skate 3's table order isn't reproduced). Body
flips (`FrontFlip` / `BackFlip`) count only while grabbing and not pushing (SK8 `permitted`).

## Operations

Run: every operation the on-board graphs reach in riding, pushing, braking, wind-up, tricks, manuals,
landings, kick turns, reverts, switch, powerslides, grinds, air and grabs. Among them `PlayAnimation`
(switch and mirror variants, parameters from intents, filtered intents and the last animation),
`CreateAttribute`, `AttachIntent`, `FilterMotionGraphIntent` (rise/fall rates, ramp, blend-out, clamps),
`SetTrickHeight`, `JumpInto`, `SetTrickAttr`/`ClearTrickAttr` (tree construction), `ScoringTrick`,
`MonitorUnderflip`, `SetDark`, `ChooseRandomLanding`, `KickTurnSteering`, the manual out timer,
`DisableTricks`, the riding behaviours above (`SetTurning`, `ApplyingBodyTilt`, `SettingBodyTilt`,
`Crouching`, `SetDistComToBoard`, `StoreLandingData`, `SetLandingData`, `SetManualAngle`); the action
graph's `CreateMGIntentFromAGIntent` (filters, defaults, scale), `BoardAdjust` (the C-stick's
`BoardAdjustMag`/`Angle` published to its `MGIntentMag`/`MGIntentAngle` every update, through
`angleFilter`, negated mirrored unless `negateOnMirror="false"`, held at +-pi once the angle crosses the
seam; SK8 action_board_adjust.rs), `CreateConstMGIntent`,
`CreateMGTimeIntentFromAGIntent`, `CreateTrickIntentFromGesture` (groups, mirror, dark catch,
underflip, `*Hold` successors for multi-flips); the `OverideNextAnimTransitionHook` and `GrabSlide`
hooks; and the conditions on intents, clip attributes, clip expiry and time windows, state times, last
and current state, physics state, grind name, stance, speed, slope, time to land, gestures, grab type,
tweak, manual timer, wipeout and runout, trucks or deck contact, crouch height, dark catch and
underflip requests.

### Audit of the no-ops (861 operations)

Every behaviour and condition that was a no-op or a fixed value, by whether it matters on the board.

Ported (they set riding tree parameters or decide states): `ScoringGrabs` (the grab's scored variant),
`TweakProject` (`TWEAK_X`, `TWEAK_Y`), `BodySpin` (the `IA_BODYSPIN_OLLIE_FS/BS` channels' influence from
the spin intent, `anim_motion/body_spin`), `ControlAirLegExtension` (`EXTEND` against seconds coming down,
`anim_motion/inair_disttocog/lo_air_arm_extend`; its centre-of-mass height part isn't run: the riding
behaviours keep `disttocog`), `SetDeckPitchAndYaw` (`SKATEYAW`, `SKATEPITCH` on begin from the imported
rider's posed deck, `host::deck_angles` after SK8 `publish_deck_angles`; its attributes default to
`skateyaw`/`skatepitch`, which Skate 3's graphs leave unset), `BoardAdjust` (below), `SetTurning` (3, was a no-op arm),
`ApplyingBodyTilt` (7), `SettingBodyTilt` (1), `Crouching` (1), `SetDistComToBoard` (31),
`GrindControlFade` (the grind trees' `twist` from `GrindBalanceX`, `anim_motion/grind_twist`, and their
`DistToCog` on begin), `ControlGrindCrouch` (`disttocog` between `anim_motion/grind_height`'s heights from
the crouch), `CreateGrindAttributes` (the grind's name and `GrindFacingBackwards`/`Forwards` as packet
values),
`StoreLandingData` (1), `SetLandingData` (1), `SetManualAngle` (2); `SetManualOutTimer` /
`UpdateManualOutTimer` now follow SK8 (set on leaving the manual); `TrucksOrDeckInContact` and
`IsCrouchedEnoughForBlendToGrabCycle` now read the physics and the crouch. Phase 8 (above, "Phase 8
operations"): `BodyFlippingSignal`, `IsBodyFlipping`, `CharacterGesture`, `Pumping`, `IsBumped`,
`SetBumpCoefficients`, `PowerSlideSpin`, `PowerSlideDecel`, `PushOut`, `MatchTwistAndLean`, `Wipeout`,
`EnableWipeoutGestures`. With them the trick tour (BlackBoxPark and the demo park) logs no tree played
with an unset parameter.

Left as no-ops on purpose:

| Operations | Why |
|---|---|
| `HandBusy`, `EndGesture` | hand services (hand IK, holding things): on the board flatground's hands are always free; gestures end through CharacterGesture's own stages |
| `SetScoreAugmentation`, `JuiceHook`, `PrintText2D` | scoring augmentations, effects and debug text: no effect on riding |
| `BipedCadence`, `MatchCadence`, `MatchAirTime`, `OffboardBodyTweakBlend`, `AirDismounting` | off the board (walking, running, carrying the board). The runner always hands the walker's step cadence (`Biped::phase`) to the animation as `BipedCadence` does, so the phase-controlled walk, run and sprint cycles play at it |
| `InitMovingObjects`, `MovingObject`, `UpdateStandingOnCar`, `Shove`, `MaintainShove`, `EndShimmy` | moving objects, shoving people, skitching: systems flatground doesn't have |
| `FootPlantAbsorb`, `SetHandPlantAnticLength`, `ScoringHandPlants` | foot and hand plants (not in flatground) |
| `AddRunoutAttribs`, `HippyJumpAntic` | the physics owns runouts and the hippie jump |
| `ForcePhysics` | flatground's physics always owns the board (no animation-driven board) |
| `UpdateTimeSinceTeleport`, `UpdateTimeSinceKickturn`, `UpdateRidingFakie` | the runner counts the times every tick; riding fakie is the physics' rolling direction |
| `ResetSkaterAnimation`, `ResetToGivenStance` | the runner starts the graphs over when the physics puts the rider back on |
| conditions of absent systems (fixed): skitching, water, moving objects, biped/off-board states and landings, hand and foot plants, debug animations, `IsProSkater` (no pro skaters), `DisableDismount`, `EnoughDistToObstacle`, `OBTrajTime`, `LocoState` | false (true for `EnoughDistToObstacle`), keeping the graph on the board |

Left, and they do touch the pose (the unset-parameter log names them; their blends sit at the neutral
point or SK8's first child):

| Operations | Parameters | What it needs |
|---|---|---|
| `PowerSlideManualAtt` | the slide trees' `balance` packet from a negative `Manual` intent | a manual's back stick in a slide (rare) |
| `FingerFlipOut`, `UpdateIsWeightOnNose` | finger flip out, nose weight | grab and nose-weight feedback from the physics |
| conditions: `MongoPushFootToFar`, `HasTiltToLargeForPreland`, `IsLandingIntoGrind`, `IsGrindApproach`, `WipeoutTimeToLand`, `WipeoutTimeSinceContact`, `IsPushOffEnabled` | (fixed false) | the mongo-push wind-up shortcut, pre-landing tilt, grind approach, wipeout timing (no ragdoll) |

Unsupported, logged once when reached: `EnterSkitchingBehaviour`, `SkitchingBehaviour`,
`SkitchShimmyingBehaviour`, `GestureType` (the wipeout body tweaks: cannonball, judo kick ...; no
ragdoll to tweak).

Approximations to revisit: the push speed is where the push coefficients' blended clips end, not the
trajectory bone's speed sampled from the posed clips; how long the push foot stays down still comes
from the push sets' contact time; the manual's deck pitch runs SK8's PID servo (with the rules pack's
`physics_manual` gains), but on flatground's kinematic board, without the corrective drag Normal
pushes at the dragging truck (the tail brake stands in) or the no-contact torque bleed (the manual
ends on leaving the ground instead); a landing in the danger zone is judged on the speed
into the ground only (Skate 3's closing-velocity check also has a sideways part); shoves don't run;
the wipeout's lean and twist come from the drawn bail, not a ragdoll's hips; the d-pad's gestures are
a fixed set (Skate 3's are the player's pick). The
turn conditioner reads the physics' smoothed steering and a centripetal lean instead of Skate 3's
processed turn and reckoning lean, and the speed wobble's amplitude (`World::wobble`, Skate 3's
`SpeedWobble` from `skater/wobble.rs`); Crouching has no board or ground absorption
force (only its restoring force) and no deck-angle crouch; the powerslide's side for SetTurning is the
physics' slide.

## The camera (`game_core/src/camera`)

After SK8's `crates/skate-core/src/camera` (shot.rs, shot_manager.rs, manager_*.rs, rig*.rs, compass*.rs,
positioner.rs, avoidance.rs, drop_predictor.rs, frame_composer.rs) and `crates/skate-game/src/camera`
(graph conditions, shot data, the subject it publishes). Every update runs once per 1/60 s tick (Skate 3's
smoothing is per update, not per second); PAL50 frames draw between the last two ticks (eye, look-at, up,
field of view).

- **Shots** (`shot.rs`): distance, elevation and heading round the skater, framing yaw, pitch and roll,
  look-at point (a weighted mix of head, hips, grind point, centre of mass, board, feet, damped centre of
  mass, lifted along the body's up by the board offset), lens, smoothing of heading, elevation, pan and
  tilt, options (follow the board into the air, mirror for the stance, keep the previous shot, drop
  predictor, avoidance off), anchor (board, hips, air, grind, damped centre of mass) and compass north.
  Blend shots mix up to three children by a value from the skater: speed along the board (1), the
  travel's slope (6), the previous shot's distance (7), progress to and past the apex (8), steering
  (13, + left, negated when the stance is mirrored), predicted air time (16), the rail's or travel's
  slope (20); the value is filtered `(1 - smoothing)^2` a tick, placed between the blend points and
  sine-eased. A shot change keeps the shot being shown as the previous one and eases every field over the
  new shot's transition time (the graph's `transitionIn`, else the last shot's `transitionOut`, else the
  shot's own) with a sine curve; `UsePreviousShot` shots (small airs, grind exits) take over the previous
  shot's arm, distance and framing. As in SK8 (shot_manager.rs `set_shot`, shot.rs `interpolate_from`),
  the previous shot's arm is a world orientation frozen when the shot changes, so a change during a
  carve eases from where the arm was, not from a heading that keeps turning with the rider; and a small
  air (`low_ollie`) keeps the arm it took over for the whole flight. An ollie popped in the middle of a
  hard carve therefore flies with the camera where the carve left it (up to about 90 degrees to the
  side), in Skate 3 as here; ridden straight for a moment first, the camera is behind (within 15
  degrees about 0.3 s after a 90-degree carve at 6 m/s, never past about 41 degrees in it; the
  `--carve-ollie` preview and `tests/camera_carve.rs`).
- **Choosing**: Skate 3's camera graph (`Default_cameragraph`, the low camera) runs on the graph runner
  (`select.rs`). Its conditions read the camera's own state: `IsWipingOut` (a bail), `IsOffboard` (a
  run-out, a hippie jump), `SkaterIsGrinding` / `SkaterWasGrinding` / `IsPlayerSkateboardOnGround` (the
  rig's mode: riding, a flight, a grind, and the one before), `PredictedAirTime` (the known landing's
  flight time), `PredictedMaxHeight`, `PredictedLandingHeight` (over the take-off),
  `PredictedHeightDistanceRatio` (the top over the camera's distance at take-off), `LaunchInclineAngle`,
  `LandingInclineAngle`, `IsReentry` (up a wall and back down one), `TransferAngle`, `FrontsideAngle`,
  `IsInManual`, `IsPreparingToJump` (the motion graph's wind-up states' `IsAnticipating`, Skate 3's
  anticipating flag; without the graphs a crouch), `TurningCentredTime` / `TurningTime` (seconds since the
  steering was last hard over / has been: SK8's `steering_for_turn`, the physics' turn or the stick's turn
  or kick-turn input when that agrees and is larger), `CameraSubjectSpeed` (the board rolls on the
  pop's tick, so it doesn't read a one-tick stop), `GrindInclineAngle`, `IsPreviousShot`, `IsGrindLedgeLeft` /
  `Right`. Skate 3 marks ledge shots with painted world regions; flatground reads the ledge's high side
  (`camera::subject::ledge_side`). Conditions about things flatground doesn't have (skitching, plants,
  bonelesses, moving objects, roads, volumes, security guards, broken-bone slow motion, body-tweak
  wipeouts) are false; `SlowMotionController` is not run. Without the disc's graph flatground chooses
  itself (`select::built_in`): wipeout, run-out, grind exit, grind (ledge side), long air or small air,
  manual, hard turn, riding.
- **Rig** (`rig.rs`): the shot's heading turns with a compass north: the movement heading moves 90% a
  tick towards the velocity's direction above 0.5 m/s (no dead zone) and, stopped for 0.05 s, lines up
  behind the board at 50% a tick; it is frozen through a flight and turns towards the flight's direction
  when the landing is more than a metre away. The arm's heading tracker smooths `1 - h^2` (h = 1 -
  `SmoothingDirection`) near the target, letting go to 0.8 at 45 degrees off, with the acceleration clamped
  harder near the target (`camera_tracker/heading`), so wiggles are ignored and big turns whip round; the
  elevation tracker likewise. The stance mirror (`mirrored != riding fakie`) eases through two trackers,
  quick for headings, slow for framing, and only on the ground. The anchor lags while pushing (and half a
  second after) and while pumping (`camera_lag_profile`), less from 8 m/s and not at all from 12. Through
  a flight with a shot that doesn't follow the board (a small ollie) the look-at height ramps from the
  take-off height to the landing's, never above the anchor; shots that follow take the anchor's height.
- **Collision**: three lines (radius 0.13 m) from the centre of mass, the head and the feet to the camera;
  only when all three are blocked is the camera pulled in in front of the first hit, and the distance
  then recovers through its tracker (speed and acceleration clamps, faster below about a metre), never
  closer than `MinDistance` (`MinDistanceDuringWipeout` in a bail). How much of the shot's distance is
  clear raises the camera: (distance - clear) / distance times 50 degrees (60 off the board), climbing a
  fifteenth of that a tick and sinking a hundredth. **Avoidance** (mode 1): when the camera's path a second
  ahead runs into something before the skater's paths do, the heading and elevation swing towards the
  skater's breadcrumb trail. **Drop predictor**: five probes ahead along the velocity (spaced over its
  look-ahead) measure the ground's drop below deck height; a drop steeper than 15 degrees raises the
  elevation towards it (at most 70 degrees, 0.1 to 1.6 degrees a tick, scaled 0.9 to 1.0 by speed).
- **Framing** (`frame.rs`): the look direction to the shot's look-at point (at most `MaxPitch` up), turned
  by the framing yaw (times the framing mirror) and pitch, eased between the previous and current shots;
  pan and tilt trackers follow it (tilt only with shots that follow the board), and the frame rolls
  towards the shot's framing roll at `DutchInterpolationSpeed`, mirrored with the stance.
- **Field of view**: from the shot's lens (12 on every gameplay shot) and the TV's aspect, as Skate 3:
  `2 atan(35 / 2 lens)` over `sqrt(aspect + 1)` (about 73 degrees vertical on a 4:3 TV, 67 on a 16:9 one).
  The Wii's aspect comes from its settings (`fg_aspect`); the view's up carries the roll
  (`fg_begin_frame`).
- **Log**: `camera: <shot> (<graph state>)` when the shot changes, at most every quarter second;
  `camera graph: unsupported <name>` for graph names flatground doesn't know.
- Not done: the free-look stick, camera shake, Skate 3's slow motion on long airs, the high camera
  (`cameragraph_high`), off-board walking shots beyond the run-out and hippie jumps, special wipeout
  shots (no body tweaks), moving obstacles in the avoidance (flatground's parks are static), the
  avoidance's swept paths (straight rays), `TransitionTimeUnits` 2 uses the distance to the new shot's arm.
  Fat lines are rays with the radius taken off the hit distance.

## Measured against SK8

SK8's own simulation (skate-game's production frame: graphs, rigid board, rider, camera) runs headless
on this PC from a scratch harness outside the repository: the skate-game modules the frame needs are
compiled as a library with the renderer, audio, SDL and mod host left out, on the stock data its setup
extracts from the player's disc (only the "core" export: miscload, miscboot, db). The same scripted
pad drives SK8 and flatground on SK8's test course (flatground rebuilt it as a park pack, quad for
quad) and both log a row a tick. Scenarios: pushing (taps, held), coasting, carves at 3 to 8.5 m/s with
half and full stick, kick turns standing and rolling, ollies and kickflips (quick and wound up), held
and let-go 180s, manuals and the tail brake, the foot brake, powerslides, a drop-in into the half pipe,
a thin-rail run, and the camera while riding and carving.

Fixed (each pinned by a test to SK8's number; skater unit tests and `tests/feel_sk8.rs`):

- **Brake contact**: `BRAKE_CONTACT` is a value (0 in the brake's `INTO` clip, 1 in `CYC`); its presence
  was read as contact, so the foot dragged from the first tick. Now 0.23 s after the press, as SK8, at
  SK8's 4.08 m/s^2 (`brake_scale` calibrated; it was 3.0).
- **Speed model**: Skate 3 holds the speed to a target only the slope and rolling resistance move
  (`UpdateSpeedModel`); carving cost 1 to 2 m/s at 6 to 8 m/s here. Now the target pulls the speed back
  (`GeneralAmount` of the gap a tick, within `Friction_MaxSpeedChange`), off while a foot pushes or
  brakes and in a slide; the target restarts from the speed after anything outside the ride sets it.
- **Steering**: the tilt was missing the trucks' tightness factor (`SteeringScalar_TightTrucks`, SK8's
  default tightness 0.7). The board follows 0.88 of the trucks' curve, turns at most 1.5 + 0.04 x speed
  rad/s and builds its yaw over 0.05 s (ours, measured): full stick 1.8 rad/s at 6.5 to 8.5 m/s, half a
  stick within 7% of SK8 (it was 1.1 to 1.25 rad/s grip-capped at speed, and half a stick 40% sharp).
- **Kick turns**: SK8's rate against speed (3.2 rad/s standing, 2.0 at 3.4 m/s), building up and
  running down by 0.7 a tick when let go (was a fixed 2.25 rad/s, instant).
- **Powerslide**: the slide's side force now brings the travel round (about 1 g, after 0.25 s): SK8's
  slide turns the travel about 150 degrees; ours went straight on.
- **Let-go spin before a pop**: a turn let go just before take-off no longer spins the board on while
  our landing prediction resolves (SK8 -7 degrees, ours was 46).
- **Pushes**: while a push clip's foot is down the board chases the posed push's trajectory speed (the
  blended clips' root motion this tick, SK8 contact_events.rs `push_speed`), within the tick's change,
  of which the board keeps 0.96 as SK8's rigid board and rider do. Held: 3.24, 6.05, 8.04 m/s pushes and
  2.63 s to 8 m/s (SK8 3.22, 6.09, 8.07, 2.63; was 4.5 m/s and 1.67 s). Tapped pushes stay within 1%.
- **Ollies**: the jump pops the animated centre of mass (the rider's pose, Skate 3's part weights) from
  over the deck the tail strike has pitched up, and the flight starts from that lifted deck; in the air
  the deck hangs under the flying body by the animated COM-to-deck distance, so the tucked legs lift it.
  The body follows its animation at SK8's measured rate from the tail strike to the top (SK8's rider is
  physical). `ControlAirLegExtension` now drives `disttocog` (down to `min_inair_disttocog`, out again
  at `preland_disttocom_vel` near the landing), and the board meets the ground and rails where it is
  (grind targeting aims the deck, the flight lifted by the tuck it expects). Peak deck 0.53
  and 1.15 m, 0.52 and 0.73 s off the ground (SK8 0.50 and 1.16, 0.53 and 0.72; was 0.40 and 0.78, 0.57
  and 0.80); a held 180 turns 125 degrees (SK8 122, was 135).
- **Pumping**: the height the pump watches is the animated COM over the board; the ground's angle
  reports its compression and least crouch to the crouch (`CompressionVsGroundAngle` x
  `CompressionGroundScalar`, `MinCrouchVsGroundAngle`), with the board's absorption and the pumping as
  its potential; the pumping and compression tuning is the disc's.
- **Drop-ins**: Skate 3's drop-in depth windows (a tip 0.06 m, a 5-0 0.13 m from the deck's centre), and
  while a board rolled off an edge is still tipping over it, rails are ridden onto, not landed on: a slow
  roll off a half pipe's deck goes into the transition (it was a 0 m/s nose blunt or lipslide).
- **Crooked landings**: the wheels' friction on the touchdown takes 0.41 m/s of sideways speed per m/s
  into the ground (the deck's): landing 70 degrees round at 6 m/s keeps 1.9 m/s (SK8 0.75, was 4.1).
- **Carves**: the steering holds `PushScalarMin` through a push's whole animation (its contact events,
  SK8's processed bit 27), not only while the foot drives, and the board's yaw follows its trucks over
  0.05 s: half a stick builds in 0.33 to 0.43 s (SK8 0.35 to 0.43, was 0.17), full in 0.08 to 0.12 s
  (SK8 0.10 to 0.12), and let go it settles in 0.18 to 0.27 s (SK8 0.18 to 0.25, was 0.27 to 0.35).
- **Grinds**: on a comparable line (SK8's board put down 0.031 rad the other way, as it settles that
  much off) both ollie onto the thin rail as a 50-50 at 6.9 to 7.0 m/s, the same tick after the pop,
  and lose 0.9 to 1.0 m/s^2 along it.

Matching within about 5%: tapped and held pushes, manual pitch (14 and 28 degrees braking) and its speed
loss, the take-off tick of an ollie, the foot brake, kick turns, carve rates, the low camera's distance.

Left, as SK8's rigid board and rider do it and flatground approximates (sizes measured):

- **Transitions**: 2.5 m drop-in, bottom speed 6.2 m/s against 9.2 (7.0 before, after a 0.4 s stop on
  the coping and a fall from rest; that extra had been a spurious pump on the landing's first sample,
  removed). The pump itself matches
  (0.117 m/s a tick while it pays, as SK8), but SK8's board pivots over the coping on its back wheels
  and rolls the transition from the top, landing on it already compressed, so it springs back up the
  whole way down; ours rolls off level and falls 1.9 m before meeting it lower down. Porting that needs
  the board's pivot over an edge (its trucks as separate contacts).
- **Ollies**: wound-up flights are 0.02 s longer, the deck 0.01 to 0.05 m lower: SK8's rider body lags
  its animation through the pop (it is physical), ours follows it at a measured rate.
- **Crooked landings** keep more speed after the impact (1.9 against 0.75 m/s at 70 degrees): SK8's deck
  comes down at 6 m/s (its legs reach for the landing), ours at about 4, and its wheels keep scrubbing.
- **Tail brake**: matches at 6.5 m/s (2.9 m/s^2); under 4.1 m/s SK8's rises by 29% (a processed flag,
  bit 16, comes on there whose producer is outside SK8's ported attribute tables), so ours stops 0.4 s
  later.
- **Carve slip**: 3 degrees against SK8's 7 to 8 at full stick (its wheels' lateral slip).
- **Camera**: in a carve's first second ours trails the travel up to 13 degrees further (63 against 53
  degrees at 6.5 m/s); steady carves match within 2 degrees.
- **Coasting**: Skate 3's no-input friction is kept here (after 3 s); SK8 never sets the flag that runs
  its timer, so its board rolls on. Its rolling resistance on smooth concrete is 0 under 8 m/s, as ours.

## Numbers (Dolphin, US/EU disc)

| | |
|---|---|
| `motion/graph` in the pack | 612 KiB (action graph 234 states, 128 transitions, 876 operations; motion graph 2,088 states, 2,390 transitions, 3,247 expressions, 8,305 operations; 2,476 strings) |
| parsed in memory | 554 KB; MEM2 about 2.2 MB lower than before the graphs (pack entry, parsed tables, typed operations) |
| load | 12.7 ms in Dolphin (was 216.8 ms when the operations were parsed at load): the typed operations read back from `motion/graph/ops` (85 KiB), strings found by binary search over an index the compiler sorts (format version 2) |
| per tick | 0.13 ms on average, 0.27 at most over the trick tour (pad intents, both graphs, the turn conditioner and the riding behaviours, push coefficients, animation and channel advance), part of `physics` in the frame line (`graph`); frame CPU 3.5 ms average, 5.3 ms peak, no dropped frames |
| air step | the skater's tick in the air (landing prediction at up to 28 chord rays, spin, flight): 0.03 to 0.06 ms typically, 1.3 ms at most (Dolphin, three trick tours; frame CPU 5.9 ms peak, no dropped frames) |
| grinds | the grind tick (contact boxes, admission, the rail, naming) and riding past rails: no change to the frame line (physics 0.1 to 0.4 ms); frame CPU 5.9 ms peak over two trick tours with grinds, no dropped frames (Dolphin); host worst tick 0.2 ms |
| camera | the camera's tick (graph shot choice, shot tree, rig, six collision rays, avoidance paths, five drop probes): 0.14 ms on average, 0.54 ms at most over two trick tours on BlackBoxPark; frame CPU 3.5 ms average, 6.0 ms peak, physics 0.4 ms average, no dropped frames (Dolphin, 16:9) |
| operations | 8,314 run, 861 deliberate no-ops (audited above), 4 unsupported (skitching, `GestureType`) |
| phase 8 | body flips, gestures, pump layers, bumps and the pose operations: no change to the frame line (graph 0.13 to 0.16 ms, air step 0.03 to 0.08 ms around the back flip); frame CPU 3.6 ms average, 6.1 ms peak over 180 s (two trick tours on BlackBoxPark), no dropped frames; the gesture clips add 12 poses to the motion pack (20.1 MiB) |

## Trick tour (`autoplay`)

`tools/dolphin_test.ps1` plays `game_core::autoplay::TrickTour` over and over and the session log shows
each phase (`autoplay: <move>`), every motion-graph state change (`graph: <path>`) and each landing. On an
imported park the tour steers for clear, flat ground (probing the park ahead) and pops only lined up with
it. It does, with the state graphs: a kickflip, a carve, a manual held 3 s (`Manual/TailManual`), a kick
turn (`Riding/KickTurn`), an ollie with an indy grab, a revert (Skate 3's `SlideFs180` shape from the
player's gesture file, played on the main stick: `Riding/Revert/Fs`), a 360 flip, a 180 to fakie,
riding fakie until the graph turns the rider round (`Turning/Switch/NoPush`), a held double kickflip, an
ollie with a late kickflip (`InAir/Lateflip/LateKickflip`), a powerslide, a hippie jump and a grab held
and tweaked into the landing (`WipeOut`, then back on the board at a safe spot). The indy grab's C-stick
picks a variant, and the 180 lets the stick go early: the let-go spin and the landing assist finish it.
Then grinds: put beside the longest straight ledge with flat ground below it, an ollie onto it (the
landing prediction steers onto the ledge: a 50-50), the C-stick back tips it onto the back truck (a 5-0);
put on a quarter pipe's deck, rolling off its edge onto the coping (a coping 50-50 from above); put on a
ledge's line before its start, an ollie with a quarter turn into a slide (a boardslide). The log shows each
grind entered (`grind: BS 50-50 (BS_50_50, FiftyFifty, AirToGrind) on rail 83 (ledge) at 4.0 m/s`), the
graph's grind state (`graph: OnBoard/Grinding/BS_5_O/Cycle`), the score and why it ended (`grind: off
(leaned off)`). Then (phase 8) the d-pad up held a second (`CharacterGesture` plays the shaka), a thin
rail: the rider dropped onto the longest gently sloped thin rail at its high end (the demo park's flat
bar; on BlackBoxPark a handrail) grinds it to its end (`grind: off (off the end)`), and a back flip: on
the demo park pushed up the kicker's run-in, an ollie at its lip, the right trigger in and the left
stick's `BackFlip` shape straight after take-off, the trigger let go near a full turn (`BACKFLIP OLLIE
BSGRAB`); an imported park has no kicker the tour knows, so the rider is launched from where it is
(6.5 m/s up) and does the same. The tour no longer winds up an ollie mid-turn (the camera would keep
the arm the carve left it). Each tour ends with a summary line (`autoplay: tour N done (manual 3.2 s,
kick turn true, revert true, auto switch true, 2 flips held, late flip true, grab variant true, ledge
grind true, 5-0 true, coping true, slide true, gesture true, handrail true, back flip true)`), and the
next one starts from the park's spawn. The frame line shows the air step's cost (`air <last tick> / <peak> ms`) and the camera's (`camera <last tick> / <peak> ms`).
