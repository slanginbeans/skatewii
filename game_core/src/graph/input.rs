//! The pad as action-graph intents: what SK8-ENGINE/skate-3-rust-engine's input intentions make of a
//! controller (GPL-3.0; crates/skate-core/src/input: riding_intentions.rs, steering_intentions.rs,
//! anticipation_intentions.rs, manual_intentions.rs, trick_intentions.rs; crates/skate-game/src/input/
//! gesture_input.rs), on a GameCube pad. The thresholds here are flatground's own round numbers.
//!
//! Buttons map by position on the pad (Xbox A -> GC A, X -> B, B -> X, Y -> Y, RB -> Z, LT/RT -> L/R):
//! - A: push (right foot, `RightPush`), B: mongo push (`LeftPush`); both at once is the hippie jump:
//!   the graphs see both pushes and wind it up themselves (`OnBoard/HippyJump`), as in Skate 3.
//! - X: foot brake (`Brake`).
//! - Z: dark catch (`DarkCatch`) and world grab (`GrabWorld`). Z with the d-pad on the ground is the
//!   session marker and replay chord (`crate::marker::Chord`): the game hides it from here.
//! - L / R: grabs (`LeftAirGrab` / `RightAirGrab`; fully in: the ground grabs) and crouch. Off the board
//!   (Skate 3's LT / RT there), pressed fully in: drop the board (`OB_DropBoard`, L) or throw it down
//!   ahead (`OB_ThrowBoard`, R), either one `OB_RetrieveBoard` (picks it back up when it isn't in hand).
//! - Left stick: steering, kick turns, powerslides, body spin; its gestures (`skaterls`, `skaterstep`)
//!   give reverts (SlideFs180 / SlideBs180) and body flips.
//! - C-stick: wind-up (`AnticMag` / `AnticAngle`), manuals (`Manual`, `ManualBrake`), tweaks, board
//!   adjust, and every right-stick gesture set (`skater`, `skater90`, `skaterN90`, `skater_air`,
//!   `skater_fingerflip`), recognised in parallel at 60 Hz.

use alloc::vec::Vec;

use super::host::Intents;
use super::{NONE, Pack, Str};
use crate::gesture::{Recognizer, Shape};
use crate::input::{Pad, btn};
use crate::math::{PI, atan2f, sqrtf};

/// A press counts as new for this long (s).
const PRESS_MEMORY: f32 = 0.3;
/// Stick past this is "full" (wind-up instead of manual; slides; hard turns).
const FULL: f32 = 0.9;
/// Gesture sticks: components below this read as zero.
const COMPONENT_DEAD: f32 = 0.1;
/// Gesture recognizer miss limit (ticks off the shape before it is dropped).
pub const MISSES: u8 = 8;

/// The angle of a stick from straight back (pulled towards the player): 0 back, +-PI forward, +PI/2
/// right.
pub fn stick_angle(x: f32, y: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    atan2f(x, -y)
}

macro_rules! names {
    ($($f:ident = $s:literal),* $(,)?) => {
        /// Intent names, as graph string ids (`NONE` when the graph never mentions one).
        #[derive(Clone, Copy, Debug)]
        pub struct Ids { $(pub $f: Str,)* }
        impl Ids {
            pub fn new(p: &Pack) -> Ids {
                Ids { $($f: p.find($s).unwrap_or(NONE),)* }
            }
        }
    };
}

