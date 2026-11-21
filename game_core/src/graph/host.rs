//! Runs the action graph and the motion graph each tick: the game side of every condition, behaviour and
//! hook ([`ops`](super::ops)), the intents passed between the graphs, and the animation the motion graph
//! plays ([`playback`](super::playback)).
//!
//! Order per tick (as SK8-ENGINE/skate-3-rust-engine runs Skate 3's; GPL-3.0, crates/skate-game/src/
//! graph_host action.rs and motion.rs): the pad's intentions become action-graph intents
//! ([`input`](super::input)); the action graph updates and its behaviours write motion-graph intents; the
//! motion graph updates (conditions read both intent sets, the physics state in [`World`] and the playing
//! clips' attributes; behaviours play trees, set parameters and emit packet values for the physics); the
//! animation advances. The physics then reads [`Runner::packet`], [`Runner::flag`] and the clips'
//! attributes and events ([`Runner::attribute`]).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::ops::{self, Beh, Cond, Flag, Group, Hook, Op, PushOp, phys};
use super::playback::{ChannelSettings, Player, PoseProgram, Tree};
use super::push::{self, PushCurves, PushSets, PushState};
use super::riding::{
    BodyTilt, Conditioner, Crouch, CrouchIntents, CrouchPhysical, GrindFade, LandingStore,
    ManualAngle, TurnContext, TurnInput, TurnOutput, Turning,
};
use super::{Controller, Frame, Graph, NONE, OpId, Pack, Str};
use crate::motion::tree::{Attr, Playback, PoseCommand};
use crate::motion::{KIND_CURVE, KIND_SCALAR, MotionDb, NameId};

/// A set of named values (intents), by graph string id. Absent is NaN.
#[derive(Clone, Debug)]
pub struct Intents {
    values: Vec<f32>,
    live: Vec<Str>,
}

impl Intents {
    pub fn new(strings: usize) -> Intents {
        Intents {
            values: vec![f32::NAN; strings],
            live: Vec::new(),
        }
    }
    pub fn get(&self, s: Str) -> Option<f32> {
        self.values.get(s as usize).copied().filter(|v| !v.is_nan())
    }
    pub fn has(&self, s: Str) -> bool {
        self.get(s).is_some()
    }
    pub fn set(&mut self, s: Str, v: f32) {
        if let Some(slot) = self.values.get_mut(s as usize) {
            if slot.is_nan() {
                self.live.push(s);
            }
            *slot = if v.is_nan() { 0.0 } else { v };
        }
    }
    pub fn remove(&mut self, s: Str) {
        if let Some(slot) = self.values.get_mut(s as usize)
            && !slot.is_nan()
        {
            *slot = f32::NAN;
            self.live.retain(|&x| x != s);
        }
    }
    pub fn clear(&mut self) {
        for &s in &self.live {
            self.values[s as usize] = f32::NAN;
        }
        self.live.clear();
    }
    pub fn live(&self) -> &[Str] {
        &self.live
    }
}

/// What the physics and the pad tell the graphs each tick.
#[derive(Clone, Copy, Debug)]
pub struct World {
    /// [`phys`] category.
    pub phys: u8,
    /// Speed (m/s) and speed along the board (negative: rolling tail first).
    pub speed: f32,
    pub forward_speed: f32,
    /// The downhill turn torque share at this speed and slope (`drive::speed_and_slope`).
    pub speed_and_slope: f32,
    /// The graph's name of the grind being done (string id), or `NONE`.
    pub grind: Str,
    /// The rider's poses are mirrored (regular stance, or goofy riding switch).
    pub mirrored: bool,
    /// The board's frame has been turned end for end (`Skater::frame_turned`): trees remember it.
    pub frame_turned: bool,
    pub fakie: bool,
    pub goofy: bool,
    pub switch: bool,
    pub time_since_input: f32,
    /// Predicted seconds until landing (in the air).
    pub time_to_land: Option<f32>,
    pub apex: bool,
    pub wants_manual_exit: bool,
    pub wants_runout: bool,
    pub wants_wipeout: bool,
    /// Slope of the ground under the board (degrees) and along the board (degrees, + uphill).
    pub surface_slope: f32,
    pub skate_slope: f32,
    pub push_brake_disabled: bool,
    /// 0 straight, 1 sketchy, 2 spin (for the landing just made).
    pub landing_type: u8,
    pub can_land: bool,
    /// Centre-of-mass velocity in the board's frame (x right, y up, z forward).
    pub com_vel: [f32; 3],
    /// The trucks or the deck touch something (grinding); wheels alone don't count.
    pub trucks_or_deck: bool,
    pub dist_to_edge: f32,
    /// Grind trick-out side allowed: 0 normal, 1 left, 2 right.
    pub grind_out: u8,
    pub blunting: bool,
    /// Grinding facing backwards (SK8 CreateGrindAttributes' `GrindFacingBackwards`).
    pub grind_backwards: bool,
    pub on_stairs: bool,
    /// The physics' powerslide: 0 none, 1 to the left, 2 to the right; and whether it is ending.
    pub slide: u8,
    pub slide_ending: bool,
    /// What the riding animation reads (`graph::riding`): the processed turn input (-1..1), the lean
    /// into the turn the physics aims for (rad), speed along the ground (m/s), world up along the
    /// board's right side, the body's spin speed (rad/s), vertical speed and speed along the surface's
    /// up (m/s), pumping potential (0..1), and the board's yaw rate at the landing (rad/s).
    pub turn: f32,
    pub lean: f32,
    pub ground_speed: f32,
    pub lateral_up: f32,
    pub body_spin: f32,
    pub vel_y: f32,
    pub vel_up: f32,
    pub pump_potential: f32,
    pub landing_spin: f32,
    /// The physics is turning a body flip (`IsBodyFlipping`).
    pub body_flipping: bool,
    /// The physics' pumping acceleration (Pumping).
    pub pump_acc: f32,
    /// The pumping's feedback to the crouch (SK8 PhysOutAnimation crouching: absorption, ground
    /// compression, least crouch, deck compression).
    pub crouch_force: f32,
    pub crouch_ground_force: f32,
    pub crouch_min: f32,
    pub crouch_deck: f32,
    /// The rider's centre of mass over the board as the physics has it (SK8 `animation_height`),
    /// when it knows (the imported rider): what `SetDistComToBoard` and the crouch read.
    pub com_height: Option<f32>,
    /// The deck's travel along its nose (-1 backwards .. 1 forwards; PowerSlideSpin).
    pub slide_dir: f32,
    /// The board's acceleration along the ground in its frame (x right, z forward; m/s^2), with the
    /// deck's and the ground's up taken out (SK8 ground_acceleration.rs).
    pub ground_acc: [f32; 2],
    /// The body's lean (about the travel) and twist (about up) while wiping out (rad).
    pub hips: [f32; 2],
    /// The main stick while wiping out (the wipeout gesture), none otherwise.
    pub wipe_stick: Option<[f32; 2]>,
    /// The pushing feet against the deck (along its nose, up; m): left, right (PushOut).
    pub feet: [[f32; 2]; 2],
}

impl Default for World {
    fn default() -> Self {
        World {
            phys: phys::GROUND,
            speed: 0.0,
            forward_speed: 0.0,
            speed_and_slope: 0.0,
            grind: NONE,
            mirrored: false,
            frame_turned: false,
            fakie: false,
            goofy: true,
            switch: false,
            time_since_input: 0.0,
            time_to_land: None,
            apex: false,
            wants_manual_exit: false,
            wants_runout: false,
            wants_wipeout: false,
            surface_slope: 0.0,
            skate_slope: 0.0,
            push_brake_disabled: false,
            landing_type: 0,
            can_land: true,
            com_vel: [0.0; 3],
            trucks_or_deck: false,
            dist_to_edge: 10.0,
            grind_out: 0,
            blunting: false,
            grind_backwards: false,
            on_stairs: false,
            slide: 0,
            slide_ending: false,
            turn: 0.0,
            lean: 0.0,
            ground_speed: 0.0,
            lateral_up: 0.0,
            body_spin: 0.0,
            vel_y: 0.0,
            vel_up: 0.0,
            pump_potential: 0.0,
            landing_spin: 0.0,
            body_flipping: false,
            pump_acc: 0.0,
            crouch_force: 0.0,
            crouch_ground_force: 0.0,
            crouch_min: 0.0,
            crouch_deck: 0.0,
            com_height: None,
            slide_dir: 1.0,
            ground_acc: [0.0; 2],
            hips: [0.0; 2],
            wipe_stick: None,
            feet: [[0.0; 2]; 2],
        }
    }
}

/// Per-behaviour instance state.
#[derive(Clone, Copy, Debug, Default)]
struct Inst {
    t: f32,
    v: f32,
    d: f32,
    n: i32,
    on: bool,
    s: Str,
    s2: Str,
}

/// Per-behaviour state of the riding behaviours (`graph::riding`), kept only for the few operations
/// that have one.
#[derive(Clone, Copy, Debug)]
enum RidingInst {
    Turning(Turning),
    Tilt(BodyTilt),
    Crouch(Crouch),
    Manual(ManualAngle),
    Value(f32),
    Spin(SpinChannels),
    Fade(GrindFade),
    AirLeg(AirLeg),
}

/// ControlAirLegExtension's state (SK8 air_leg_extension.rs): its mode (0 to begin, 1 going up, 2
/// coming down, 3 reaching for the landing), the centre of mass over the board it drives, and the
/// seconds coming down.
#[derive(Clone, Copy, Debug, Default)]
pub struct AirLeg {
    mode: u8,
    height: f32,
    down_time: f32,
}

/// BodySpin's state (SK8 motion_spin.rs): its mode (0 idle, 1 armed, 2 spinning on the ground or
/// starting, 3 spinning in the air, 4 off: hands busy or grinding) and the two channels' influences and
/// their rates.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpinChannels {
    mode: u8,
    front: f32,
    front_d: f32,
    back: f32,
    back_d: f32,
}

/// The body spin channels' clips, and their names (ids no graph string has).
pub const SPIN_FRONT: &str = "IA_BODYSPIN_OLLIE_FS_0_N";
pub const SPIN_BACK: &str = "IA_BODYSPIN_OLLIE_BS_0_N";
pub const CHANNEL_SPIN_FRONT: u16 = NONE - 2;
pub const CHANNEL_SPIN_BACK: u16 = NONE - 3;
/// The spin intent past which a spin channel starts (SK8's 0.3), and the time to land under which none
/// starts (flatground's stand-in for SK8's prelanding query).
const SPIN_START: f32 = 0.3;
const SPIN_NEAR_LANDING: f32 = 0.25;

/// Names the runner itself reads or writes.
#[derive(Clone, Copy, Debug)]
pub struct Names {
    pub gesture_speed: Str,
    pub trick_height: Str,
    pub dark_catch: Str,
    pub underflip: Str,
    pub dont_mirror: Str,
    pub jump_override: Str,
    pub is_dark: Str,
    pub tweak_x: Str,
    pub tweak_y: Str,
    pub phys_body_spin: Str,
    pub pushing: Str,
    pub new_push: Str,
    /// The push trees' parameters (motion-library names): `HSTR_VEL_B`, `LSTR_VEL_B`, `VEL_E`.
    push_params: [Option<NameId>; 3],
    /// The fakie head and torso channel's tree and its `torso` parameter.
    fakie_tree: Option<crate::motion::Node>,
    torso: Option<NameId>,
    /// Motion-library names: the construction keys `Trick` and `Random`, `manualInto`, `defaultcyc`.
    trick_key: Option<NameId>,
    random_key: Option<NameId>,
    manual_into: Option<NameId>,
    default_cyc: Option<NameId>,
    /// Riding: the playing clips' centre-of-mass height (`DISTTOCOG`), the landing trees' `Spin`,
    /// `AvgVelY` and `disttocog`, the manual trees' `manual_angle`.
    disttocog: Option<NameId>,
    spin: Option<NameId>,
    avg_vel_y: Option<NameId>,
    manual_angle: Option<NameId>,
    /// Graph strings: the intents the riding behaviours read and the packet values they emit.
    fakie_turn: Str,
    left_slide: Str,
    right_slide: Str,
    crouch: Str,
    hard_turn_crouch: Str,
    manual: Str,
    auto_pump_angle: Str,
    auto_pump_mag: Str,
    turn_packet: Str,
    slide_packet: Str,
    new_auto_pump: Str,
    player_pump: Str,
    /// BodyFlippingSignal's action intents and the motion intents it makes of them.
    front_flip: Str,
    back_flip: Str,
    body_spin: Str,
    /// CreateGrindAttributes' packet values.
    grind_backwards: Str,
    grind_forwards: Str,
    spin_front: Option<crate::motion::Node>,
    /// The pump layer's tree (`B_PUMP`) and PushOut's parameters (`OUTDISTANCEX`, `OUTDISTANCEY`).
    pump_tree: Option<crate::motion::Node>,
    pub out_distance: [Option<NameId>; 2],
    spin_back: Option<crate::motion::Node>,
}

