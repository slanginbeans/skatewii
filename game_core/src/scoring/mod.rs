//! Trick scoring, combos and lines as Skate 3 does them (SK8 `skate-core/src/scoring/`, `session.rs`,
//! `timer.rs` and `skate-game/src/scoring_runtime.rs`, Skate 3's ScoreModule, GPL-3.0).
//!
//! - Every landed trick ([`crate::skater::Event::Landed`]) carries what it was ([`Scored`]): Skate 3's
//!   trick ids ([`catalog`]) with how long they were held. Its reward is Skate 3's: the trick's points
//!   times its repetition penalty (the more often in the line, the less), a grind's, manual's or grab's
//!   held reward from the collectors' curves scaled by the trick's announcement factor, the air's
//!   distance, height, spin and body flips, and the landing's factor (clean, sketchy, onto a rail).
//! - A **sequence** is tricks linked without rolling in between (an air into a grind out into a
//!   manual): it is published three ticks after the last one ends, times the multiplier.
//! - Its raw reward fills the **combo timer**, whose points set the **multiplier** (Skate 3's levels:
//!   x1.5 at 50, x2 at 450, x3 at 800); once over x1, it also fills the **line timer** (at most 400
//!   points, drained at 50 a second, slower while a trick goes on). The published reward adds to the
//!   **line**.
//! - The line ends when its timer runs out: it is banked into the total and the multiplier drops to 1.
//!   A **bail** scores its sequence at the bail factor (Skate 3's: nothing) and ends the line too.

pub mod catalog;
pub mod rules;
pub mod timer;

use alloc::string::String;
use alloc::vec::Vec;

use crate::math::Vec3;
use crate::skater::{Mode, Skater};
pub use catalog::Catalog;
pub use rules::Rules;
use timer::{ComboTimer, PointTimer};

/// An air trick as scored: what was done in it and how it came down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Air {
    /// The flip, foot jump or hippie jump.
    pub trick: Option<u16>,
    /// The grab held longest, and for how long (s).
    pub grab: Option<(u16, f32)>,
    /// A foot plant in the air.
    pub footplant: Option<u16>,
    /// Half turns spun, whole body flips.
    pub halves: u32,
    pub body_flips: u32,
    pub landing: Landing,
}

/// How an air came down (its landing factor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Landing {
    /// On a wall, or a spin landing: no factor.
    #[default]
    Plain,
    Clean,
    Sketchy,
}

/// What a landed trick was, for its reward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scored {
    /// An air landed (on the ground, or onto a wall).
    Air(Air),
    /// A grind or slide (the trick into it, if any), held this long (s).
    Grind {
        into: Option<Air>,
        id: Option<u16>,
        /// What Skate 3 scores it as when `id` has no record (a stall: the grind it is).
        base: Option<u16>,
        held: f32,
    },
    /// A manual, revert or powerslide, held this long (s).
    Ground { id: Option<u16>, held: f32 },
    /// flatground's own points for something Skate 3 has no record of (a wallride).
    Flat(u32),
}

/// Where the rider is, for the collector the tricks are scored by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Collector {
    /// Bailed (the sequence is lost).
    #[default]
    None,
    Ground,
    Air,
    Grind,
    Offboard,
}

/// What the scoring reads of the rider each tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub collector: Collector,
    /// A trick is going on (in the air, on a rail, a manual, revert, powerslide or wallride).
    pub active: bool,
    pub manual: bool,
    pub pos: Vec3,
    /// Speed across the ground (m/s).
    pub speed: f32,
    pub switch: bool,
    pub fakie: bool,
}

impl Frame {
    pub fn of(s: &Skater) -> Frame {
        let collector = match s.mode {
            Mode::Air { .. } => Collector::Air,
            Mode::Grind => Collector::Grind,
            Mode::Rolling => Collector::Ground,
            Mode::Walk | Mode::Runout { .. } => Collector::Offboard,
            Mode::Bailed { .. } => Collector::None,
        };
        let manual = s.manual.is_some();
        Frame {
            collector,
            active: matches!(collector, Collector::Air | Collector::Grind)
                || manual
                || s.revert.is_some()
                || s.slide.is_some()
                || s.wallride.is_some()
                || s.ground_plant.is_some(),
            manual,
            pos: s.pos,
            speed: s.vel.horizontal().length(),
            switch: s.switch,
            fakie: s.fakie,
        }
    }
}