names! {
    brake = "Brake", new_push = "NewPush", right_push = "RightPush", left_push = "LeftPush",
    pushing = "Pushing", body_spin = "BodySpin", phys_body_spin = "PhysBodySpin",
    kick_turn = "KickTurn", hard_turn = "HardTurn", hard_turn_crouch = "HardTurnCrouch",
    crouch = "Crouch", turn = "Turn", right_slide_start = "RightSlideStart",
    left_slide_start = "LeftSlideStart", left_slide = "LeftSlide", right_slide = "RightSlide",
    grab_world = "GrabWorld", antic_mag = "AnticMag", antic_angle = "AnticAngle",
    manual = "Manual", manual_brake = "ManualBrake", left_ground_grab = "LeftGroundGrab",
    left_air_grab = "LeftAirGrab", right_ground_grab = "RightGroundGrab",
    right_air_grab = "RightAirGrab", dark_catch = "DarkCatch", new_dark_catch = "NewDarkCatch",
    board_adjust_angle = "BoardAdjustAngle", board_adjust_mag = "BoardAdjustMag",
    tweak_x = "TweakX", tweak_y = "TweakY", trick = "Trick", gesture_speed = "GestureSpeed",
    hold_pattern = "HoldPattern", no_input = "NoInput",
    gesture_left_start = "GestureLeftStart", gesture_right_start = "GestureRightStart",
    gesture_up_start = "GestureUpStart", gesture_down_start = "GestureDownStart",
    gesture_left_held = "GestureLeftHeld", gesture_right_held = "GestureRightHeld",
    gesture_up_held = "GestureUpHeld", gesture_down_held = "GestureDownHeld",
    toggle_off_board = "ToggleOffBoardState", new_toggle_off_board = "NewToggleOffBoardState",
    ob_jump = "OB_Jump", ob_sprint = "OB_Sprint", ob_mag = "OB_Mag", ob_steer = "OB_Steer",
    ob_world_x = "OB_BipedWorldX", ob_world_z = "OB_BipedWorldZ", ob_stick_mag = "OB_BipedStickMag",
    ob_look_x = "OB_LookAtX", ob_look_y = "OB_LookAtY",
    ob_drop = "OB_DropBoard", ob_throw = "OB_ThrowBoard", ob_retrieve = "OB_RetrieveBoard",
}

/// A trigger this far in is fully in (Skate 3 reads a trigger at 1.0: a ground grab riding, a drop or a
/// throw off the board).
pub const TRIGGER_FULL: f32 = 0.95;

/// Off the board, the triggers that went fully in this tick (L, R): SK8 offboard_intentions.rs drops on
/// LT and throws on RT as each reaches 1.0, and either asks to pick the board back up. `prev`: last
/// tick's fully-in state, updated.
pub fn board_triggers(pad: &Pad, prev: &mut [bool; 2]) -> [bool; 2] {
    let full = [pad.trig_l >= TRIGGER_FULL, pad.trig_r >= TRIGGER_FULL];
    let new = [full[0] && !prev[0], full[1] && !prev[1]];
    *prev = full;
    new
}

/// Gates from the game (SK8's actor flags): what the rider may do right now.
#[derive(Clone, Copy, Debug, Default)]
pub struct Gates {
    /// In the air: late flips are not allowed until the first trick is done.
    pub late_flips_blocked: bool,
    /// Finger flips and varials need a grab first.
    pub finger_flips_blocked: bool,
    /// Reverts (SlideFs180/SlideBs180) are not allowed (tricks disabled after landing).
    pub reverts_blocked: bool,
    /// Grinding: only the grind trick-out flicks.
    pub grinding: bool,
    /// A grab trigger is in, a push button down (body flips need the one and not the other).
    pub grabbing: bool,
    pub pushing: bool,
    /// Off the board: the walker's reading of the stick (`skater::biped::read_stick`): move amount,
    /// turn and the stick in the world.
    pub off_board: bool,
    pub ob_mag: f32,
    pub ob_steer: f32,
    pub ob_world: [f32; 2],
    /// Y as the physics takes it: getting off or on the board (held, and new this tick)
    /// (`Skater::board_toggle_intent`).
    pub toggle_board: bool,
    pub toggle_board_new: bool,
}

struct Set {
    /// 0 left stick, 1 right stick.
    stick: u8,
    rec: Recognizer,
    /// Graph string id of each shape's name.
    ids: Vec<Str>,
}

