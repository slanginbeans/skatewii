//! The air, as Skate 3 flies it (after SK8-ENGINE/skate-3-rust-engine, GPL-3.0: `air/body_spin.rs`,
//! `air/known/{body,trajectory,update}.rs`, `air/state/update.rs`, `air/reckoning/mod.rs`,
//! `air/trajectory/{launch,scoring,selector}.rs`):
//!
//! - [`BodySpin`]: the stick's spin, its wind-up bonus, a decaying copy of the stick that keeps a spin
//!   going after the stick is let go, and the landing assist's automatic spin.
//! - [`Selector`]: at take-off (and every few ticks until a landing is known) a cone of candidate
//!   flights around the launch velocity is followed through the park and scored; the best becomes the
//!   [`Plan`] the board then flies ("known air"), its start error blended away.
//! - [`auto_spin`]: with no stick, the body turns towards the predicted landing direction (or the other
//!   way round, landing fakie or switch, when that is nearer).
//!
//! Flights are followed with chord rays a few ticks long, a bounded number per tick (Skate 3 queries
//! them asynchronously too), so a long drop takes some ticks to resolve while short airs resolve at once.
//!
//! Grind targeting (SK8 `air/trajectory/grind{,/admission,/candidate}.rs`, `scoring.rs`): the launch's
//! own flight looks for a rail it comes down onto within the lock distance (a ledge's by where the flight
//! and the body are against its high side), not too fast across or down onto it and not turned more than
//! `GrindAdjustMaxAngle`; it is steered onto the rail (within `MaxTrajectoryAdjust`) and scores
//! `ScoreGrind`. Other flights landing near a rail are penalised by `GrindPenaltyVsDistToGrind`.

use crate::math::{PI, Vec3, atan2f, clamp, cosf, sinf, sqrtf, v3};
use crate::park::{Terrain, surface_category};
use crate::rails::{RailClass, upright};
use crate::tuning::Tuning;

/// The wind-up history (ticks).
pub const SPIN_HISTORY: usize = 30;
/// Skate 3's fixed step.
const STEP: f32 = 1.0 / 60.0;
/// Below this the stick counts as let go: the spin's input decays (SK8 body_spin.rs).
const FADE_INPUT: f32 = 1.5e-5;
/// The let-go stick's copy decays by this per tick; held, the copy follows the stick by 0.2 a tick.
const FADE: f32 = 0.96;
const FOLLOW: f32 = 0.2;
/// The automatic spin is never faster than this through the spin controller (rad/s).
const AUTO_LIMIT: f32 = 2.0;
/// The landing assist aims to finish 1.2 times as fast as the time left asks, never in less than this
/// (s), and keeps a spin going the other way round only within 45 degrees.
const AUTO_SCALE: f32 = 1.2;
const AUTO_MIN_TIME: f32 = 0.032;
const AUTO_KEEP_GOING: f32 = PI / 4.0;
/// Opposing the current spin, the assist's target counts this much; slowing it, this much.
const AUTO_REVERSE: f32 = 0.005;
const AUTO_DECEL: f32 = 0.75;
/// The board's up arrives this many ticks before touchdown, and never in fewer than `MIN_ALIGN` ticks.
const ALIGN_LEAD: f32 = 3.0;
const MIN_ALIGN: f32 = 2.0;
/// The centre of mass's distance to the deck in the air (m): what `air_up_max_delta` reads. flatground
/// doesn't simulate the body, so a riding crouch's value (ours).
pub const COM_TO_DECK: f32 = 0.8;
/// Candidates per pass (the middle one and six round the cone).
pub const CANDIDATES: usize = 7;
/// Ticks between flight queries while no landing is known.
pub const QUERY_EVERY: u32 = 7;
/// How long one chord of a followed flight is (ticks), and how many chords all candidates together
/// may follow per tick (flatground's budget for the Wii).
const CHORD_TICKS: f32 = 4.0;
pub const RAYS_PER_TICK: usize = 28;
/// Wallride candidates: the floor probe's length (m) and the score offsets for a wall that isn't one.
const WALL_PROBE: f32 = 10.0;
const NOT_A_WALLRIDE: f32 = -1000.0;
const HIGHEST_BONUS: f32 = 2000.0;

/// Skate 3's body spin controller (SK8 `air/body_spin.rs`). `speed` is the spin (rad/s, + turning the
/// board's nose towards +X seen from above, the stick to the left).
#[derive(Clone, Debug, PartialEq)]
pub struct BodySpin {
    history: [f32; SPIN_HISTORY],
    at: usize,
    /// The stick (x) this tick: its decaying copy once let go in the air.
    input: f32,
    /// This tick's wind-up (the stick moving towards where it points, x 1.5).
    derivative: f32,
    /// The stick low-passed (what decays once let go).
    filtered: f32,
    /// The strongest wind-up (signed), and seconds in the air (0: on the ground).
    pub peak: f32,
    pub time: f32,
    pub speed: f32,
}

