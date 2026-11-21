//! The game's side of the graph runner: builds it from `motion.fgpak`, feeds it the pad and the skater
//! each tick, and times it.

use alloc::string::String;
use alloc::vec::Vec;

use super::host::{Runner, World};
use super::input::{Gates, Intentions};
use super::ops::phys;
use super::{Error, Pack, Str};
use crate::gesture::Shape;
use crate::input::Pad;
use crate::motion::MotionDb;
use crate::skater::{Mode, Skater, Stance};

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