/// Turns the pad into action-graph intents each tick.
pub struct Intentions {
    sets: Vec<Set>,
    pub ids: Ids,
    prev_held: u32,
    /// Seconds since A and B went down (for remembered presses).
    since: [f32; 2],
    held_pattern: Str,
    idle: f32,
    /// The triggers fully in last tick ([`board_triggers`]).
    full: [bool; 2],
}

/// The names (gesture files) each stick reads, in recognition order.
pub const RIGHT_SETS: [&str; 5] = [
    "skater",
    "skater90",
    "skaterN90",
    "skater_air",
    "skater_fingerflip",
];
pub const LEFT_SETS: [&str; 2] = ["skaterls", "skaterstep"];

const HOLDABLE: [&str; 4] = ["Kickflip", "Heelflip", "N_Kickflip", "N_Heelflip"];

impl Intentions {
    /// `sets`: (file name, shapes) for every gesture file there is.
    pub fn new(p: &Pack, sets: Vec<(&str, Vec<Shape>)>) -> Intentions {
        let mut out = Vec::new();
        for (name, shapes) in sets {
            let stick = if LEFT_SETS.contains(&name) { 0 } else { 1 };
            let ids = shapes
                .iter()
                .map(|s| p.find(&s.name).unwrap_or(NONE))
                .collect();
            out.push(Set {
                stick,
                rec: Recognizer::new(shapes),
                ids,
            });
        }
        Intentions {
            sets: out,
            ids: Ids::new(p),
            prev_held: 0,
            since: [10.0; 2],
            held_pattern: NONE,
            idle: 0.0,
            full: [false; 2],
        }
    }

    pub fn reset(&mut self) {
        for s in &mut self.sets {
            s.rec.reset();
        }
        self.held_pattern = NONE;
    }

    /// Seconds since the last input.
    pub fn idle(&self) -> f32 {
        self.idle
    }