impl Default for BodySpin {
    fn default() -> Self {
        BodySpin {
            history: [0.0; SPIN_HISTORY],
            at: 0,
            input: 0.0,
            derivative: 0.0,
            filtered: 0.0,
            peak: 0.0,
            time: 0.0,
            speed: 0.0,
        }
    }
}

impl BodySpin {
    fn record(&mut self, input: f32) {
        let d = 1.5 * (input - self.input);
        self.input = input;
        // Only movement towards where the stick points winds up.
        let d = if d * input > 0.1 {
            d.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.derivative = d;
        self.history[self.at] = d;
        self.at = (self.at + 1) % SPIN_HISTORY;
    }

    /// A tick on the ground (or a rail): the history keeps the wind-up, the spin stops.
    pub fn ground(&mut self, input: f32) {
        self.record(input);
        self.time = 0.0;
        self.peak = 0.0;
        self.speed = 0.0;
        self.filtered = self.filtered * (1.0 - FOLLOW) + input * FOLLOW;
    }

    /// The let-go spin held at nothing (after [`BodySpin::air`]) while the take-off's landing is still
    /// being looked for: Skate 3's known-air assist owns the spin from the first tick and starts it from
    /// no spin.
    pub fn hold_still(&mut self) -> f32 {
        self.speed = 0.0;
        0.0
    }

    /// A tick in the air with the stick at `input` and the landing assist's spin `auto` (rad/s, 0:
    /// none). Returns the spin speed.
    pub fn air(&mut self, t: &Tuning, input: f32, auto: f32) -> f32 {
        self.record(input);
        if self.time == 0.0 {
            self.scan_history(t);
        }
        let time = self.time;
        let accel = t.spin_change.eval(time);
        let auto_accel = t.spin_auto_change.eval(time);
        let candidate = t.spin_windup.eval(time) * self.derivative;
        if candidate.abs() > self.peak.abs() {
            self.peak = candidate;
        }
        self.time = time + STEP;
        if self.input.abs() < FADE_INPUT {
            // Let go: a decaying copy of the stick keeps the spin going.
            self.filtered *= FADE;
            self.input = self.filtered;
        } else {
            self.filtered = self.filtered * (1.0 - FOLLOW) + input * FOLLOW;
        }
        let prop = t.spin_speed.eval(self.time) * self.input;
        // A wind-up the other way is lost.
        if prop * self.peak < 0.0 {
            self.peak = 0.0;
        }
        let floor = t.spin_no_windup;
        let weight = floor + (1.0 - floor) * self.peak.abs();
        let mut target = -(weight * prop);
        let mut accel = accel;
        let old = self.speed;
        if auto.abs() > FADE_INPUT && (old * auto > 0.0 || old.abs() < 0.02) {
            accel = auto_accel;
            target = auto.clamp(-AUTO_LIMIT, AUTO_LIMIT);
        }
        // Slowing down is limited further (Skate 3's opposite-direction limit).
        let slow = accel.min(t.spin_reverse_change);
        let (lo, hi) = if old > 0.0 {
            (-slow, accel)
        } else {
            (-accel, slow)
        };
        self.speed = old + clamp(target - old, lo, hi);
        self.speed
    }

    /// The strongest wind-up in the history before take-off, weighted by how long before it was.
    fn scan_history(&mut self, t: &Tuning) {
        let mut peak: f32 = 0.0;
        let mut time = 0.0;
        // The newest entry first, one tick before take-off.
        for k in 0..SPIN_HISTORY - 1 {
            let i = (self.at + SPIN_HISTORY - 1 - k) % SPIN_HISTORY;
            time -= STEP;
            let c = t.spin_windup.eval(time) * self.history[i];
            if c.abs() > peak.abs() {
                peak = c;
            }
        }
        self.peak = peak;
    }
}

/// A ballistic flight: where it is `t` seconds after its start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub pos: Vec3,
    pub vel: Vec3,
    pub gravity: f32,
}

impl Flight {
    pub fn at(&self, t: f32) -> Vec3 {
        self.pos + self.vel * t - Vec3::UP * (0.5 * self.gravity * t * t)
    }

    pub fn vel_at(&self, t: f32) -> Vec3 {
        self.vel - Vec3::UP * (self.gravity * t)
    }

    /// Seconds to the top (0 when already coming down).
    fn apex(&self) -> f32 {
        if self.vel.y < 0.0 || self.gravity <= 0.0 {
            0.0
        } else {
            self.vel.y / self.gravity
        }
    }
}

/// Where a followed flight met the park.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// Seconds after the flight's start.
    pub time: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub surface: u16,
}

/// A rail the planned flight comes down onto (a grind is coming).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrindTarget {
    pub rail: usize,
    pub seg: usize,
    /// Where on the rail, and its direction.
    pub point: Vec3,
    pub dir: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Candidate {
    flight: Flight,
    grind: Option<GrindTarget>,
    /// How far it has been followed (s), and whether it is done.
    followed: f32,
    done: bool,
    contact: Option<Contact>,
    /// What the board aligns to there (the contact's normal, or up for walls and surfaces the board
    /// doesn't align to).
    normal: Vec3,
    score: f32,
    wall_score: f32,
}

