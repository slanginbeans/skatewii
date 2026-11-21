//! The game's side of the graph runner: builds it from `motion.fgpak`, feeds it the pad and the skater
//! each tick, and times it.

use alloc::string::String;
use alloc::vec::Vec;

use super::host::{Runner, World};
use super::input::{Gates, Intentions};
use super::ops::phys;
use super::{Error, Pack, Str};
use crate::gesture::Flick;
use crate::gesture::Shape;
use crate::input::Pad;
use crate::math::Vec3;
use crate::motion::MotionDb;
use crate::motion::NameId;
use crate::skater::{AirTrick, Family, GRAPH_NAMES, GraphControl, Mode, Skater, Stance};
use crate::tuning::Tuning;

/// Motion-library names the driver reads from the playing clips.
#[derive(Clone, Copy, Debug, Default)]
struct Clips {
    push_contact: Option<NameId>,
    brake_contact: Option<NameId>,
    jump: Option<NameId>,
    trick_height: Option<NameId>,
    danger: Option<NameId>,
    /// The stance events (SK8 skater_animation/state.rs `apply_stance_events`): `MIRRORED` and
    /// `SWITCH` toggle the rider's mirror and switch flags when a playing clip crosses them (the
    /// fakie-to-switch clips `R_SWITCH_RIDE_*`, just after they start).
    mirrored: Option<NameId>,
    switch: Option<NameId>,
}

pub struct Driver {
    pub runner: Runner<'static>,
    pub input: Intentions,
    recognised: Vec<(Str, f32)>,
    /// Microseconds the last tick took (input, both graphs, animation advance).
    pub cost_us: u32,
    /// Memory the parsed graphs take (bytes).
    pub memory: usize,
    /// Microseconds parsing the graphs and their operations took.
    pub load_us: u32,
    shown: String,
    clips: Clips,
    /// The playing clips carried the jump attribute last tick.
    jumping: bool,
    /// The motion state's path last tick (for edges).
    last_path: String,
    /// The graph's string for each grind name the physics publishes (`skater::GRAPH_NAMES`: Skate 3's
    /// `IsGrinding` grindName values, `BF_` variants included).
    grinds: Vec<Str>,
}

impl Driver {
    /// `entry`: the `motion/graph` pack entry. `sets`: gesture files by name (`skater`, `skaterls`, ...).
    /// `ops`: the `motion/graph/ops` entry (the operations typed at import), if the pack has it.
    pub fn new(
        entry: &'static [u8],
        ops: Option<&[u8]>,
        db: &MotionDb,
        sets: Vec<(&str, Vec<Shape>)>,
    ) -> Result<Driver, Error> {
        let pack = Pack::parse(entry)?;
        let typed = ops.and_then(|o| super::opcode::decode(o, entry.len()));
        let memory = pack.graphs.iter().map(|g| g.memory()).sum();
        let input = Intentions::new(&pack, sets);
        let grinds = GRAPH_NAMES
            .iter()
            .map(|n| pack.find(n).unwrap_or(super::NONE))
            .collect();
        let runner = Runner::with_ops(pack, Some(db), typed).ok_or(Error::Bad)?;
        Ok(Driver {
            runner,
            input,
            recognised: Vec::new(),
            cost_us: 0,
            memory,
            load_us: 0,
            shown: String::new(),
            clips: Clips {
                push_contact: db.name_id("PUSHCONTACT"),
                brake_contact: db.name_id("BRAKE_CONTACT"),
                jump: db.name_id("JUMP"),
                trick_height: db.name_id("TRICKHEIGHT"),
                danger: db.name_id("DANGERZONE"),
                mirrored: db.name_id("MIRRORED"),
                switch: db.name_id("SWITCH"),
            },
            jumping: false,
            last_path: String::new(),
            grinds,
        })
    }

    pub fn set_gestures(&mut self, sets: Vec<(&str, Vec<Shape>)>) {
        self.input = Intentions::new(&self.runner.pack, sets);
    }

    /// The tuning the push behaviours read (`anim_motion/pushing`).
    pub fn set_tuning(&mut self, t: &Tuning) {
        self.runner.s.push_curves = super::push::PushCurves { tuning: *t };
    }

