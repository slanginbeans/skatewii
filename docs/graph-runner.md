# The state-graph runner

Skate 3 decides what the rider is doing (riding, pushing, winding up, which trick, manual, landing,
grinding ...) and what animation plays with two state graphs from its disc: `ActionGraph_OnBoard`
(turns controller intents into motion intents) and `MotionGraph_OnBoard` (states, animation trees,
values for the physics). flatground runs the player's own copies of them.

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
     out) and never tip over: a held stick holds a manual as long as it is held; the deck pitch servoes to
     (0.25 + 0.75 lean) of the most tilt, or to the brake angle (`PhysicsWantsManualExit` stays false:
     the servo has no opposing motion to report);
   - in the air the board does what the clips do: a late flip, finger flip, under flip or dark catch
     becomes the trick (its look and flips from the graph's `ScoringTrick`, `Kickflip2`..`4` for held
     multi-flips), a dark catch rides dark (`SetDark`, the board grip down), and a landing bails only
     while the playing clips are in their `DANGERZONE` (the board not caught; within its last 0.1 s
     counts as caught) instead of after a fixed trick time. The imported rider's board is a skinned
     part on the clips' `SKATEBOARD_ROOT`, so it flips exactly as the clip does; the built-in board
     follows the graph's trick and its progress;
   - kick turns turn the board while `KickTurnSteering` runs (at its spin scale; the action graph gates
     them on the stick, the speed (lower riding fakie) and `PhysicsSpeedAndSlopeCompare`, which is
     Skate 3's heading adjust factor from the turn torque curves, SK8 speed_and_slope.rs), reverts start on
     entering `Revert/Fs|Bs`, and entering `Switch` or `PushFromFakie` turns the rider round (fakie to
     switch);
   - powerslides stay in the physics (flatground's Skate 3 rules); `CanEnterSlide` and
     `ShouldLeaveSlide` read them;
   - the physics' grinds reach `IsGrinding` by Skate 3's names (`FS_`/`BS_` + `50_50`, `5_O`,
     `NOSEGRIND`, `CROOK`, `WILLY`, `FEEBLE`, `SMITH`, `BOARD`, `LIP`, `NOSESLIDE`, `TAILSLIDE`).
4. The rider (`Rider::update_graph`) runs the motion graph's pose program (`playback::PoseOp`): each
   tree's clips blended as one pose, sampled and mirrored for the stance the tree started in
   (`anim::mirror_pose`, mode 1) before blending so stance changes crossfade, then the transitions'
   blends. A `channelblend` transition (29 in the on-board graph, 28 of them `useChannelFromWeights`)
   blends each bone by the weight times its channel weight (SK8 ACSChannelBlend; from the older pose
   with `useChannelFromWeights`, else the newer): clips that are channel animations (525 of the
   bank's, kept by the importer as a weight per bone) take only their bones; other clips weigh 1
   everywhere. Persistent channels are blended over the result the same way. Clips that don't fit the
   skeleton fall back to the clip roles.

Channels the on-board graph uses: `channelblend` transitions (landings into riding, pushes, slides,
grind exits, anticipation); `FakieHeadChannel`, the one persistent channel (`fakie`, tree
`B_FAKIE_CHANNEL`: fades in over 0.3 s when the rider starts riding fakie, its `torso` parameter
snapped on entry to 1 in a manual, 0 in a powerslide, else 0.5, then eased at 0.01 a tick; fades out
when riding fakie ends). Not run: `CharacterGesture` (`GestureBoth` / `GestureRight` / `GestureLeft`,
the d-pad taunts; their clips come from a gesture catalogue flatground doesn't read) and the shove
channel (`Shove` / `MaintainShove`, shoving people).

The graphs start over when the physics puts the rider back on the board after a wipeout or runout.
Without the entry everything above is flatground's hand-written logic, unchanged.

## Controls (GameCube pad)

Buttons map by position on the pad: Xbox A to GC A, X to B, B to X, Y to Y, RB to Z, LT/RT to L/R.

| GC | Intents | Does |
|---|---|---|
| A | `RightPush`, `Pushing`, `NewPush` | push (a press is remembered for 0.3 s) |
| B | `LeftPush`, `Pushing`, `NewPush` | mongo push |
| A and B | neither push | hippie jump (flatground's physics) |
| X | `Brake` | foot brake |
| Z | `DarkCatch`, `NewDarkCatch`, `GrabWorld` | dark catch. SK8 also reads Xbox B for it; on the GameCube that is X, the foot brake, so only Z |
| L / R | `LeftAirGrab` / `RightAirGrab`, fully in `Left/RightGroundGrab`, `Crouch` | grabs, crouch |
| main stick | `Turn`, `HardTurn`, `HardTurnCrouch`, `KickTurn`, `BodySpin`, `Left/RightSlide`, `Left/RightSlideStart` | steering, kick turns, powerslides, spins |
| C-stick | `AnticMag`, `AnticAngle`, `Manual`, `ManualBrake`, `TweakX/Y`, `BoardAdjustAngle/Mag` | wind-up (past 0.9), manuals (held below full for 0.2 s), tweaks |
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
`DisableTricks`; the action graph's `CreateMGIntentFromAGIntent` (filters, defaults, scale),
`CreateConstMGIntent`, `CreateMGTimeIntentFromAGIntent`, `CreateTrickIntentFromGesture` (groups, mirror,
dark catch, underflip, `*Hold` successors for multi-flips); the `OverideNextAnimTransitionHook` and
`GrabSlide` hooks; and the conditions on intents, clip attributes, clip expiry and time windows, state
times, last and current state, physics state, grind name, stance, speed, slope, time to land, gestures, grab
type, tweak, manual timer, wipeout and runout, dark catch and underflip requests.

No-ops on purpose (1,005 operations): hand and arm services, scoring augmentations, body tilt and crouch
rigs, leg extension, off-board walking, hand plants, moving objects, shoves; conditions of systems
flatground doesn't have take a fixed value (skitching, water, hand plants, holding the board ...).

Unsupported, logged once when reached: `EnterSkitchingBehaviour`, `SkitchingBehaviour`,
`SkitchShimmyingBehaviour`, `GestureType` (wipeout gestures).

Approximations to revisit: the push speed is where the push coefficients' blended clips end, not the
trajectory bone's speed sampled from the posed clips; how long the push foot stays down still comes
from the push sets' contact time; the manual servo is a kinematic stand-in for SK8's PID on the rigid
board (so `PhysicsWantsManualExit` never fires); a landing in the danger zone bails outright where
Skate 3 makes its physical wipeout checks more sensitive; character gestures and shoves don't run.

## Numbers (Dolphin, US/EU disc)

| | |
|---|---|
| `motion/graph` in the pack | 612 KiB (action graph 234 states, 128 transitions, 876 operations; motion graph 2,088 states, 2,390 transitions, 3,247 expressions, 8,305 operations; 2,476 strings) |
| parsed in memory | 554 KB; MEM2 about 2.2 MB lower than before the graphs (pack entry, parsed tables, typed operations) |
| load | 12.7 ms in Dolphin (was 216.8 ms when the operations were parsed at load): the typed operations read back from `motion/graph/ops` (85 KiB), strings found by binary search over an index the compiler sorts (format version 2) |
| per tick | 0.10 ms on average, 0.15 at most over the trick tour (pad intents, both graphs, push coefficients, animation and channel advance), part of `physics` in the frame line (`graph`); frame CPU 3.3 ms average, 4.5 ms peak, no dropped frames |
| operations | 8,169 run, 1,006 deliberate no-ops (systems flatground doesn't have), 4 unsupported (skitching, `GestureType`) |

## Trick tour (`autoplay`)

`tools/dolphin_test.ps1` plays `game_core::autoplay::TrickTour` over and over and the session log shows
each phase (`autoplay: <move>`), every motion-graph state change (`graph: <path>`) and each landing. On an
imported park the tour steers for clear, flat ground (probing the park ahead) and pops only lined up with
it. It does, with the state graphs: a kickflip, a carve, a manual held 3 s (`Manual/TailManual`), a kick
turn (`Riding/KickTurn`), an ollie with an indy grab, a revert (Skate 3's `SlideFs180` shape from the
player's gesture file, played on the main stick: `Riding/Revert/Fs`), a 360 flip, a 180 to fakie,
riding fakie until the graph turns the rider round (`Turning/Switch/NoPush`), a held double kickflip, an
ollie with a late kickflip (`InAir/Lateflip/LateKickflip`), a powerslide, a hippie jump and a grab held
into the landing (`WipeOut`, then back on the board). Each tour ends with a summary line
(`autoplay: tour N done (manual 3.2 s, kick turn true, revert true, auto switch true, 2 flips held, late
flip true)`).