/// The landing a flight is known to make: what the board follows ("known air").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub flight: Flight,
    /// The flight's start in air time (s), and when the board started following it.
    pub start: f32,
    pub adopted: f32,
    /// The board's offset from the flight when it was adopted (blended away).
    pub offset: Vec3,
    pub contact: Contact,
    /// The surface the board aligns to on landing.
    pub normal: Vec3,
    /// The travel direction along it at the landing (unit), when fast enough to steer the spin by.
    pub heading: Option<Vec3>,
    /// Which candidate won (0: the launch velocity itself) and which pass.
    pub index: usize,
    pub pass: u8,
    /// The flight was steered onto a rail (Skate 3's grind targeting).
    pub grind: Option<GrindTarget>,
}

impl Plan {
    /// Seconds left to the landing at air time `now`.
    pub fn remaining(&self, now: f32) -> f32 {
        self.contact.time - (now - self.start)
    }

    /// Where the board should be at air time `now`.
    pub fn target(&self, now: f32, blend_time: f32) -> Vec3 {
        let w = if blend_time > 0.0 {
            ((blend_time - (now - self.adopted)) / blend_time).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.flight.at(now - self.start) + self.offset * w
    }
}

/// The trajectory selector (SK8 `air/trajectory/selector.rs`, scoring.rs, launch.rs).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Selector {
    cands: [Option<Candidate>; CANDIDATES],
    count: usize,
    pass: u8,
    pending: bool,
    /// The launch: air time, position, velocity, the ground normal left, whether the pop was re-aimed up
    /// a vert face (and the stick's forward lean then), the travel direction.
    start: f32,
    origin: Vec3,
    velocity: Vec3,
    ground: Vec3,
    on_vert: bool,
    lean: f32,
    heading: Vec3,
    max_time: f32,
    /// Grind targeting is allowed (not just after a weak pop out of a grind).
    grind: bool,
    /// How far the legs are expected to hold the deck over the flight's path (m) when it meets a rail.
    lift: f32,
    /// Ticks until the next query while no landing is known.
    pub countdown: u32,
    /// The last result's normal when it was too short to be a known landing.
    pub suggested: Option<Vec3>,
    /// Chords followed this tick (for the cost log) and in total.
    pub rays: usize,
}

/// What a launch needs to know.
#[derive(Clone, Copy, Debug)]
pub struct Launch {
    pub time: f32,
    pub pos: Vec3,
    pub vel: Vec3,
    /// The normal of the surface left (up for a launch in the air).
    pub ground: Vec3,
    /// The pop was re-aimed up a vert face; the stick's forward lean (0..1) for the transition score.
    pub on_vert: bool,
    pub lean: f32,
    /// Grind targeting is allowed.
    pub grind: bool,
    /// How far the legs are expected to hold the deck over the flight's path (m): rails are targeted
    /// for the deck, which flies that much higher (the tucked legs; 0 without the imported rider).
    pub lift: f32,
}

impl Selector {
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// Starts a query: the cone of candidates round the launch velocity (first pass).
    pub fn launch(&mut self, t: &Tuning, l: Launch) {
        self.start = l.time;
        self.origin = l.pos;
        self.velocity = l.vel;
        self.ground = l.ground;
        self.on_vert = l.on_vert;
        self.lean = l.lean;
        self.grind = l.grind;
        self.lift = l.lift;
        let h = l.vel.horizontal();
        self.heading = h.normalize_or(Vec3::ZERO);
        self.max_time = max_time(l.vel, t);
        self.pass = 1;
        self.fill(t, l.vel, t.traj_cone_x, t.traj_cone_z);
    }

    fn fill(&mut self, t: &Tuning, v: Vec3, cone_x: f32, cone_z: f32) {
        let velocities = cone(v, cone_x, cone_z, t);
        self.count = velocities.len();
        for (i, slot) in self.cands.iter_mut().enumerate() {
            *slot = velocities.get(i).map(|&vel| Candidate {
                flight: Flight {
                    pos: self.origin,
                    vel,
                    gravity: t.gravity,
                },
                grind: None,
                followed: 0.0,
                done: false,
                contact: None,
                normal: Vec3::UP,
                score: 0.0,
                wall_score: 0.0,
            });
        }
        self.pending = true;
    }

