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
use super::playback::{Leaf, Player, Tree};
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
    /// Speed plus the downhill pull of the slope (m/s).
    pub speed_and_slope: f32,
    /// The graph's name of the grind being done (string id), or `NONE`.
    pub grind: Str,
    /// The rider's poses are mirrored (regular stance, or goofy riding switch).
    pub mirrored: bool,
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
    /// The rider is crouched (0..1).
    pub crouch: f32,
    pub trucks_or_deck: bool,
    pub dist_to_edge: f32,
    /// Grind trick-out side allowed: 0 normal, 1 left, 2 right.
    pub grind_out: u8,
    pub blunting: bool,
    pub on_stairs: bool,
    /// The physics' powerslide: 0 none, 1 to the left, 2 to the right; and whether it is ending.
    pub slide: u8,
    pub slide_ending: bool,
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
            crouch: 0.0,
            trucks_or_deck: true,
            dist_to_edge: 10.0,
            grind_out: 0,
            blunting: false,
            on_stairs: false,
            slide: 0,
            slide_ending: false,
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
    pub pushing: Str,
    /// Motion-library names: the construction keys `Trick` and `Random`, `manualInto`, `defaultcyc`.
    trick_key: Option<NameId>,
    random_key: Option<NameId>,
    manual_into: Option<NameId>,
    default_cyc: Option<NameId>,
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
    pub ag_ctl: Controller,
    pub mg_ctl: Controller,
    pub s: Shared,
    pub names: Names,
    /// Gesture intent ids per group, for HasGestureIntent and CreateTrickIntentFromGesture.
    gestures: Vec<Str>,
    scratch: Vec<PoseCommand>,
}

pub const ACTION: &str = "ActionGraph_OnBoard";
pub const MOTION: &str = "MotionGraph_OnBoard";

impl<'a> Runner<'a> {
    pub fn new(pack: Pack<'a>, db: Option<&MotionDb>) -> Option<Runner<'a>> {
        let action = pack
            .graphs
            .iter()
            .position(|g| pack.str(g.name) == ACTION)?;
        let motion = pack
            .graphs
            .iter()
            .position(|g| pack.str(g.name) == MOTION)?;
        let ag_ops = ops::parse(&pack, &pack.graphs[action], db);
        let mg_ops = ops::parse(&pack, &pack.graphs[motion], db);
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
            pushing: id("Pushing"),
            trick_key: lib("Trick"),
            random_key: lib("Random"),
            manual_into: lib("manualInto"),
            default_cyc: lib("defaultcyc"),
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
        let n = pack.strings();
        let ag_ctl = Controller::new(&pack.graphs[action]);
        let mg_ctl = Controller::new(&pack.graphs[motion]);
        Some(Runner {
            ag_inst: vec![Inst::default(); ag_ops.len()],
            mg_inst: vec![Inst::default(); mg_ops.len()],
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
        self.s.kickturn = None;
        self.s.tricks_disabled = (self.s.tricks_disabled - dt).max(0.0);
        self.s.time_since_kickturn += dt;
        {
            let g = &self.pack.graphs[self.action];
            let mut h = Host {
                p: &self.pack,
                g,
                ops: &self.ag_ops,
                inst: &mut self.ag_inst,
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

    /// The clips to pose this tick (weights sum to 1); commits the clips' event windows.
    pub fn leaves(&mut self, out: &mut Vec<Leaf>) {
        out.clear();
        if let Some(c) = &mut self.s.player.current {
            c.leaves(true, out, &mut self.scratch);
        }
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
                let len = s.player.length();
                (len <= 0.0 || s.player.time() >= 0.6 * len) && !s.ag.has(self.n.pushing)
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
            Cond::CrouchedForGrab => w.crouch > 0.5,
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
            Cond::BodyFlipping => false,
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
            Beh::SetManualOutTimer(len) => {
                if phase == 0 {
                    self.s.manual_out_timer = *len;
                }
            }
            Beh::UpdateManualOutTimer => {
                if phase == 1 {
                    self.s.manual_out_timer = (self.s.manual_out_timer - dt).max(0.0);
                }
            }
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
            Beh::UpdateTimeSinceKickturn | Beh::UpdateRidingFakie | Beh::SetTurning => {}
            Beh::SetGrabType(g) => {
                if phase == 0 {
                    self.s.grab = *g;
                }
            }
            Beh::ScoringGrabs => {}
            Beh::Push(p) => {
                if phase == 0 && matches!(p, PushOp::Init | PushOp::Cycle) {
                    self.s.push_count += 1;
                }
            }
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
            Beh::BodyFlippingSignal | Beh::Nothing => {}
            Beh::Unsupported(n) => self.s.unsupported(*n),
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