    /// What the graphs read of the skater.
    pub fn world(&self, s: &Skater, time_to_land: Option<f32>, idle: f32) -> World {
        let phys = match s.mode {
            Mode::Rolling => phys::GROUND,
            Mode::Air { .. } => phys::AIR,
            Mode::Grind => phys::GRIND,
            Mode::Bailed { .. } | Mode::Runout { .. } => phys::WIPEOUT,
        };
        let up = s.normal;
        let surface = libm::acosf(up.y.clamp(-1.0, 1.0)) * (180.0 / crate::math::PI);
        let along = libm::asinf(s.fwd.y.clamp(-1.0, 1.0)) * (180.0 / crate::math::PI);
        World {
            phys,
            speed: s.vel.length(),
            forward_speed: s.speed,
            speed_and_slope: speed_and_slope(s, up.y),
            mirrored: s.mirrored(),
            frame_turned: s.frame_turned,
            fakie: s.fakie,
            goofy: (s.stance == Stance::Goofy) != s.switch,
            switch: s.switch,
            time_since_input: idle,
            time_to_land,
            apex: matches!(s.mode, Mode::Air { .. }) && s.vel.y <= 0.0,
            surface_slope: surface,
            skate_slope: if s.speed < 0.0 { -along } else { along },
            push_brake_disabled: surface > 45.0,
            com_vel: [s.slip, s.vel.y, s.speed],
            wants_runout: matches!(s.mode, Mode::Runout { .. }),
            wants_wipeout: matches!(s.mode, Mode::Bailed { .. }),
            grind: s.grind.as_ref().map_or(super::NONE, |g| {
                let name = g.graph_name();
                GRAPH_NAMES
                    .iter()
                    .position(|n| *n == name)
                    .map_or(super::NONE, |k| self.grinds[k])
            }),
            // A blunt (Skate 3's backslash family) and a darkslide's way out (SK8 grind/contact.rs:
            // on a ledge, 2 when the board's nose points to the high side, else 1; elsewhere by which
            // side of the rail the board is on).
            blunting: s
                .grind
                .as_ref()
                .is_some_and(|g| g.family == Family::Backslash),
            // SK8 grind_control/facing.rs: the deck's velocity against its nose, seen from above,
            // flipped riding switch and for the mirrored rider.
            grind_backwards: s.grind.is_some()
                && (s.vel.cross(s.fwd).y > 0.0) != (s.switch != s.mirrored()),
            grind_out: s.grind.as_ref().map_or(0, |g| darkslide_out(s, g)),
            slide: s
                .slide
                .as_ref()
                .map_or(0, |p| if p.side > 0.0 { 2 } else { 1 }),
            slide_ending: s.slide.as_ref().is_some_and(|p| p.releasing),
            // Only the trucks or the deck (a grind), not the wheels: SK8 TrucksOrDeckInContact reads the
            // truck and deck contacts. Counting the wheels (or nothing) wiped grabs out in the air.
            trucks_or_deck: s.mode == Mode::Grind,
            // Skate 3's processed turn (and its lean) are positive turning left, the way the board
            // yaws (`s.steer` is positive to the right): so the rider leans into the turn.
            turn: -s.steer,
            // The lean into the turn: the angle that balances the turn's centripetal pull (signed
            // like the turn, rolling either way).
            lean: libm::atanf(s.yaw_rate * s.speed / s.tuning.gravity.max(0.1)),
            ground_speed: s.ground_velocity().length(),
            lateral_up: up.cross(s.fwd).y,
            body_spin: s.spin_rate,
            vel_y: s.vel.y,
            vel_up: s.vel.dot(up),
            // SK8 PhysOutAnimation's crouching feedback: the pumping (its potential), the board's
            // absorption, the ground's compression and the least crouch.
            pump_potential: s.pump.pumping,
            crouch_force: s.pump.absorption,
            crouch_ground_force: s.pump.ground_absorption,
            crouch_min: s.pump.min_crouch,
            crouch_deck: s.pump.deck_absorption,
            // In the air, the rider's body over the board as the physics follows it (what SK8's air
            // idle reads at take-off: tucked, its legs stay in); riding, the clips' own.
            com_height: s.body_com.filter(|_| s.is_airborne()),
            // The landing classifier's signed magnitude (SK8 SetLandingData's spin).
            landing_spin: s.landing.map_or(0.0, |l| l.quality.magnitude),
            // Skate 3's landing classifier (0 straight, 1 sketchy, 2 spin, 3 slightly off).
            landing_type: s.landing.map_or(0, |l| l.quality.kind),
            body_flipping: s.body_flip.is_some(),
            pump_acc: s.pump.dv,
            slide_dir: {
                let v = s.vel.normalize_or(Vec3::ZERO);
                if v == Vec3::ZERO { 1.0 } else { v.dot(s.fwd) }
            },
            ground_acc: {
                // SK8 ground_acceleration.rs: the deck's up taken out, then the ground's, in the
                // board's frame.
                let a = s.accel - s.normal * s.accel.dot(s.normal);
                let a = a - s.contact_n * a.dot(s.contact_n);
                let side = s.contact_n.cross(s.fwd).normalize_or(Vec3::ZERO);
                [a.dot(side), a.dot(s.fwd)]
            },
            hips: match s.mode {
                // flatground's bail tips the body over sideways (as drawn) and keeps its heading
                // against the travel.
                Mode::Bailed { time } | Mode::Runout { time } => {
                    let lean = -(time * 5.0).min(1.0) * crate::math::PI * 0.48;
                    let v = s.vel.horizontal();
                    let twist = if v.length() > 0.3 {
                        let f = s.fwd.horizontal();
                        libm::atan2f(f.cross(v).y, f.dot(v))
                    } else {
                        0.0
                    };
                    [lean, twist]
                }
                _ => [0.0; 2],
            },
            ..World::default()
        }
    }