    /// Follows the pending candidates a bounded number of chords; when all are done, scores them.
    /// Returns a known landing when this pass found one (the second, narrower pass may follow).
    pub fn update(&mut self, t: &Tuning, park: &dyn Terrain) -> Option<Plan> {
        self.rays = 0;
        if !self.pending {
            return None;
        }
        let chord = CHORD_TICKS * STEP;
        let mut budget = RAYS_PER_TICK;
        'outer: loop {
            let mut any = false;
            for c in self.cands.iter_mut().take(self.count).flatten() {
                if c.done {
                    continue;
                }
                if budget == 0 {
                    break 'outer;
                }
                budget -= 1;
                self.rays += 1;
                any = true;
                let t0 = c.followed;
                let t1 = (t0 + chord).min(self.max_time);
                let (a, b) = (c.flight.at(t0), c.flight.at(t1));
                let d = b - a;
                let len = d.length();
                if len > 1e-5
                    && let Some(h) = park.raycast(a, d * (1.0 / len), len)
                {
                    c.contact = Some(Contact {
                        time: t0 + (t1 - t0) * (h.t / len).clamp(0.0, 1.0),
                        point: h.point,
                        normal: h.normal,
                        surface: h.surface,
                    });
                    c.done = true;
                } else if t1 >= self.max_time {
                    c.done = true;
                }
                c.followed = t1;
            }
            if !any {
                break;
            }
        }
        if self
            .cands
            .iter()
            .take(self.count)
            .flatten()
            .any(|c| !c.done)
        {
            return None;
        }
        self.pending = false;
        self.score(t, park);
        let (index, best) = self
            .cands
            .iter()
            .take(self.count)
            .enumerate()
            .filter_map(|(i, c)| c.map(|c| (i, c)))
            .fold(None::<(usize, Candidate)>, |acc, (i, c)| match acc {
                Some((_, b)) if b.score >= c.score => acc,
                _ => Some((i, c)),
            })?;
        let contact = best.contact?;
        if contact.time < t.traj_min_valid_time {
            self.suggested = Some(best.normal);
            return None;
        }
        self.suggested = None;
        // The travel direction along the landing surface just before touching down.
        let before = (contact.time - STEP).max(0.0);
        let v = best.flight.vel_at(before);
        let along = v - best.normal * v.dot(best.normal);
        let heading = (along.length() >= t.min_heading_vel).then(|| along.normalize_or(Vec3::ZERO));
        let plan = Plan {
            flight: best.flight,
            start: self.start,
            adopted: self.start,
            offset: Vec3::ZERO,
            contact,
            normal: best.normal,
            heading,
            index,
            pass: self.pass,
            grind: best.grind,
        };
        // A narrower second pass round the winner when it lands on something other than the ground left
        // (not when it was steered onto a rail).
        let turned = libm::acosf(best.normal.dot(self.ground).clamp(-1.0, 1.0));
        if self.pass == 1
            && self.count >= 2
            && turned >= t.traj_second_pass_delta
            && best.grind.is_none()
        {
            self.pass = 2;
            self.fill(t, best.flight.vel, t.traj_cone_x2, t.traj_cone_z2);
        }
        Some(plan)
    }

    /// Scores every candidate (SK8 scoring.rs): the launch's own flight may be steered onto a rail
    /// (`ScoreGrind`); others landing near a rail are penalised.
    fn score(&mut self, t: &Tuning, park: &dyn Terrain) {
        let mut highest = -1_000_000.0f32;
        let pass = self.pass;
        let lift = self.lift;
        let (on_vert, lean, ground, heading, origin, grind_ok) = (
            self.on_vert,
            self.lean,
            self.ground,
            self.heading,
            self.origin,
            self.grind,
        );
        for (i, slot) in self.cands.iter_mut().take(self.count).enumerate() {
            let Some(c) = slot else { continue };
            c.score = 0.0;
            c.wall_score = 0.0;
            c.grind = None;
            let Some(natural) = c.contact else { continue };
            // Grinds: the middle flight of the first pass is steered onto a rail it comes down near.
            let mut grind_score = 0.0;
            if grind_ok {
                if i == 0
                    && pass == 1
                    && let Some((flight, target, contact)) =
                        grind_target(&c.flight, &natural, origin, lift, t, park)
                {
                    c.flight = flight;
                    c.contact = Some(contact);
                    c.grind = Some(target);
                    grind_score = t.grind.score;
                } else if let Some(d) = rail_distance(natural.point, park) {
                    grind_score = t.grind.penalty.eval(d);
                }
            }
            let Some(hit) = c.contact else { continue };
            let middle = if i == 0 && pass == 1 && !on_vert {
                t.traj_score_middle
            } else {
                0.0
            };
            c.normal = hit.normal;
            let v = c.flight.vel_at(hit.time);
            let force_dot = hit.normal.dot(v);
            let sideways = hit.normal.cross(v.normalize_or(Vec3::ZERO)).length();
            let k = t.traj_force_scalar.eval(force_dot.abs());
            let force = force_dot * t.traj_score_force * k;
            let direction = sideways * t.traj_score_direction * k;
            // Walls: a wallride when there is a floor well below, else a penalty.
            if grind_score <= 0.0 && hit.normal.y.abs() < 0.5 {
                c.wall_score = wall_score(c, &hit, origin, t, park, &mut highest);
                if c.wall_score < 0.0 {
                    c.normal = Vec3::UP;
                }
            }
            let category = if grind_score <= 1.0 {
                surface_category(hit.surface)
            } else {
                0
            };
            let surface = match category {
                6 => {
                    c.normal = Vec3::UP;
                    t.traj_score_unrideable
                }
                7 => {
                    c.normal = Vec3::UP;
                    t.traj_score_dont_align
                }
                8 => t.traj_score_dont_align,
                _ => 0.0,
            };
            let mut transition = 0.0;
            if on_vert {
                let sign = if lean >= 0.5 { 1.0 } else { -1.0 };
                let since_apex = (hit.time - c.flight.apex()).abs();
                let penalty = if since_apex >= t.traj_min_after_apex {
                    0.0
                } else {
                    -500.0
                };
                let k = (1.0 - ground.y.abs()) * sign * t.traj_score_transition;
                transition = c.normal.dot(heading) * k + penalty;
            }
            c.score = t.traj_time_bonus.eval(hit.time)
                + direction
                + 1.0
                + force
                + middle
                + grind_score
                + transition
                + surface;
        }
        // The highest wallride wins its bonus (after every candidate, as Skate 3 adds them).
        for c in self.cands.iter_mut().take(self.count).flatten() {
            if let Some(hit) = c.contact {
                if c.wall_score == 0.0 && hit.point.y > highest {
                    c.wall_score = HIGHEST_BONUS;
                }
                c.score += c.wall_score;
            }
        }
    }
}

