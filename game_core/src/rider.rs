//! Drives an imported [`Character`] from the skater's state: picks a clip for what the skater is doing,
//! crossfades between clips, lays the fakie layers on top, poses and skins it once per step (mirrored for
//! a regular or switch rider: the clips are authored goofy).
//!
//! Clip roles are the `skater/clip/<role>` entries `fg_import skater` writes (see its `CLIPS` table):
//! riding (`idle`, `crouch`, `crouch_nollie`, `push/*`, `mongo/*`, `brake/*`, `tailbrake`, `nosebrake`,
//! `slide/fs|bs`, `kickturn/fs|bs`, `lean/*`), `land/*`, `manual`, `nosemanual`, `grab/*`, `bail/*`,
//! `grind/*`, `hippy/*`, `fakie` (a pose the head and upper back are taken from when riding tail
//! first), and per flick trick
//! `<trick>/pop`, `/air`, `/flip`, `/out`. A missing clip falls back to something close (`idle` last).

use alloc::vec;
use alloc::vec::Vec;

use crate::character::{Character, Local};
use crate::gesture::{Flick, Trick};
use crate::gfx::SUN;
use crate::math::{Mat3, PI, sqrtf, v3};
use crate::skater::{GrabKind, GrindKind, LandKind, Landing, Mode, Skater};

/// Crossfade time between clips (s).
const FADE: f32 = 0.12;
/// Crouch amount above which the crouch clip plays.
const CROUCH_MIN: f32 = 0.05;
/// Playback speed limits when fitting a trick into the flight.
const MIN_SPEED: f32 = 0.6;
const MAX_SPEED: f32 = 2.5;
/// How far full steering blends towards the lean pose.
const LEAN_WEIGHT: f32 = 0.85;
/// The fakie look fades in and out over this (Skate 3: 0.3 s).
const FAKIE_FADE: f32 = 0.3;
/// Bones the fakie pose takes over fully (the head looks round)...
const FAKIE_HEAD: [&str; 3] = ["NECK", "NECK1", "HEAD"];
/// ...and by the torso amount (the upper back turns with it).
const FAKIE_TORSO_BONES: [&str; 2] = ["SPINE2", "SPINE3"];
/// How much of the upper back to turn: in a manual, in a powerslide, otherwise.
const FAKIE_TORSO_MANUAL: f32 = 1.0;
const FAKIE_TORSO_SLIDE: f32 = 0.0;
const FAKIE_TORSO: f32 = 0.5;

/// Which clip to show and how far into it (seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub role: &'static str,
    pub time: f32,
}

macro_rules! stages {
    ($k:literal) => {
        [
            concat!($k, "/pop"),
            concat!($k, "/air"),
            concat!($k, "/flip"),
            concat!($k, "/out"),
        ]
    };
}

/// The clip roles of a trick, played back to back: pop, air, flip, catch (any may be missing).
pub fn trick_roles(f: Flick) -> [&'static str; 4] {
    use Trick::*;
    match (f.trick, f.nollie) {
        (Ollie, false) => stages!("ollie"),
        (Ollie, true) => stages!("nollie"),
        (Kickflip, false) => stages!("kickflip"),
        (Kickflip, true) => stages!("n_kickflip"),
        (Heelflip, false) => stages!("heelflip"),
        (Heelflip, true) => stages!("n_heelflip"),
        (PopShuvit, false) => stages!("popshuvit"),
        (PopShuvit, true) => stages!("n_popshuvit"),
        (FsPopShuvit, false) => stages!("fspopshuvit"),
        (FsPopShuvit, true) => stages!("n_fspopshuvit"),
        (Bs360Shuvit, false) => stages!("360popshuvit"),
        (Bs360Shuvit, true) => stages!("n_360popshuvit"),
        (Fs360Shuvit, false) => stages!("fs360popshuvit"),
        (Fs360Shuvit, true) => stages!("n_fs360popshuvit"),
        (VarialKickflip, false) => stages!("varialkickflip"),
        (VarialKickflip, true) => stages!("n_varialkickflip"),
        (VarialHeelflip, false) => stages!("varialheelflip"),
        (VarialHeelflip, true) => stages!("n_varialheelflip"),
        (Hardflip, false) => stages!("hardflip"),
        (Hardflip, true) => stages!("n_hardflip"),
        (InwardHeelflip, false) => stages!("inwardheelflip"),
        (InwardHeelflip, true) => stages!("n_inwardheelflip"),
        (TreFlip, false) => stages!("360flip"),
        (TreFlip, true) => stages!("n_360flip"),
        (Laserflip, false) => stages!("laserflip"),
        (Laserflip, true) => stages!("n_laserflip"),
        (Hardflip360, false) => stages!("360hardflip"),
        (Hardflip360, true) => stages!("n_360hardflip"),
        (InwardHeelflip360, false) => stages!("360inwardheelflip"),
        (InwardHeelflip360, true) => stages!("n_360inwardheelflip"),
    }
}