    /// This tick's intents into `ag` (cleared first). Returns the recognised gestures' ids.
    pub fn produce(
        &mut self,
        p: &Pack,
        pad: &Pad,
        gates: Gates,
        dt: f32,
        ag: &mut Intents,
        recognised: &mut Vec<(Str, f32)>,
    ) {
        let id = self.ids;
        ag.clear();
        recognised.clear();
        let mut put = |s: Str, v: f32| {
            if s != NONE {
                ag.set(s, v);
            }
        };
        let held = pad.held;
        let new = held & !self.prev_held;
        self.prev_held = held;
        for (k, b) in [btn::A, btn::B].into_iter().enumerate() {
            self.since[k] = if new & b != 0 {
                0.0
            } else {
                self.since[k] + dt
            };
        }
        // Pushes: A right foot, B mongo. Both together is the hippie jump: SK8's riding intentions pass
        // both on (they're dropped only while an actor flag blocks pushing), and the action graph takes
        // the pair as the hippie jump's wind-up.
        let (right, left) = (held & btn::A != 0, held & btn::B != 0);
        let new_right = right && self.since[0] < PRESS_MEMORY;
        let new_left = left && self.since[1] < PRESS_MEMORY;
        if held & btn::X != 0 {
            put(id.brake, 1.0);
        }
        // Off the board (SK8 offboard_intentions.rs, by position on the pad): Y gets off and on, B
        // (Skate 3's X) jumps, A without X (Skate 3's A without B) sprints.
        if gates.toggle_board {
            put(id.toggle_off_board, 1.0);
        }
        if gates.toggle_board_new {
            put(id.new_toggle_off_board, 1.0);
        }
        if new & btn::B != 0 {
            put(id.ob_jump, 1.0);
        }
        if held & btn::A != 0 && held & btn::X == 0 {
            put(id.ob_sprint, 1.0);
        }
        // L fully in drops the board, R throws it; either picks it back up (SK8: both publish
        // `OB_RetrieveBoard`; ToggleBoard decides by whether the board is in hand).
        let [drop, throw] = board_triggers(pad, &mut self.full);
        if gates.off_board {
            if drop {
                put(id.ob_drop, 1.0);
            }
            if throw {
                put(id.ob_throw, 1.0);
            }
            if drop || throw {
                put(id.ob_retrieve, 1.0);
            }
        }
        // The d-pad: character gestures (Skate 3's on the Xbox pad's d-pad, the same place).
        for (b, start, hold) in [
            (btn::DPAD_LEFT, id.gesture_left_start, id.gesture_left_held),
            (
                btn::DPAD_RIGHT,
                id.gesture_right_start,
                id.gesture_right_held,
            ),
            (btn::DPAD_UP, id.gesture_up_start, id.gesture_up_held),
            (btn::DPAD_DOWN, id.gesture_down_start, id.gesture_down_held),
        ] {
            if new & b != 0 {
                put(start, 1.0);
            }
            if held & b != 0 {
                put(hold, 1.0);
            }
        }
        if new_right || new_left {
            put(id.new_push, 1.0);
        }
        if right {
            put(id.right_push, 1.0);
            put(id.pushing, 1.0);
        }
        if left {
            put(id.left_push, 1.0);
            put(id.pushing, 1.0);
        }
        let [lx, ly] = pad.stick;
        {
            let len = sqrtf(lx * lx + ly * ly).min(1.0);
            let (world, mag, steer) = if gates.off_board {
                (gates.ob_world, gates.ob_mag, gates.ob_steer)
            } else {
                ([lx, ly], len, lx)
            };
            if world != [0.0, 0.0] {
                put(id.ob_world_x, world[0]);
                put(id.ob_world_z, world[1]);
                put(id.ob_stick_mag, len);
            }
            if mag != 0.0 {
                put(id.ob_mag, mag);
            }
            if steer != 0.0 {
                put(id.ob_steer, steer);
            }
        }
        if pad.flick != [0.0, 0.0] {
            put(id.ob_look_x, pad.flick[0]);
            put(id.ob_look_y, pad.flick[1]);
        }
        if lx != 0.0 {
            put(id.body_spin, lx);
            put(id.phys_body_spin, lx);
            put(id.kick_turn, lx);
        }
        // Steering (SK8 steering_intentions): turn by x, fading out as the stick goes forward past the
        // side; hard turns with the stick pulled back to the side.
        let angle = stick_angle(lx, ly);
        let abs = angle.abs();
        let len = sqrtf(lx * lx + ly * ly);
        let signed = if lx >= 0.0 { len } else { -len };
        let turn = if ly > 0.0 {
            if abs > 3.05 {
                0.0
            } else if abs > 2.35 {
                (3.05 - abs) * (1.0 / 0.7) * signed
            } else {
                signed
            }
        } else {
            lx
        };
        if len > FULL && (0.91..=1.39).contains(&abs) {
            let hard = ((abs - 1.39) * -2.08).max(0.001);
            let hard = if angle >= 0.0 { hard } else { -hard };
            put(id.hard_turn, hard);
            put(id.hard_turn_crouch, (hard * 0.85).abs());
        }
        let trig = pad.trig_l.max(pad.trig_r);
        if trig > 0.0 {
            put(id.crouch, trig);
        }
        if turn != 0.0 {
            put(id.turn, turn);
        }
        if len > FULL {
            const SLIDE_SCALE: f32 = 0.28;
            if angle > 0.0 && angle < 0.91 {
                put(id.right_slide_start, 1.0);
            }
            if angle > -0.91 && angle < 0.0 {
                put(id.left_slide_start, 1.0);
            }
            if angle > -2.0 && angle < PI / 2.0 {
                put(id.left_slide, -((angle + 2.0) * SLIDE_SCALE));
            }
            if angle > -PI / 2.0 && angle < 2.0 {
                put(id.right_slide, (2.0 - angle) * SLIDE_SCALE);
            }
        }
        if held & btn::Z != 0 {
            put(id.grab_world, 1.0);
            put(id.dark_catch, 1.0);
            if new & btn::Z != 0 {
                put(id.new_dark_catch, 1.0);
            }
        }
        // Right stick: wind-up, manual, tweaks.
        let [rx, ry] = pad.flick;
        let rlen = sqrtf(rx * rx + ry * ry);
        if rx != 0.0 || ry != 0.0 {
            put(id.antic_mag, rlen);
            put(id.antic_angle, stick_angle(rx, ry));
            put(id.board_adjust_angle, atan2f(rx, -ry));
            put(id.board_adjust_mag, rlen);
        }
        if ry != 0.0 {
            put(id.manual, if ry > 0.0 { rlen } else { -rlen });
            put(id.tweak_y, ry);
        }
        if rx != 0.0 {
            put(id.tweak_x, rx);
        }
        if rlen > FULL {
            let b = (rlen - FULL) * 10.0;
            put(id.manual_brake, if ry > 0.0 { b } else { -b });
        }
        // Grabs: any pull is an air grab, fully in a ground grab.
        for (t, air, ground) in [
            (pad.trig_l, id.left_air_grab, id.left_ground_grab),
            (pad.trig_r, id.right_air_grab, id.right_ground_grab),
        ] {
            if t > 0.0 {
                put(air, 1.0);
            }
            if t >= TRIGGER_FULL {
                put(ground, 1.0);
            }
        }
        let any = held != 0 || len > 0.0 || rlen > 0.0 || trig > 0.0;
        self.idle = if any { 0.0 } else { self.idle + dt };
        if !any {
            put(id.no_input, 1.0);
        }

        // Gestures: every set on its stick, in parallel.
        let samples = [pad.stick, pad.flick]
            .map(|[x, y]| [x, -y].map(|v| if v.abs() < COMPONENT_DEAD { 0.0 } else { v }));
        let mut still_held = false;
        for s in &mut self.sets {
            let sample = samples[s.stick as usize];
            if let Some(i) = s.rec.held(sample)
                && self.held_pattern != NONE
                && s.ids[i] == self.held_pattern
            {
                still_held = true;
            }
            if let Some(r) = s.rec.sample(sample, MISSES) {
                let gid = s.ids[r.shape];
                if gid == NONE {
                    continue;
                }
                if HOLDABLE.contains(&s.rec.shapes()[r.shape].name.as_str()) {
                    self.held_pattern = gid;
                }
                recognised.push((gid, r.strength));
            }
        }
        if !still_held && recognised.is_empty() {
            self.held_pattern = NONE;
        }
        if still_held {
            put(id.hold_pattern, 1.0);
        }
        let gates = Gates {
            grabbing: gates.grabbing || trig > 0.0,
            pushing: gates.pushing || right || left,
            ..gates
        };
        for &(gid, strength) in recognised.iter() {
            if permitted(p.str(gid), gates) {
                put(id.trick, 1.0);
                put(gid, 1.0);
                put(id.gesture_speed, strength);
            }
        }
    }
}

/// SK8 gesture_input::permitted: which recognised gestures count right now.
pub fn permitted(name: &str, g: Gates) -> bool {
    if g.finger_flips_blocked && matches!(name, "FingerFlip" | "FS_Varial" | "BS_Varial") {
        return false;
    }
    if g.late_flips_blocked
        && matches!(
            name,
            "L_F_Kickflip"
                | "L_B_Kickflip"
                | "L_F_Heelflip"
                | "L_B_Heelflip"
                | "L_FS_Shuvit"
                | "L_BS_Shuvit"
        )
    {
        return false;
    }
    if g.reverts_blocked && matches!(name, "SlideFs180" | "SlideBs180") {
        return false;
    }
    // Body flips only while grabbing, not pushing.
    if matches!(name, "FrontFlip" | "BackFlip") && (!g.grabbing || g.pushing) {
        return false;
    }
    true
}