/// A wall candidate's score: a wallride (scored by height, its flight nudged up) or a penalty.
fn wall_score(
    c: &mut Candidate,
    hit: &Contact,
    origin: Vec3,
    t: &Tuning,
    park: &dyn Terrain,
    highest: &mut f32,
) -> f32 {
    let start = hit.point + hit.normal * t.traj_wall_test_dist;
    let Some(floor) = park.raycast(start, Vec3::UP * -1.0, WALL_PROBE) else {
        return 0.0;
    };
    if floor.normal.dot(hit.normal) >= t.wallride_max_dot_floor {
        return 0.0;
    }
    let height = floor.t;
    let mut delta = origin - hit.point;
    let mut slope = delta.y;
    delta.y = 0.0;
    let across = delta.length();
    if across > 0.01 {
        slope /= across;
    }
    if slope > t.traj_wall_land_angle {
        return height * t.traj_wall_score + NOT_A_WALLRIDE;
    }
    if height < t.traj_wall_min_height {
        return height + NOT_A_WALLRIDE;
    }
    if hit.point.y > *highest {
        *highest = hit.point.y;
    }
    // A little lift onto the wall, over the flight's time (no more than the most a flight is nudged).
    let boost = t.traj_wall_boost.eval(height);
    if hit.time > 0.0 {
        let mut dv = Vec3::UP * (boost / hit.time);
        if dv.length() > t.traj_max_adjust {
            dv = dv * (t.traj_max_adjust / dv.length());
        }
        c.flight.vel += dv;
    }
    height * t.traj_wall_score
}

/// The distance from `p` to the nearest rail within reach (SK8 scoring's `penalty_input`, without the
/// lock-distance stretch), if one is near.
fn rail_distance(p: Vec3, park: &dyn Terrain) -> Option<f32> {
    park.rails()
        .iter()
        .filter(|r| r.near(p, 2.0))
        .filter_map(|r| r.nearest_3d(p).map(|n| n.2))
        .fold(None, |a: Option<f32>, d| Some(a.map_or(d, |a| a.min(d))))
}

/// A rail segment a flight comes down onto (SK8 `consider_grind_primitive`).
#[derive(Clone, Copy, Debug)]
struct RailCandidate {
    rail: usize,
    seg: usize,
    /// The flight's point when it crosses the rail's plane, the rail's point nearest it, the rail's
    /// direction, the distance between them, the time, the approach's angle to the rail (0..pi/2).
    at: Vec3,
    point: Vec3,
    dir: Vec3,
    distance: f32,
    time: f32,
    angle: f32,
}