/// Seconds until a body at `height` above the ground, moving up at `vel_y`, lands under `gravity`.
pub fn time_to_land(height: f32, vel_y: f32, gravity: f32) -> f32 {
    (vel_y + sqrtf((vel_y * vel_y + 2.0 * gravity * height.max(0.0)).max(0.0))) / gravity
}

/// "fs" or "bs" for a turn of the board `dir` (+1 left), as seen in the clips' own goofy frame.
fn side_role(s: &Skater, dir: f32, fs: &'static str, bs: &'static str) -> &'static str {
    let mut g = dir;
    if s.mirrored() {
        g = -g;
    }
    if s.speed < 0.0 {
        g = -g;
    }
    if g > 0.0 { bs } else { fs }
}

/// Picks the clip for the skater's current state. `duration` gives a clip's length in seconds, 0 when
/// the pack does not have it. `land_in` is the predicted time left in the air: the trick stages after
/// the pop are sped up or slowed down (within limits) so the catch ends as the board lands.
pub fn pick(s: &Skater, clock: f32, land_in: f32, duration: &dyn Fn(&str) -> f32) -> Pick {
    let idle = Pick {
        role: "idle",
        time: clock,
    };
    let has = |r: &str| duration(r) > 0.0;
    match s.mode {
        // A runout reuses the bail clips for now.
        Mode::Bailed { time } | Mode::Runout { time } => {
            // The fall, then getting up (holding its last frame until the respawn).
            let fall = duration("bail/fall");
            if time < fall {
                Pick {
                    role: "bail/fall",
                    time,
                }
            } else if has("bail/getup") {
                Pick {
                    role: "bail/getup",
                    time: time - fall,
                }
            } else if has("bail/fall") {
                Pick {
                    role: "bail/fall",
                    time: fall,
                }
            } else {
                idle
            }
        }
        Mode::Grind => {
            let Some(g) = &s.grind else {
                return idle;
            };
            // This grind on this side, else the other side, else a 50-50.
            let [want, other] = grind_roles(g.kind(), g.frontside());
            let fallback = grind_roles(GrindKind::FiftyFifty, g.frontside())[0];
            match [want, other, fallback].into_iter().find(|r| has(r)) {
                Some(role) => Pick { role, time: g.time },
                None => idle,
            }
        }
        Mode::Air { time } => {
            if let Some(g) = &s.grab {
                let role = grab_role(g.kind);
                if has(role) {
                    return Pick { role, time: g.time };
                }
            }
            if s.hippie_air() {
                let into = duration("hippy/into");
                if time < into {
                    return Pick {
                        role: "hippy/into",
                        time,
                    };
                }
                if has("hippy/air") {
                    return Pick {
                        role: "hippy/air",
                        time: time - into,
                    };
                }
            }
            let Some(t) = &s.trick else {
                return idle;
            };
            let mut stages = trick_roles(t.flick);
            if !stages.iter().any(|r| has(r)) {
                stages = trick_roles(if t.flick.nollie {
                    Flick::NOLLIE
                } else {
                    Flick::OLLIE
                });
            }
            if !stages.iter().any(|r| has(r)) {
                stages = trick_roles(Flick::OLLIE);
            }
            let [pop, air, flip, out] = stages;
            let pop_d = duration(pop);
            if time < pop_d {
                return Pick { role: pop, time };
            }
            // A held kickflip plays its flip stage once per flip.
            let flips = t.flips.max(1) as usize;
            let rest: Vec<&'static str> = core::iter::once(air)
                .chain(core::iter::repeat_n(flip, flips))
                .chain(core::iter::once(out))
                .filter(|r| has(r))
                .collect();
            // Fit the rest of the trick into the flight, within sensible speeds.
            let rest_d: f32 = rest.iter().map(|r| duration(r)).sum();
            let window = time + land_in - pop_d;
            let speed = if window.is_finite() && window > 0.05 && rest_d > 0.0 {
                (rest_d / window).clamp(MIN_SPEED, MAX_SPEED)
            } else {
                1.0
            };
            let mut t = (time - pop_d) * speed;
            // Play the stages in order; the last one present holds its final frame until landing.
            let mut last = if has(pop) {
                Some(Pick {
                    role: pop,
                    time: pop_d,
                })
            } else {
                None
            };
            for role in rest {
                let d = duration(role);
                last = Some(Pick { role, time: t });
                if t < d {
                    break;
                }
                t -= d;
            }
            last.unwrap_or(idle)
        }
        Mode::Rolling => {
            if let Some(r) = &s.revert {
                let role = side_role(s, r.side, "slide/fs", "slide/bs");
                if has(role) {
                    // The whole 180 slide over the revert.
                    return Pick {
                        role,
                        time: duration(role) * (r.turned / PI).min(1.0),
                    };
                }
            }
            if let Some(sl) = &s.slide {
                let role = side_role(s, sl.side, "slide/fs", "slide/bs");
                if has(role) {
                    // Held at the sideways point of the 180 slide.
                    return Pick {
                        role,
                        time: duration(role) * 0.4 * sl.amount,
                    };
                }
            }
            if let Some(m) = &s.manual {
                let brake = if m.nose { "nosebrake" } else { "tailbrake" };
                if m.brake > 0.5 && has(brake) {
                    return Pick {
                        role: brake,
                        time: m.time,
                    };
                }
                let role = if m.nose { "nosemanual" } else { "manual" };
                if has(role) {
                    return Pick { role, time: m.time };
                }
            }
            if let Some(t) = s.hippy
                && has("hippy/antic")
            {
                return Pick {
                    role: "hippy/antic",
                    time: t,
                };
            }
            if let Some((dir, t)) = s.kickturn {
                let role = side_role(s, dir, "kickturn/fs", "kickturn/bs");
                if has(role) {
                    return Pick { role, time: t };
                }
            }
            if let Some(b) = &s.brake {
                let p = match b.released {
                    Some(r) => Pick {
                        role: "brake/out",
                        time: r,
                    },
                    None if b.time < duration("brake/into") => Pick {
                        role: "brake/into",
                        time: b.time,
                    },
                    None => Pick {
                        role: "brake/cyc",
                        time: b.time - duration("brake/into"),
                    },
                };
                if has(p.role) {
                    return p;
                }
            }
            if let Some(p) = &s.push {
                let set = if p.mongo { MONGO } else { PUSH };
                let set = if has(set[0]) || has(set[1]) {
                    set
                } else {
                    PUSH
                };
                // The foot coming down (the first push winds up with `into`), on the ground, swinging
                // back.
                let (stage, time) = p.stage();
                let (role, time) = match stage {
                    0 if p.count == 0 && has(set[0]) => (set[0], time),
                    0 => (set[2], duration(set[2]) - p.down + time),
                    1 => (set[1], time),
                    _ => (set[2], time),
                };
                let role = if has(role) { role } else { set[1] };
                if p.turning <= 0.0 && has(role) {
                    return Pick { role, time };
                }
            }
            let landing = s.landing.map(|l| (land_role(&l), l.time));
            let crouch = if s.crouch_nollie() && has("crouch_nollie") {
                "crouch_nollie"
            } else {
                "crouch"
            };
            if s.crouch > CROUCH_MIN && has(crouch) {
                Pick {
                    role: crouch,
                    time: s.crouch.min(1.0) * duration(crouch),
                }
            } else if let Some((role, time)) = landing.filter(|(r, t)| *t < duration(r)) {
                Pick { role, time }
            } else {
                idle
            }
        }
    }
}