/// Mutable state both graphs share.
pub struct Shared {
    pub ag: Intents,
    pub mg: Intents,
    pub filtered: Intents,
    pub world: World,
    pub player: Player,
    /// Packet values this tick (CreateAttribute, AttachIntent, SetTrickHeight ...), latest per name.
    pub packet: Vec<(Str, f32)>,
    flags: [u8; 7],
    pub dark: bool,
    pub manual_out_timer: f32,
    pub tricks_disabled: f32,
    pub time_since_kickturn: f32,
    pub grab: Str,
    /// The grab's scored name this tick (ScoringGrabs), or `NONE`.
    pub grab_score: Str,
    /// The trick being scored (ScoringTrick), or `NONE`.
    pub trick: Str,
    /// Tree construction values: (key, value) as motion-library names (`Trick`, `Random`, ...).
    construction: Vec<(NameId, NameId)>,
    pub underflip_requested: bool,
    pub dark_catch_requested: bool,
    pub kickturn: Option<f32>,
    pub slide_request: Option<bool>,
    pub slide_grabbed: [bool; 2],
    override_transition: Option<ops::TransitionDef>,
    rng: u32,
    /// Unsupported operations reached (string ids), each reported once.
    pub unsupported: Vec<Str>,
    pub new_unsupported: Vec<Str>,
    /// Sources the motion graph reads from the previous animation (`lastAnim`, SetTrickHeight `from`).
    last_sources: Vec<NameId>,
    pub push_count: u32,
    /// The push behaviours' shared state, the push sets (regular foot, mongo) and the curves.
    pub push: PushState,
    pub push_sets: [PushSets; 2],
    pub push_curves: PushCurves,
    /// The speed the playing push coefficients end at (m/s): what the board chases in push contact.
    pub push_speed: f32,
    /// Seconds since the rider was (re)placed on the board.
    pub time_since_teleport: f32,
    /// The physics' animation turn conditioner and this tick's output (SetTurning reads it).
    pub conditioner: Conditioner,
    pub turn_out: TurnOutput,
    /// ApplyingBodyTilt is running (SettingBodyTilt sets `TILT_X` only then).
    pub applying_tilt: bool,
    /// StoreLandingData's landing speed.
    pub landing: LandingStore,
    /// The rider's centre of mass over the board (m): the playing clips' `DISTTOCOG`.
    pub com_height: f32,
    /// BodyFlippingSignal's windows (s): since the flip gesture, since take-off; and the flip chosen.
    pub flip_signal: (f32, f32, Option<usize>),
    /// EnableWipeoutGestures is on, and the wipeout gesture (x, y) the stick has steered.
    pub wipe_gestures: bool,
    pub wipe_gesture: [f32; 2],
    /// The d-pad gesture playing (CharacterGesture): direction (0 left, 1 right, 2 up, 3 down) and
    /// stage (0 into, 1 cycle, 2 out).
    pub gesture: Option<(u8, u8)>,
}

impl Shared {
    fn emit(&mut self, s: Str, v: f32) {
        if s == NONE {
            return;
        }
        match self.packet.iter_mut().find(|p| p.0 == s) {
            Some(p) => p.1 = v,
            None => self.packet.push((s, v)),
        }
    }
    fn random(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.rng >> 8
    }
    fn unsupported(&mut self, s: Str) {
        if !self.unsupported.contains(&s) {
            self.unsupported.push(s);
            self.new_unsupported.push(s);
        }
    }
    fn set_construction(&mut self, key: Option<NameId>, value: Option<NameId>) {
        let Some(key) = key else { return };
        self.construction.retain(|c| c.0 != key);
        if let Some(v) = value {
            self.construction.push((key, v));
        }
    }
}

fn flag_index(f: Flag) -> usize {
    match f {
        Flag::Trick => 0,
        Flag::Manual => 1,
        Flag::PowerSlide => 2,
        Flag::Landing => 3,
        Flag::Anticipating => 4,
        Flag::SlideCandidate => 5,
        Flag::NoPumping => 6,
    }
}

/// Gesture names each group accepts: the plain ones, plus `90_` / `N90_` versions for the 90 groups.
const TAIL_KEYS: [&str; 15] = [
    "Ollie",
    "PopShuvit",
    "FSPopShuvit",
    "VarialKickflip",
    "VarialHeelflip",
    "Hardflip",
    "InwardHeelflip",
    "360PopShuvit",
    "FS360PopShuvit",
    "360Flip",
    "Laserflip",
    "360Hardflip",
    "360InwardHeelflip",
    "Kickflip",
    "Heelflip",
];

/// The hash bucket Skate 3's gesture-to-trick table keeps a gesture in (SK8 gesture_mapping.rs
/// `select`): the sum of the name's encoded words (`intent_key`: the engine's five-word text encoding of
/// the first 30 characters, plus the first word of the rest) modulo 43. When several gestures are
/// recognised on the same tick, the one in the lowest bucket becomes the trick.
pub fn gesture_bucket(name: &str) -> u32 {
    fn encode(text: &[u8]) -> [u32; 5] {
        let mut words = [0u32; 5];
        let mut at = 0;
        while at < 30 && at < text.len() && text[at] != 0 {
            let mut weight = 79_235_168u32; // 38^5
            let word = at / 6;
            let mut n = 0;
            while n < 6 && at < text.len() && text[at] != 0 {
                let byte = text[at] as i8 as i32;
                let digit = if byte >= 97 {
                    byte - 86
                } else if byte > 90 {
                    byte - 58
                } else if byte > 57 {
                    byte - 54
                } else {
                    byte - 47
                };
                words[word] = words[word].wrapping_add((digit as u32).wrapping_mul(weight));
                weight /= 38;
                n += 1;
                at += 1;
            }
        }
        words
    }
    let b = name.as_bytes();
    let tail = if b.len() > 30 && !b[..30].contains(&0) {
        encode(&b[30..])[0]
    } else {
        0
    };
    encode(b).iter().fold(tail, |sum, w| sum.wrapping_add(*w)) % 43
}

/// The other-foot version of a trick (a mirrored rider's flick does the opposite trick).
pub fn mirror_trick(t: &str) -> String {
    let (prefix, base) = match t.strip_prefix("N_") {
        Some(b) => ("N_", b),
        None => ("", t),
    };
    let m = match base {
        "PopShuvit" => "FSPopShuvit",
        "FSPopShuvit" => "PopShuvit",
        "360PopShuvit" => "FS360PopShuvit",
        "FS360PopShuvit" => "360PopShuvit",
        "VarialKickflip" => "VarialHeelflip",
        "VarialHeelflip" => "VarialKickflip",
        "Hardflip" => "InwardHeelflip",
        "InwardHeelflip" => "Hardflip",
        "360Flip" => "Laserflip",
        "Laserflip" => "360Flip",
        "360Hardflip" => "360InwardHeelflip",
        "360InwardHeelflip" => "360Hardflip",
        "Kickflip" => "Heelflip",
        "Heelflip" => "Kickflip",
        b => b,
    };
    let mut s = String::from(prefix);
    s.push_str(m);
    s
}

/// The trick a gesture gives in a group (before mirroring), and its dark-catch side
/// (0 none, 1 tail, 2 tail heel side, 3 nose, 4 nose heel side). `None`: the group doesn't take it.
///
/// Square takes every flick as it is. Nose groups keep nose flicks; a tail flick becomes a nollie (or the
/// nose shove-it in its direction). Tail groups do the same the other way round. The 90 groups also
/// take their `90_` / `N90_` flicks.
pub fn group_trick(group: Group, gesture: &str) -> Option<(String, u8)> {
    let (base, prefixed) = match group {
        Group::Nose90 | Group::Tail90 => match gesture.strip_prefix("90_") {
            Some(b) => (b, true),
            None => (gesture, false),
        },
        Group::NoseN90 | Group::TailN90 => match gesture.strip_prefix("N90_") {
            Some(b) => (b, true),
            None => (gesture, false),
        },
        _ => (gesture, false),
    };
    let (nose, tail_base) = match base.strip_prefix("N_") {
        Some(b) => (true, b),
        None if base == "Nollie" => (true, "Ollie"),
        None => (false, base),
    };
    if !TAIL_KEYS.contains(&tail_base) {
        return None;
    }
    // The prefixed 90 keys are only the group's own end.
    let nose_group = matches!(group, Group::Nose | Group::Nose90 | Group::NoseN90);
    if prefixed && nose != nose_group {
        return None;
    }
    let heel_side = matches!(
        tail_base,
        "FSPopShuvit"
            | "VarialHeelflip"
            | "InwardHeelflip"
            | "FS360PopShuvit"
            | "Laserflip"
            | "360InwardHeelflip"
            | "Heelflip"
    );
    let flip = matches!(
        tail_base,
        "360Flip" | "Laserflip" | "360Hardflip" | "360InwardHeelflip" | "Kickflip" | "Heelflip"
    );
    let named = |nose: bool, b: &str| -> String {
        if nose {
            if b == "Ollie" {
                "Nollie".into()
            } else {
                let mut s = String::from("N_");
                s.push_str(b);
                s
            }
        } else {
            b.into()
        }
    };
    let side = |nose: bool| match (nose, heel_side) {
        (false, false) => 1,
        (false, true) => 2,
        (true, false) => 3,
        (true, true) => 4,
    };
    if group == Group::Square || nose == nose_group {
        // Kept as flicked (flips have no dark-catch side).
        return Some((named(nose, tail_base), if flip { 0 } else { side(nose) }));
    }
    // Off the other end: shove-its keep their direction as half turns, the rest are plain pops.
    let t = match tail_base {
        "PopShuvit" | "360PopShuvit" => "PopShuvit",
        "FSPopShuvit" | "FS360PopShuvit" => "FSPopShuvit",
        _ => "Ollie",
    };
    Some((named(nose_group, t), side(nose_group)))
}

/// The runner: both graphs, their operations, controllers and shared state.
pub struct Runner<'a> {
    pub pack: Pack<'a>,
    pub action: usize,
    pub motion: usize,
    ag_ops: Vec<Op>,
    mg_ops: Vec<Op>,
    ag_inst: Vec<Inst>,
    mg_inst: Vec<Inst>,
    /// The riding behaviours' state, by operation (few operations have one).
    ag_riding: Vec<(OpId, RidingInst)>,
    mg_riding: Vec<(OpId, RidingInst)>,
    pub ag_ctl: Controller,
    pub mg_ctl: Controller,
    pub s: Shared,
    pub names: Names,
    /// Gesture intent ids per group, for HasGestureIntent and CreateTrickIntentFromGesture.
    gestures: Vec<Str>,
    scratch: Vec<PoseCommand>,
}

pub const ACTION: &str = "ActionGraph_OnBoard";
/// The fakie head and torso channel's tree (SK8 FakieHeadChannel).
pub const FAKIE_CHANNEL: &str = "B_FAKIE_CHANNEL";
/// The channel's name (an id no graph string has).
pub const CHANNEL_FAKIE: u16 = NONE - 1;
/// The pump layers' channels (SK8 `PUMP0`..`PUMP4`) and the d-pad gesture's (`GestureBoth`).
pub const CHANNEL_PUMP: u16 = NONE - 10;
pub const PUMP_CHANNELS: u16 = 5;
pub const CHANNEL_GESTURE: u16 = NONE - 20;
pub use super::ops::{GESTURE_STAGES, GESTURES, PUMP_TREE, gesture_clip};
pub const MOTION: &str = "MotionGraph_OnBoard";