/// Steers a flight onto a rail it comes down near (SK8 grind.rs `evaluate`, admission.rs): the flight,
/// the target and the contact it now makes, or `None`.
fn grind_target(
    path: &Flight,
    natural: &Contact,
    body: Vec3,
    lift: f32,
    t: &Tuning,
    park: &dyn Terrain,
) -> Option<(Flight, GrindTarget, Contact)> {
    let g = &t.grind;
    // The deck's flight: the board's, lifted by the legs (its result is moved back down).
    let flight = &Flight {
        pos: path.pos + Vec3::UP * lift,
        ..*path
    };
    let near = natural.point;
    let (lo, hi) = (near - v3(2.0, 0.5, 2.0), near + v3(2.0, 4.0, 2.0));
    let mut cands: alloc::vec::Vec<RailCandidate> = alloc::vec::Vec::new();
    for (ri, r) in park.rails().iter().enumerate() {
        if r.max.x < lo.x
            || r.min.x > hi.x
            || r.max.y < lo.y
            || r.min.y > hi.y
            || r.max.z < lo.z
            || r.min.z > hi.z
        {
            continue;
        }
        for (si, w) in r.points.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let delta = b - a;
            let dir = delta.normalize_or(Vec3::UP);
            let n = upright(dir);
            // The later time the flight crosses the rail's plane (a quadratic in time).
            let qa = -0.5 * flight.gravity * n.y;
            let qb = n.dot(flight.vel);
            let qc = n.dot(flight.pos - a);
            let time = if qa.abs() < 1e-6 {
                if qb.abs() < 1e-6 {
                    continue;
                }
                -qc / qb
            } else {
                let disc = qb * qb - 4.0 * qa * qc;
                if disc < 0.0 {
                    continue;
                }
                let r = sqrtf(disc);
                ((-qb + r) / (2.0 * qa)).max((-qb - r) / (2.0 * qa))
            };
            if time <= 0.0 || time > t.traj_max_time {
                continue;
            }
            let at = flight.at(time);
            let point = a + dir * dir.dot(at - a);
            let ext = delta * 0.1;
            if (point - (a - ext)).dot(point - (b + ext)) >= 0.0 {
                continue;
            }
            let distance = (at - a).cross(dir).length();
            let mut h = point - flight.pos;
            h.y = 0.0;
            let angle = if h.length() > 1e-3 {
                libm::acosf(h.normalize_or(dir).dot(dir).abs().min(1.0))
            } else {
                0.0
            };
            cands.push(RailCandidate {
                rail: ri,
                seg: si,
                at,
                point,
                dir,
                distance,
                time,
                angle,
            });
        }
    }
    // Within the lock distance, the squarest approach first (SK8 `take_best_grind`, without its height
    // preference).
    cands.retain(|c| c.distance < g.lock_dist);
    cands.sort_by(|a, b| a.angle.total_cmp(&b.angle));
    for c in cands {
        let rail = &park.rails()[c.rail];
        if c.point.y - natural.point.y < -0.4 || c.dir.y > 0.9 {
            continue;
        }
        let v = flight.vel_at(c.time);
        let across = v - c.dir * c.dir.dot(v);
        if (across.y * (1.0 - c.dir.y.abs())).abs() > g.max_speed_down {
            continue;
        }
        let s = rail.segment_start(c.seg) + (c.point - rail.points[c.seg]).length();
        let (class, high) = rail.class_at(s);
        if class == RailClass::Blocked {
            continue;
        }
        let mut h = across;
        h.y = 0.0;
        let max = if class.is_ledge() {
            g.max_speed_sq_ledge
        } else {
            g.max_speed_sq_rail
        };
        if h.dot(h) > max {
            continue;
        }
        let lock = if class.is_ledge() {
            let incoming = (c.at - c.point).dot(high) > 0.0;
            let on_high = (body - rail.points[c.seg]).dot(high) > 0.0;
            let k = if incoming { 0 } else { 2 } + if on_high { 0 } else { 1 };
            g.lock_dist * g.lock_ledge[k]
        } else {
            g.lock_dist
        }
        .max(0.1);
        if c.distance >= lock {
            continue;
        }
        let mut correction = c.point - c.at;
        let d = correction.length();
        if d >= g.lock_dist && d > 0.0 {
            let half = 0.5 * g.deck_mid_length + 0.5 * g.deck_end_size;
            correction = correction * ((d - g.tip_scalar * half) / d);
        }
        let adjusted = flight.vel + correction * (1.0 / c.time);
        let cosang = flight
            .vel
            .normalize_or(Vec3::UP)
            .dot(adjusted.normalize_or(Vec3::UP));
        if libm::acosf(cosang.clamp(-1.0, 1.0)) > g.adjust_max_angle * PI / 180.0 {
            continue;
        }
        // Steer the flight onto the rail, no more than the most a flight is nudged.
        let mut dv = correction * (1.0 / c.time);
        if dv.length() > t.traj_max_adjust {
            dv = dv * (t.traj_max_adjust / dv.length());
        }
        let steered = Flight {
            vel: flight.vel + dv,
            ..*path
        };
        let normal = landing_normal(c.dir, steered.vel_at(c.time), t);
        return Some((
            steered,
            GrindTarget {
                rail: c.rail,
                seg: c.seg,
                point: c.point,
                dir: c.dir,
            },
            Contact {
                time: c.time,
                point: steered.at(c.time),
                normal,
                surface: 0,
            },
        ));
    }
    None
}

/// The board's up landing on a rail (SK8 admission.rs `landing_normal`): up, leaned against the
/// cross-rail velocity, at most `GrindLandingMaxAngle` off the rail's up, with no part along the rail.
fn landing_normal(dir: Vec3, v: Vec3, t: &Tuning) -> Vec3 {
    let support = upright(dir);
    let across = v - dir * dir.dot(v);
    let target = (Vec3::UP - across * t.grind.landing_vel_scalar).normalize_or(Vec3::UP);
    let max = t.grind.landing_max_angle * PI / 180.0;
    let angle = libm::acosf(target.dot(support).clamp(-1.0, 1.0));
    let limited = if angle > max {
        tilt_towards(support, target, 1.0, max)
    } else {
        target
    };
    let along = limited.dot(dir);
    let n = if along > 0.0 {
        limited - dir * along
    } else {
        limited
    };
    n.normalize_or(Vec3::UP)
}