/// The clip roles for a grind: this side first, then the other side.
pub fn grind_roles(kind: GrindKind, frontside: bool) -> [&'static str; 2] {
    let [fs, bs] = match kind {
        GrindKind::FiftyFifty => ["grind/5050_fs", "grind/5050_bs"],
        // The built-in roles have no salad, overcrook, overwilly, blunts or darkslide: the nearest.
        GrindKind::FiveO | GrindKind::Salad => ["grind/50_fs", "grind/50_bs"],
        GrindKind::Nosegrind => ["grind/ngrind_fs", "grind/ngrind_bs"],
        GrindKind::Crooked | GrindKind::Overcrook => ["grind/crooks_fs", "grind/crooks_bs"],
        GrindKind::Willy | GrindKind::Overwilly => ["grind/willy_fs", "grind/willy_bs"],
        GrindKind::Feeble => ["grind/feeble_fs", "grind/feeble_bs"],
        GrindKind::Smith => ["grind/smith_fs", "grind/smith_bs"],
        GrindKind::Boardslide | GrindKind::Darkslide => ["grind/bslide_fs", "grind/bslide_bs"],
        GrindKind::Lipslide => ["grind/lipslide_fs", "grind/lipslide_bs"],
        GrindKind::Noseslide | GrindKind::Noseblunt => ["grind/nslide_fs", "grind/nslide_bs"],
        GrindKind::Tailslide | GrindKind::Bluntslide => ["grind/tslide_fs", "grind/tslide_bs"],
    };
    if frontside { [fs, bs] } else { [bs, fs] }
}