impl<'a> Runner<'a> {
    pub fn new(pack: Pack<'a>, db: Option<&MotionDb>) -> Option<Runner<'a>> {
        Runner::with_ops(pack, db, None)
    }

    /// `typed`: every graph's operations as the importer typed them (`graph::opcode`), in pack order;
    /// without them they are parsed here.
    pub fn with_ops(
        pack: Pack<'a>,
        db: Option<&MotionDb>,
        typed: Option<Vec<Vec<Op>>>,
    ) -> Option<Runner<'a>> {
        let action = pack
            .graphs
            .iter()
            .position(|g| pack.str(g.name) == ACTION)?;
        let motion = pack
            .graphs
            .iter()
            .position(|g| pack.str(g.name) == MOTION)?;
        let (ag_ops, mg_ops) = match typed {
            Some(mut t) if t.len() == pack.graphs.len() => (
                core::mem::take(&mut t[action]),
                core::mem::take(&mut t[motion]),
            ),
            _ => (
                ops::parse(&pack, &pack.graphs[action], db),
                ops::parse(&pack, &pack.graphs[motion], db),
            ),
        };
        if ag_ops.len() != pack.graphs[action].ops.len()
            || mg_ops.len() != pack.graphs[motion].ops.len()
        {
            return None;
        }
        let id = |s: &str| pack.find(s).unwrap_or(NONE);
        let lib = |s: &str| db.and_then(|d| d.name_id(s));
        let names = Names {
            gesture_speed: id("GestureSpeed"),
            trick_height: id("TrickHeight"),
            dark_catch: id("DarkCatch"),
            underflip: id("Underflip"),
            dont_mirror: id("DontMirrorTrick"),
            jump_override: id("JumpHeightOverride"),
            is_dark: id("IsDark"),
            tweak_x: id("TweakX"),
            tweak_y: id("TweakY"),
            phys_body_spin: id("PhysBodySpin"),
            pushing: id("Pushing"),
            new_push: id("NewPush"),
            push_params: ["HSTR_VEL_B", "LSTR_VEL_B", "VEL_E"]
                .map(|n| lib(n).or_else(|| lib(&n.to_ascii_lowercase()))),
            fakie_tree: db.and_then(|d| d.find(FAKIE_CHANNEL)),
            torso: lib("TORSO").or_else(|| lib("torso")),
            trick_key: lib("Trick"),
            random_key: lib("Random"),
            manual_into: lib("manualInto"),
            default_cyc: lib("defaultcyc"),
            disttocog: lib("DISTTOCOG"),
            spin: lib("Spin"),
            avg_vel_y: lib("AvgVelY"),
            manual_angle: lib("manual_angle"),
            fakie_turn: id("FakieTurn"),
            left_slide: id("LeftSlide"),
            right_slide: id("RightSlide"),
            crouch: id("Crouch"),
            hard_turn_crouch: id("HardTurnCrouch"),
            manual: id("Manual"),
            auto_pump_angle: id("AutoPumpAngle"),
            auto_pump_mag: id("AutoPumpMag"),
            turn_packet: id("Turn"),
            slide_packet: id("Slide"),
            new_auto_pump: id("NewAutoPump"),
            grind_backwards: id("GrindFacingBackwards"),
            grind_forwards: id("GrindFacingForwards"),
            player_pump: id("PlayerControlledPump"),
            front_flip: id("FrontFlip"),
            back_flip: id("BackFlip"),
            body_spin: id("BodySpin"),
            spin_front: db.and_then(|d| d.find(SPIN_FRONT)),
            pump_tree: db.and_then(|d| d.find(PUMP_TREE)),
            out_distance: [lib("OUTDISTANCEX"), lib("OUTDISTANCEY")],
            spin_back: db.and_then(|d| d.find(SPIN_BACK)),
        };
        let mut last_sources = Vec::new();
        for o in &mg_ops {
            match o {
                Op::Beh(Beh::SetTrickHeight { from: Some(f), .. }) => last_sources.push(*f),
                Op::Beh(Beh::Play(p)) => last_sources.extend(
                    p.params
                        .iter()
                        .filter(|q| q.from == 2)
                        .filter_map(|q| q.source_name),
                ),
                _ => {}
            }
        }
        last_sources.sort_unstable();
        last_sources.dedup();
        let mut gestures = Vec::new();
        for i in 0..pack.strings() as u16 {
            let s = pack.str(i);
            let base = s
                .strip_prefix("90_")
                .or_else(|| s.strip_prefix("N90_"))
                .unwrap_or(s);
            let base = base.strip_prefix("N_").unwrap_or(base);
            if base == "Nollie" || TAIL_KEYS.contains(&base) {
                gestures.push(i);
            }
        }
        // In the order SK8's gesture selection visits them: by hash bucket, lowest first (a stable
        // sort keeps string order within a bucket).
        gestures.sort_by_key(|&g| gesture_bucket(pack.str(g)));
        let n = pack.strings();
        let sets = |names| {
            db.and_then(|d| PushSets::from_motion(d, names))
                .unwrap_or(PushSets::new(push::BUILT_IN))
        };
        let push_sets = [sets(push::REGULAR_CLIPS), sets(push::MONGO_CLIPS)];
        let ag_ctl = Controller::new(&pack.graphs[action]);
        let mg_ctl = Controller::new(&pack.graphs[motion]);
        Some(Runner {
            ag_inst: vec![Inst::default(); ag_ops.len()],
            mg_inst: vec![Inst::default(); mg_ops.len()],
            ag_riding: Vec::new(),
            mg_riding: Vec::new(),
            ag_ops,
            mg_ops,
            ag_ctl,
            mg_ctl,
            s: Shared {
                ag: Intents::new(n),
                mg: Intents::new(n),
                filtered: Intents::new(n),
                world: World::default(),
                player: Player::default(),
                packet: Vec::new(),
                flags: [0; 7],
                dark: false,
                manual_out_timer: 0.0,
                tricks_disabled: 0.0,
                time_since_kickturn: 10.0,
                grab: NONE,
                grab_score: NONE,
                trick: NONE,
                construction: Vec::new(),
                underflip_requested: false,
                dark_catch_requested: false,
                kickturn: None,
                slide_request: None,
                slide_grabbed: [false; 2],
                override_transition: None,
                rng: 0x2545_f491,
                unsupported: Vec::new(),
                new_unsupported: Vec::new(),
                last_sources,
                push_count: 0,
                push: PushState::default(),
                push_sets,
                push_curves: PushCurves {
                    tuning: crate::tuning::Tuning::BUILT_IN,
                },
                push_speed: 0.0,
                time_since_teleport: 0.0,
                conditioner: Conditioner::default(),
                turn_out: [0.0; 8],
                applying_tilt: false,
                landing: LandingStore::default(),
                com_height: crate::tuning::AnimTuning::BUILT_IN
                    .crouch_max_height
                    .eval(0.0),
                flip_signal: (0.0, 0.0, None),
                wipe_gestures: false,
                wipe_gesture: [0.0; 2],
                gesture: None,
            },
            names,
            gestures,
            pack,
            action,
            motion,
            scratch: Vec::new(),
        })
    }

    pub fn action_graph(&self) -> &Graph {
        &self.pack.graphs[self.action]
    }

    pub fn motion_graph(&self) -> &Graph {
        &self.pack.graphs[self.motion]
    }

    /// (implemented, no-op, unsupported names) over both graphs.
    pub fn coverage(&self) -> (usize, usize, Vec<&'a str>) {
        let (a1, a2, mut an) = ops::coverage(&self.pack, &self.ag_ops);
        let (m1, m2, mn) = ops::coverage(&self.pack, &self.mg_ops);
        for n in mn {
            if !an.contains(&n) {
                an.push(n);
            }
        }
        an.sort_unstable();
        (a1 + m1, a2 + m2, an)
    }

    /// One tick: `ag` must already hold this tick's action-graph intents (from the pad).
    pub fn update(&mut self, db: Option<&MotionDb>, dt: f32) {
        self.s.packet.clear();
        self.s.grab_score = NONE;
        self.s.kickturn = None;
        self.s.tricks_disabled = (self.s.tricks_disabled - dt).max(0.0);
        self.s.time_since_kickturn += dt;
        self.s.time_since_teleport += dt;
        // The physics' animation turn conditioner runs once per physics update (SK8
        // physical_feedback.rs), before the graphs read it.
        let w = self.s.world;
        let anim = self.s.push_curves.tuning.anim;
        self.s.turn_out = self.s.conditioner.update(
            TurnInput {
                speed: w.speed,
                wobble: 0.0,
                lean: w.lean,
                turn: w.turn,
                ground: w.phys == phys::GROUND,
                mirrored: w.mirrored,
            },
            &anim,
        );
        // The rider's centre-of-mass height over the board: the physics' (the posed rider's) when it
        // has one, as SK8 reads it (PhysOutAnimation's height), else where the playing clips put it.
        if let Some(h) = w.com_height {
            self.s.com_height = h;
        } else if let (Some(db), Some(n)) = (db, self.names.disttocog)
            && let Some(a) = self.s.player.attribute(db, n)
            && a.value().is_finite()
            && a.value() > 0.0
        {
            self.s.com_height = a.value();
        }
        {
            let g = &self.pack.graphs[self.action];
            let mut h = Host {
                p: &self.pack,
                g,
                ops: &self.ag_ops,
                inst: &mut self.ag_inst,
                riding: &mut self.ag_riding,
                s: &mut self.s,
                n: &self.names,
                db,
                gestures: &self.gestures,
                motion: false,
            };
            self.ag_ctl.update(g, dt, &mut h);
        }
        {
            let g = &self.pack.graphs[self.motion];
            let mut h = Host {
                p: &self.pack,
                g,
                ops: &self.mg_ops,
                inst: &mut self.mg_inst,
                riding: &mut self.mg_riding,
                s: &mut self.s,
                n: &self.names,
                db,
                gestures: &self.gestures,
                motion: true,
            };
            self.mg_ctl.update(g, dt, &mut h);
        }
        if let Some(db) = db {
            self.s.player.advance(db, dt);
        }
    }

    /// Starts both graphs over (the rider was put back on the board): every behaviour ends, the next
    /// update resolves from the roots.
    pub fn reset(&mut self, db: Option<&MotionDb>) {
        for motion in [true, false] {
            let (g, ops, inst, riding, ctl) = if motion {
                (
                    &self.pack.graphs[self.motion],
                    &self.mg_ops,
                    &mut self.mg_inst,
                    &mut self.mg_riding,
                    &mut self.mg_ctl,
                )
            } else {
                (
                    &self.pack.graphs[self.action],
                    &self.ag_ops,
                    &mut self.ag_inst,
                    &mut self.ag_riding,
                    &mut self.ag_ctl,
                )
            };
            let mut h = Host {
                p: &self.pack,
                g,
                ops,
                inst,
                riding,
                s: &mut self.s,
                n: &self.names,
                db,
                gestures: &self.gestures,
                motion,
            };
            ctl.end_all(&mut h);
            *ctl = Controller::new(g);
        }
        self.s.mg.clear();
        self.s.filtered.clear();
        self.s.flags = [0; 7];
        self.s.push = PushState::default();
        self.s.time_since_teleport = 0.0;
        self.s.applying_tilt = false;
        self.mg_riding.clear();
        self.ag_riding.clear();
    }

    /// What to pose this tick (the clips and how they blend, channels included); commits the clips'
    /// event windows.
    pub fn pose(&mut self, out: &mut PoseProgram) {
        self.s.player.program(out, &mut self.scratch);
    }

    pub fn packet(&self, s: Str) -> Option<f32> {
        self.s.packet.iter().find(|p| p.0 == s).map(|p| p.1)
    }

    pub fn packet_named(&self, name: &str) -> Option<f32> {
        self.pack.find(name).and_then(|s| self.packet(s))
    }

    pub fn flag(&self, f: Flag) -> bool {
        self.s.flags[flag_index(f)] > 0
    }

    /// The attribute of the playing clips (active now, or crossed during the last step).
    pub fn attribute(&self, db: &MotionDb, name: NameId) -> Option<Attr> {
        self.s.player.attribute(db, name)
    }

    /// The motion graph's current state as `Parent/Child/Leaf`.
    pub fn motion_state(&self) -> String {
        self.pack
            .state_path(self.motion_graph(), self.mg_ctl.frame.current)
    }

    /// The name of the motion graph's top-level state the rider is in (`OnBoard`, `WipeOut` ...).
    pub fn mg_state_root(&self) -> Option<&'a str> {
        let g = self.motion_graph();
        let mut s = self.mg_ctl.frame.current?;
        while let Some(p) = g.parent(s) {
            if p == g.root {
                break;
            }
            s = p;
        }
        Some(self.pack.state_name(g, s))
    }

    pub fn action_state(&self) -> String {
        self.pack
            .state_path(self.action_graph(), self.ag_ctl.frame.current)
    }

    pub fn str(&self, s: Str) -> &'a str {
        if s == NONE { "" } else { self.pack.str(s) }
    }
}

struct Host<'x, 'a> {
    p: &'x Pack<'a>,
    g: &'x Graph,
    ops: &'x [Op],
    inst: &'x mut [Inst],
    riding: &'x mut Vec<(OpId, RidingInst)>,
    s: &'x mut Shared,
    n: &'x Names,
    db: Option<&'x MotionDb<'x>>,
    gestures: &'x [Str],
    motion: bool,
}