/// How long a flight is followed: until it has dropped `traj_max_drop` or `traj_max_time` passed.
fn max_time(v: Vec3, t: &Tuning) -> f32 {
    let g = t.gravity.max(0.1);
    let fall = (v.y + sqrtf((v.y * v.y + 2.0 * g * t.traj_max_drop).max(0.0))) / g;
    fall.min(t.traj_max_time)
}

/// The candidate launch velocities (SK8 `candidate_velocities`): `v` itself, then six round a cone
/// across (`cone_x` degrees) and along (`cone_z` degrees, scaled with speed) the travel, at a cone speed
/// clamped to the speed range; none faster than `v`.
fn cone(v: Vec3, cone_x: f32, cone_z: f32, t: &Tuning) -> alloc::vec::Vec<Vec3> {
    let mut out = alloc::vec::Vec::with_capacity(CANDIDATES);
    out.push(v);
    let right = Vec3::UP.cross(v).normalize_or(Vec3::ZERO);
    let forward = right.cross(Vec3::UP).normalize_or(Vec3::ZERO);
    let speed = v.length();
    let deg = PI / 180.0;
    let x = cone_x * deg;
    let z = t.traj_cone_z_vs_speed.eval(speed * 0.05) * cone_z * deg;
    let cone_speed = speed.clamp(t.traj_speed_min, t.traj_speed_max.max(t.traj_speed_min));
    let right = right * (sinf(x) * cone_speed);
    let forward = forward * (sinf(z) * cone_speed);
    let step = 2.0 * PI / (CANDIDATES - 1) as f32;
    for k in 0..CANDIDATES - 1 {
        let a = k as f32 * step;
        let c = v + right * sinf(a) + forward * cosf(a);
        let len = c.length();
        out.push(if len > 1e-6 {
            c * (len.min(speed) / len)
        } else {
            c
        });
    }
    out
}

/// The landing assist's spin (SK8 KnownAir `CalculateBodySpinSpeed`): towards the landing direction
/// from the board's leading end (`lead`: the nose, or the tail when it took off fakie), or turned the
/// other way round when that is nearer; `current` is the spin now, `left` the seconds to the landing,
/// `up_y` the body's up. Only with the stick let go.
#[allow(clippy::too_many_arguments)]
pub fn auto_spin(
    t: &Tuning,
    lead: Vec3,
    heading: Vec3,
    normal: Vec3,
    up_y: f32,
    current: f32,
    left: f32,
    dt: f32,
) -> f32 {
    let unwrapped = signed_angle(lead, heading, normal);
    let mut corrected = wrap(unwrapped);
    if corrected.abs() * (180.0 / PI) > t.heading_adjust_vs_up.eval(up_y) {
        corrected = wrap(PI + corrected);
    }
    let time = if left >= AUTO_MIN_TIME {
        left
    } else {
        AUTO_MIN_TIME
    };
    let recip = 1.0 / time;
    let mut target = recip * corrected * AUTO_SCALE;
    let current = if current.abs() < 0.001 { 0.0 } else { current };
    if current * target < 0.0 {
        // Already spinning the other way: carry on round to the other end when that is close.
        let alt = wrap(unwrapped + PI);
        if alt.abs() < AUTO_KEEP_GOING && recip * alt * current > 0.0 {
            target = recip * alt;
        }
    }
    let mut r = clamp(target, -t.max_auto_spin, t.max_auto_spin);
    if r.abs() < t.min_auto_spin {
        r *= 0.5;
    }
    if unwrapped.abs() < t.min_auto_spin {
        r = unwrapped / dt.max(1e-4) * 0.5;
    }
    if r * current < 0.0 {
        r *= AUTO_REVERSE;
    } else if current.abs() > r.abs() {
        r *= AUTO_DECEL;
    }
    r
}

/// Without a known landing, the stick's spin goes through the assist's path (SK8 PhysicsAir): the stick
/// (`input`, + right) at `max_spin_speed`, shaped by `spin_input_filter` against twice the air time.
pub fn physics_air_spin(t: &Tuning, input: f32, time: f32) -> f32 {
    -t.spin_input_filter.eval(time * 2.0) * input * t.max_spin_speed * (PI / 180.0)
}

/// The board's up one tick on towards `to` (unit vectors): by `share` of the angle, at most `max` rad.
pub fn tilt_towards(from: Vec3, to: Vec3, share: f32, max: f32) -> Vec3 {
    let angle = libm::acosf(from.dot(to).clamp(-1.0, 1.0));
    let by = (angle * share.clamp(0.0, 1.0)).min(max);
    if angle < 1e-5 || by <= 0.0 {
        return from;
    }
    if by >= angle {
        return to;
    }
    let axis = from.cross(to);
    if axis.length() < 1e-6 {
        return from;
    }
    super::rotate(from, axis.normalize_or(Vec3::UP), by).normalize_or(from)
}