const PUSH: [&str; 3] = ["push/into", "push/cyc1", "push/cyc2"];
const MONGO: [&str; 3] = ["mongo/into", "mongo/cyc1", "mongo/cyc2"];

pub fn grab_role(k: GrabKind) -> &'static str {
    match k {
        GrabKind::Indy => "grab/fs",
        GrabKind::Melon => "grab/bs",
        // The two-handed grab clip (the clip set's role keeps its old name).
        GrabKind::Double => "grab/mute",
    }
}

/// A hard landing's clip for the built-in rider (cosmetic only: Skate 3 has no heavy landing type).
const HEAVY_LOOK: f32 = 7.0;

pub fn land_role(l: &Landing) -> &'static str {
    match l.kind {
        LandKind::Clean | LandKind::Spin if l.impact > HEAVY_LOOK => "land/heavy",
        LandKind::Clean | LandKind::Spin => "land/clean",
        LandKind::Sketchy { frontside: true } => "land/sketchy_fs",
        LandKind::Sketchy { frontside: false } => "land/sketchy_bs",
    }
}

/// The steering lean laid over the riding clips: which pose to blend towards, and how far (0..1).
/// `None` when the skater is doing anything but rolling along (manuals, tricks, bails, landings) or
/// the pack has no lean poses.
pub fn lean(s: &Skater, base: &str, has: &dyn Fn(&str) -> bool) -> Option<(&'static str, f32)> {
    let riding = base == "idle"
        || base == "crouch"
        || base == "crouch_nollie"
        || base.starts_with("push/")
        || base.starts_with("mongo/");
    if s.mode != Mode::Rolling || s.manual.is_some() || !riding {
        return None;
    }
    let crouched = base.starts_with("crouch");
    // Stick right (positive) turns right. The poses are in the clips' frame: a mirrored rider, or one
    // rolling tail first, leans the other way for the same turn.
    let right = (s.steer >= 0.0) != (s.mirrored() != (s.speed < 0.0));
    let role = match (right, crouched) {
        (true, false) => "lean/right",
        (false, false) => "lean/left",
        (true, true) => "lean/crouch_right",
        (false, true) => "lean/crouch_left",
    };
    let w = (s.steer.abs() * LEAN_WEIGHT).min(1.0);
    (w > 0.02 && has(role)).then_some((role, w))
}