impl Host<'_, '_> {
    fn anim_attr(&self, name: Option<NameId>) -> Option<Attr> {
        let (db, name) = (self.db?, name?);
        self.s.player.attribute(db, name)
    }

    fn gesture_in_group(&self, group: Group) -> bool {
        self.gestures
            .iter()
            .any(|&s| self.s.ag.has(s) && group_trick(group, self.p.str(s)).is_some())
    }

    fn cond(&mut self, c: &Cond, frame: &Frame) -> bool {
        let s = &*self.s;
        let w = &s.world;
        match *c {
            Cond::Intent(i, n) => n.test(if self.motion {
                s.mg.get(i)
            } else {
                s.ag.get(i)
            }),
            Cond::AgIntent(i, n) => n.test(s.ag.get(i)),
            Cond::Filtered(i, n) => n.test(s.filtered.get(i)),
            Cond::AnimAttr(name, n) => self.anim_attr(name).is_some_and(|a| {
                n.cmp == ops::Cmp::None
                    || !matches!(a.kind, KIND_SCALAR | KIND_CURVE)
                    || n.matches(a.value())
            }),
            Cond::WillExpire { in_time, wait } => {
                if wait && s.player.in_transition() {
                    false
                } else {
                    match s.player.property {
                        Some(p) if p.crossed_end || p.remaining_before_wrap >= 0.0 => {
                            p.crossed_end || p.remaining_before_wrap <= in_time
                        }
                        _ => {
                            let rem = s.player.length() - s.player.time();
                            s.player.current.is_some() && rem <= in_time
                        }
                    }
                }
            }
            Cond::ExpireInTime(n) => {
                let rem = s
                    .player
                    .property
                    .map_or(s.player.length() - s.player.time(), |p| {
                        p.remaining_before_wrap
                    });
                n.matches(rem)
            }
            Cond::InTimeWindow { start, length } => {
                !s.player.in_transition() && {
                    let t = s.player.time();
                    t >= start && t <= start + length
                }
            }
            Cond::StateTime(st, n) => st != NONE && frame.time_in(st).is_some_and(|t| n.matches(t)),
            Cond::LastState(st) => st != NONE && self.g.is_within(frame.last, st),
            Cond::CurrentState(st) => st != NONE && self.g.is_within(frame.current, st),
            Cond::PhysState(k) => w.phys == k,
            Cond::Grinding(name) => {
                w.phys == phys::GRIND && w.grind != NONE && name.is_none_or(|n| n == w.grind)
            }
            Cond::Mirrored => w.mirrored,
            Cond::RidingFakie => w.fakie,
            Cond::RidingGoofy => w.goofy,
            Cond::RidingSwitch => w.switch,
            Cond::Speed { along, num } => {
                num.matches(if along { w.forward_speed } else { w.speed })
            }
            Cond::SpeedAndSlope(n) => n.matches(w.speed_and_slope),
            Cond::TimeSinceLastInput(n) => n.matches(w.time_since_input),
            Cond::TimeToLand(n) => w.time_to_land.is_some_and(|t| n.matches(t)),
            Cond::DisablePushBrake => w.push_brake_disabled,
            Cond::Gesture(g) => self.gesture_in_group(g),
            Cond::GrabType(g) => g != NONE && s.grab == g,
            Cond::HasTweak(n) => {
                let (x, y) = (
                    s.filtered.get(self.n.tweak_x),
                    s.filtered.get(self.n.tweak_y),
                );
                (x.is_some() || y.is_some())
                    && (n.cmp == ops::Cmp::None || {
                        let (x, y) = (x.unwrap_or(0.0), y.unwrap_or(0.0));
                        n.matches(libm::sqrtf(x * x + y * y))
                    })
            }
            Cond::ManualOutTimer => s.manual_out_timer > 0.0,
            Cond::WantsManualExit => w.wants_manual_exit,
            Cond::WantsRunout => w.wants_runout,
            Cond::WantsWipeout => w.wants_wipeout,
            Cond::Dark => s.dark,
            Cond::DarkCatchRequested => s.dark_catch_requested,
            Cond::UnderflipRequested => s.underflip_requested,
            Cond::Random => self.s.random() & 1 == 0,
            Cond::ComVel { axis, num } => num.matches(w.com_vel[axis as usize]),
            // Past the push's out point with no push button held (SK8: the repush deadline keeps a
            // held push cycling).
            Cond::BreakOutOfPush => {
                s.player.time() >= s.push.out_factor * s.player.length() && !s.push.continue_push
            }
            Cond::SurfaceSlope(n) => n.matches(w.surface_slope),
            Cond::SkateSlope(n) => n.matches(w.skate_slope),
            Cond::GroundSlopeType(t) => {
                let k = if w.skate_slope.abs() < 5.0 {
                    0
                } else if w.skate_slope > 0.0 {
                    1
                } else {
                    2
                };
                t == 3 || k == t
            }
            Cond::AllowedToTrick => s.tricks_disabled <= 0.0,
            Cond::OkToTrickOnStairs => !w.on_stairs,
            Cond::Tricking => s.flags[flag_index(Flag::Trick)] > 0,
            Cond::LandingType(t) => w.landing_type == t,
            Cond::CanLandOnBoard => w.can_land,
            // SK8 IsCrouchedEnoughForBlendToGrabCycle: the centre of mass under 0.6 m over the board.
            Cond::CrouchedForGrab => s.com_height < 0.6,
            // The physics decides powerslides (flatground's Skate 3 rules); the graph's side is the
            // rider's, flipped when rolling fakie (SK8 CanEnterSlide).
            Cond::ShouldLeaveSlide { .. } => w.slide == 0 || w.slide_ending,
            Cond::CanEnterSlide { right } => {
                let r = right != w.fakie;
                w.slide == if r { 2 } else { 1 } && !w.slide_ending
            }
            Cond::GrindTrickOutAllowed(t) => {
                let k = match self.p.str(t) {
                    "left" => 1,
                    "right" => 2,
                    _ => 0,
                };
                k == w.grind_out
            }
            Cond::Blunting => w.blunting,
            Cond::TrucksOrDeck => w.trucks_or_deck,
            Cond::Apex => w.apex,
            Cond::DistToEdge(n) => n.matches(w.dist_to_edge),
            Cond::AngleAdjustLess => true,
            Cond::BodyFlipping => self.s.world.body_flipping,
            Cond::Bumped => {
                let m = self.s.push_curves.tuning.moves;
                let [x, z] = self.s.world.ground_acc;
                let x = x * m.bump_scale_x;
                libm::sqrtf(x * x + z * z) > m.bump_min
            }
            Cond::DoneWipingOut => w.phys != phys::WIPEOUT,
            Cond::Fixed(v) => v,
            Cond::Unsupported(n) => {
                self.s.unsupported(n);
                false
            }
        }
    }

    fn set_intent(&mut self, s: Str, v: Option<f32>) {
        if s == NONE {
            return;
        }
        match v {
            Some(v) => self.s.mg.set(s, v),
            None => self.s.mg.remove(s),
        }
    }

    fn run(&mut self, op: OpId, phase: u8, frame: &Frame) {
        let ops = self.ops;
        match &ops[op as usize] {
            Op::Beh(b) => self.behaviour(op, b, phase, frame),
            Op::Hook(h) if phase == 3 => match *h {
                Hook::Override(t) => self.s.override_transition = Some(t),
                Hook::GrabSlide { right } => self.s.slide_grabbed[usize::from(right)] = true,
                Hook::MongoPushToAntic => {}
                Hook::Unsupported(n) => self.s.unsupported(n),
            },
            _ => {}
        }
    }