    /// One tick: pad -> action-graph intents, both graphs, animation. `clock`: a microsecond clock.
    pub fn tick(
        &mut self,
        db: &MotionDb,
        pad: &Pad,
        world: World,
        gates: Gates,
        dt: f32,
        clock: Option<fn() -> u32>,
    ) {
        let t0 = clock.map_or(0, |f| f());
        let r = &mut self.runner;
        self.input
            .produce(&r.pack, pad, gates, dt, &mut r.s.ag, &mut self.recognised);
        // Back on the board after a wipeout or runout (flatground's physics decides when): start the
        // graphs over, as Skate 3 does after a teleport.
        let off = matches!(world.phys, phys::GROUND | phys::AIR)
            && matches!(r.mg_state_root(), Some("WipeOut" | "Dismount" | "Default"));
        if off {
            r.reset(Some(db));
        }
        r.s.world = world;
        r.update(Some(db), dt);
        self.cost_us = clock.map_or(0, |f| f()).wrapping_sub(t0);
    }

    /// What the graphs decided for the physics this tick (after [`Driver::tick`]).
    pub fn control(&mut self, db: &MotionDb, t: &Tuning) -> GraphControl {
        let r = &self.runner;
        let path = r.motion_state();
        let has = |n: Option<NameId>| n.is_some_and(|n| r.attribute(db, n).is_some());
        let entered = |part: &str| path.contains(part) && !self.last_path.contains(part);
        let push_contact = has(self.clips.push_contact) || path.contains("/PushContact/");
        let mut c = GraphControl {
            push_contact,
            push_target: push_contact.then_some(r.s.push_speed),
            // `BRAKE_CONTACT` is a value: 0 while the foot comes down (the brake's `INTO` clip), 1 once it
            // drags (`CYC`). Skate 3 brakes only on a contact of strength 1 (SK8 contact_events.rs).
            brake_contact: self
                .clips
                .brake_contact
                .and_then(|n| r.attribute(db, n))
                .is_some_and(|a| a.value() >= 0.5),
            kickturn: r.s.kickturn,
            ..GraphControl::default()
        };
        if path.contains("/Manual/") {
            let lean = r
                .pack
                .find("Manual")
                .and_then(|m| r.s.mg.get(m))
                .map_or(0.0, f32::abs);
            c.manual = Some((
                path.contains("NoseManual"),
                lean.min(1.0),
                path.contains("Manual/Brake") || path.contains("Manual/Out"),
            ));
        }
        // Turning round from fakie to switch (`Switch`, `PushFromFakie`: both play `B_SWITCH`) happens
        // when its clip crosses the stance events, as in Skate 3; a library without them turns round
        // on entering those states.
        c.turn_round = if self.clips.mirrored.is_some() || self.clips.switch.is_some() {
            has(self.clips.mirrored) || has(self.clips.switch)
        } else {
            entered("/Switch/") || entered("/PushFromFakie/")
        };
        if entered("/Revert/") {
            c.revert = Some(path.contains("/Revert/Fs"));
        }
        // The takeoff clip's jump: pop on its first tick, as the trick the graph scores.
        // (On the ground or a rail: a takeoff clip still carrying its jump onto a rail is not a new pop.)
        let attr = has(self.clips.jump);
        let jump = attr && matches!(r.s.world.phys, phys::GROUND | phys::GRIND);
        if jump && !self.jumping {
            let name = r.str(r.s.trick);
            let name = name.strip_prefix("U_").unwrap_or(name);
            let flick = Flick::from_gesture(name).unwrap_or(if path.contains("Nollie") {
                Flick::NOLLIE
            } else {
                Flick::OLLIE
            });
            let gesture = r.s.mg.get(r.names.gesture_speed).unwrap_or(0.0);
            let height = self
                .clips
                .trick_height
                .and_then(|n| r.s.player.params.iter().find(|p| p.name == n))
                .map_or(gesture, |p| p.value);
            let over = r.packet(r.names.jump_override).unwrap_or(0.0);
            c.pop = Some((flick, jump_strength(height, over, t)));
        }
        self.jumping = attr;
        // In the air: the trick the clips animate, whether the board is caught yet, a dark catch.
        // The danger zone's end (seconds into the clip; untimed: the whole clip): landing within the
        // last moment of it (`DANGER_GRACE`) still counts as caught.
        c.danger = self
            .clips
            .danger
            .and_then(|n| r.attribute(db, n))
            .is_some_and(|a| {
                let end = if a.end >= 0.0 {
                    a.end
                } else {
                    r.s.player.length()
                };
                end - r.s.player.time() > DANGER_GRACE
            });
        c.dark = r.s.dark;
        // The wind-up states' `IsAnticipating` (Skate 3's anticipating flag, which its camera reads as
        // `IsPreparingToJump`).
        c.anticipating = r.flag(super::ops::Flag::Anticipating);
        // A body flip the motion graph hands the physics (`AttachIntent FrontFlip` / `BackFlip`) and
        // the grab it reads with it.
        c.body_flip = if r.packet_named("FrontFlip").is_some_and(|v| v != 0.0) {
            Some(true)
        } else if r.packet_named("BackFlip").is_some_and(|v| v != 0.0) {
            Some(false)
        } else {
            None
        };
        c.grabbing = ["Grabbing", "WantsLeftAirGrab", "WantsRightAirGrab"]
            .iter()
            .any(|n| r.packet_named(n).is_some());
        // The graph went to its wipeout state on its own (the physics' own wipeouts take it there too,
        // once the physics is already off the board).
        c.wipeout = path.starts_with("WipeOut")
            && !self.last_path.starts_with("WipeOut")
            && matches!(r.s.world.phys, phys::GROUND | phys::AIR);
        if r.s.grab_score != super::NONE {
            c.grab_name = Some(r.pack.str(r.s.grab_score));
        }
        let tx = r.s.filtered.get(r.names.tweak_x).unwrap_or(0.0);
        let ty = r.s.filtered.get(r.names.tweak_y).unwrap_or(0.0);
        c.tweak = libm::sqrtf(tx * tx + ty * ty);
        c.body_spin = r.packet(r.names.phys_body_spin).unwrap_or(0.0);
        if r.s.world.phys == phys::AIR && air_trick_state(&path) {
            let name = r.str(r.s.trick);
            let (flick, flips) = air_flick(name);
            let len = r.s.player.length();
            c.air_trick = Some(AirTrick {
                flick,
                flips,
                progress: if len > 0.0 {
                    (r.s.player.time() / len).clamp(0.0, 1.0)
                } else {
                    1.0
                },
                late: [
                    "/Lateflip/",
                    "/DarkLateflip",
                    "/DarkToLightflip",
                    "FingerFlip",
                ]
                .iter()
                .any(|k| path.contains(k)),
                id: r.s.trick,
            });
        }
        self.last_path = path;
        c
    }