pub struct Rider {
    pub character: Character,
    cur: Pick,
    /// The clip faded out from, frozen where it was left.
    prev: Option<Pick>,
    fade: f32,
    lean: Option<(&'static str, f32)>,
    /// 0..1 how much of the fakie look is on.
    fakie: f32,
    torso: f32,
    head_bones: Vec<usize>,
    torso_bones: Vec<usize>,
    mirror: bool,
    graph_posed: bool,
    a: Vec<Local>,
    b: Vec<Local>,
    /// The state graphs' pose stack: poses and their bones' channel weights (see
    /// `graph::playback::PoseOp`), and a sampled clip's channel weights.
    stack: Vec<(Vec<Local>, Vec<f32>, bool)>,
    weights: Vec<f32>,
}

impl Rider {
    pub fn new(character: Character) -> Rider {
        let n = character.bones();
        let find = |names: &[&str]| -> Vec<usize> {
            names.iter().filter_map(|b| character.bone(b)).collect()
        };
        let (head_bones, torso_bones) = (find(&FAKIE_HEAD), find(&FAKIE_TORSO_BONES));
        let mut r = Rider {
            character,
            cur: Pick {
                role: "idle",
                time: 0.0,
            },
            prev: None,
            fade: FADE,
            lean: None,
            fakie: 0.0,
            torso: FAKIE_TORSO,
            head_bones,
            torso_bones,
            mirror: false,
            graph_posed: false,
            a: vec![Local::IDENTITY; n],
            b: vec![Local::IDENTITY; n],
            stack: Vec::new(),
            weights: vec![1.0; n],
        };
        r.pose_and_skin(Mat3::IDENTITY);
        r
    }

    pub fn current(&self) -> Pick {
        self.cur
    }

    /// The lean pose blended over the current clip, and how far.
    pub fn lean(&self) -> Option<(&'static str, f32)> {
        self.lean
    }

    /// How much of the fakie look is laid over the clip (0..1).
    pub fn fakie_weight(&self) -> f32 {
        self.fakie
    }

    /// The current clip turns the board itself (powerslides and reverts), so the drawn board should not be
    /// turned again.
    pub fn turns_board(&self) -> bool {
        self.cur.role.starts_with("slide/")
    }

    /// One step: choose the clip, blend, pose and skin. `rot` is the character's world rotation (for
    /// lighting).
    pub fn update(&mut self, s: &Skater, clock: f32, land_in: f32, dt: f32, rot: Mat3) {
        let ch = &self.character;
        let mut next = pick(s, clock, land_in, &|r| ch.duration(r));
        if next.role == self.cur.role && next.role.ends_with("/cyc") {
            // A looping clip keeps going instead of restarting.
            next.time = self.cur.time + dt;
        }
        self.lean = lean(s, next.role, &|r| ch.has_clip(r));
        if next.role != self.cur.role {
            self.prev = Some(self.cur);
            self.fade = 0.0;
        }
        // A stance change flips the mirror at once: blend from the old pose to hide the jump.
        let mirror = s.mirrored();
        if mirror != self.mirror {
            self.prev = Some(self.cur);
            self.fade = 0.0;
            self.mirror = mirror;
        }
        self.cur = next;
        self.fade = (self.fade + dt).min(FADE);
        let fakie_on = s.speed < 0.0 && matches!(s.mode, Mode::Rolling | Mode::Grind);
        let step = dt / FAKIE_FADE;
        self.fakie = (self.fakie + if fakie_on { step } else { -step }).clamp(0.0, 1.0);
        let torso = if s.manual.is_some() {
            FAKIE_TORSO_MANUAL
        } else if s.slide.is_some() {
            FAKIE_TORSO_SLIDE
        } else {
            FAKIE_TORSO
        };
        self.torso += (torso - self.torso).clamp(-0.01 * dt * 60.0, 0.01 * dt * 60.0);
        self.pose_and_skin(rot);
    }