    fn behaviour(&mut self, op: OpId, b: &Beh, phase: u8, frame: &Frame) {
        let dt = frame.dt;
        match b {
            Beh::Play(def) => {
                if phase == 2 {
                    return;
                }
                let Some(db) = self.db else { return };
                // Tree parameters from intents (every update; `lastAnim` only when starting).
                for p in &def.params {
                    let Some(target) = p.target else { continue };
                    let (value, normalized) = match p.from {
                        0 => (self.s.mg.get(p.source).or(p.default), p.normalized),
                        1 => {
                            let f = self.s.filtered.get(p.source);
                            (f.or(p.default), f.is_some() && p.normalized)
                        }
                        _ => {
                            if phase != 0 {
                                continue;
                            }
                            let v = p.source_name.and_then(|n| {
                                self.s
                                    .player
                                    .last_attrs
                                    .iter()
                                    .find(|a| a.0 == n)
                                    .map(|a| a.1)
                            });
                            (v.or(p.default), false)
                        }
                    };
                    if let Some(v) = value {
                        self.s.player.set_param(target, v, normalized);
                    }
                }
                if phase != 0 {
                    return;
                }
                let t = self
                    .s
                    .override_transition
                    .take()
                    .filter(|t| t.kind != 0)
                    .unwrap_or(def.transition);
                let w = self.s.world;
                let mut node = def.anim;
                if w.switch && def.switch_anim.is_some() {
                    node = def.switch_anim;
                }
                if w.mirrored && def.mirror_anim.is_some() {
                    node = def.mirror_anim;
                }
                let Some(node) = node else {
                    // Not converted: keep what plays.
                    return;
                };
                // What the previous animation leaves behind for `lastAnim` and SetTrickHeight.
                let mut last = Vec::new();
                for &n in &self.s.last_sources {
                    if let Some(a) = self.s.player.attribute(db, n) {
                        last.push((n, a.value()));
                    }
                }
                self.s.player.last_attrs = last;
                if let Ok(play) =
                    Playback::build_node(db, node, &self.s.construction, &mut Vec::new())
                {
                    self.s.player.play(
                        db,
                        Tree {
                            play,
                            mirror: w.mirrored,
                            turned: w.frame_turned,
                            name: def.anim_name,
                        },
                        def.speed,
                        t,
                    );
                }
            }
            Beh::CreateAttribute {
                name,
                tree,
                values,
                set,
            } => {
                if let Some(v) = values[phase as usize] {
                    self.s.emit(*name, v);
                    if *set && let Some(t) = tree {
                        self.s.player.set_param(*t, v, false);
                    }
                }
            }
            Beh::AttachIntent { intent, attr } => {
                if phase == 1
                    && let Some(v) = self.s.mg.get(*intent)
                {
                    self.s.emit(*attr, v);
                }
            }
            Beh::Filter(f) => {
                let i = &mut self.inst[op as usize];
                match phase {
                    0 => {
                        i.t = 0.0;
                        i.d = 0.0;
                        i.v = f.starting;
                        let v = i.v;
                        self.s.filtered.set(f.filtered, v);
                    }
                    1 => {
                        let input = self.s.mg.get(f.intent);
                        let (fakie, mirror) = (self.s.world.fakie, self.s.world.mirrored);
                        let i = &mut self.inst[op as usize];
                        let v = filter_step(f, i, input, dt, fakie, mirror);
                        self.s.filtered.set(f.filtered, v);
                    }
                    _ => self.s.filtered.remove(f.filtered),
                }
            }
            Beh::SetTrickHeight {
                from,
                rename,
                value,
                manual,
                grind,
            } => {
                if phase != 0 {
                    return;
                }
                let gesture = self.s.mg.get(self.n.gesture_speed).unwrap_or(0.0);
                let height = if let Some(v) = value {
                    *v
                } else if let Some(v) = self.s.mg.get(self.n.trick_height) {
                    v
                } else {
                    let last = from
                        .and_then(|f| self.s.player.last_attrs.iter().find(|a| a.0 == f))
                        .map_or(0.0, |a| a.1);
                    last.min(gesture.max(0.0))
                };
                let k = if *manual {
                    0.27
                } else if *grind {
                    1.0
                } else {
                    0.2
                };
                self.s.emit(self.n.jump_override, gesture * k);
                if let Some(r) = rename {
                    self.s.player.set_param(*r, height, true);
                }
            }
            Beh::JumpInto => {
                let i = &mut self.inst[op as usize];
                if phase == 0 {
                    i.on = true;
                } else if phase == 1 && i.on {
                    i.on = false;
                    if let (Some(db), Some(name)) = (self.db, self.n.manual_into)
                        && let Some(c) = &mut self.s.player.current
                    {
                        let mut a = Attr::scalar(name, 0.0);
                        if c.query_attribute(db, name, 31, &mut a) && a.begin >= 0.0 {
                            c.set_time(a.begin);
                        }
                    }
                }
            }
            Beh::ScoringTrick(t) => {
                if phase == 1 {
                    self.s.trick = *t;
                }
            }
            Beh::SetTrickAttr(t) => {
                if let Some(db) = self.db {
                    let key = self.n.trick_key;
                    if phase == 0 {
                        let v = db.name_id(self.p.str(*t));
                        self.s.set_construction(key, v);
                    } else if phase == 2
                        && self
                            .n
                            .default_cyc
                            .is_some_and(|d| self.s.player.attribute(db, d).is_some())
                    {
                        self.s.set_construction(key, None);
                    }
                }
            }
            Beh::ClearTrickAttr => {
                if phase == 0 {
                    self.s.set_construction(self.n.trick_key, None);
                }
            }
            Beh::Flag(f) => {
                let k = flag_index(*f);
                match phase {
                    0 => self.s.flags[k] = self.s.flags[k].saturating_add(1),
                    2 => self.s.flags[k] = self.s.flags[k].saturating_sub(1),
                    _ => {}
                }
            }
            Beh::SetDark => {
                self.s.dark = phase != 2;
                if phase == 1 {
                    self.s.emit(self.n.is_dark, 1.0);
                }
            }
            Beh::MonitorUnderflip => {
                let i = &mut self.inst[op as usize];
                match phase {
                    0 => i.n = 0,
                    2 => {
                        self.s.underflip_requested = false;
                        self.s.dark_catch_requested = false;
                    }
                    _ => {
                        i.n += 1;
                        if i.n > 1 {
                            let under = self
                                .s
                                .mg
                                .live()
                                .iter()
                                .any(|&x| self.p.str(x).starts_with("U_"));
                            if under {
                                self.s.underflip_requested = true;
                            } else if self.s.mg.has(self.n.dark_catch) {
                                self.s.dark_catch_requested = true;
                            }
                        }
                    }
                }
            }
            Beh::ChooseRandomLanding(n) => {
                if let Some(db) = self.db {
                    let key = self.n.random_key;
                    if phase == 0 {
                        let r = self.s.random() % (*n).max(1) as u32;
                        let mut buf = [0u8; 4];
                        let txt = small_number(r, &mut buf);
                        let v = db.name_id(txt);
                        self.s.set_construction(key, v);
                    } else if phase == 2 {
                        self.s.set_construction(key, None);
                    }
                }
            }
            Beh::KickTurnSteering { spin_scale, .. } => {
                if phase == 1 {
                    self.s.kickturn = Some(*spin_scale);
                }
            }
            // SK8 riding: the timer starts when the manual ends (so a trick just after it still comes
            // out of the manual), runs down only while not manualing, and stops when riding ends.
            Beh::SetManualOutTimer(len) => {
                if phase == 2 {
                    self.s.manual_out_timer = *len;
                }
            }
            Beh::UpdateManualOutTimer => match phase {
                1 if self.s.flags[flag_index(Flag::Manual)] == 0 => {
                    self.s.manual_out_timer = (self.s.manual_out_timer - dt).max(0.0);
                }
                2 => self.s.manual_out_timer = 0.0,
                _ => {}
            },
            Beh::DisableTricks(len) => {
                if phase == 0 {
                    self.s.tricks_disabled = *len;
                }
            }
            Beh::ResetTimeSinceKickturn => {
                if phase == 0 {
                    self.s.time_since_kickturn = 0.0;
                }
            }
            // The runner counts the time since the kick turn every tick; the physics decides riding
            // fakie (its rolling direction).
            Beh::UpdateTimeSinceKickturn | Beh::UpdateRidingFakie => {}
            Beh::SetTurning { names } => self.set_turning(op, names, phase, dt),
            Beh::ApplyingBodyTilt => match phase {
                0 => self.s.applying_tilt = true,
                2 => self.s.applying_tilt = false,
                _ => {}
            },
            Beh::SettingBodyTilt(name) => {
                if phase == 1 {
                    let w = self.s.world;
                    let anim = self.s.push_curves.tuning.anim;
                    let on = self.s.applying_tilt;
                    let RidingInst::Tilt(t) =
                        self.riding_inst(op, || RidingInst::Tilt(BodyTilt::default()))
                    else {
                        return;
                    };
                    if on {
                        let v = t.update(
                            w.mirrored,
                            w.lateral_up,
                            w.body_spin,
                            w.phys == phys::AIR,
                            &anim,
                        );
                        if let Some(n) = name {
                            self.s.player.set_param(*n, v, false);
                        }
                    } else {
                        t.disable();
                    }
                }
            }
            Beh::Crouching(name) => self.crouching(op, *name, phase, dt),
            Beh::SetDistComToBoard {
                attr,
                manually,
                adjust,
            } => {
                if phase == 0
                    && let Some(a) = attr
                {
                    // SK8 riding SetDistComToBoard: the physical height (0.65 when set by hand), less
                    // the landing speed's share of this tick (at most 0.15) when asked.
                    let height = if *manually { 0.65 } else { self.s.com_height };
                    let correction = if *adjust {
                        (dt * self.s.landing.landing_velocity).min(0.15)
                    } else {
                        0.0
                    };
                    self.s.player.set_param(*a, height - correction, false);
                }
            }
            Beh::SetLandingData => match phase {
                0 => {
                    let w = self.s.world;
                    let spin = if w.mirrored {
                        w.landing_spin
                    } else {
                        -w.landing_spin
                    };
                    if let Some(n) = self.n.spin {
                        self.s.player.set_param(n, spin, false);
                    }
                    if let Some(n) = self.n.avg_vel_y {
                        let v = self.s.landing.landing_velocity;
                        self.s.player.set_param(n, v, false);
                    }
                    let h = self.s.com_height;
                    self.set_riding(op, RidingInst::Value(h));
                }
                1 => {
                    if let (Some(n), Some(RidingInst::Value(h))) =
                        (self.n.disttocog, self.get_riding(op))
                    {
                        self.s.player.set_param(n, h, false);
                    }
                }
                _ => {}
            },
            Beh::StoreLandingData => {
                if phase == 1 {
                    let w = self.s.world;
                    self.s.landing.update(w.phys == phys::AIR, w.vel_up);
                }
            }
            Beh::SetManualAngle => match phase {
                0 => self.set_riding(op, RidingInst::Manual(ManualAngle::default())),
                1 => {
                    let manual = self.s.mg.get(self.n.manual);
                    let anim = self.s.push_curves.tuning.anim;
                    let RidingInst::Manual(m) =
                        self.riding_inst(op, || RidingInst::Manual(ManualAngle::default()))
                    else {
                        return;
                    };
                    let v = m.update(manual, &anim);
                    if let Some(n) = self.n.manual_angle {
                        self.s.player.set_param(n, v, false);
                    }
                }
                _ => {}
            },
            Beh::SetGrabType(g) => {
                if phase == 0 {
                    self.s.grab = *g;
                }
            }
            Beh::ScoringGrabs(g) => {
                if phase == 1 {
                    let first = self.s.filtered.get(g.x).unwrap_or(0.0);
                    let second = self.s.filtered.get(g.y).unwrap_or(0.0);
                    let (x, y) = if g.polar {
                        let y = second * libm::cosf(first);
                        (second * libm::sinf(first), if g.invert_y { -y } else { y })
                    } else {
                        (first, second)
                    };
                    let has = g.dirs.map(|d| d != NONE);
                    self.s.grab_score =
                        crate::skater::grab_variant(has, x, y).map_or(g.base, |i| g.dirs[i]);
                }
            }
            Beh::TweakProject { x, y } => {
                if phase == 1 {
                    let sx = self.s.filtered.get(self.n.tweak_x);
                    let sy = self.s.filtered.get(self.n.tweak_y);
                    if sx.is_some() || sy.is_some() {
                        let (mut tx, mut ty) = (sx.unwrap_or(0.0), sy.unwrap_or(0.0));
                        let total = tx.abs() + ty.abs();
                        if total > 1.0 {
                            tx /= total;
                            ty /= total;
                        }
                        for (n, v) in [(*x, tx), (*y, ty)] {
                            if let Some(n) = n {
                                self.s.player.set_param(n, v, false);
                            }
                        }
                    }
                }
            }
            Beh::Push(p) => self.push(op, *p, phase, dt),
            Beh::CreateSlide { right } => {
                if phase == 0 {
                    self.s.slide_request = Some(*right);
                    self.s.slide_grabbed = [false; 2];
                }
            }
            Beh::SetSpeed { attr, value } => {
                if phase == 0
                    && let Some(a) = attr
                {
                    let v = value.unwrap_or(self.s.world.speed);
                    self.s.player.set_param(*a, v, false);
                }
            }
            // Action graph.
            Beh::MgFromAg {
                mg,
                ag,
                default,
                scale,
                filters,
                on_update,
            } => {
                let i = &mut self.inst[op as usize];
                let emit = |s: &Shared| {
                    s.ag.get(*ag)
                        .or(*default)
                        .map(|v| filter_chain(scale * v, *filters, s.world.fakie, s.world.mirrored))
                };
                match phase {
                    0 => {
                        i.on = true;
                        let v = emit(self.s);
                        self.set_intent(*mg, v);
                    }
                    1 => {
                        let created = core::mem::replace(&mut i.on, false);
                        if *on_update {
                            let v = emit(self.s);
                            self.set_intent(*mg, v);
                        } else if !created {
                            self.set_intent(*mg, None);
                        }
                    }
                    _ => self.set_intent(*mg, None),
                }
            }
            Beh::MgConst {
                mg,
                value,
                on_update,
            } => {
                let i = &mut self.inst[op as usize];
                match phase {
                    0 => {
                        i.on = true;
                        self.set_intent(*mg, Some(*value));
                    }
                    1 => {
                        let created = core::mem::replace(&mut i.on, false);
                        if !(*on_update || created) {
                            self.set_intent(*mg, None);
                        }
                    }
                    _ => self.set_intent(*mg, None),
                }
            }
            Beh::MgTime { mg, ag } => {
                let present = self.s.ag.has(*ag);
                let i = &mut self.inst[op as usize];
                match phase {
                    0 => i.t = 0.0,
                    1 => {
                        if present {
                            i.t += dt;
                            let t = i.t;
                            self.set_intent(*mg, Some(t));
                        } else {
                            i.t = 0.0;
                            self.set_intent(*mg, None);
                        }
                    }
                    _ => self.set_intent(*mg, None),
                }
            }
            Beh::TrickFromGesture(group) => self.trick_from_gesture(op, *group, phase),
            Beh::BoardAdjust { mag, angle } => {
                if phase == 2 {
                    self.set_intent(*mag, None);
                    self.set_intent(*angle, None);
                }
            }
            Beh::FakieHead => {
                if phase == 1 {
                    self.fakie_head(op);
                }
            }
            Beh::BodySpin => match phase {
                0 => self.set_riding(op, RidingInst::Spin(SpinChannels::default())),
                1 => self.body_spin(op),
                _ => {}
            },
            Beh::AirLegExtension(n) => match phase {
                0 => self.set_riding(op, RidingInst::AirLeg(AirLeg::default())),
                1 => self.air_leg(op, *n, dt),
                _ => {}
            },
            Beh::DeckPitchAndYaw { yaw, pitch } => {
                // flatground's board isn't posed apart from the body: the deck is square to it.
                if phase == 0 {
                    for n in [yaw, pitch].into_iter().flatten() {
                        self.s.player.set_param(*n, 0.0, false);
                    }
                }
            }
            Beh::GrindFade {
                height,
                twist,
                intent,
            } => {
                let g = self.s.push_curves.tuning.grind;
                match phase {
                    0 => {
                        // The height as the riding left it; the twist from the middle of its range.
                        if let Some(h) = height {
                            self.s.player.set_param(*h, self.s.com_height, false);
                        }
                        let mut f = GrindFade::default();
                        let v = f.begin(0.5);
                        if let Some(t) = twist {
                            self.s.player.set_param(*t, v, true);
                        }
                        self.set_riding(op, RidingInst::Fade(f));
                    }
                    1 => {
                        let input = self.s.mg.get(*intent).unwrap_or(0.0);
                        let RidingInst::Fade(f) =
                            self.riding_inst(op, || RidingInst::Fade(GrindFade::default()))
                        else {
                            return;
                        };
                        if let Some(v) = f.update(dt, input, &g)
                            && let Some(t) = twist
                        {
                            self.s.player.set_param(*t, v, true);
                        }
                    }
                    _ => {}
                }
            }
            Beh::GrindCrouch(name) => {
                let g = self.s.push_curves.tuning.grind;
                match phase {
                    0 => self.set_riding(op, RidingInst::Value(self.s.com_height)),
                    1 => {
                        let intent = self.s.mg.get(self.n.crouch).unwrap_or(0.0);
                        let RidingInst::Value(h) =
                            self.riding_inst(op, || RidingInst::Value(g.height_max))
                        else {
                            return;
                        };
                        *h = super::riding::grind_crouch(*h, intent, 0.0, &g, dt);
                        let v = *h;
                        if let Some(n) = name {
                            self.s.player.set_param(*n, v, false);
                        }
                    }
                    _ => {}
                }
            }
            Beh::GrindAttributes => {
                let w = self.s.world;
                if phase == 1 && w.phys == phys::GRIND && w.grind != NONE {
                    self.s.emit(w.grind, 1.0);
                    let facing = if w.grind_backwards {
                        self.n.grind_backwards
                    } else {
                        self.n.grind_forwards
                    };
                    self.s.emit(facing, 1.0);
                }
            }
            Beh::BodyFlippingSignal => self.body_flip_signal(phase, dt),
            Beh::Pumping => self.pumping(op, phase),
            Beh::SlideSpin(n) => {
                // SK8 PowerSlideSpin: on begin, the spin from how the deck travels along its nose.
                if phase == 0
                    && let Some(n) = *n
                {
                    let m = self.s.push_curves.tuning.moves;
                    let v = -m.slide_spin.eval(self.s.world.slide_dir);
                    self.s.player.set_param(n, v, false);
                }
            }
            Beh::SlideDecel(n) => {
                // SK8 PowerSlideDecel: the lean from the speed along the ground, smoothed.
                let i = &mut self.inst[op as usize];
                match phase {
                    0 => i.v = 0.0,
                    1 => {
                        let m = self.s.push_curves.tuning.moves;
                        let target = m.slide_speed_to_lean.eval(self.s.world.ground_speed);
                        i.v = target * m.slide_smooth + (1.0 - m.slide_smooth) * i.v;
                        if let Some(n) = *n {
                            self.s.player.set_param(n, i.v, false);
                        }
                    }
                    _ => {}
                }
            }
            Beh::BumpCoefs { x, y } => {
                // SK8 SetBumpCoefficients (animation/bump.rs), on begin.
                if phase == 0 {
                    let m = self.s.push_curves.tuning.moves;
                    let [ax, az] = self.s.world.ground_acc;
                    let ax = ax * m.bump_scale_x;
                    let mag = libm::sqrtf(ax * ax + az * az);
                    let k =
                        ((mag - m.bump_min) / (m.bump_max - m.bump_min).max(1e-3)).clamp(0.0, 1.0);
                    let w = m.bump_min_blend + (1.0 - m.bump_min_blend) * k;
                    let (dx, dz) = if mag > 1e-6 {
                        (ax / mag, az / mag)
                    } else {
                        (0.0, 0.0)
                    };
                    let sign = if self.s.world.mirrored { -1.0 } else { 1.0 };
                    for (n, v) in [(*x, dx), (*y, dz)] {
                        if let Some(n) = n {
                            self.s.player.set_param(n, v * w * sign, false);
                        }
                    }
                }
            }
            Beh::TwistLean {
                always,
                twist,
                lean,
            } => {
                // SK8 MatchTwistAndLean: the body's twist (mirrored with the stance) and lean, read on
                // begin (every update with `update="always"`).
                let [hl, ht] = self.s.world.hips;
                let tw = if self.s.world.mirrored { -ht } else { ht };
                let i = &mut self.inst[op as usize];
                if phase == 0 || (phase == 1 && *always) {
                    i.v = tw;
                    i.t = hl;
                }
                if phase == 1 {
                    let (v, t) = (i.v, i.t);
                    for (n, v) in [(*twist, v), (*lean, t)] {
                        if let Some(n) = n {
                            self.s.player.set_param(n, v, false);
                        }
                    }
                }
            }
            Beh::Wipeout {
                lean,
                twist,
                gesture_x,
                gesture_y,
            } => self.wipeout(op, phase, [*lean, *twist, *gesture_x, *gesture_y]),
            Beh::WipeoutGestures => {
                if phase != 1 {
                    self.s.wipe_gestures = phase == 0;
                }
            }
            Beh::CharacterGesture => self.character_gesture(phase),
            Beh::Nothing => {}
            Beh::Unsupported(n) => self.s.unsupported(*n),
        }
    }