/// The air being flown (or the one that took the rider onto the rail): where it started, how high it
/// went, and its factor from the take-off.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Flight {
    start: Vec3,
    peak: f32,
    end: Vec3,
    factor: f32,
}

impl Default for Flight {
    fn default() -> Self {
        Flight {
            start: Vec3::ZERO,
            peak: 0.0,
            end: Vec3::ZERO,
            factor: 1.0,
        }
    }
}

/// Most trick names kept for the line shown.
const LINE_NAMES: usize = 8;
/// Idle ticks after which a sequence is published (Skate 3's ground collector).
const SETTLE_TICKS: u32 = 3;

/// The scoring session: the catalogue and rules, the sequence being skated, the line and the total.
#[derive(Clone, Debug, PartialEq)]
pub struct Scoring {
    pub rules: Rules,
    pub catalog: Catalog,
    combo: ComboTimer,
    line: PointTimer,
    /// The sequence's raw reward so far (before the multiplier).
    sequence: f32,
    sequence_active: bool,
    idle_ticks: u32,
    line_hold_ticks: u32,
    /// The line's score (multiplied rewards) and the lines banked before it.
    line_score: f32,
    banked: f32,
    /// The last sequence's published reward.
    pub last_reward: f32,
    repetitions: [u8; catalog::COUNT],
    collector: Collector,
    was_manual: bool,
    flight: Flight,
    /// The flight onto the rail being ground.
    into: Flight,
    /// Distance ridden in the manual or grind going on (m).
    distance: f32,
    prev: Vec3,
    /// The names of the line's tricks, and of the sequence's not yet in it.
    line_names: Vec<String>,
    sequence_names: Vec<String>,
}

impl Default for Scoring {
    fn default() -> Self {
        Scoring::new()
    }
}

impl Scoring {
    pub fn new() -> Scoring {
        Scoring {
            rules: Rules::BUILT_IN,
            catalog: Catalog::built_in(),
            combo: ComboTimer::default(),
            line: PointTimer::default(),
            sequence: 0.0,
            sequence_active: false,
            idle_ticks: 0,
            line_hold_ticks: 0,
            line_score: 0.0,
            banked: 0.0,
            last_reward: 0.0,
            repetitions: [0; catalog::COUNT],
            collector: Collector::None,
            was_manual: false,
            flight: Flight::default(),
            into: Flight::default(),
            distance: 0.0,
            prev: Vec3::ZERO,
            line_names: Vec::with_capacity(LINE_NAMES),
            sequence_names: Vec::with_capacity(LINE_NAMES),
        }
    }

    /// The total: every line banked, and the one going on.
    pub fn total(&self) -> f32 {
        self.banked + self.line_score
    }

    /// The line's score so far (published sequences, multiplied).
    pub fn line_score(&self) -> f32 {
        self.line_score
    }

    /// The sequence going on, as it would score now (multiplied), 0 when none.
    pub fn sequence_score(&self) -> f32 {
        if self.sequence_active {
            self.sequence * self.combo.multiplier
        } else {
            0.0
        }
    }

    pub fn multiplier(&self) -> f32 {
        self.combo.multiplier
    }

    /// The line timer: seconds left at the idle drain, and how full it is (0..1).
    pub fn line_seconds(&self) -> f32 {
        self.line.seconds(self.rules.line_drain)
    }

