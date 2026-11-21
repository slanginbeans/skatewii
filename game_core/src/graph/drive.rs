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
use crate::motion::MotionDb;
use crate::motion::NameId;
use crate::skater::{GraphControl, Mode, Skater, Stance};
use crate::tuning::Tuning;

/// Motion-library names the driver reads from the playing clips.
#[derive(Clone, Copy, Debug, Default)]
struct Clips {
    push_contact: Option<NameId>,
    brake_contact: Option<NameId>,
    jump: Option<NameId>,
    trick_height: Option<NameId>,
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
}

impl Driver {
    /// `entry`: the `motion/graph` pack entry. `sets`: gesture files by name (`skater`, `skaterls`, ...).
    pub fn new(
        entry: &'static [u8],
        db: &MotionDb,
        sets: Vec<(&str, Vec<Shape>)>,
    ) -> Result<Driver, Error> {
        let pack = Pack::parse(entry)?;
        let memory = pack.graphs.iter().map(|g| g.memory()).sum();
        let input = Intentions::new(&pack, sets);
        let runner = Runner::new(pack, Some(db)).ok_or(Error::Bad)?;
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
            },
            jumping: false,
            last_path: String::new(),
        })
    }

    pub fn set_gestures(&mut self, sets: Vec<(&str, Vec<Shape>)>) {
        self.input = Intentions::new(&self.runner.pack, sets);
    }

    /// What the graphs read of the skater.
    pub fn world(s: &Skater, time_to_land: Option<f32>, idle: f32) -> World {
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
            speed_and_slope: s.speed.abs() - s.fwd.y * s.speed.signum() * 2.0,
            mirrored: s.mirrored(),
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
            slide: s
                .slide
                .as_ref()
                .map_or(0, |p| if p.side > 0.0 { 2 } else { 1 }),
            slide_ending: s.slide.as_ref().is_some_and(|p| p.releasing),
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
        let mut c = GraphControl {
            push_contact: has(self.clips.push_contact) || path.contains("/PushContact/"),
            brake_contact: has(self.clips.brake_contact),
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
        c.turn_round = entered("/Switch/") || entered("/PushFromFakie/");
        if entered("/Revert/") {
            c.revert = Some(path.contains("/Revert/Fs"));
        }
        // The takeoff clip's jump: pop on its first tick, as the trick the graph scores.
        let jump = has(self.clips.jump) && r.s.world.phys == phys::GROUND;
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
        self.jumping = jump;
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

    /// Unsupported operations reached for the first time since the last call.
    pub fn new_unsupported(&mut self) -> Vec<&'static str> {
        let r = &mut self.runner;
        let names = r.s.new_unsupported.iter().map(|&s| r.pack.str(s)).collect();
        r.s.new_unsupported.clear();
        names
    }
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