/// The share of the angle to the landing surface the board's up turns this tick in known air: it
/// arrives `ALIGN_LEAD` ticks before touchdown.
pub fn align_share(left: f32, dt: f32) -> f32 {
    let frames = left / dt.max(1e-4) - ALIGN_LEAD;
    1.0 / frames.max(MIN_ALIGN)
}

/// The angle (rad, -pi..pi) from `a` to `b` turning about `n` (right-handed), both taken along the
/// plane of `n`.
pub fn signed_angle(a: Vec3, b: Vec3, n: Vec3) -> f32 {
    let a = a - n * a.dot(n);
    let b = b - n * b.dot(n);
    atan2f(n.dot(a.cross(b)), a.dot(b))
}

/// An angle wrapped to -pi..pi.
pub fn wrap(a: f32) -> f32 {
    crate::math::wrap_angle(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    #[test]
    fn a_let_go_stick_keeps_spinning_and_decays() {
        let t = Tuning::BUILT_IN;
        let mut b = BodySpin::default();
        for _ in 0..20 {
            b.air(&t, -1.0, 0.0);
        }
        let held = b.speed;
        assert!(held > 1.0, "{held}");
        for _ in 0..30 {
            b.air(&t, 0.0, 0.0);
        }
        // Half a second after letting go it is still turning.
        assert!(b.speed > 0.2 * held, "{} of {held}", b.speed);
        // And it slows no faster than the opposite-direction limit.
        let mut c = BodySpin::default();
        for _ in 0..20 {
            c.air(&t, -1.0, 0.0);
        }
        let before = c.speed;
        c.air(&t, 0.0, 0.0);
        assert!(before - c.speed <= t.spin_reverse_change + 1e-4);
    }

    #[test]
    fn the_wind_up_is_signed_and_lost_by_reversing() {
        let t = Tuning::BUILT_IN;
        // The stick snapped over just before take-off (a wind-up), then held `spin` in the air.
        let wound = |spin: f32| {
            let mut b = BodySpin::default();
            for _ in 0..SPIN_HISTORY + 4 {
                b.ground(0.0);
            }
            b.ground(1.0);
            for _ in 0..20 {
                b.air(&t, spin, 0.0);
            }
            b.speed.abs()
        };
        // Held still since long before take-off: no wind-up.
        let none = |spin: f32| {
            let mut b = BodySpin::default();
            for _ in 0..SPIN_HISTORY + 4 {
                b.ground(spin);
            }
            for _ in 0..20 {
                b.air(&t, spin, 0.0);
            }
            b.speed.abs()
        };
        assert!(
            wound(1.0) > none(1.0) * 1.2,
            "with {} without {}",
            wound(1.0),
            none(1.0)
        );
        // Reversing in the air loses it.
        let mut b = BodySpin::default();
        for _ in 0..SPIN_HISTORY + 4 {
            b.ground(0.0);
        }
        b.ground(1.0);
        for _ in 0..5 {
            b.air(&t, 1.0, 0.0);
        }
        assert!(b.peak > 0.5);
        b.air(&t, -1.0, 0.0);
        assert_eq!(b.peak, 0.0);
    }

    #[test]
    fn the_assist_turns_to_the_nearer_end() {
        let t = Tuning::BUILT_IN;
        let n = Vec3::UP;
        let heading = v3(0.0, 0.0, 1.0);
        // 20 degrees short of forward: turn on towards forward.
        let lead = v3(sinf(-0.35), 0.0, cosf(-0.35));
        let s = auto_spin(&t, lead, heading, n, 1.0, 0.0, 0.3, STEP);
        assert!(s > 0.0, "{s}");
        // 160 degrees round: finish to fakie, not back round to forward.
        let lead = v3(sinf(2.8), 0.0, cosf(2.8));
        let s = auto_spin(&t, lead, heading, n, 1.0, 0.0, 0.3, STEP);
        assert!(s > 0.0, "carry on round to fakie: {s}");
        let lead = v3(sinf(-2.8), 0.0, cosf(-2.8));
        assert!(auto_spin(&t, lead, heading, n, 1.0, 0.0, 0.3, STEP) < 0.0);
    }

    #[test]
    fn the_cone_spreads_round_the_launch_and_is_never_faster() {
        let t = Tuning::BUILT_IN;
        let v = v3(0.0, 3.0, 4.0);
        let c = cone(v, t.traj_cone_x, t.traj_cone_z, &t);
        assert_eq!(c.len(), CANDIDATES);
        assert_eq!(c[0], v);
        assert!(c.iter().all(|w| w.length() <= v.length() + 1e-4));
        assert!(c.iter().any(|w| w.x > 0.2) && c.iter().any(|w| w.x < -0.2));
    }
}