    pub fn line_fill(&self) -> f32 {
        if self.rules.line_capacity > 0.0 {
            (self.line.points / self.rules.line_capacity).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// A line or a sequence is going on.
    pub fn in_line(&self) -> bool {
        self.line.points > 0.0 || self.sequence_active
    }

    /// The names of the line's tricks so far, oldest first (the sequence's last).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.line_names
            .iter()
            .chain(self.sequence_names.iter())
            .map(String::as_str)
    }

    /// How often a trick was done in this line already.
    pub fn repetition(&self, id: u16) -> u8 {
        self.repetitions.get(id as usize).copied().unwrap_or(0)
    }

    /// The repetition penalty for doing a trick again (metrics and handplants have none).
    fn penalty(&self, id: u16) -> f32 {
        let (class, _) = catalog::kind(id);
        if class == catalog::CLASS_METRIC || class == catalog::CLASS_HANDPLANT {
            1.0
        } else {
            self.rules.repetition.eval(self.repetition(id) as f32)
        }
    }

    fn count(&mut self, id: u16) {
        if let Some(r) = self.repetitions.get_mut(id as usize) {
            *r = r.saturating_add(1);
        }
    }

    /// A held reward: `curve` (per second, against the seconds held since the trick was announced)
    /// summed tick by tick as Skate 3's collectors do.
    fn held(curve: &crate::tuning::Curve, seconds: f32) -> f32 {
        let dt = crate::game::TICK;
        let ticks = libm::roundf(seconds.max(0.0) / dt).min(60.0 * 600.0) as u32;
        (1..=ticks).map(|k| curve.eval(k as f32 * dt) * dt).sum()
    }

    /// An air's reward before its landing factor: its tricks (counted for repetition) and its metrics.
    fn air(&mut self, air: &Air, flight: &Flight) -> f32 {
        let r = self.rules;
        let mut first_penalty = None;
        let mut reward = 0.0;
        let parts = [
            air.trick.map(|t| (t, 0.0)),
            air.footplant.map(|t| (t, 0.0)),
            air.grab,
        ];
        for (id, held) in parts.into_iter().flatten() {
            let Some((points, delay)) = self.catalog.get(id) else {
                continue;
            };
            let penalty = self.penalty(id);
            first_penalty.get_or_insert(penalty);
            let factor = penalty * flight.factor;
            let points = points as f32;
            reward += if catalog::kind(id).0 == catalog::CLASS_GRAB {
                if held >= delay {
                    // Announced: its points, then its hold (Skate 3's grab collector).
                    points * factor
                        + Self::held(&r.grab_hold, held - delay)
                            * r.announcement.eval(points)
                            * factor
                } else {
                    points * factor * r.unannounced_factor
                }
            } else {
                points * factor
            };
            self.count(id);
        }
        let scale = flight.factor * first_penalty.unwrap_or(1.0);
        let across = (flight.end - flight.start).horizontal().length();
        let metrics = r.air_distance.eval(across)
            + r.air_peak.eval(flight.peak - flight.start.y)
            + r.air_height_gain.eval(flight.end.y - flight.start.y)
            + r.air_spin.eval((air.halves * 180) as f32)
            + air.body_flips as f32 * r.body_flip;
        reward + metrics * scale
    }

    /// The landing factor for the stance landed in.
    fn stance_factor(&self, f: &Frame) -> f32 {
        let mut k = 1.0;
        if f.switch && !f.fakie {
            k *= self.rules.land_switch;
        }
        if f.fakie && !f.switch {
            k *= self.rules.land_fakie;
        }
        k
    }

    /// A ground or grind trick held `held` s over the distance ridden: Skate 3's collectors' held
    /// reward (a powerslide at least its points; a revert, which isn't held, its points).
    fn held_trick(&mut self, id: u16, held: f32, grind: bool) -> f32 {
        let Some((points, delay)) = self.catalog.get(id) else {
            return 0.0;
        };
        let r = self.rules;
        let factor = self.penalty(id);
        self.count(id);
        let points = points as f32;
        let (_, kind) = catalog::kind(id);
        let carrier = if held >= delay {
            points * factor
        } else {
            points * factor * r.unannounced_factor
        };
        let curves = if grind {
            Some((&r.grind_time, &r.grind_distance))
        } else if kind == catalog::TYPE_MANUAL {
            Some((&r.manual_time, &r.manual_distance))
        } else if kind == catalog::TYPE_SLIDE {
            Some((&r.slide_time, &r.slide_distance))
        } else {
            None
        };
        let Some((time, distance)) = curves else {
            return carrier;
        };
        if held < delay {
            // Never announced: no held reward (a powerslide keeps its points).
            return if kind == catalog::TYPE_SLIDE {
                carrier
            } else {
                0.0
            };
        }
        let metric = (Self::held(time, held - delay)
            + (distance.eval(self.distance) - distance.eval(0.0)).max(0.0))
            * r.announcement.eval(points)
            * factor;
        if kind == catalog::TYPE_SLIDE {
            metric.max(carrier)
        } else {
            metric
        }
    }

    /// Scores a landed trick into the sequence (counting its tricks for repetition): returns its raw
    /// reward (before the multiplier). `name` is kept for the line shown.
    pub fn credit(&mut self, name: &str, scored: &Scored, frame: &Frame) -> f32 {
        let r = self.rules;
        let reward = match scored {
            Scored::Air(air) => {
                let mut flight = self.flight;
                flight.end = frame.pos;
                let mut k = self.stance_factor(frame);
                match air.landing {
                    Landing::Clean => k *= r.land_clean,
                    Landing::Sketchy => k *= r.land_sketchy,
                    Landing::Plain => {}
                }
                self.air(air, &flight) * k
            }
            Scored::Grind {
                into,
                id,
                base,
                held,
            } => {
                let into = into.map_or(0.0, |a| {
                    let flight = self.into;
                    self.air(&a, &flight) * self.stance_factor(frame) * r.land_grind
                });
                let id = id.filter(|i| self.catalog.get(*i).is_some()).or(*base);
                into + id.map_or(0.0, |id| self.held_trick(id, *held, true))
            }
            Scored::Ground { id, held } => id.map_or(0.0, |id| self.held_trick(id, *held, false)),
            Scored::Flat(points) => *points as f32,
        };
        self.sequence += reward;
        self.sequence_active = true;
        self.idle_ticks = 0;
        if self.sequence_names.len() + self.line_names.len() >= LINE_NAMES {
            if self.line_names.is_empty() {
                self.sequence_names.remove(0);
            } else {
                self.line_names.remove(0);
            }
        }
        self.sequence_names.push(String::from(name));
        reward
    }

    /// One tick after the rider moved (and its tricks were credited): the collectors, the timers, the
    /// sequence published once nothing goes on, the line settled. `teleported`: the rider was put
    /// somewhere else (a respawn): the line ends.
    pub fn advance(&mut self, f: &Frame, dt: f32, teleported: bool) {
        let r = self.rules;
        if f.collector != self.collector {
            if self.collector == Collector::Air {
                self.flight.end = self.prev;
                if f.collector == Collector::Grind {
                    self.into = self.flight;
                }
            }
            if f.collector == Collector::Air {
                let mut factor = 1.0;
                if self.collector == Collector::Grind {
                    factor *= r.air_after_grind;
                }
                if self.collector == Collector::Ground && self.was_manual {
                    factor *= r.air_after_manual;
                }
                if f.speed < r.stationary_speed {
                    factor *= r.stationary;
                }
                if f.switch && !f.fakie {
                    factor *= r.air_switch;
                }
                if f.fakie && !f.switch {
                    factor *= r.air_fakie;
                }
                self.flight = Flight {
                    start: f.pos,
                    peak: f.pos.y,
                    end: f.pos,
                    factor,
                };
                self.sequence_active = true;
            }
            self.collector = f.collector;
        }
        if self.collector == Collector::Air {
            self.flight.peak = self.flight.peak.max(f.pos.y);
        }
        if f.manual || self.collector == Collector::Grind {
            self.distance += (f.pos - self.prev).length();
            self.sequence_active = true;
        } else {
            self.distance = 0.0;
        }
        self.was_manual = f.manual;
        self.idle_ticks = if f.active {
            0
        } else {
            self.idle_ticks.saturating_add(1)
        };
        let (line_scale, combo_scale) = if f.active {
            match self.collector {
                Collector::Air => (r.line_scale_air, r.combo_scale_air),
                Collector::Grind => (r.line_scale_grind, r.combo_scale_grind),
                Collector::Offboard => (r.line_scale_offboard, r.combo_scale_offboard),
                _ => (r.line_scale_ground, r.combo_scale_ground),
            }
        } else {
            (1.0, 1.0)
        };
        let ground_hold = (r.ground_hold * 60.0) as u32;
        let hold = self.sequence_active
            && (self.collector != Collector::Ground || self.line_hold_ticks < ground_hold);
        let held = self.line.advance(dt, r.line_drain, line_scale, hold);
        self.line_hold_ticks = if held && self.collector == Collector::Ground {
            self.line_hold_ticks.saturating_add(1)
        } else {
            0
        };
        self.combo
            .timer
            .advance(dt, r.combo_drain, combo_scale, false);
        let bailout = self.collector == Collector::None && self.sequence_active;
        if self.sequence_active && (bailout || self.idle_ticks >= SETTLE_TICKS) {
            self.publish(bailout);
        }
        // A sequence still to be published keeps the line's repetition too (Skate 3 keeps it while a
        // collector is active; flatground's landing has no collector left by then).
        let active = f.active || self.sequence_active;
        self.settle(teleported || bailout, active);
        self.prev = f.pos;
    }

    /// Publishes the sequence (SK8 `Session::publish_sequence`): its raw reward (at the bail factor
    /// after a bail) times the multiplier from before it; it fills the combo timer, and the line timer
    /// once the multiplier is over 1.
    fn publish(&mut self, penalized: bool) {
        let r = self.rules;
        let mut raw = self.sequence;
        if penalized && r.bail_factor < 1.0 {
            raw *= r.bail_factor;
        }
        let multiplier = self.combo.multiplier;
        self.combo
            .credit(raw, r.combo_capacity, r.combo_levels, r.combo_refresh);
        if self.combo.multiplier > 1.01 {
            self.line.credit(raw, r.line_capacity);
        }
        let reward = multiplier * raw;
        if self.line.points > 0.0 {
            self.line_score += reward;
            if !penalized {
                self.line_names.append(&mut self.sequence_names);
            }
        }
        self.sequence_names.clear();
        self.last_reward = reward;
        self.sequence = 0.0;
        self.sequence_active = false;
    }

    /// SK8 `Session::settle_line`: a reset (bail, respawn) or the line timer running out ends the line
    /// (banked, the multiplier back to 1, repetition forgotten); an empty line timer banks what the
    /// line has.
    fn settle(&mut self, reset: bool, active: bool) {
        if reset || self.line.expired {
            self.line = PointTimer::default();
            self.combo.multiplier = 1.0;
            self.bank(true);
        } else if self.line.points <= 0.0 {
            self.bank(!active);
        }
    }

    fn bank(&mut self, forget: bool) {
        self.banked += self.line_score;
        self.line_score = 0.0;
        self.line_names.clear();
        if forget {
            self.repetitions = [0; catalog::COUNT];
        }
    }

    /// The rider was put somewhere else: the sequence is dropped and the line ends.
    pub fn teleport(&mut self) {
        self.sequence = 0.0;
        self.sequence_active = false;
        self.sequence_names.clear();
        self.collector = Collector::None;
        self.settle(true, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    const DT: f32 = crate::game::TICK;

    fn ground(x: f32) -> Frame {
        Frame {
            collector: Collector::Ground,
            pos: v3(x, 0.0, 0.0),
            speed: 6.0,
            ..Frame::default()
        }
    }

    fn air(x: f32, y: f32) -> Frame {
        Frame {
            collector: Collector::Air,
            active: true,
            pos: v3(x, y, 0.0),
            speed: 6.0,
            ..Frame::default()
        }
    }

    fn kickflip() -> Scored {
        Scored::Air(Air {
            trick: catalog::id("kickflip"),
            landing: Landing::Clean,
            ..Air::default()
        })
    }

    /// Rolls `ticks` ticks on the ground.
    fn roll(s: &mut Scoring, ticks: u32) {
        for _ in 0..ticks {
            s.advance(&ground(0.0), DT, false);
        }
    }

    /// A small air (a hop in place) landed as `scored`: returns its raw reward.
    fn hop(s: &mut Scoring, scored: &Scored) -> f32 {
        for k in 0..20 {
            s.advance(
                &air(0.0, 0.3 * libm::sinf(k as f32 / 20.0 * crate::math::PI)),
                DT,
                false,
            );
        }
        let r = s.credit("TRICK", scored, &ground(0.0));
        s.advance(&ground(0.0), DT, false);
        r
    }

    #[test]
    fn a_kickflip_scores_its_points_with_skate_3s_clean_landing() {
        let mut s = Scoring::new();
        roll(&mut s, 5);
        let r = hop(&mut s, &kickflip());
        // 100 points, nothing for so small an air, x1.1 clean.
        assert!((r - 110.0).abs() < 0.01, "{r}");
        assert_eq!(s.repetition(96), 1);
        // Published three idle ticks later at x1; over 50 combo points the multiplier goes to x1.5 and
        // the line starts.
        assert!(s.sequence_score() > 0.0);
        roll(&mut s, 3);
        assert_eq!(s.sequence_score(), 0.0);
        assert!((s.line_score() - 110.0).abs() < 0.01);
        assert_eq!(s.multiplier(), 1.5);
        assert!(s.line_seconds() > 2.0 && s.in_line());
        assert_eq!(s.names().collect::<Vec<_>>(), ["TRICK"]);
    }

    #[test]
    fn a_repeated_trick_scores_less_each_time_in_the_line() {
        let mut s = Scoring::new();
        roll(&mut s, 5);
        let a = hop(&mut s, &kickflip());
        roll(&mut s, 5);
        let b = hop(&mut s, &kickflip());
        roll(&mut s, 5);
        let c = hop(&mut s, &kickflip());
        assert!(
            (b - a * 0.5).abs() < 0.01 && (c - a * 0.25).abs() < 0.01,
            "{a} {b} {c}"
        );
    }

    #[test]
    fn the_multiplier_applies_to_the_next_sequence_and_the_line_runs_out() {
        let mut s = Scoring::new();
        roll(&mut s, 5);
        hop(&mut s, &kickflip());
        roll(&mut s, 3);
        let line = s.line_score();
        hop(
            &mut s,
            &Scored::Air(Air {
                trick: catalog::id("heelflip"),
                landing: Landing::Clean,
                ..Air::default()
            }),
        );
        roll(&mut s, 3);
        assert!(
            (s.line_score() - line - 110.0 * 1.5).abs() < 0.1,
            "{}",
            s.line_score()
        );
        assert_eq!(s.names().count(), 2);
        // Rolling with nothing on: the timer drains at 50 a second, then the line is banked.
        let mut ticks = 0;
        while s.in_line() {
            roll(&mut s, 1);
            ticks += 1;
        }
        assert!(ticks > 60 && ticks <= 60 * 8, "{ticks}");
        assert_eq!(s.multiplier(), 1.0);
        assert_eq!(s.line_score(), 0.0);
        assert!((s.total() - line - 165.0).abs() < 0.1);
        assert_eq!(s.repetition(96), 0, "forgotten with the line");
        assert_eq!(s.names().count(), 0);
    }

    #[test]
    fn a_bail_scores_nothing_for_its_sequence_and_ends_the_line() {
        let mut s = Scoring::new();
        roll(&mut s, 5);
        hop(&mut s, &kickflip());
        roll(&mut s, 3);
        let banked = s.line_score();
        assert_eq!(s.multiplier(), 1.5);
        // A grind, then a bail before the sequence is published.
        for _ in 0..30 {
            s.advance(
                &Frame {
                    collector: Collector::Grind,
                    active: true,
                    ..ground(0.0)
                },
                DT,
                false,
            );
        }
        let g = s.credit(
            "FS 50-50",
            &Scored::Grind {
                into: None,
                id: catalog::id("fs_50_50"),
                base: None,
                held: 0.5,
            },
            &ground(0.0),
        );
        assert!(g > 0.0);
        s.advance(
            &Frame {
                collector: Collector::None,
                ..ground(0.0)
            },
            DT,
            false,
        );
        assert_eq!(s.last_reward, 0.0, "Skate 3's bail factor is 0");
        assert_eq!(s.multiplier(), 1.0);
        assert!(!s.in_line());
        assert_eq!(s.line_score(), 0.0);
        assert!(
            (s.total() - banked).abs() < 0.01,
            "what was landed before stays"
        );
    }

    #[test]
    fn a_stall_without_a_record_scores_as_the_grind_it_is() {
        // Skate 3's catalogue has `fakie_rock` but the disc gives it no record: the rock scores as its
        // grind (a lipslide); with a record for it, as itself.
        let score = |catalog: Catalog| {
            let mut s = Scoring::new();
            s.catalog = catalog;
            roll(&mut s, 5);
            s.credit(
                "ROCK TO FAKIE",
                &Scored::Grind {
                    into: None,
                    id: catalog::id("fakie_rock"),
                    base: catalog::id("fs_lip"),
                    held: 0.5,
                },
                &ground(0.0),
            )
        };
        let as_lip = score(Catalog::built_in());
        assert!(as_lip > 0.0);
        assert_eq!(
            Catalog::built_in().get(catalog::id("fakie_rock").unwrap()),
            None
        );
        let rock = catalog::id("fakie_rock").unwrap();
        let with = Catalog::parse(&catalog::encode(&[(rock, 500, 0.2)])).unwrap();
        assert!(with.get(rock).is_some());
        assert!(score(with) > as_lip);
    }

    #[test]
    fn grinds_and_manuals_score_by_time_and_distance() {
        let mut s = Scoring::new();
        roll(&mut s, 5);
        // A 50-50 held 2 s over 8 m: Skate 3's 40 a second for the first second and its distance
        // curve, times the announcement factor of its 20 points (0.5).
        for k in 0..120 {
            s.advance(
                &Frame {
                    collector: Collector::Grind,
                    active: true,
                    ..ground(k as f32 * 8.0 / 120.0)
                },
                DT,
                false,
            );
        }
        let r = s.credit(
            "FS 50-50",
            &Scored::Grind {
                into: None,
                id: catalog::id("fs_50_50"),
                base: None,
                held: 2.0,
            },
            &ground(8.0),
        );
        let rules = Rules::BUILT_IN;
        let expect = (40.0 * 1.0 + rules.grind_distance.eval(8.0 - 8.0 / 120.0)) * 0.5;
        assert!((r - expect).abs() < 1.5, "{r} {expect}");
        // A manual too short to be announced scores nothing.
        let m = s.credit(
            "MANUAL",
            &Scored::Ground {
                id: catalog::id("tailmanual"),
                held: 0.1,
            },
            &ground(8.0),
        );
        assert_eq!(m, 0.0);
        // A revert is its points.
        let v = s.credit(
            "REVERT",
            &Scored::Ground {
                id: catalog::id("fsrevert"),
                held: 0.3,
            },
            &ground(8.0),
        );
        assert_eq!(v, 10.0);
    }

    #[test]
    fn big_airs_score_their_height_distance_and_spin() {
        let mut s = Scoring::new();
        roll(&mut s, 5);
        // 20 m across, 5 m up, landing 2 m above the take-off: a 360.
        for k in 0..=60 {
            let t = k as f32 / 60.0;
            s.advance(
                &air(20.0 * t, 5.0 * libm::sinf(t * crate::math::PI) + 2.0 * t),
                DT,
                false,
            );
        }
        let r = s.credit(
            "FS 360",
            &Scored::Air(Air {
                halves: 2,
                ..Air::default()
            }),
            &Frame {
                pos: v3(20.0, 2.0, 0.0),
                ..ground(20.0)
            },
        );
        let rules = Rules::BUILT_IN;
        let peak = 5.0 * libm::sinf(0.5 * crate::math::PI) + 1.0;
        let expect = rules.air_distance.eval(20.0)
            + rules.air_peak.eval(peak)
            + rules.air_height_gain.eval(2.0)
            + 360.0;
        assert!((r - expect).abs() < 5.0, "{r} {expect}");
        // Taken off at walking pace: a fifth of that.
        let mut s = Scoring::new();
        let slow = |p: Frame| Frame { speed: 1.0, ..p };
        s.advance(&slow(ground(0.0)), DT, false);
        s.advance(&slow(air(0.0, 0.1)), DT, false);
        let r = s.credit(
            "BS 180",
            &Scored::Air(Air {
                halves: 1,
                ..Air::default()
            }),
            &ground(0.0),
        );
        assert!((r - 180.0 * 0.2).abs() < 1.0, "{r}");
    }

    #[test]
    fn a_grab_held_scores_its_hold_and_a_short_one_a_share() {
        let mut s = Scoring::new();
        let grab = |held| {
            Scored::Air(Air {
                grab: Some((catalog::id("mutegrab").unwrap(), held)),
                ..Air::default()
            })
        };
        roll(&mut s, 5);
        let long = hop(&mut s, &grab(1.2));
        roll(&mut s, 60 * 10);
        let short = hop(&mut s, &grab(0.1));
        // Mute: 120 points; held 1 s past its 0.2 s delay, Skate 3's hold curve times its announcement
        // factor at 120 points.
        let rules = Rules::BUILT_IN;
        let hold = Scoring::held(&rules.grab_hold, 1.0) * rules.announcement.eval(120.0);
        assert!((long - 120.0 - hold).abs() < 0.5, "{long} {hold}");
        assert!((short - 120.0 * 0.3).abs() < 0.01, "{short}");
    }

    #[test]
    fn a_respawn_ends_the_line() {
        let mut s = Scoring::new();
        roll(&mut s, 5);
        hop(&mut s, &kickflip());
        roll(&mut s, 3);
        assert!(s.in_line());
        s.teleport();
        assert!(!s.in_line() && s.multiplier() == 1.0);
        assert!(s.total() > 0.0);
    }
}
