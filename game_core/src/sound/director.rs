//! The sound event layer: once a displayed frame, after the simulation's ticks, it reads what happened
//! (the frame's events) and the rider's state and turns them into [`Command`]s. It only reads: nothing
//! here feeds back into the simulation, and its own randomness (which variation plays) has its own seed.
//!
//! Cues, levels and timings follow SK8's play-tested audio (`docs/hails-additions/11-audio.md`, "Cues"):
//! "level" there is ours here, against the raw recordings. Fraction = speed / 10 m/s.
//!
//! | Sound | When | Level |
//! |---|---|---|
//! | rolling loop | wheels down, faster than 0.3 m/s (low-passed 0.25 s); band by speed, surface under the board | 0.15 x min(speed / 1.5, 1) |
//! | pop | a pop or a foot jump; a flip whoosh with a flip trick | 0.6; whoosh 0.45 |
//! | catch | a flip trick's board caught in the air | 0.5 |
//! | land | air to wheels: by impact, a heavy thud over 5 m/s, a skid on a sketchy or spin landing | 0.6 x (0.3 + 0.7 x impact / 12) |
//! | grind | on a rail: metal or ledge loop, a slide pitched down; a knock on locking on | 0.26 x (0.5 + 0.5 x fraction) |
//! | powerslide | a skid piece every 0.08 s (+-20 %) | 0.3 x (0.3 + 0.7 x fraction) |
//! | foot brake | a drag piece every 0.11 s once the foot is down | 0.25 x (0.3 + 0.7 x fraction) |
//! | push | the pushing foot touching down | 0.6 |
//! | footsteps | on foot, each foot strike of the step cycle; running steps when running | 0.6 / 0.55 |
//! | bail | the body's hit and slide by speed (three tiers), a thud, the board clattering | 0.52 x tier + 0.25 |
//! | board | stepping on it; it hitting the ground on its own | 0.6; by impact |
//! | wheel spin | taking off faster than 1 m/s, cut on landing | 0.15 x fraction |
//!
//! Every sound is placed by the camera: quieter further than 5 m (1 / distance), panned by its side.

use alloc::vec::Vec;

use super::{Bank, Command, Cue, ROLL_BANDS};
use crate::math::{Vec3, v3};
use crate::park::Terrain;
use crate::rails::RailClass;
use crate::skater::biped::Loco;
use crate::skater::possession::Hold;
use crate::skater::{Event, Skater};

/// What the director listens from: the camera's eye and its screen-right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ear {
    pub pos: Vec3,
    pub right: Vec3,
}

impl Ear {
    /// From the drawn camera.
    pub fn of(cam: &crate::gfx::Camera) -> Ear {
        let f = (cam.target - cam.eye).normalize_or(v3(0.0, 0.0, 1.0));
        Ear {
            pos: cam.eye,
            right: f.cross(cam.up).normalize_or(v3(1.0, 0.0, 0.0)),
        }
    }
}

/// One frame's view of the game.
pub struct Scene<'a> {
    pub skater: &'a Skater,
    pub events: &'a [Event],
    pub terrain: &'a dyn Terrain,
    pub ear: Ear,
    /// Shown seconds this frame.
    pub dt: f32,
    /// A menu, the replay or a loading screen: the loops stop, nothing new starts.
    pub paused: bool,
}

/// Levels (see the table in the module docs).
pub mod level {
    pub const ROLL: f32 = 0.15;
    pub const POP: f32 = 0.6;
    pub const FLIP: f32 = 0.45;
    pub const CATCH: f32 = 0.5;
    pub const LAND: f32 = 0.6;
    pub const GRIND: f32 = 0.26;
    pub const GRIND_START: f32 = 0.5;
    pub const SKID: f32 = 0.3;
    pub const BRAKE: f32 = 0.25;
    pub const PUSH: f32 = 0.6;
    pub const STEP: f32 = 0.6;
    pub const RUN_STEP: f32 = 0.55;
    pub const BAIL: f32 = 0.52;
    pub const BOARD: f32 = 0.6;
    pub const SPIN: f32 = 0.15;
}