    /// Pumping (SK8 motion_execute_leaf.rs, animation/pumping_channel.rs): the physics' pumping
    /// acceleration over `pump_max`, filtered and amplified, starts a pump layer (`B_PUMP` on a free
    /// channel) each time it rises past `pump_new` and sets that layer's weight while it rises.
    fn pumping(&mut self, op: OpId, phase: u8) {
        let m = self.s.push_curves.tuning.moves;
        if phase == 2 {
            for k in 0..PUMP_CHANNELS {
                self.s.player.end_channel(CHANNEL_PUMP - k);
            }
            return;
        }
        if phase != 1 {
            return;
        }
        let input = (self.s.world.pump_acc / m.pump_max.max(1e-4)).clamp(0.0, 1.0);
        let i = &mut self.inst[op as usize];
        let previous = i.v;
        i.d = input * m.pump_blend + (1.0 - m.pump_blend) * i.d;
        i.v = m.pump_amplify.eval(i.d);
        let (value, starts) = (i.v, previous <= m.pump_new && i.v > m.pump_new);
        if starts {
            let free = (0..PUMP_CHANNELS).find(|&k| !self.s.player.has_channel(CHANNEL_PUMP - k));
            if let (Some(k), Some(db), Some(node)) = (free, self.db, self.n.pump_tree)
                && let Ok(play) =
                    Playback::build_node(db, node, &self.s.construction, &mut Vec::new())
            {
                let settings = ChannelSettings {
                    priority: 0,
                    keep_alive: false,
                    speed: 1.0,
                    blend_in: m.pump_blend_in,
                    hold_in: false,
                    blend_out: m.pump_blend_out,
                    hold_out: true,
                };
                let tree = Tree {
                    play,
                    mirror: self.s.world.mirrored,
                    turned: self.s.world.frame_turned,
                    name: NONE,
                };
                let blend = ops::TransitionDef {
                    kind: 2,
                    seconds: 0.1,
                    under: false,
                    matching: 0,
                    channels_from_weights: false,
                };
                self.s.player.transition_channel(
                    db,
                    CHANNEL_PUMP - k,
                    tree,
                    settings,
                    blend,
                    true,
                    true,
                );
                let i = &mut self.inst[op as usize];
                i.n = k as i32;
                i.t = value;
            }
        }
        let i = &mut self.inst[op as usize];
        if i.n >= 0 && value >= previous && value >= i.t {
            i.t = value;
            let c = CHANNEL_PUMP - i.n as u16;
            self.s.player.set_channel_influence(c, value);
        }
    }

    /// Wipeout (SK8 motion_wipeout_execute.rs): the bail trees' lean and twist follow the body's,
    /// rate-limited; with EnableWipeoutGestures on, the stick steers the wipeout gesture (once it has
    /// been let go to the middle, or after 80 ticks).
    fn wipeout(&mut self, op: OpId, phase: u8, names: [Option<NameId>; 4]) {
        let m = self.s.push_curves.tuning.moves;
        let w = self.s.world;
        let tau = crate::math::TAU;
        let twist_now = {
            let t = if w.mirrored { -w.hips[1] } else { w.hips[1] };
            if t < 0.0 { t + tau } else { t }
        };
        let enabled = self.s.wipe_gestures;
        let mut gesture = self.s.wipe_gesture;
        let i = &mut self.inst[op as usize];
        match phase {
            0 => {
                i.n = 0;
                i.v = w.hips[0];
                i.t = twist_now;
                i.d = 0.0;
                i.on = false;
                gesture = [0.0; 2];
            }
            1 => {
                if i.n > 80 {
                    i.on = true;
                }
                if enabled {
                    let target = (1.0 - m.wipe_lean_blend) * i.v + w.hips[0] * m.wipe_lean_blend;
                    i.v += (target - i.v).clamp(-m.wipe_lean_vel, m.wipe_lean_vel);
                    let mut delta = twist_now - i.t;
                    if delta > crate::math::PI {
                        delta -= tau;
                    } else if delta < -crate::math::PI {
                        delta += tau;
                    }
                    let mut vel =
                        (delta * m.wipe_twist_blend).clamp(-m.wipe_twist_vel, m.wipe_twist_vel);
                    if i.n >= 2 {
                        vel = (vel - i.d).clamp(-m.wipe_twist_acc, m.wipe_twist_acc) + i.d;
                    }
                    i.d = vel;
                    let t = (i.t + vel) % tau;
                    i.t = if t < 0.0 { t + tau } else { t };
                    let [xv, yv] = w.wipe_stick.unwrap_or([0.0; 2]);
                    if !i.on && w.wipe_stick.is_some() && xv.abs() < 0.1 && yv.abs() < 0.1 {
                        i.on = true;
                    }
                    if i.on {
                        gesture[0] = (1.0 - m.wipe_gesture_x_blend) * gesture[0]
                            + xv * m.wipe_gesture_x_blend;
                        gesture[1] = (1.0 - m.wipe_gesture_y_blend) * gesture[1]
                            + yv * m.wipe_gesture_y_blend;
                    }
                }
                i.n = i.n.saturating_add(1);
            }
            _ => return,
        }
        let values = [i.v, i.t, gesture[0], gesture[1]];
        self.s.wipe_gesture = gesture;
        if phase == 1 {
            for (n, v) in names.into_iter().zip(values) {
                if let Some(n) = n {
                    self.s.player.set_param(n, v, false);
                }
            }
        }
    }

    /// CharacterGesture (SK8 motion_character_gesture.rs): a d-pad press (`GestureLeftStart` ...)
    /// plays that direction's gesture on the gesture channel (both hands: flatground's hands are always
    /// free on the board): its `INTO` clip, the `CYC` clip while the d-pad is held, then `OUT`; another
    /// direction takes over; a forced brake doesn't start one.
    fn character_gesture(&mut self, phase: u8) {
        if phase == 2 {
            self.s.player.end_channel(CHANNEL_GESTURE);
            self.s.gesture = None;
            return;
        }
        if phase != 1 {
            return;
        }
        let Some(db) = self.db else { return };
        let pack = self.p;
        let id = |n: &str| pack.find(n).unwrap_or(NONE);
        let mg = &self.s.mg;
        let has = |n: Str| n != NONE && mg.has(n);
        let starts = [
            (3u8, id("GestureDownStart")),
            (0, id("GestureLeftStart")),
            (2, id("GestureUpStart")),
            (1, id("GestureRightStart")),
        ];
        let held = [
            id("GestureLeftHeld"),
            id("GestureRightHeld"),
            id("GestureUpHeld"),
            id("GestureDownHeld"),
        ]
        .map(has);
        let braking = has(id("ForceBrake"));
        let selected = starts.iter().find(|(_, n)| has(*n)).map(|(d, _)| *d);
        // The channel went (faded out): the gesture is over.
        if self.s.gesture.is_some() && !self.s.player.has_channel(CHANNEL_GESTURE) {
            self.s.gesture = None;
        }
        let current = self.s.gesture;
        let start = selected
            .filter(|&d| !braking && current.is_none_or(|(c, _)| !held[c as usize] && c != d));
        let play = |s: &mut Shared, direction: u8, stage: u8| -> bool {
            let first = stage == 0;
            let Some(node) = db.find(&gesture_clip(direction as usize, stage as usize)) else {
                return false;
            };
            let Ok(play) = Playback::build_node(db, node, &Vec::new(), &mut Vec::new()) else {
                return false;
            };
            let settings = ChannelSettings {
                priority: 0,
                keep_alive: stage == 1,
                speed: 1.0,
                blend_in: 0.1,
                hold_in: first,
                blend_out: 0.1,
                hold_out: true,
            };
            let blend = ops::TransitionDef {
                kind: if first { 2 } else { 4 },
                seconds: if first { 0.2 } else { 0.0 },
                under: false,
                matching: 0,
                channels_from_weights: false,
            };
            let tree = Tree {
                play,
                mirror: false,
                turned: s.world.frame_turned,
                name: NONE,
            };
            s.player
                .transition_channel(db, CHANNEL_GESTURE, tree, settings, blend, true, true);
            true
        };
        if let Some(d) = start {
            if play(self.s, d, 0) {
                self.s.gesture = Some((d, 0));
            }
            return;
        }
        let Some((d, stage)) = current else { return };
        let remaining = self.s.player.channel_remaining(CHANNEL_GESTURE);
        if self.s.player.channel_in_transition(CHANNEL_GESTURE) || remaining >= 0.1 {
            return;
        }
        match stage {
            0 => {
                if play(self.s, d, 1) {
                    self.s.gesture = Some((d, 1));
                }
            }
            1 if !held[d as usize] => {
                if play(self.s, d, 2) {
                    self.s.gesture = Some((d, 2));
                }
            }
            2 => {
                self.s.player.end_channel(CHANNEL_GESTURE);
                self.s.gesture = None;
            }
            _ => {}
        }
    }