    /// One step driven by the state graphs: blends the clips the motion graph plays (`leaves`, weights
    /// summing to 1), each sampled and mirrored for the stance its tree started in before blending, so a
    /// stance change crossfades instead of popping. False (nothing drawn new) when the clips can't be
    /// posed on this skeleton.
    pub fn update_graph(
        &mut self,
        lib: &crate::motion::Library,
        prog: &crate::graph::playback::PoseProgram,
        rot: Mat3,
    ) -> bool {
        use crate::graph::playback::PoseOp;
        let n = self.a.len();
        let mut depth = 0usize;
        for op in &prog.ops {
            match *op {
                PoseOp::Group { start, end } => {
                    if self.stack.len() <= depth {
                        self.stack
                            .push((vec![Local::IDENTITY; n], vec![1.0; n], false));
                    }
                    let (pose, chan, valid) = &mut self.stack[depth];
                    let mut total = 0.0;
                    for l in &prog.leaves[start as usize..end as usize] {
                        let Some(clip) = lib.pose(l.clip) else {
                            continue;
                        };
                        if !self
                            .character
                            .sample_clip(&clip, l.time, l.mirror, &mut self.b)
                        {
                            return false;
                        }
                        self.character
                            .channel_weights(&clip, l.mirror, &mut self.weights);
                        if total == 0.0 {
                            pose.copy_from_slice(&self.b);
                            chan.copy_from_slice(&self.weights);
                        } else {
                            let k = l.weight / (total + l.weight);
                            for ((a, b), (c, w)) in pose
                                .iter_mut()
                                .zip(&self.b)
                                .zip(chan.iter_mut().zip(&self.weights))
                            {
                                *a = a.blend(b, k);
                                *c += (w - *c) * k;
                            }
                        }
                        total += l.weight;
                    }
                    *valid = total > 0.0;
                    depth += 1;
                }
                PoseOp::Blend(_) | PoseOp::Channel { .. } => {
                    if depth < 2 {
                        return false;
                    }
                    let (low, high) = self.stack.split_at_mut(depth - 1);
                    let (a, ca, va) = &mut low[depth - 2];
                    let (b, cb, vb) = &high[0];
                    match (*va, *vb) {
                        (_, false) => {}
                        (false, true) => {
                            a.copy_from_slice(b);
                            ca.copy_from_slice(cb);
                            *va = true;
                        }
                        (true, true) => {
                            for i in 0..n {
                                let k = match *op {
                                    PoseOp::Channel { weight, first } => {
                                        (weight * if first { ca[i] } else { cb[i] }).clamp(0.0, 1.0)
                                    }
                                    PoseOp::Blend(w) => w,
                                    PoseOp::Group { .. } => 0.0,
                                };
                                a[i] = a[i].blend(&b[i], k);
                                ca[i] += (cb[i] - ca[i]) * k;
                            }
                        }
                    }
                    depth -= 1;
                }
            }
        }
        if depth == 0 || !self.stack[0].2 {
            return false;
        }
        self.a.copy_from_slice(&self.stack[0].0);
        self.graph_posed = true;
        let ch = &mut self.character;
        ch.pose(&self.a);
        let c = &rot.cols;
        ch.skin(v3(c[0].dot(SUN), c[1].dot(SUN), c[2].dot(SUN)), false);
        true
    }

    /// The last pose came from the state graphs' clips (mirrored per tree, not at skin time).
    pub fn graph_posed(&self) -> bool {
        self.graph_posed
    }