/// Sounds closer than this (m) play at full level; further, 1 / distance.
const NEAR: f32 = 5.0;
/// How far to the side a sound can be panned.
const PAN: f32 = 0.7;
/// The rolling speed's smoothing (s), the slowest speed that rolls (m/s) and the speed of the top band.
const ROLL_SMOOTH: f32 = 0.25;
const ROLL_MIN: f32 = 0.3;
const ROLL_TOP: f32 = 10.0;
/// A band changes only after this long on the current one (s), and the cross-fade.
const BAND_HOLD: f32 = 1.0;
const CROSS_FADE: f32 = 0.15;
/// A new surface must stay under the wheels this long before the loop changes (s).
const SURFACE_HOLD: f32 = 0.25;
/// Off the ground this long (s) stops the loop (it starts again on what it lands on).
const ROLL_OFF: f32 = 0.15;
/// Within a band the pitch follows the speed by this much either way.
const BAND_PITCH: f32 = 0.06;
const SKID_EVERY: f32 = 0.08;
const BRAKE_EVERY: f32 = 0.11;
/// The brake's foot is on the ground this long after the press (s; SK8: 0.23 s into the brake clip).
const BRAKE_DOWN: f32 = 0.23;
/// Landings harder than this (m/s into the ground) get the heavy thud.
const HEAVY: f32 = 5.0;

/// Skate 3's rolling surface for a collision surface id: its audio material (bits 0..6) through the
/// game's `AudioSurfaceMap` (SK8 `cues::grain_for`, from retail's own table; asphalt is the default).
pub fn roll_cue(surface: u16) -> Cue {
    match surface & 0x7F {
        2 => Cue::RollAsphaltRough,
        4 | 66 => Cue::RollConcreteRough,
        3 | 51..=54 | 57 | 60 | 63..=65 | 92 | 93 => Cue::RollConcreteSmooth,
        6 | 7 | 41..=50 | 58 | 59 | 90 | 94 => Cue::RollWood,
        5 | 8 | 10 | 70 => Cue::RollConcreteAggregate,
        9 | 11..=40 | 67..=69 | 84 | 85 | 89 | 91 => Cue::RollMetal,
        _ => Cue::RollAsphaltSmooth,
    }
}

/// The rolling band for a speed (m/s).
pub fn band_for(speed: f32) -> usize {
    ((speed / ROLL_TOP * ROLL_BANDS as f32) as usize).min(ROLL_BANDS - 1)
}