    /// The motion graph's state when it changed since the last call (for the log).
    pub fn state_change(&mut self) -> Option<String> {
        let now = self.runner.motion_state();
        if now == self.shown {
            return None;
        }
        self.shown = now.clone();
        Some(now)
    }

    /// Trees played with a parameter no behaviour has set (tree, parameter), each parameter once, since
    /// the last call.
    pub fn new_unset(&mut self, db: &MotionDb) -> Vec<(&'static str, String)> {
        let r = &mut self.runner;
        let out =
            r.s.player
                .unset
                .iter()
                .map(|&(t, n)| (r.pack.str(t), String::from(db.name(n))))
                .collect();
        r.s.player.unset.clear();
        out
    }

    /// Unsupported operations reached for the first time since the last call.
    pub fn new_unsupported(&mut self) -> Vec<&'static str> {
        let r = &mut self.runner;
        let names = r.s.new_unsupported.iter().map(|&s| r.pack.str(s)).collect();
        r.s.new_unsupported.clear();
        names
    }
}

/// `PhysicsSpeedAndSlopeCompare`'s value (SK8 riding/speed_and_slope.rs, Skate 3's heading adjust
/// factor): the downhill turn torque's share at this speed and slope, from the turn torque curves
/// against speed (as a share of the heading adjust's top speed) and slope (as a share of a right
/// angle; nothing past 1.2). Kick turns need it under 0.1.
pub fn speed_and_slope(s: &Skater, normal_y: f32) -> f32 {
    let t = &s.tuning;
    let mut slope = libm::acosf(normal_y.clamp(-1.0, 1.0)) / (crate::math::PI / 2.0);
    if slope > 1.2 {
        slope = 0.0;
    }
    let top = t.heading_adjust_max_speed.max(0.01);
    let k = s.speed.abs().min(top) / top;
    t.turn_torque_vs_speed.eval(k) * t.turn_torque_vs_slope.eval(slope)
}