    fn pose_and_skin(&mut self, rot: Mat3) {
        self.graph_posed = false;
        let ch = &mut self.character;
        if !ch.sample(self.cur.role, self.cur.time, &mut self.a) {
            ch.sample("idle", self.cur.time, &mut self.a);
        }
        if let Some(p) = self.prev.filter(|_| self.fade < FADE)
            && ch.sample(p.role, p.time, &mut self.b)
        {
            let k = self.fade / FADE;
            for (a, b) in self.a.iter_mut().zip(&self.b) {
                *a = b.blend(a, k);
            }
        }
        // The lean poses are single frames: blend the whole body part way towards one.
        if let Some((role, w)) = self.lean
            && ch.sample(role, 0.0, &mut self.b)
        {
            for (a, b) in self.a.iter_mut().zip(&self.b) {
                *a = a.blend(b, w);
            }
        }
        // Riding tail first: the head (and some of the upper back) turns to look where the board is going.
        if self.fakie > 0.0 && ch.sample("fakie", self.cur.time, &mut self.b) {
            for &i in &self.head_bones {
                self.a[i] = self.a[i].blend(&self.b[i], self.fakie);
            }
            for &i in &self.torso_bones {
                self.a[i] = self.a[i].blend(&self.b[i], self.fakie * self.torso);
            }
        }
        ch.pose(&self.a);
        // The sun in the character's own space: R^T * sun.
        let c = &rot.cols;
        ch.skin(v3(c[0].dot(SUN), c[1].dot(SUN), c[2].dot(SUN)), self.mirror);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::Park;
    use crate::skater::{ActiveTrick, FootBrake, Grab, Landing, Manual, Powerslide, Push, Stance};
    use crate::tuning::Tuning;

    fn dur(r: &str) -> f32 {
        match r {
            "idle" => 2.0,
            "crouch" => 0.5,
            "push/into" => 0.4,
            "push/cyc1" => 0.6,
            "push/cyc2" => 0.6,
            "mongo/cyc1" => 0.6,
            "brake/into" => 0.2,
            "brake/cyc" => 1.0,
            "brake/out" => 0.3,
            "tailbrake" => 1.0,
            "slide/fs" => 1.0,
            "slide/bs" => 1.0,
            "ollie/pop" => 0.2,
            "ollie/air" => 0.5,
            "kickflip/pop" => 0.2,
            "kickflip/air" => 0.2,
            "kickflip/flip" => 0.2,
            "kickflip/out" => 0.3,
            "land/clean" => 0.6,
            "land/sketchy_fs" => 1.0,
            "manual" => 3.0,
            "grab/fs" => 1.0,
            "bail/fall" => 0.6,
            "bail/getup" => 1.5,
            "lean/left" | "lean/right" => 3.0,
            _ => 0.0,
        }
    }

    fn trick(t: Trick, flips: u8) -> Option<ActiveTrick> {
        let mut a = ActiveTrick::new(Flick::of(t));
        (a.t, a.duration, a.flips) = (0.1, 0.4, flips);
        Some(a)
    }

    #[test]
    fn picks_riding_clips() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "idle");
        s.crouch = 0.5;
        assert_eq!(
            pick(&s, 3.0, f32::INFINITY, &dur),
            Pick {
                role: "crouch",
                time: 0.25
            }
        );
        s.crouch = 0.0;
        let mut push = Push {
            t: 0.3,
            down: 0.4,
            contact: 0.15,
            recover: 0.8,
            ..Push::default()
        };
        // Foot coming down, on the ground, swinging back.
        s.push = Some(push);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "push/into");
        push.t = 0.45;
        s.push = Some(push);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "push/cyc1");
        push.t = 0.7;
        s.push = Some(push);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "push/cyc2");
        // Mongo: this pack only has its first cycle.
        push.mongo = true;
        push.t = 0.45;
        s.push = Some(push);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "mongo/cyc1");
        s.push = None;

        s.brake = Some(FootBrake {
            time: 0.1,
            released: None,
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "brake/into");
        s.brake = Some(FootBrake {
            time: 0.5,
            released: None,
        });
        let p = pick(&s, 0.0, 0.0, &dur);
        assert_eq!(p.role, "brake/cyc");
        assert!((p.time - 0.3).abs() < 1e-5);
        s.brake = Some(FootBrake {
            time: 0.5,
            released: Some(0.1),
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "brake/out");
        s.brake = None;

        s.slide = Some(Powerslide {
            side: 1.0,
            amount: 1.0,
            time: 0.4,
            ..Powerslide::default()
        });
        // Goofy, board turned left: backside.
        let p = pick(&s, 0.0, 0.0, &dur);
        assert_eq!(p.role, "slide/bs");
        assert!((p.time - 0.4).abs() < 1e-5);
        // Regular: the mirrored clip, so the other one.
        s.stance = Stance::Regular;
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "slide/fs");
        s.slide = None;

        s.manual = Some(Manual {
            nose: false,
            balance: 0.0,
            time: 1.2,
            brake: 1.0,
            ..Manual::default()
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "tailbrake");
        s.manual = Some(Manual {
            nose: true,
            balance: 0.0,
            time: 1.2,
            ..Manual::default()
        });
        // No nose manual clip in this pack: plain riding.
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "idle");
    }

    #[test]
    fn picks_trick_stages_fitted_into_the_flight() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        s.mode = Mode::Air { time: 0.1 };
        s.trick = trick(Trick::Kickflip, 1);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "kickflip/pop");
        s.mode = Mode::Air { time: 0.3 };
        let p = pick(&s, 3.0, f32::INFINITY, &dur);
        assert_eq!(p.role, "kickflip/air");
        assert!((p.time - 0.1).abs() < 1e-6);
        // Past the end: the catch holds.
        s.mode = Mode::Air { time: 2.0 };
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "kickflip/out");
        // A double kickflip plays the flip stage twice: still flipping where a single would catch.
        s.mode = Mode::Air { time: 0.65 };
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "kickflip/out");
        s.trick = trick(Trick::Kickflip, 2);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "kickflip/flip");
        // No hardflip clips in this pack: the ollie stands in.
        s.mode = Mode::Air { time: 0.3 };
        s.trick = trick(Trick::Hardflip, 1);
        assert_eq!(pick(&s, 3.0, f32::INFINITY, &dur).role, "ollie/air");
        // Fitted: 0.6 s of flight for 0.7 s of kickflip after the pop plays a little fast.
        s.trick = trick(Trick::Kickflip, 1);
        s.mode = Mode::Air { time: 0.3 };
        let p = pick(&s, 0.0, 0.3, &dur);
        assert_eq!(p.role, "kickflip/air");
        assert!((p.time - 0.1 * 0.7 / 0.4).abs() < 1e-4, "{p:?}");
    }

    #[test]
    fn landing_time_is_ballistic() {
        assert!((time_to_land(0.0, 4.0, 10.0) - 0.8).abs() < 1e-5);
        assert!((time_to_land(5.0, 0.0, 10.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn landings_grabs_and_bails() {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        s.landing = Some(Landing {
            kind: LandKind::Sketchy { frontside: true },
            time: 0.4,
            crooked: 0.3,
            impact: 2.0,
            quality: Default::default(),
        });
        assert_eq!(
            pick(&s, 0.0, 0.0, &dur),
            Pick {
                role: "land/sketchy_fs",
                time: 0.4
            }
        );
        s.landing = Some(Landing {
            kind: LandKind::Clean,
            time: 0.7,
            crooked: 0.0,
            impact: 2.0,
            quality: Default::default(),
        });
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "idle");
        s.landing = None;

        s.mode = Mode::Air { time: 0.5 };
        s.grab = Some(Grab {
            kind: GrabKind::Indy,
            time: 0.3,
            name: "INDY",
            base: "INDY",
            tweak: 0.0,
        });
        assert_eq!(pick(&s, 0.0, 0.2, &dur).role, "grab/fs");
        s.grab = None;

        s.mode = Mode::Bailed { time: 0.2 };
        assert_eq!(pick(&s, 0.0, 0.0, &dur).role, "bail/fall");
        s.mode = Mode::Bailed { time: 1.0 };
        let p = pick(&s, 0.0, 0.0, &dur);
        assert_eq!(p.role, "bail/getup");
        assert!((p.time - 0.4).abs() < 1e-5);
    }

    #[test]
    fn leans_into_turns_and_mirrors_the_lean() {
        let park = Park::demo();
        let mut s = Skater::spawn_with(&park, Tuning::BUILT_IN, Stance::Goofy);
        let has = |r: &str| dur(r) > 0.0;
        assert_eq!(lean(&s, "idle", &has), None, "straight: no lean");
        s.steer = -1.0;
        let (role, w) = lean(&s, "idle", &has).unwrap();
        assert_eq!(role, "lean/left");
        assert!((w - LEAN_WEIGHT).abs() < 1e-6);
        s.steer = 0.5;
        assert_eq!(lean(&s, "push/cyc1", &has).map(|l| l.0), Some("lean/right"));
        // Mirrored (switch) or tail first: the other pose for the same turn.
        s.switch = true;
        assert_eq!(lean(&s, "idle", &has).map(|l| l.0), Some("lean/left"));
        s.speed = -2.0;
        assert_eq!(lean(&s, "idle", &has).map(|l| l.0), Some("lean/right"));
        assert_eq!(lean(&s, "land/clean", &has), None);
        s.mode = Mode::Air { time: 0.1 };
        assert_eq!(lean(&s, "idle", &has), None);
    }

    #[test]
    fn every_trick_has_its_own_roles() {
        let mut seen = Vec::new();
        for t in crate::gesture::TRICKS {
            for nollie in [false, true] {
                let r = trick_roles(Flick { trick: t, nollie });
                assert!(!seen.contains(&r[0]), "{t:?} {nollie}");
                seen.push(r[0]);
            }
        }
        assert_eq!(trick_roles(Flick::NOLLIE)[0], "nollie/pop");
    }
}