fn band_centre(band: usize) -> f32 {
    (band as f32 + 0.5) * ROLL_TOP / ROLL_BANDS as f32
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Roll {
    voice: u16,
    cue: Cue,
    band: usize,
    since: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Prev {
    air: bool,
    grind: bool,
    walk: bool,
    trick_caught: bool,
    push_contact: bool,
    phase: f32,
    biped_air: bool,
    board_ground: bool,
    board_vy: f32,
    free: bool,
}

/// The director. Keep one for the game; it remembers the loops it started.
#[derive(Clone, Debug)]
pub struct Director {
    /// This frame's commands (the platform takes them).
    pub commands: Vec<Command>,
    next_voice: u16,
    rng: u32,
    last: [u16; Cue::COUNT],
    roll: Option<Roll>,
    speed: f32,
    /// The surface under the wheels, and one that has been there `.1` seconds.
    surface: Cue,
    surface_new: (Cue, f32),
    off_ground: f32,
    grind: Option<(u16, Cue)>,
    spin: Option<u16>,
    skid_in: f32,
    brake_in: f32,
    prev: Prev,
    frames: u32,
}

impl Default for Director {
    fn default() -> Self {
        Self::new()
    }
}

impl Director {
    pub fn new() -> Director {
        Director {
            commands: Vec::new(),
            next_voice: 1,
            rng: 0x2545_F491,
            last: [u16::MAX; Cue::COUNT],
            roll: None,
            speed: 0.0,
            surface: Cue::RollAsphaltSmooth,
            surface_new: (Cue::RollAsphaltSmooth, 0.0),
            off_ground: 1.0,
            grind: None,
            spin: None,
            skid_in: 0.0,
            brake_in: 0.0,
            prev: Prev::default(),
            frames: 0,
        }
    }

    /// The rolling loop's cue and band, if it plays.
    pub fn rolling(&self) -> Option<(Cue, usize)> {
        self.roll.map(|r| (r.cue, r.band))
    }

    /// The grind loop's cue, if one plays.
    pub fn grinding(&self) -> Option<Cue> {
        self.grind.map(|g| g.1)
    }

    fn voice(&mut self) -> u16 {
        let v = self.next_voice;
        self.next_voice = self.next_voice.checked_add(1).unwrap_or(1);
        v
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// A clip of `cue` from its clips `from..to` (shares of its list, 0..1), not the one played last when
    /// there is a choice.
    fn pick(&mut self, bank: &Bank, cue: Cue, from: f32, to: f32) -> Option<u16> {
        let all = bank.of(cue);
        if all.is_empty() {
            return None;
        }
        let n = all.len();
        let lo = ((from * n as f32) as usize).min(n - 1);
        let hi = ((to * n as f32) as usize).clamp(lo + 1, n);
        let mut k = lo + ((self.rand() * (hi - lo) as f32) as usize).min(hi - lo - 1);
        if hi - lo > 1 && all[k] == self.last[cue as usize] {
            k = lo + (k - lo + 1) % (hi - lo);
        }
        self.last[cue as usize] = all[k];
        Some(all[k])
    }

    /// Gain and pan for a sound at `at`.
    fn place(ear: &Ear, at: Vec3) -> (f32, f32) {
        let d = at - ear.pos;
        let dist = d.length();
        if dist < 1e-3 {
            return (1.0, 0.0);
        }
        let g = (NEAR / dist).min(1.0);
        let pan = (d.dot(ear.right) / dist).clamp(-1.0, 1.0) * PAN;
        (g, pan)
    }

    /// A one-shot of `cue` (clips `from..to` of its list) at `at`.
    #[allow(clippy::too_many_arguments)]
    fn shot(&mut self, bank: &Bank, ear: &Ear, cue: Cue, at: Vec3, gain: f32, from: f32, to: f32) {
        let Some(clip) = self.pick(bank, cue, from, to) else {
            return;
        };
        let (g, pan) = Self::place(ear, at);
        // A little pitch variation keeps repeats from sounding the same (SK8: +-4 %).
        let pitch = 0.96 + 0.08 * self.rand();
        let voice = self.voice();
        self.commands.push(Command::Play {
            voice,
            clip,
            gain: gain * g,
            pan,
            pitch,
            fade: 0.0,
        });
    }

    fn stop(&mut self, voice: u16, fade: f32) {
        self.commands.push(Command::Stop { voice, fade });
    }

    /// Stops every loop.
    pub fn stop_loops(&mut self, fade: f32) {
        if let Some(r) = self.roll.take() {
            self.stop(r.voice, fade);
        }
        if let Some((v, _)) = self.grind.take() {
            self.stop(v, fade);
        }
        if let Some(v) = self.spin.take() {
            self.stop(v, fade);
        }
    }

    /// One displayed frame: fills [`Director::commands`].
    pub fn frame(&mut self, scene: &Scene, bank: &Bank) {
        self.commands.clear();
        self.frames = self.frames.wrapping_add(1);
        let s = scene.skater;
        let now = Prev {
            air: s.is_airborne(),
            grind: s.grinding(),
            walk: s.walking(),
            trick_caught: s.trick.as_ref().is_some_and(|t| t.progress() >= 1.0),
            push_contact: s.graph.push_contact,
            phase: s.biped.phase,
            biped_air: s.biped.air.is_some(),
            board_ground: s.possession.board.on_ground,
            board_vy: s.possession.board.vel.y,
            free: s.possession.hold == Hold::Free,
        };
        if scene.paused {
            self.stop_loops(0.1);
            self.prev = now;
            return;
        }
        let prev = self.prev;
        self.prev = now;
        let ear = scene.ear;
        let at = s.pos;
        let speed = s.vel.length();
        let fraction = (speed / 10.0).min(1.0);
        let dt = scene.dt.max(0.0);

        for e in scene.events {
            match e {
                Event::Popped(_) => {
                    self.shot(bank, &ear, Cue::Pop, at, level::POP, 0.0, 1.0);
                    if s.trick.is_some() {
                        self.shot(bank, &ear, Cue::Flip, at, level::FLIP, 0.0, 1.0);
                    }
                }
                Event::FootJump(_) => self.shot(bank, &ear, Cue::Pop, at, level::POP, 0.0, 1.0),
                Event::Bailed => {
                    // Three tiers by the body's speed (SK8: >= 3.5, >= 7 m/s), each a third of the clips.
                    let (tier, from) = if speed >= 7.0 {
                        (1.0, 2.0 / 3.0)
                    } else if speed >= 3.5 {
                        (0.8, 1.0 / 3.0)
                    } else {
                        (0.6, 0.0)
                    };
                    let g = level::BAIL * tier + 0.25;
                    self.shot(bank, &ear, Cue::Bail, at, g, from, from + 1.0 / 3.0);
                    self.shot(bank, &ear, Cue::LandHeavy, at, 0.5 * tier, 0.0, 1.0);
                    self.shot(bank, &ear, Cue::BoardDrop, at, 0.4, 0.0, 1.0);
                    self.stop_loops(0.05);
                }
                Event::SteppedOn | Event::DroppedIn => {
                    self.shot(bank, &ear, Cue::BoardOn, at, level::BOARD, 0.0, 1.0)
                }
                Event::Respawned => self.stop_loops(0.05),
                _ => {}
            }
        }

        // Landing and taking off.
        if prev.air && !now.air {
            if let Some(v) = self.spin.take() {
                self.stop(v, 0.05);
            }
            if s.rolling() {
                let (impact, sketchy) = s
                    .landing
                    .map_or((2.0, false), |l| (l.impact, l.quality.kind != 0));
                let g = level::LAND * (0.3 + 0.7 * (impact / 12.0).min(1.0));
                self.shot(bank, &ear, Cue::Land, at, g, 0.0, 1.0);
                if impact > HEAVY {
                    let h = level::LAND * ((impact - HEAVY) / 5.0).min(1.0);
                    self.shot(bank, &ear, Cue::LandHeavy, at, h, 0.0, 1.0);
                }
                if sketchy {
                    let k = level::SKID * (0.3 + 0.7 * fraction);
                    self.shot(bank, &ear, Cue::Skid, at, k, 0.0, 1.0);
                }
            }
        }
        if now.air
            && !prev.air
            && !prev.walk
            && speed > 1.0
            && self.spin.is_none()
            && let Some(clip) = self.pick(bank, Cue::WheelSpin, 0.0, 1.0)
        {
            let (g, pan) = Self::place(&ear, at);
            let voice = self.voice();
            self.commands.push(Command::Play {
                voice,
                clip,
                gain: level::SPIN * fraction * g,
                pan,
                pitch: 0.9 + 0.2 * fraction,
                fade: 0.03,
            });
            self.spin = Some(voice);
        }
        // The board caught after a flip.
        if now.air && now.trick_caught && !prev.trick_caught {
            self.shot(bank, &ear, Cue::Catch, at, level::CATCH, 0.0, 1.0);
        }

        self.rolling_loop(scene, bank, dt);
        self.grind_loop(scene, bank, prev);

        // Powerslide and foot brake pieces.
        let rolling = s.rolling();
        match s.slide {
            Some(p) if rolling && !p.releasing && speed > 1.0 => {
                self.skid_in -= dt;
                if self.skid_in <= 0.0 {
                    let g = level::SKID * (0.3 + 0.7 * fraction) * p.amount.clamp(0.3, 1.0);
                    self.shot(bank, &ear, Cue::Skid, at, g, 0.0, 1.0);
                    self.skid_in += SKID_EVERY * (0.8 + 0.4 * self.rand());
                    self.skid_in = self.skid_in.max(0.02);
                }
            }
            _ => self.skid_in = 0.0,
        }
        match s.brake {
            Some(b) if rolling && b.released.is_none() && b.time >= BRAKE_DOWN && speed > 0.5 => {
                self.brake_in -= dt;
                if self.brake_in <= 0.0 {
                    let g = level::BRAKE * (0.3 + 0.7 * fraction);
                    self.shot(bank, &ear, Cue::Brake, at, g, 0.0, 1.0);
                    self.brake_in += BRAKE_EVERY * (0.8 + 0.4 * self.rand());
                    self.brake_in = self.brake_in.max(0.02);
                }
            }
            _ => self.brake_in = 0.0,
        }

        // The pushing foot touching down.
        if rolling && now.push_contact && !prev.push_contact {
            self.shot(bank, &ear, Cue::Step, at, level::PUSH, 0.0, 1.0);
        }

        // Footsteps: the step cycle's two foot strikes (phase 0 and 0.5).
        if now.walk && prev.walk {
            let b = &s.biped;
            if !now.biped_air && prev.biped_air {
                self.shot(bank, &ear, Cue::Step, at, level::STEP * 1.2, 0.0, 1.0);
            } else if !now.biped_air && b.speed > 0.3 {
                let crossed = (prev.phase < 0.5 && now.phase >= 0.5) || now.phase < prev.phase;
                if crossed {
                    let run = matches!(b.loco, Loco::Run | Loco::Sprint);
                    let (cue, g) = if run {
                        (Cue::RunStep, level::RUN_STEP)
                    } else {
                        (Cue::Step, level::STEP)
                    };
                    self.shot(bank, &ear, cue, at, g, 0.0, 1.0);
                }
            }
        }

        // The board on its own hitting the ground.
        if now.free && prev.free && now.board_ground && !prev.board_ground {
            let impact = (-prev.board_vy).max(0.0);
            if impact > 0.3 {
                let g = level::BOARD * (impact / 4.0).min(1.0);
                let p = s.possession.board.pos;
                self.shot(bank, &ear, Cue::BoardDrop, p, g, 0.0, 1.0);
            }
        }
    }

    fn rolling_loop(&mut self, scene: &Scene, bank: &Bank, dt: f32) {
        let s = scene.skater;
        let k = (dt / ROLL_SMOOTH).min(1.0);
        let on_wheels = s.rolling();
        // Off the wheels the speed is kept: a landing comes back in the band it left (a fresh start, at
        // the speed it starts at).
        if on_wheels {
            if self.roll.is_none() && self.off_ground > ROLL_OFF {
                self.speed = s.speed.abs();
            }
            self.speed += (s.speed.abs() - self.speed) * k;
        }
        if on_wheels {
            self.off_ground = 0.0;
        } else {
            self.off_ground += dt;
        }
        // The surface under the board (every other frame is plenty).
        if on_wheels && self.frames.is_multiple_of(2) {
            let from = s.pos + Vec3::UP * 0.3;
            let cue = scene
                .terrain
                .raycast(from, -Vec3::UP, 1.0)
                .map_or(Cue::RollAsphaltSmooth, |h| roll_cue(h.surface));
            if cue == self.surface {
                self.surface_new = (cue, 0.0);
            } else if cue == self.surface_new.0 {
                self.surface_new.1 += dt * 2.0;
            } else {
                self.surface_new = (cue, 0.0);
            }
            // Coming back onto the ground with no loop: whatever is under the wheels now.
            if self.roll.is_none() || self.surface_new.1 >= SURFACE_HOLD {
                self.surface = cue;
                self.surface_new = (cue, 0.0);
            }
        }
        let want = (on_wheels && self.speed > ROLL_MIN)
            || (!on_wheels && self.roll.is_some() && self.off_ground < ROLL_OFF);
        if !want {
            if let Some(r) = self.roll.take() {
                self.stop(r.voice, 0.05);
            }
            return;
        }
        let (g, pan) = Self::place(&scene.ear, s.pos);
        // In the air the loop is held silent until it stops (or the wheels come back down).
        let gain = if on_wheels {
            level::ROLL * (self.speed / 1.5).min(1.0) * g
        } else {
            0.0
        };
        let mut band = self.roll.map_or(band_for(self.speed), |r| r.band);
        if let Some(r) = self.roll {
            let away = (self.speed - band_centre(r.band)).abs();
            if band_for(self.speed) != r.band
                && away > ROLL_TOP / ROLL_BANDS as f32
                && r.since >= BAND_HOLD
            {
                band = band_for(self.speed);
            }
        }
        let half = ROLL_TOP / ROLL_BANDS as f32 * 0.5;
        let pitch = 1.0 + BAND_PITCH * ((self.speed - band_centre(band)) / half).clamp(-1.0, 1.0);
        let cue = self.surface;
        match self.roll {
            Some(r) if r.cue == cue && r.band == band => {
                self.commands.push(Command::Set {
                    voice: r.voice,
                    gain,
                    pan,
                    pitch,
                });
                if let Some(r) = &mut self.roll {
                    r.since += dt;
                }
            }
            old => {
                let fade = if old.is_some() { CROSS_FADE } else { 0.05 };
                if let Some(o) = old {
                    self.stop(o.voice, CROSS_FADE);
                }
                let clips = bank.of(cue);
                if let Some(&clip) = clips.get(band.min(clips.len().saturating_sub(1))) {
                    let voice = self.voice();
                    self.commands.push(Command::Play {
                        voice,
                        clip,
                        gain,
                        pan,
                        pitch,
                        fade,
                    });
                    self.roll = Some(Roll {
                        voice,
                        cue,
                        band,
                        since: 0.0,
                    });
                } else {
                    self.roll = None;
                }
            }
        }
    }

    fn grind_loop(&mut self, scene: &Scene, bank: &Bank, prev: Prev) {
        let s = scene.skater;
        let Some(gr) = s.grind.as_ref().filter(|_| s.grinding()) else {
            if let Some((v, _)) = self.grind.take() {
                self.stop(v, 0.08);
            }
            return;
        };
        let metal = matches!(gr.class, RailClass::Thin | RailClass::Fat);
        let cue = if metal {
            Cue::GrindMetal
        } else {
            Cue::GrindLedge
        };
        let fraction = (gr.speed / 10.0).min(1.0);
        let slide = gr.family.is_slide();
        let gain = level::GRIND * (0.5 + 0.5 * fraction);
        let pitch = (0.9 + 0.2 * fraction) * if slide { 0.85 } else { 1.0 };
        let (g, pan) = Self::place(&scene.ear, gr.point);
        let ear = scene.ear;
        match self.grind {
            Some((voice, c)) if c == cue => self.commands.push(Command::Set {
                voice,
                gain: gain * g,
                pan,
                pitch,
            }),
            old => {
                if let Some((v, _)) = old {
                    self.stop(v, 0.05);
                }
                if !prev.grind {
                    let k = level::GRIND_START * if metal { 1.6 } else { 1.0 };
                    self.shot(bank, &ear, Cue::Catch, gr.point, k, 0.0, 1.0);
                    if slide {
                        self.shot(bank, &ear, Cue::SlideScrape, gr.point, 0.4, 0.0, 1.0);
                    }
                }
                if let Some(clip) = self.pick(bank, cue, 0.0, 1.0) {
                    let voice = self.voice();
                    self.commands.push(Command::Play {
                        voice,
                        clip,
                        gain: gain * g,
                        pan,
                        pitch,
                        fade: 0.02,
                    });
                    self.grind = Some((voice, cue));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::Park;
    use crate::skater::Skater;

    fn scene<'a>(s: &'a Skater, events: &'a [Event], park: &'a Park, paused: bool) -> Scene<'a> {
        Scene {
            skater: s,
            events,
            terrain: park,
            ear: Ear {
                pos: s.pos + v3(0.0, 1.5, -3.5),
                right: v3(-1.0, 0.0, 0.0),
            },
            dt: 1.0 / 60.0,
            paused,
        }
    }

    fn plays(d: &Director, bank: &Bank, cue: Cue) -> Vec<(f32, f32, f32)> {
        d.commands
            .iter()
            .filter_map(|c| match *c {
                Command::Play {
                    clip,
                    gain,
                    pan,
                    pitch,
                    ..
                } if bank.of(cue).contains(&clip) => Some((gain, pan, pitch)),
                _ => None,
            })
            .collect()
    }

    fn rolling(speed: f32) -> (Park, Skater) {
        let park = Park::demo();
        let mut s = Skater::spawn(&park);
        s.force_state(crate::skater::State::PhysicsGround);
        s.speed = speed;
        s.vel = s.fwd * speed;
        (park, s)
    }

    #[test]
    fn faster_rolling_plays_a_higher_band_louder() {
        let bank = Bank::builtin();
        let mut out = Vec::new();
        for speed in [1.0f32, 5.0, 9.5] {
            let (park, s) = rolling(speed);
            let mut d = Director::new();
            // Settle the smoothed speed.
            for _ in 0..120 {
                d.frame(&scene(&s, &[], &park, false), &bank);
            }
            let (cue, band) = d.rolling().expect("rolling");
            assert_eq!(cue, Cue::RollAsphaltSmooth, "the demo park is asphalt");
            let gain = d
                .commands
                .iter()
                .find_map(|c| match *c {
                    Command::Set { gain, .. } => Some(gain),
                    _ => None,
                })
                .expect("the loop is updated each frame");
            out.push((band, gain));
        }
        assert_eq!(
            out.iter().map(|o| o.0).collect::<Vec<_>>(),
            [0, 3, 5],
            "started at its speed"
        );
        assert!(out[0].1 < out[1].1 && out[1].1 <= out[2].1, "{out:?}");
        assert!((out[2].1 - level::ROLL).abs() < 0.02);
        // Standing still: no loop.
        let (park, s) = rolling(0.0);
        let mut d = Director::new();
        for _ in 0..30 {
            d.frame(&scene(&s, &[], &park, false), &bank);
        }
        assert!(d.rolling().is_none());
    }

    #[test]
    fn within_a_band_pitch_follows_speed_and_bands_hold() {
        let bank = Bank::builtin();
        let (park, mut s) = rolling(4.5);
        let mut d = Director::new();
        let pitch_at = |d: &mut Director, s: &Skater| {
            for _ in 0..60 {
                d.frame(&scene(s, &[], &park, false), &bank);
            }
            d.commands
                .iter()
                .find_map(|c| match *c {
                    Command::Set { pitch, .. } => Some(pitch),
                    _ => None,
                })
                .unwrap()
        };
        let p1 = pitch_at(&mut d, &s);
        assert_eq!(d.rolling().unwrap().1, 2);
        s.speed = 4.9;
        s.vel = s.fwd * 4.9;
        let p2 = pitch_at(&mut d, &s);
        // Just over into band 2's neighbour: held (not a full band away), pitched up instead.
        assert_eq!(d.rolling().unwrap().1, 2);
        assert!(p2 > p1 && p2 <= 1.0 + BAND_PITCH + 1e-4);
        // Well into the next band after the hold: it changes, cross-faded.
        s.speed = 8.0;
        s.vel = s.fwd * 8.0;
        for _ in 0..180 {
            d.frame(&scene(&s, &[], &park, false), &bank);
        }
        assert_eq!(d.rolling().unwrap().1, 4);
    }

    #[test]
    fn a_pop_makes_a_pop_and_a_flip_trick_a_whoosh() {
        let bank = Bank::builtin();
        let (park, s) = rolling(5.0);
        let mut d = Director::new();
        let ev = [Event::Popped(crate::gesture::Flick::OLLIE)];
        d.frame(&scene(&s, &ev, &park, false), &bank);
        let p = plays(&d, &bank, Cue::Pop);
        assert_eq!(p.len(), 1);
        // The camera is 3.8 m away: full level, centred.
        assert!((p[0].0 - level::POP).abs() < 1e-4 && p[0].1.abs() < 0.01);
        assert!(
            plays(&d, &bank, Cue::Flip).is_empty(),
            "no trick, no whoosh"
        );
        // Nothing the next frame.
        d.frame(&scene(&s, &[], &park, false), &bank);
        assert!(plays(&d, &bank, Cue::Pop).is_empty());
    }

    #[test]
    fn landings_sound_by_impact_and_quality() {
        let bank = Bank::builtin();
        let (park, mut s) = rolling(5.0);
        let mut land = |impact: f32, kind: u8| {
            let mut d = Director::new();
            s.force_state(crate::skater::State::PhysicsAir);
            s.air_time = 0.5;
            s.states.time = 0.5;
            d.frame(&scene(&s, &[], &park, false), &bank);
            s.force_state(crate::skater::State::PhysicsGround);
            s.landing = Some(crate::skater::Landing {
                kind: crate::skater::LandKind::Clean,
                time: 0.0,
                crooked: 0.0,
                impact,
                quality: crate::skater::Quality {
                    kind,
                    magnitude: 0.0,
                    side_speed: 0.0,
                    forward_speed: 5.0,
                },
            });
            d.frame(&scene(&s, &[], &park, false), &bank);
            (
                plays(&d, &bank, Cue::Land),
                plays(&d, &bank, Cue::LandHeavy).len(),
                plays(&d, &bank, Cue::Skid).len(),
            )
        };
        let (soft, heavy, skid) = land(2.0, 0);
        assert_eq!((soft.len(), heavy, skid), (1, 0, 0));
        let (hard, heavy, _) = land(8.0, 0);
        assert!(hard[0].0 > soft[0].0 && heavy == 1);
        let (_, _, skid) = land(2.0, 1);
        assert_eq!(skid, 1, "a sketchy landing skids");
    }

    #[test]
    fn sounds_are_placed_by_the_camera() {
        let ear = Ear {
            pos: v3(0.0, 0.0, 0.0),
            right: v3(1.0, 0.0, 0.0),
        };
        let (g, pan) = Director::place(&ear, v3(3.0, 0.0, 0.0));
        assert!((g - 1.0).abs() < 1e-5 && (pan - PAN).abs() < 1e-5);
        let (g, pan) = Director::place(&ear, v3(-20.0, 0.0, 0.0));
        assert!((g - 0.25).abs() < 1e-5 && (pan + PAN).abs() < 1e-5);
        let (_, pan) = Director::place(&ear, v3(0.0, 0.0, 10.0));
        assert!(pan.abs() < 1e-5);
        // From a camera: looking along +z with y up, screen-right is -x.
        let cam = crate::gfx::Camera {
            eye: v3(0.0, 1.0, 0.0),
            target: v3(0.0, 1.0, 5.0),
            up: Vec3::UP,
            fov_y_deg: 60.0,
            near: 0.1,
            far: 100.0,
        };
        assert!((Ear::of(&cam).right - v3(-1.0, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn pausing_stops_the_loops() {
        let bank = Bank::builtin();
        let (park, s) = rolling(5.0);
        let mut d = Director::new();
        for _ in 0..10 {
            d.frame(&scene(&s, &[], &park, false), &bank);
        }
        assert!(d.rolling().is_some());
        d.frame(&scene(&s, &[], &park, true), &bank);
        assert!(d.rolling().is_none());
        assert!(matches!(d.commands[..], [Command::Stop { .. }]));
        d.frame(&scene(&s, &[], &park, true), &bank);
        assert!(d.commands.is_empty());
    }

    #[test]
    fn surfaces_map_to_skate_3s_grains() {
        assert_eq!(roll_cue(0), Cue::RollAsphaltSmooth);
        assert_eq!(roll_cue(2), Cue::RollAsphaltRough);
        assert_eq!(roll_cue(3), Cue::RollConcreteSmooth);
        assert_eq!(roll_cue(4), Cue::RollConcreteRough);
        assert_eq!(roll_cue(5), Cue::RollConcreteAggregate);
        assert_eq!(roll_cue(41), Cue::RollWood);
        assert_eq!(roll_cue(9), Cue::RollMetal);
        // Only the material bits count (the category above them is the physics').
        assert_eq!(roll_cue((8 << 7) | 41), Cue::RollWood);
        assert_eq!(roll_cue(120), Cue::RollAsphaltSmooth);
        assert_eq!((band_for(0.0), band_for(5.0), band_for(30.0)), (0, 3, 5));
    }
}