/// Seconds before the end of a clip's `DANGERZONE` (the board not caught yet) from which a landing
/// counts as caught. flatground's own number: Skate 3 doesn't bail on the zone itself but makes its
/// physical wipeout checks more sensitive there, which flatground's landing rules don't model.
pub const DANGER_GRACE: f32 = 0.1;

/// The motion graph's states that animate a trick in the air: a flicked trick after its takeoff, late
/// flips, finger flips, under flips and dark catches.
pub fn air_trick_state(path: &str) -> bool {
    (path.contains("/Trick/") && !path.contains("/Takeoff/"))
        || [
            "/Lateflip/",
            "/DarkLateflip",
            "/DarkToLightflip",
            "FingerFlip",
            "Underflip",
            "DarkCatch",
        ]
        .iter()
        .any(|k| path.contains(k))
}

/// What a trick the graph scores (by name) looks like on the board, and its flips (a held kickflip
/// scores `Kickflip2`, `Kickflip3` ...).
pub fn air_flick(name: &str) -> (Flick, u8) {
    use crate::gesture::Trick;
    let digits = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let flips = name[digits.len()..].parse::<u8>().unwrap_or(1).clamp(1, 4);
    let base = digits.strip_prefix("U_").unwrap_or(digits);
    if let Some(f) = Flick::from_gesture(base) {
        return (f, flips);
    }
    let l = base.to_ascii_lowercase();
    let late = l.strip_prefix("l_b_").or_else(|| l.strip_prefix("l_f_"));
    let l = late.unwrap_or(&l);
    let trick = if l.contains("varial") && l.contains("heel") {
        Trick::VarialHeelflip
    } else if l.contains("varial") {
        Trick::VarialKickflip
    } else if l.contains("heelflip") {
        Trick::Heelflip
    } else if l.contains("fs_shuv") || l.contains("fsshuv") {
        Trick::FsPopShuvit
    } else if l.contains("shuv") || l.contains("_to_") {
        Trick::PopShuvit
    } else if l.contains("flip") || l.contains("dark") {
        Trick::Kickflip
    } else {
        Trick::Ollie
    };
    (Flick::of(trick), flips)
}

/// SK8's jump finalisation (animation attribute finalization): a trick height at or under the low clamp
/// is the minimum hop, at or over the high clamp full height; a flick-strength override can only raise it.
pub fn jump_strength(height: f32, override_height: f32, t: &Tuning) -> f32 {
    let h = if height <= t.jump_clamp_low {
        0.0
    } else if height >= t.jump_clamp_high {
        1.0
    } else {
        height
    };
    h.max(override_height).clamp(0.0, 1.0)
}

/// A darkslide's way out for the graph's `GrindTrickOutTypeAllowed` (SK8 grind/contact.rs): 0 for every
/// other grind; on a ledge 2 when the board's nose points to the high side, else 1; on a rail 1 when the
/// board's middle is to its nose side of the rail, else 2.
fn darkslide_out(s: &Skater, g: &crate::skater::Grind) -> u8 {
    if g.family != Family::Darkslide {
        return 0;
    }
    if g.class.is_ledge() {
        return if g.high.dot(s.fwd) > 0.0 { 2 } else { 1 };
    }
    let across = (s.pos - g.point).horizontal();
    if across.dot(s.fwd) > 0.0 { 1 } else { 2 }
}