    /// BodyFlippingSignal (SK8 input/body_flip_signal.rs): a `FrontFlip` or `BackFlip` gesture (the
    /// left stick's, counted only while grabbing and not pushing) becomes that motion intent while the
    /// gesture is recent (`extend_bodyflip_gesture`) and the rider has just left the ground
    /// (`extend_takeoff_point`); otherwise neither.
    fn body_flip_signal(&mut self, phase: u8, dt: f32) {
        let m = self.s.push_curves.tuning.moves;
        let (gw, tw) = (m.flip_gesture_window, m.flip_takeoff_window);
        let names = [self.n.front_flip, self.n.back_flip];
        let st = &mut self.s.flip_signal;
        match phase {
            0 => *st = (gw, tw, st.2),
            1 => {
                match names.iter().position(|&n| n != NONE && self.s.ag.has(n)) {
                    Some(i) => {
                        st.0 = 0.0;
                        st.2 = Some(i);
                    }
                    None => st.0 = (st.0 + dt).min(gw),
                }
                let gesture = st.0 < gw;
                st.1 = if self.s.world.phys == phys::AIR {
                    (st.1 + dt).min(tw)
                } else {
                    0.0
                };
                let takeoff = st.1 > 0.0 && st.1 < tw;
                let chosen = if gesture && takeoff { st.2 } else { None };
                for (i, &n) in names.iter().enumerate() {
                    if n == NONE {
                        continue;
                    }
                    if chosen == Some(i) {
                        self.s.mg.set(n, 1.0);
                    } else {
                        self.s.mg.remove(n);
                    }
                }
            }
            _ => {}
        }
    }

    fn get_riding(&self, op: OpId) -> Option<RidingInst> {
        self.riding.iter().find(|r| r.0 == op).map(|r| r.1)
    }

    fn set_riding(&mut self, op: OpId, v: RidingInst) {
        match self.riding.iter_mut().find(|r| r.0 == op) {
            Some(r) => r.1 = v,
            None => self.riding.push((op, v)),
        }
    }

    /// The operation's riding state, made with `init` if it has none yet.
    fn riding_inst(&mut self, op: OpId, init: impl FnOnce() -> RidingInst) -> &mut RidingInst {
        let i = match self.riding.iter().position(|r| r.0 == op) {
            Some(i) => i,
            None => {
                self.riding.push((op, init()));
                self.riding.len() - 1
            }
        };
        &mut self.riding[i].1
    }

    /// SetTurning (SK8 set_turning.rs): the carve's tree parameters from the turn conditioner, and the
    /// `Turn` / `Slide` packet values.
    fn set_turning(&mut self, op: OpId, names: &[Option<NameId>; 5], phase: u8, dt: f32) {
        match phase {
            0 => self.set_riding(op, RidingInst::Turning(Turning::default())),
            1 => {
                let w = self.s.world;
                let t = self.s.push_curves.tuning;
                let get = |s: Str| self.s.mg.get(s);
                let ctx = TurnContext {
                    // The graph's slide side: the physics' side, the other way round riding fakie
                    // (as CanEnterSlide reads it).
                    slide: (w.slide != 0 && !w.slide_ending).then_some((w.slide == 2) != w.fakie),
                    fakie: w.fakie,
                    mirrored: w.mirrored,
                    forward_speed: w.forward_speed,
                    fakie_turn: get(self.n.fakie_turn),
                    left_slide: get(self.n.left_slide),
                    right_slide: get(self.n.right_slide),
                    slide_turn_value: t.slide_turn_value,
                };
                let c = self.s.turn_out;
                let RidingInst::Turning(st) =
                    self.riding_inst(op, || RidingInst::Turning(Turning::default()))
                else {
                    return;
                };
                let v = st.update(&c, ctx, dt, &t.anim);
                for (n, value) in
                    names
                        .iter()
                        .zip([v.angle, v.dir, v.quickness, v.speed_tuck, v.holding])
                {
                    if let Some(n) = n {
                        self.s.player.set_param(*n, value, false);
                    }
                }
                if let Some(turn) = v.turn {
                    self.s.emit(self.n.turn_packet, turn);
                }
                if let Some(slide) = v.slide {
                    self.s.emit(self.n.slide_packet, slide);
                }
            }
            _ => {}
        }
    }

    /// ControlAirLegExtension (SK8 air_leg_extension.rs): the rider's centre of mass over the board
    /// (`disttocog`) starts where the last clip had it and closes in going up and coming down, to at
    /// least `air_com_min` (the legs tucked); near the landing it reaches out at `preland_com_speed`.
    /// The arms' reach (`extend`) follows the seconds coming down.
    fn air_leg(&mut self, op: OpId, extend: Option<NameId>, dt: f32) {
        let w = self.s.world;
        let t = self.s.push_curves.tuning;
        let first = self
            .db
            .zip(self.n.disttocog)
            .and_then(|(db, n)| self.s.player.attribute(db, n))
            .map(|a| a.value());
        let com = self.s.com_height;
        let mut a = match self.get_riding(op) {
            Some(RidingInst::AirLeg(a)) => a,
            _ => AirLeg::default(),
        };
        let ascending = w.vel_y > 0.0;
        // SK8 PrelandingPhysical::near_landing: coming down, within the time the legs need to reach
        // out from where the body is now.
        let near = !ascending
            && w.time_to_land
                .is_some_and(|l| l < (t.preland_com - com) / t.preland_com_speed.max(0.01));
        match a.mode {
            0 => {
                a.mode = if ascending { 1 } else { 2 };
                a.height = first.unwrap_or(com);
            }
            1 if w.vel_y >= 0.0 => {
                a.height = (a.height - dt * t.air_com_up_speed).max(t.air_com_min);
            }
            1 | 2 if near => {
                a.mode = 3;
                a.height += dt * t.preland_com_speed;
            }
            1 | 2 => {
                a.mode = 2;
                let speed = (-w.vel_y).max(0.0).min(t.air_com_down_speed);
                a.height = (a.height - speed * dt).max(t.air_com_min);
            }
            _ => a.height += dt * t.preland_com_speed,
        }
        a.down_time = if ascending { 0.0 } else { a.down_time + dt };
        self.set_riding(op, RidingInst::AirLeg(a));
        if let Some(n) = self.n.disttocog {
            self.s.player.set_param(n, a.height, false);
        }
        if let Some(n) = extend {
            let v = t.air_arm_extend.eval(a.down_time);
            self.s.player.set_param(n, v, false);
        }
    }

    /// Crouching (SK8 crouching.rs): the centre-of-mass height from the crouch intents, the auto pump
    /// and the absorption, as the `crouchName` tree parameter.
    fn crouching(&mut self, op: OpId, name: Option<NameId>, phase: u8, dt: f32) {
        let w = self.s.world;
        let anim = self.s.push_curves.tuning.anim;
        let p = CrouchPhysical {
            vel_y: w.vel_y,
            ground_speed: w.ground_speed,
            pump_potential: w.pump_potential,
            force: w.crouch_force,
            ground_force: w.crouch_ground_force,
            min_crouch: w.crouch_min,
            deck_angle: w.crouch_deck,
            height: self.s.com_height,
        };
        match phase {
            0 => self.set_riding(op, RidingInst::Crouch(Crouch::begin(p, &anim))),
            1 => {
                let get = |s: Str| self.s.mg.get(s);
                let i = CrouchIntents {
                    auto_pump_angle: get(self.n.auto_pump_angle),
                    auto_pump_mag: get(self.n.auto_pump_mag),
                    crouch: get(self.n.crouch),
                    hard_turn_crouch: get(self.n.hard_turn_crouch),
                    manual: get(self.n.manual),
                    sliding: self.s.flags[flag_index(Flag::PowerSlide)] > 0,
                };
                let RidingInst::Crouch(c) =
                    self.riding_inst(op, || RidingInst::Crouch(Crouch::begin(p, &anim)))
                else {
                    return;
                };
                let out = c.update(p, i, dt, &anim);
                if out.new_auto_pump {
                    self.s.emit(self.n.new_auto_pump, 1.0);
                }
                if out.player_pump {
                    self.s.emit(self.n.player_pump, 1.0);
                }
                if let Some(n) = name {
                    self.s.player.set_param(n, out.height, false);
                }
            }
            _ => {}
        }
    }

    /// BodySpin (SK8 motion_spin.rs): the body spin channels follow the `BodySpin` intent (mapped by
    /// `spin_map`) in the air or during a trick, their influence eased by `spin_blend_in` / `_out` within
    /// rate limits; off while the hands are busy (grabbing) or grinding.
    fn body_spin(&mut self, op: OpId) {
        let w = self.s.world;
        let t = self.s.push_curves.tuning;
        let air = w.phys == phys::AIR;
        let doing_trick = self.s.flags[flag_index(Flag::Trick)] > 0;
        let active = air || doing_trick;
        let busy = self.s.grab != NONE;
        let mut spin = self.s.mg.get(self.n.body_spin).unwrap_or(0.0);
        if w.mirrored {
            spin = -spin;
        }
        let mapped = t.spin_map.eval(spin.abs());
        let influence = if spin >= 0.0 { mapped } else { -mapped };
        let near = air && w.time_to_land.is_some_and(|l| l < SPIN_NEAR_LANDING);
        let (front_node, back_node) = (self.n.spin_front, self.n.spin_back);
        let db = self.db;
        let RidingInst::Spin(mut c) =
            *self.riding_inst(op, || RidingInst::Spin(SpinChannels::default()))
        else {
            return;
        };
        let mut mode = c.mode;
        match mode {
            0 => {
                if w.phys == phys::GRIND {
                    mode = 4;
                }
                if active {
                    mode = if busy { 4 } else { 1 };
                }
            }
            1 => {
                if !active {
                    mode = 0;
                }
                if busy {
                    mode = 4;
                }
                if spin.abs() > SPIN_START {
                    mode = 2;
                }
            }
            2 => {
                if !active {
                    mode = 0;
                }
                if busy {
                    mode = 4;
                }
                if air {
                    mode = 3;
                }
            }
            3 => {
                if busy {
                    mode = 4;
                }
                if !air {
                    mode = 0;
                }
            }
            _ => {
                if !active && w.phys == phys::GROUND {
                    mode = 0;
                }
            }
        }
        let player = &mut self.s.player;
        match mode {
            0 | 4 => {
                player.end_channel(CHANNEL_SPIN_FRONT);
                player.end_channel(CHANNEL_SPIN_BACK);
            }
            1 => {
                c = SpinChannels {
                    mode,
                    ..SpinChannels::default()
                }
            }
            _ => {
                if mode == 2
                    && let Some(db) = db
                {
                    let settings = ChannelSettings {
                        priority: 0,
                        keep_alive: false,
                        speed: 1.0,
                        blend_in: 0.2,
                        hold_in: false,
                        blend_out: 0.3,
                        hold_out: false,
                    };
                    let start =
                        |player: &mut Player, name: u16, node: Option<crate::motion::Node>| {
                            if player.has_channel(name) || near {
                                return;
                            }
                            let Some(node) = node else { return };
                            let Ok(play) =
                                Playback::build_node(db, node, &Vec::new(), &mut Vec::new())
                            else {
                                return;
                            };
                            let tree = Tree {
                                play,
                                mirror: w.mirrored,
                                turned: w.frame_turned,
                                name: NONE,
                            };
                            let blend = ops::TransitionDef {
                                kind: 2,
                                seconds: 0.1,
                                under: false,
                                matching: 0,
                                channels_from_weights: false,
                            };
                            player.transition_channel(db, name, tree, settings, blend, true, true);
                        };
                    if spin > SPIN_START {
                        start(player, CHANNEL_SPIN_FRONT, front_node);
                        player.end_channel(CHANNEL_SPIN_BACK);
                    }
                    if spin < -SPIN_START {
                        start(player, CHANNEL_SPIN_BACK, back_node);
                        player.end_channel(CHANNEL_SPIN_FRONT);
                    }
                }
                let step = |value: &mut f32, delta: &mut f32, target: f32, blend: f32| {
                    let desired = blend * target + (1.0 - blend) * *value - *value;
                    *delta = desired.clamp(*delta - t.spin_max_acc, *delta + t.spin_max_acc);
                    let next = (*value + *delta)
                        .clamp(*value - t.spin_max_delta, *value + t.spin_max_delta);
                    *value = next.clamp(0.0, 1.0);
                };
                if player.has_channel(CHANNEL_SPIN_FRONT) {
                    let target = influence.clamp(0.0, 1.0);
                    let blend = if target < c.front {
                        t.spin_blend_out
                    } else {
                        t.spin_blend_in
                    };
                    step(&mut c.front, &mut c.front_d, target, blend);
                    player.set_channel_influence(CHANNEL_SPIN_FRONT, c.front);
                }
                if player.has_channel(CHANNEL_SPIN_BACK) {
                    let target = -influence.clamp(-1.0, 0.0);
                    // SK8 keeps the original's reversed comparison on this side.
                    let blend = if target > c.back {
                        t.spin_blend_out
                    } else {
                        t.spin_blend_in
                    };
                    step(&mut c.back, &mut c.back_d, target, blend);
                    player.set_channel_influence(CHANNEL_SPIN_BACK, c.back);
                }
            }
        }
        c.mode = mode;
        self.set_riding(op, RidingInst::Spin(c));
    }

    /// FakieHeadChannel (SK8 motion_channels.rs FakieHead): riding fakie, the fakie channel (head and
    /// torso looking where the board goes) fades in over the animation, its `torso` share snapped to
    /// its target on entry, then eased towards it: all of it in a manual, none in a powerslide, half
    /// otherwise. It fades out when the rider stops riding fakie.
    fn fakie_head(&mut self, op: OpId) {
        let s = &mut *self.s;
        let i = &mut self.inst[op as usize];
        let fakie = s.world.fakie;
        let target = if s.flags[flag_index(Flag::Manual)] > 0 {
            1.0
        } else if s.flags[flag_index(Flag::PowerSlide)] > 0 {
            0.0
        } else {
            0.5
        };
        if fakie && !i.on {
            if let (Some(db), Some(node)) = (self.db, self.n.fakie_tree)
                && let Ok(play) = Playback::build_node(db, node, &s.construction, &mut Vec::new())
            {
                let settings = ChannelSettings {
                    priority: 0,
                    keep_alive: true,
                    speed: 1.0,
                    blend_in: 0.3,
                    hold_in: false,
                    blend_out: 0.3,
                    hold_out: false,
                };
                let blend = ops::TransitionDef {
                    kind: 2,
                    seconds: 0.1,
                    under: false,
                    matching: 0,
                    channels_from_weights: false,
                };
                let tree = Tree {
                    play,
                    mirror: s.world.mirrored,
                    turned: s.world.frame_turned,
                    name: NONE,
                };
                s.player
                    .transition_channel(db, CHANNEL_FAKIE, tree, settings, blend, true, true);
            }
            i.v = target;
        } else if !fakie && i.on {
            s.player.end_channel(CHANNEL_FAKIE);
        }
        i.on = fakie;
        if fakie {
            i.v += (target - i.v).clamp(-0.01, 0.01);
            if let Some(t) = self.n.torso {
                s.player.set_param(t, i.v, false);
            }
        }
    }

    /// The push behaviours (SK8 graph_host pushing.rs): push strength from the button, the push trees'
    /// coefficients, the repush deadline.
    fn push(&mut self, op: OpId, p: PushOp, phase: u8, dt: f32) {
        let s = &mut *self.s;
        let speed = s.world.forward_speed;
        let curves = s.push_curves;
        let sets = |regular: bool, s: &Shared| s.push_sets[usize::from(regular == s.world.switch)];
        let i = &mut self.inst[op as usize];
        match (p, phase) {
            (PushOp::Out { right }, 0) => {
                // SK8 PushOut: the pushing foot against the deck (along its nose, up), on begin.
                let foot = s.world.feet[usize::from(right)];
                for (n, v) in self.n.out_distance.into_iter().zip(foot) {
                    if let Some(n) = n {
                        s.player.set_param(n, v, false);
                    }
                }
            }
            (PushOp::Init, 0) => {
                s.push.init();
                s.push_count += 1;
            }
            (PushOp::FirstStrength, 0) => {
                let f = push::FirstPush::begin(
                    s.time_since_teleport,
                    curves.tuning.push_teleport_window,
                );
                (i.t, i.n, i.on) = (f.simulated, i32::from(f.from_teleport), f.first);
            }
            (PushOp::FirstStrength, 1) => {
                let mut f = push::FirstPush {
                    simulated: i.t,
                    from_teleport: i.n != 0,
                    first: i.on,
                };
                let pushing = s.mg.get(self.n.pushing);
                f.update(&mut s.push, &curves, pushing, speed, dt);
                (i.t, i.on) = (f.simulated, f.first);
            }
            (PushOp::TargetCoefs { regular }, 1) => {
                s.push.regular = regular;
                s.push.target = sets(regular, s).target(speed, s.push.dv);
            }
            (PushOp::Coefs { .. }, 0) => i.on = true,
            (PushOp::Coefs { first_only }, 1) => {
                if !first_only || i.on {
                    s.push.current = curves.ease(s.push.current, s.push.target, speed, dt);
                    let c = s.push.current;
                    for (name, v) in self
                        .n
                        .push_params
                        .iter()
                        .zip([c.strong, c.gentle, c.strength])
                    {
                        if let Some(n) = name {
                            s.player.set_param(*n, v, true);
                        }
                    }
                    s.push_speed = sets(s.push.regular, s).end_speed(c);
                }
                i.on = false;
            }
            (PushOp::Cycle { foot }, 0 | 1) => {
                let intents = push::Intents {
                    pushing: s.mg.get(self.n.pushing),
                    new_push: s.mg.has(self.n.new_push),
                    foot: foot != NONE && s.mg.has(foot),
                };
                if phase == 0 {
                    s.push_count += 1;
                    let c = push::Cycle::begin(&mut s.push, &curves, intents);
                    (i.n, i.v) = (c.phase as i32, c.next_dv);
                } else {
                    let phases = [
                        push::Phase::HoldingFirst,
                        push::Phase::WaitingNext,
                        push::Phase::HoldingNext,
                        push::Phase::NextReleased,
                    ];
                    let mut c = push::Cycle {
                        phase: phases[(i.n as usize).min(3)],
                        next_dv: i.v,
                    };
                    c.update(&mut s.push, &curves, intents, speed);
                    (i.n, i.v) = (c.phase as i32, c.next_dv);
                }
            }
            (PushOp::RepushDeadline { regular }, 2) => {
                let t = &curves.tuning;
                s.push.out_factor = sets(regular, s).out_factor(
                    speed,
                    s.push.dv,
                    t.push_out_speed_weight,
                    t.push_out_max,
                );
            }
            _ => {}
        }
    }

    /// CreateTrickIntentFromGesture (SK8 gesture_mapping): the flicked gesture, fitted to the group and
    /// mirrored for the stance, becomes a motion-graph intent; a held kickflip or heelflip becomes its
    /// `Hold` intent after the first update (multi-flips).
    fn trick_from_gesture(&mut self, op: OpId, group: Group, phase: u8) {
        match phase {
            0 => {
                let mut chosen: Option<(String, u8)> = None;
                for &g in self.gestures {
                    if self.s.ag.has(g)
                        && let Some(t) = group_trick(group, self.p.str(g))
                    {
                        chosen = Some(t);
                        break;
                    }
                }
                let i = &mut self.inst[op as usize];
                i.on = true;
                i.s = NONE;
                i.s2 = NONE;
                let Some((trick, side)) = chosen else { return };
                let mirrored = self.s.world.mirrored && !self.s.ag.has(self.n.dont_mirror);
                let mut name = if self.s.ag.has(self.n.dark_catch) && side != 0 {
                    String::from(match (side, mirrored) {
                        (2, false) | (1, true) => "Heelflip",
                        (3, true) | (4, false) => "N_Heelflip",
                        (3, false) | (4, true) => "N_Kickflip",
                        _ => "Kickflip",
                    })
                } else if mirrored {
                    mirror_trick(&trick)
                } else {
                    trick
                };
                if self.s.ag.has(self.n.underflip) {
                    name.insert_str(0, "U_");
                }
                let hold = match name.as_str() {
                    "Kickflip" => "KickflipHold",
                    "Heelflip" => "HeelflipHold",
                    "N_Kickflip" => "N_KickflipHold",
                    "N_Heelflip" => "N_HeelflipHold",
                    _ => "",
                };
                let id = self.p.find(&name).unwrap_or(NONE);
                let hold = if hold.is_empty() {
                    NONE
                } else {
                    self.p.find(hold).unwrap_or(NONE)
                };
                let i = &mut self.inst[op as usize];
                i.s = id;
                i.s2 = hold;
                self.set_intent(id, Some(0.0));
            }
            1 => {
                let i = &mut self.inst[op as usize];
                let first = core::mem::replace(&mut i.on, false);
                let (s, s2) = (i.s, i.s2);
                if !first && s != NONE && s2 != NONE {
                    self.set_intent(s, None);
                    self.set_intent(s2, Some(0.0));
                }
            }
            _ => {
                let i = self.inst[op as usize];
                self.set_intent(self.n.dark_catch, None);
                self.set_intent(i.s2, None);
                self.set_intent(i.s, None);
            }
        }
    }
}

fn small_number(v: u32, buf: &mut [u8; 4]) -> &str {
    let mut n = v.min(999);
    let mut len = 0;
    let mut digits = [0u8; 3];
    loop {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for k in 0..len {
        buf[k] = digits[len - 1 - k];
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("0")
}

/// SK8's intent filters (graph_intents::apply_filter): negate, abs, one-minus, clamp to 0..1 and the
/// angle flips.
pub fn apply_filter(v: f32, kind: u8) -> f32 {
    use crate::math::PI;
    match kind {
        1 => -v,
        2 => v.abs(),
        3 => {
            if v >= 0.0 {
                1.0 - v
            } else {
                -1.0 - v
            }
        }
        4 => v.clamp(0.0, 1.0),
        5 => {
            if v >= 0.0 {
                PI - v
            } else {
                -PI - v
            }
        }
        6 => {
            let r = v - PI / 2.0;
            if r < -PI { r + 2.0 * PI } else { r }
        }
        7 => {
            let r = v + PI / 2.0;
            if r > PI { r - 2.0 * PI } else { r }
        }
        _ => v,
    }
}

/// Filters 1 and 2 always; 3 only riding fakie; 4 only mirrored.
pub fn filter_chain(mut v: f32, filters: [u8; 4], fakie: bool, mirror: bool) -> f32 {
    for (k, on) in filters.into_iter().zip([true, true, fakie, mirror]) {
        if on {
            v = apply_filter(v, k);
        }
    }
    v
}

/// One step of FilterMotionGraphIntent (SK8 intent_filter): blend towards the (filtered) intent at the
/// rising or falling rate, ramped in over `rampTime`, at the `blendOut` rate once the intent is gone,
/// with optional velocity and acceleration clamps.
fn filter_step(
    f: &ops::FilterDef,
    i: &mut Inst,
    input: Option<f32>,
    dt: f32,
    fakie: bool,
    mirror: bool,
) -> f32 {
    i.t += dt;
    let target = filter_chain(
        f.scale * input.unwrap_or(f.default),
        f.filters,
        fakie,
        mirror,
    );
    let mut blend = if target < i.v { f.falling } else { f.rising };
    if input.is_some() {
        if let Some(r) = f.ramp_time {
            let k = if r > 0.0 {
                (i.t / r).clamp(0.0, 1.0)
            } else {
                1.0
            };
            blend *= k;
        }
    } else if let Some(o) = f.out {
        blend = o;
    }
    let candidate = (1.0 - blend) * i.v + blend * target;
    let mut delta = candidate - i.v;
    if let Some(l) = f.clamp_acc {
        delta = delta.clamp(i.d - l, i.d + l);
    }
    if let Some(l) = f.clamp_vel {
        delta = delta.clamp(-l, l);
    }
    i.d = delta;
    i.v += delta;
    i.v
}

impl super::Host for Host<'_, '_> {
    fn condition(&mut self, op: OpId, frame: &Frame) -> bool {
        match &self.ops[op as usize] {
            Op::Cond(c) => {
                let c = *c;
                self.cond(&c, frame)
            }
            _ => false,
        }
    }
    fn begin(&mut self, op: OpId, frame: &Frame) {
        self.inst[op as usize] = Inst::default();
        self.run(op, 0, frame);
    }
    fn update(&mut self, op: OpId, frame: &Frame) {
        self.run(op, 1, frame);
    }
    fn end(&mut self, op: OpId, frame: &Frame) {
        self.run(op, 2, frame);
    }
    fn hook(&mut self, op: OpId, frame: &Frame) {
        self.run(op, 3, frame);
    }
}
