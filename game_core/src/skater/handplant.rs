//! Hand plants (inverts), as Skate 3 does them (after SK8-ENGINE/skate-3-rust-engine, GPL-3.0:
//! `skate-game/src/physics/handplant.rs` and its `contact.rs`, `trajectory.rs`, `rotation.rs`; TU3's
//! HandPlantManager `0x82D61040` and physical state 600):
//!
//! - **The search** ([`select`], SK8 contact.rs, TU3 `0x82D63D40`): riding up a ramp steeper than
//!   `min_slope` faster than `min_speed` with the world grab held (Skate 3's RB, `GrabWorld`: here Z), the
//!   apexes the flight could reach (the window's point graphs against the speed up the ramp's plane)
//!   make a polygon across the ramp; a rail (the coping) running across the ramp, no further into it than
//!   `depth` and inside that polygon, is the hand plant's coping, at the point nearest the window's middle
//!   at its height.
//! - **Found on the ground** ([`Manager`], SK8 `launch`): the path is laid out from the centre of mass to
//!   the apex (`apex_radius` from the coping, `apex_angle` round from straight above it, back over the
//!   ramp); the manager's flags say it is found (31), whether the coping moved (30), frontside (29) and
//!   committed (28, the apex nearer than `committed_time`). Its time to the apex (`phase`, the plant's
//!   own slowed clock, [`estimate_apex`]) is what the clips' `ShouldPlayHandPlantAnim` read.
//! - **The plant** ([`Plant`], physical state 600, SK8 `enter`, `update`, `select_outgoing`, `values`):
//!   the centre of mass flies the path to the apex (blended in from the flight it came in on over
//!   `entry_blend`), round the apex on a curve (`curve_half_time` either side), and down a fall from the
//!   apex to the best landing back in the ramp (the steepest of six flights swept out over the ramp), its
//!   clock slowed over the apex by `time_warp`; the rider turns from riding up the ramp to upside down
//!   over the hand and round to ride back down it (`into_rotation`, `out_heading`, `out_rotation`). The
//!   hand lets go `hand_release` after the apex and the rider falls on into the landing.
//!
//! flatground's rider has no physical body: the centre of mass is the board's point plus its height
//! over the deck along the board's up (upside down in the plant), and the clips' feet don't lift the
//! path (SK8 `values`' foot correction).

use crate::math::{Vec3, v3};
use crate::park::Terrain;
use crate::tuning::handplant::HandPlantTuning;

/// Skate 3's fixed step.
const STEP: f32 = 1.0 / 60.0;
/// SK8 contact.rs: the broadphase box round the centre of mass rails are looked for in (m).
const SEARCH_BOX: [f32; 3] = [2.0, 4.0, 2.0];
/// SK8 contact.rs (`0x82D64218`): a rail must run at least this much across the ramp (its direction's
/// share along the ramp's width).
const ACROSS_MIN: f32 = 0.71;
/// SK8 launch: the coping moved more than this since the last one is a new position (m).
const NEW_POSITION: f32 = 0.15;
/// SK8 select_outgoing: the six flights out over the ramp, turned this far from along it (rad).
const OUT_ANGLES: [f32; 6] = [0.0, 0.24, 0.48, 0.72, 0.96, 1.2];
/// SK8 select_outgoing: the flights' start this far out over the ramp from the coping (m), the most
/// speed along the coping kept (m/s), the landing at least this far under the coping (m), the centre of
/// mass this far over the landing (m), a landing steeper than this taken at once (its normal's y).
const OUT_START: f32 = 0.1;
const OUT_ALONG_MAX: f32 = 2.0;
const OUT_DROP_MIN: f32 = 0.5;
const OUT_COM: f32 = 0.56;
const OUT_STEEP: f32 = 0.71;
/// The flights are swept for this long (s, SK8's three-second query), in chords of this length (s).
const OUT_TIME: f32 = 3.0;
const OUT_CHORD: f32 = 1.0 / 30.0;

/// A flight under gravity (SK8 `Trajectory`).
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
    /// Seconds to the top of the flight (0 when already falling).
    pub fn apex_time(&self) -> f32 {
        if self.vel.y < 0.0 || self.gravity <= 0.0 {
            0.0
        } else {
            self.vel.y / self.gravity
        }
    }
    /// The flight from `start` to `end` in `time` (SK8 `arc_between`).
    pub fn between(start: Vec3, end: Vec3, time: f32, gravity: f32) -> Flight {
        let t = time.max(1e-3);
        let vel = (end - start + Vec3::UP * (0.5 * gravity * t * t)) * (1.0 / t);
        Flight {
            pos: start,
            vel,
            gravity,
        }
    }
    /// Up to `end` arriving at the top of the flight (SK8 `to_apex`): its time and the flight.
    pub fn to_apex(start: Vec3, end: Vec3, gravity: f32) -> (Flight, f32) {
        let t = libm::sqrtf((2.0 * gravity * (end.y - start.y)).max(0.0)) / gravity.max(1e-3);
        (Flight::between(start, end, t, gravity), t)
    }
    /// From the top of the flight at `start` down to `end` (SK8 `from_apex`).
    pub fn from_apex(start: Vec3, end: Vec3, gravity: f32) -> (Flight, f32) {
        let t = libm::sqrtf((2.0 * gravity * (start.y - end.y)).max(0.0)) / gravity.max(1e-3);
        (Flight::between(start, end, t, gravity), t)
    }
}

/// A coping the search found: the point the hand goes, the rail's ends there, and which way across the
/// ramp the rider was going (1 one way, 2 the other: SK8's side).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub point: Vec3,
    pub edge: [Vec3; 2],
    pub side: u8,
}

/// The ramp's frame at the centre of mass (SK8 contact.rs): x along the ramp's width (`up x normal`),
/// y up, z out of the ramp (away from it).
#[derive(Clone, Copy, Debug)]
struct RampFrame {
    x: Vec3,
    z: Vec3,
    origin: Vec3,
}

impl RampFrame {
    fn of(normal: Vec3, origin: Vec3) -> RampFrame {
        let (x, z) = if normal.y.abs() <= 0.99 {
            let x = Vec3::UP.cross(normal).normalize_or(v3(1.0, 0.0, 0.0));
            (x, x.cross(Vec3::UP))
        } else {
            (v3(1.0, 0.0, 0.0), v3(0.0, 0.0, 1.0))
        };
        RampFrame { x, z, origin }
    }
    fn local(&self, p: Vec3) -> Vec3 {
        let d = p - self.origin;
        v3(d.dot(self.x), d.y, d.dot(self.z))
    }
    fn world(&self, l: Vec3) -> Vec3 {
        self.origin + self.x * l.x + Vec3::UP * l.y + self.z * l.z
    }
    fn dir(&self, d: Vec3) -> Vec3 {
        v3(d.dot(self.x), d.y, d.dot(self.z))
    }
}

/// The search (SK8 contact.rs `select`, TU3 `0x82D63D40` and the window `0x82D64420`): a coping across
/// the ramp the flight from `com` at `vel` could put the hand on. `normal`: the ground's. `hint`: the
/// side kept from before (0 none).
pub fn select(
    t: &HandPlantTuning,
    gravity: f32,
    rails: &[crate::rails::Rail],
    com: Vec3,
    vel: Vec3,
    normal: Vec3,
    hint: u8,
) -> Option<Candidate> {
    let slope = libm::acosf(normal.y.clamp(-1.0, 1.0)).to_degrees();
    if !(vel.length() > t.min_speed && vel.y > 0.0 && normal.y.abs() <= 0.95 && slope > t.min_slope)
    {
        return None;
    }
    let frame = RampFrame::of(normal, com);
    let lv = frame.dir(vel);
    // SK8 keeps the sideways speed and folds the rest of the plane's into the up speed.
    let mut vx = lv.x;
    let vy = libm::sqrtf(lv.x * lv.x + lv.y * lv.y);
    if hint != 0 && hint != if vx > 0.0 { 1 } else { 2 } {
        vx = if hint == 1 { 1.0 } else { -1.0 } * 1.525_878_9e-5;
    }
    let g = t.window.map(|c| c.eval(vy));
    let lo_x = g[1].max(vx.abs() - g[5]);
    let hi_x = g[6].min(vx.abs() + g[4]);
    if hi_x <= lo_x {
        return None;
    }
    let lo_y = g[0].max(vy - g[3]);
    let hi_y = vy + g[2];
    let sign = if vx > 0.0 { 1.0 } else { -1.0 };
    // The apex of a flight from the centre of mass at (x, y) across and up, the hand under it.
    let apex = |x: f32, y: f32| {
        let f = Flight {
            pos: Vec3::ZERO,
            vel: v3(x * sign, y, 0.0),
            gravity,
        };
        let mut p = f.at(f.apex_time());
        p.y -= t.window_drop;
        p
    };
    let (a, b, c, d) = (
        apex(lo_x, lo_y),
        apex(hi_x, lo_y),
        apex(lo_x, hi_y),
        apex(hi_x, hi_y),
    );
    let polygon = if d.x > c.x {
        [c, d, b, a]
    } else {
        [d, c, a, b]
    };
    let mut best: Option<([Vec3; 2], [Vec3; 2])> = None;
    let (mut height, mut depth) = (f32::MIN, f32::MIN);
    for rail in rails {
        // Rails whose box is out of reach are skipped whole (imported parks have thousands).
        if !rail.near(com, SEARCH_BOX[1]) {
            continue;
        }
        for w in rail.points.windows(2) {
            let (s, e) = (w[0], w[1]);
            let near = (0..3).all(|i| {
                let (lo, hi, c) = match i {
                    0 => (s.x.min(e.x), s.x.max(e.x), com.x),
                    1 => (s.y.min(e.y), s.y.max(e.y), com.y),
                    _ => (s.z.min(e.z), s.z.max(e.z), com.z),
                };
                lo <= c + SEARCH_BOX[i] && hi >= c - SEARCH_BOX[i]
            });
            if !near {
                continue;
            }
            let mut seg = [frame.local(s), frame.local(e)];
            if (seg[1] - seg[0]).normalize_or(Vec3::ZERO).x.abs() <= ACROSS_MIN {
                continue;
            }
            // Into the ramp (out of it is +z), no further than its depth.
            if !clip_depth(&mut seg, -t.depth, true) || !clip_depth(&mut seg, 0.0, false) {
                continue;
            }
            let mut inside = true;
            for i in 0..4 {
                let delta = polygon[(i + 1) % 4] - polygon[i];
                let n = v3(delta.y, -delta.x, 0.0).normalize_or(Vec3::ZERO);
                let d0 = (seg[0] - polygon[i]).dot(n);
                let d1 = (seg[1] - polygon[i]).dot(n);
                if !clip(&mut seg, d0, d1) {
                    inside = false;
                    break;
                }
            }
            if !inside {
                continue;
            }
            let p = if seg[0].y > seg[1].y { seg[0] } else { seg[1] };
            if p.y > height + 0.1 || (p.y > height - 0.1 && p.y <= height + 0.1 && p.z > depth) {
                best = Some((seg, [s, e]));
                height = p.y;
                depth = p.z;
            }
        }
    }
    let (seg, edge) = best?;
    // The window's middle at the coping's height, and the coping's point nearest it.
    let bottom = (polygon[2] + polygon[3]) * 0.5;
    let top = (polygon[0] + polygon[1]) * 0.5;
    let delta = top - bottom;
    let k = if delta.y.abs() > 1e-6 {
        ((height - bottom.y) / delta.y).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let target = bottom + delta * k;
    let target = v3(target.x, target.y, 0.0);
    let d = seg[1] - seg[0];
    let sq = d.dot(d);
    let near = if sq > 1.525_878_9e-5 {
        seg[0] + d * ((target - seg[0]).dot(d) / sq).clamp(0.0, 1.0)
    } else {
        seg[0]
    };
    Some(Candidate {
        point: frame.world(near),
        edge,
        side: if vx > 0.0 { 1 } else { 2 },
    })
}

/// SK8 contact.rs `clip_depth` (TU3 `0x82D658A0`): keeps the segment's part beyond `value` in z (`above`:
/// over it, else under it).
fn clip_depth(seg: &mut [Vec3; 2], value: f32, above: bool) -> bool {
    let sign = if above { 1.0 } else { -1.0 };
    let a = (seg[0].z - value) * sign;
    let b = (seg[1].z - value) * sign;
    if a <= 0.0 && b <= 0.0 {
        return false;
    }
    let delta = seg[1].z - seg[0].z;
    if delta.abs() >= 1.525_878_9e-5 {
        let t = (value - seg[0].z) / delta;
        if t > 0.0 && t < 1.0 {
            let p = seg[0] + (seg[1] - seg[0]) * t;
            seg[usize::from(b <= 0.0)] = p;
        }
    }
    true
}

/// SK8 contact.rs `clip`: keeps the segment's part on the positive side of a line (`a`, `b`: its ends'
/// distances).
fn clip(seg: &mut [Vec3; 2], a: f32, b: f32) -> bool {
    if a < 0.0 && b < 0.0 {
        return false;
    }
    if (a < 0.0) != (b < 0.0) {
        let p = seg[0] + (seg[1] - seg[0]) * (a / (a - b));
        seg[usize::from(a >= 0.0)] = p;
    }
    true
}

/// The apex over the coping (SK8 `apex_position`): `radius` from it, `angle` round from straight up
/// towards `side` (the far side of the coping from the ramp; a negative angle leans back over the ramp).
pub fn apex_position(coping: Vec3, side: Vec3, radius: f32, angle: f32) -> Vec3 {
    coping + side * (radius * libm::sinf(angle)) + Vec3::UP * (radius * libm::cosf(angle))
}

/// The plant's clock to the apex (SK8 `estimate_apex`, TU3 `0x82D63280`): the time left, read through
/// four samples of the time warp ahead.
pub fn estimate_apex(t: &HandPlantTuning, warped: f32, apex: f32) -> Option<f32> {
    let offset = warped - apex;
    let stride = (offset * (1.0 / 3.0)).abs();
    let mean = (0..4)
        .map(|i| 1.0 + t.time_warp.eval(offset + i as f32 * stride))
        .sum::<f32>()
        * 0.25;
    (mean != 0.0).then(|| (-1.0 / mean) * offset)
}

/// A frame (SK8 `rotation::frame`, TU3 `0x82D61B60`): right, up, forward from a heading and an up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Basis {
    pub right: Vec3,
    pub up: Vec3,
    pub fwd: Vec3,
}

impl Basis {
    pub fn of(mut heading: Vec3, up: Vec3) -> Basis {
        let up = up.normalize_or(Vec3::UP);
        if heading.dot(up).abs() > 0.99999 {
            heading = v3(1.0, 0.0, 0.0);
            if heading.dot(up).abs() > 0.99999 {
                heading = v3(0.0, 0.0, 1.0);
            }
        }
        let right = up.cross(heading).normalize_or(v3(1.0, 0.0, 0.0));
        let fwd = right.cross(up).normalize_or(heading);
        Basis {
            right,
            up: fwd.cross(right).normalize_or(up),
            fwd,
        }
    }

    /// Turned `w` of the way to `b` (SK8 `rotation::blend`: about the relative rotation's axis).
    pub fn blend(self, b: Basis, w: f32) -> Basis {
        if w >= 1.0 {
            return b;
        }
        if w <= 0.0 {
            return self;
        }
        // The relative rotation's axis and angle (from its matrix's trace and skew part).
        let m = |v: Vec3| v3(v.dot(self.right), v.dot(self.up), v.dot(self.fwd));
        let (r, u, f) = (m(b.right), m(b.up), m(b.fwd));
        // Columns of the relative rotation in self's frame: R = [r u f].
        let trace = r.x + u.y + f.z;
        let cos = ((trace - 1.0) * 0.5).clamp(-1.0, 1.0);
        let angle = libm::acosf(cos);
        if angle < 0.034_906_6 {
            let n = |a: Vec3, c: Vec3| (a + (c - a) * w).normalize_or(c);
            return Basis::of(n(self.fwd, b.fwd), n(self.up, b.up));
        }
        let mut axis = v3(u.z - f.y, f.x - r.z, r.y - u.x);
        if axis.length() < 1e-4 {
            // Half a turn: the axis is the column of R + I with the most in it.
            let cands = [
                r + v3(1.0, 0.0, 0.0),
                u + v3(0.0, 1.0, 0.0),
                f + v3(0.0, 0.0, 1.0),
            ];
            axis = cands.into_iter().fold(
                Vec3::ZERO,
                |a, c| if c.length() > a.length() { c } else { a },
            );
        }
        let axis = axis.normalize_or(v3(0.0, 1.0, 0.0));
        let world = self.right * axis.x + self.up * axis.y + self.fwd * axis.z;
        let turn = angle * w;
        Basis::of(
            super::rotate(self.fwd, world, turn),
            super::rotate(self.up, world, turn),
        )
    }
}

/// The hand plant manager on the ground (SK8 `ground_query` and `launch`): the coping found this tick,
/// its path's apex and the flags the graphs read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manager {
    pub candidate: Option<Candidate>,
    /// The last coping (for `HasNewHandPlantPos`), the side hint and how long it has disagreed (ticks).
    previous: Option<Vec3>,
    hint: u8,
    hint_count: u32,
    /// The path to the apex from the centre of mass, the apex and its time (s), and the time to it on
    /// the plant's clock (`phase`, `f32::MAX` none).
    pub initial: Flight,
    pub apex: Vec3,
    pub apex_time: f32,
    pub phase: f32,
    /// Flags (SK8's 28..31): committed, frontside, a new position, found.
    pub committed: bool,
    pub frontside: bool,
    pub new_pos: bool,
    pub found: bool,
}

impl Default for Manager {
    fn default() -> Self {
        Manager {
            candidate: None,
            previous: None,
            hint: 0,
            hint_count: 0,
            initial: Flight {
                pos: Vec3::ZERO,
                vel: Vec3::ZERO,
                gravity: 9.8,
            },
            apex: Vec3::ZERO,
            apex_time: 0.0,
            phase: f32::MAX,
            committed: false,
            frontside: false,
            new_pos: false,
            found: false,
        }
    }
}

/// What the manager reads of the rider on the ground.
#[derive(Clone, Copy, Debug)]
pub struct Ground {
    pub com: Vec3,
    pub vel: Vec3,
    /// The ground's normal, the board's nose and its right (`up x nose`).
    pub normal: Vec3,
    pub heading: Vec3,
    pub side: Vec3,
    pub gravity: f32,
    /// The graphs' hand plant is under way (SK8 flags 28/29 of 2480: `AnimCommittedHandPlant` and not
    /// a new one): the side hint is kept.
    pub anim_committed: bool,
}

impl Manager {
    /// One tick on the ground with the world grab held (SK8 `ground_query`, then `launch` once the
    /// coping is found).
    pub fn query(&mut self, t: &HandPlantTuning, rails: &[crate::rails::Rail], g: &Ground) {
        if !g.anim_committed {
            self.hint = 0;
            self.hint_count = 0;
        } else if g.normal.y < 0.95 {
            if self.hint == 0 {
                self.hint = self.candidate.map_or(0, |c| c.side);
            }
            let across = Vec3::UP.cross(g.normal).normalize_or(Vec3::ZERO);
            let side = if g.vel.dot(across) > 0.0 { 1 } else { 2 };
            if self.hint == side {
                self.hint_count = 0;
            } else {
                self.hint_count += 1;
                if self.hint_count > t.direction_frames {
                    self.hint = 0;
                }
            }
        } else {
            self.hint = 0;
            self.hint_count = 0;
        }
        let candidate = select(t, g.gravity, rails, g.com, g.vel, g.normal, self.hint);
        let previous = self.candidate.map(|c| c.point).or(self.previous);
        let (hint, count) = (self.hint, self.hint_count);
        *self = Manager {
            previous,
            hint,
            hint_count: count,
            ..Manager::default()
        };
        if let Some(c) = candidate {
            self.launch(t, c, g);
        }
    }

    /// SK8 `launch` (TU3 `0x82D613B0`): the path to the apex and the flags.
    fn launch(&mut self, t: &HandPlantTuning, c: Candidate, g: &Ground) {
        let side = far_side(c, g.normal);
        let apex = apex_position(c.point, side, t.apex_radius, t.apex_angle);
        let (initial, time) = Flight::to_apex(g.com, apex, g.gravity);
        self.initial = initial;
        self.apex = apex;
        self.apex_time = time;
        self.phase = estimate_apex(t, 0.0, time).unwrap_or(time);
        let changed = self
            .previous
            .is_none_or(|p| (p - c.point).length() > NEW_POSITION);
        self.found = true;
        self.new_pos = changed;
        self.frontside = (c.point - g.com).dot(g.side) < 0.0;
        self.committed = self.phase < t.committed_time;
        self.candidate = Some(c);
    }
}

/// The coping's far side from the ramp (SK8 `launch`: across the coping, away from the ramp's normal).
fn far_side(c: Candidate, normal: Vec3) -> Vec3 {
    let side = (c.edge[1] - c.edge[0])
        .cross(Vec3::UP)
        .normalize_or(v3(0.0, 0.0, 1.0));
    if side.dot(normal) > 0.0 { -side } else { side }
}

/// The hand plant itself (SK8 physical state 600).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plant {
    /// The flight it came in on, the path to the apex, the falls out of it (as found, and with the least
    /// speed back over the ramp), the curve round the apex.
    pub entry: Flight,
    pub initial: Flight,
    pub outgoing: [Flight; 2],
    pub curve: [Vec3; 4],
    /// The turn: riding up the ramp, upside down over the hand, coming round, landing.
    pub rotations: [Basis; 4],
    /// Back over the ramp (horizontal), +1 riding forwards up it, the coping's point and the apex.
    pub direction: Vec3,
    pub travel_sign: f32,
    pub anchor: Vec3,
    pub apex_pos: Vec3,
    /// The plant's own clock (slowed over the apex), the real time in it, the apex's time on the path
    /// and the time left to it on the clock (s).
    pub warped: f32,
    pub elapsed: f32,
    pub apex: f32,
    pub phase: f32,
    pub out_duration: f32,
    /// Where the fall lands (and its ground's normal).
    pub landing: Vec3,
    pub landing_normal: Vec3,
    pub frontside: bool,
}

impl Plant {
    /// SK8 `enter` and `select_outgoing`: into the plant from the centre of mass `com` flying at `vel`,
    /// with the manager's path; `heading` the board's nose, `normal` the ground's.
    pub fn start(
        t: &HandPlantTuning,
        park: &dyn Terrain,
        m: &Manager,
        com: Vec3,
        vel: Vec3,
        heading: Vec3,
        normal: Vec3,
    ) -> Option<Plant> {
        let c = m.candidate?;
        let gravity = m.initial.gravity;
        let side = far_side(c, normal);
        let direction = -side;
        let apex_t = m.apex_time;
        let apex = m.apex;
        let travel_sign = if heading.dot(vel) > 0.0 { 1.0 } else { -1.0 };
        // The turn (SK8 `launch`'s rotations).
        let r0 = Basis::of(heading, normal);
        let mut along = (c.edge[0] - c.edge[1]).normalize_or(v3(1.0, 0.0, 0.0));
        let toward = v3(along.x, 0.0, along.z).dot(apex - com) > 0.0;
        along = along * if toward { travel_sign } else { -travel_sign };
        let r1 = Basis::of(along, (c.point - apex).normalize_or(-Vec3::UP));
        let r2 = Basis::of(Vec3::UP * -travel_sign, normal);
        // The fall out of it (SK8 `select_outgoing`).
        let incoming = m.initial.vel_at(apex_t);
        let across = direction * incoming.dot(direction);
        let tangent = incoming - across;
        let along_out = tangent.normalize_or(Vec3::ZERO);
        let tl = tangent.length();
        let speed = (across
            + tangent
                * if tl > OUT_ALONG_MAX {
                    OUT_ALONG_MAX / tl
                } else {
                    1.0
                })
        .length();
        let mut start = c.point + direction * OUT_START;
        start.y = apex.y;
        let mut selected: Option<(Vec3, Vec3)> = None;
        let mut best_up = 1.1;
        let mut last = Flight {
            pos: start,
            vel: Vec3::ZERO,
            gravity,
        };
        for angle in OUT_ANGLES {
            let v = (direction * libm::sinf(angle) + along_out * libm::cosf(angle)) * speed;
            let f = Flight {
                pos: start,
                vel: v,
                gravity,
            };
            last = f;
            let Some((time, n)) = sweep(park, &f, OUT_TIME) else {
                continue;
            };
            let p = f.at(time);
            if c.point.y - p.y <= OUT_DROP_MIN {
                continue;
            }
            let dest = p + n * OUT_COM;
            if !dest.y.is_finite() || dest.y >= apex.y {
                continue;
            }
            if n.y < OUT_STEEP {
                selected = Some((p, n));
                break;
            }
            if n.y < best_up {
                best_up = n.y;
                selected = Some((p, n));
            }
        }
        let (landing, landing_normal) = selected.unwrap_or((last.at(OUT_TIME), Vec3::UP));
        let dest = landing + landing_normal * OUT_COM;
        let (fall, fall_time) = Flight::from_apex(apex, dest, gravity);
        // On the path's clock (time 0 at the plant's start).
        let out = Flight {
            pos: fall.at(-apex_t),
            vel: fall.vel_at(-apex_t),
            gravity,
        };
        let mut boosted = out;
        let s = direction.dot(boosted.vel);
        if s < t.min_out_speed {
            boosted.vel += direction * (t.min_out_speed - s);
        }
        let time = apex_t + fall_time;
        let r3 = Basis::of(
            out.vel_at(time).normalize_or(direction) * travel_sign,
            landing_normal,
        );
        let phase = estimate_apex(t, 0.0, apex_t).unwrap_or(apex_t);
        let h = t.curve_half_time;
        let (s0, s1) = (apex_t - h, apex_t + h);
        let c0 = m.initial.at(s0);
        let c3 = boosted.at(s1);
        let curve = [
            c0,
            c0 + (m.initial.at(s0 + STEP) - c0) * 6.0,
            c3 + (boosted.at(s1 - STEP) - c3) * 6.0,
            c3,
        ];
        Some(Plant {
            entry: Flight {
                pos: com,
                vel,
                gravity,
            },
            initial: m.initial,
            outgoing: [out, boosted],
            curve,
            rotations: [r0, r1, r2, r3],
            direction,
            travel_sign,
            anchor: c.point,
            apex_pos: apex,
            warped: 0.0,
            elapsed: 0.0,
            apex: apex_t,
            phase,
            out_duration: (time - phase).max(0.5),
            landing,
            landing_normal,
            frontside: m.frontside,
        })
    }

    /// One tick (SK8 `update` and `values`): the centre of mass and the rider's frame; true once the hand
    /// has let go (`hand_release` past the apex).
    pub fn step(&mut self, t: &HandPlantTuning) -> (Vec3, Basis, bool) {
        self.warped += (1.0 + t.time_warp.eval(self.warped - self.apex)) * STEP;
        if let Some(p) = estimate_apex(t, self.warped, self.apex) {
            self.phase = p;
        }
        self.elapsed += STEP;
        let h = t.curve_half_time;
        let relative = self.warped - self.apex;
        let fraction = (1.0 + relative / h) * 0.5;
        let com = if fraction < 0.0 {
            self.initial.at(self.warped)
        } else if fraction > 1.0 {
            let k = ((relative - h) / self.out_duration).clamp(0.0, 1.0);
            self.outgoing[0].at(self.warped) * k + self.outgoing[1].at(self.warped) * (1.0 - k)
        } else {
            bezier(self.curve, fraction)
        };
        let blend = (1.0 - self.elapsed / t.entry_blend).max(0.0);
        let blended = com * (1.0 - blend) + self.entry.at(self.elapsed) * blend;
        if blend > 0.0 && fraction < 0.0 {
            let v = self.initial.vel_at(self.warped);
            let len = v.length();
            if len > 1e-4 {
                let correction = (blended - com).dot(v * (1.0 / len)) / len;
                self.warped += correction.clamp(-1.0 / 120.0, 1.0 / 120.0);
            }
        }
        let [r0, r1, r2, r3] = self.rotations;
        let frame = if self.warped < self.apex {
            let k = (1.0 - (self.apex - self.warped) / self.apex.min(t.rotation_time).max(1e-3))
                .max(0.0);
            r0.blend(r1, t.into_rotation.eval(k))
        } else {
            let f = self.warped - self.apex;
            let out = t
                .out_heading
                .eval((f / self.out_duration.max(1e-3)).clamp(0.0, 1.0));
            let into = t
                .out_rotation
                .eval((f / t.rotation_time.max(1e-3)).clamp(0.0, 1.0));
            r1.blend(r2.blend(r3, out), into)
        };
        let released = self.phase < -t.hand_release;
        (blended, frame, released)
    }

    /// The centre of mass's velocity now (the path's).
    pub fn velocity(&self, t: &HandPlantTuning) -> Vec3 {
        let relative = self.warped - self.apex;
        if relative > t.curve_half_time {
            self.outgoing[1].vel_at(self.warped)
        } else if relative < -t.curve_half_time {
            self.initial.vel_at(self.warped)
        } else {
            let a = self.initial.vel_at(self.warped);
            let b = self.outgoing[1].vel_at(self.warped);
            let k = (relative / t.curve_half_time + 1.0) * 0.5;
            a + (b - a) * k
        }
    }
}

/// The first ground a flight meets within `time` (s): when, and the ground's normal there.
fn sweep(park: &dyn Terrain, f: &Flight, time: f32) -> Option<(f32, Vec3)> {
    let mut t0 = 0.0;
    let mut p0 = f.at(0.0);
    while t0 < time {
        let t1 = (t0 + OUT_CHORD).min(time);
        let p1 = f.at(t1);
        let d = p1 - p0;
        let len = d.length();
        if len > 1e-5
            && let Some(hit) = park.raycast(p0, d * (1.0 / len), len)
        {
            return Some((t0 + (t1 - t0) * (hit.t / len), hit.normal));
        }
        t0 = t1;
        p0 = p1;
    }
    None
}

/// SK8 trajectory.rs `bezier`.
fn bezier(p: [Vec3; 4], t: f32) -> Vec3 {
    let s = 1.0 - t;
    p[0] * (s * s * s) + p[1] * (3.0 * t * s * s) + p[2] * (3.0 * t * t * s) + p[3] * (t * t * t)
}

/// The hand plant's scored name (Skate 3's identifiers, `fs_handplant_up` ...): the tweak picks a
/// direction with ScoringGrabs' windows (`skater::grab_variant`), a push button held the one-foot plants
/// (A the front foot, B the back, for the unmirrored rider: Skate 3's A and X), X (Skate 3's B) the
/// no-foot plant.
pub fn name(frontside: bool, tweak: [f32; 2], feet: Feet) -> &'static str {
    const FS: [&str; 8] = [
        "fs_handplant",
        "fs_handplant_up",
        "fs_handplant_left",
        "fs_handplant_down",
        "fs_handplant_right",
        "fs_handplant_onefoot_front",
        "fs_handplant_onefoot_back",
        "fs_handplant_nofoot",
    ];
    const BS: [&str; 8] = [
        "bs_handplant",
        "bs_handplant_up",
        "bs_handplant_left",
        "bs_handplant_down",
        "bs_handplant_right",
        "bs_handplant_onefoot_front",
        "bs_handplant_onefoot_back",
        "bs_handplant_nofoot",
    ];
    let names = if frontside { &FS } else { &BS };
    match feet {
        Feet::OneFront => names[5],
        Feet::OneBack => names[6],
        Feet::None => names[7],
        Feet::Both => match super::grab_variant([true; 4], tweak[0], tweak[1]) {
            Some(i) => names[1 + i],
            None => names[0],
        },
    }
}

/// The feet in a hand plant (Skate 3's `HandPlantOneFootRight`/`Left`, `HandPlantDismount`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feet {
    Both,
    OneFront,
    OneBack,
    None,
}

/// As shown: `fs_handplant_up` is `FS HANDPLANT UP`.
pub fn display(id: &str) -> alloc::string::String {
    id.to_ascii_uppercase().replace('_', " ")
}

impl super::Skater {
    /// A hand plant is under way: a coping found riding up to it with the world grab held, or the plant
    /// itself (the free look and the markers leave Z and the C-stick to it).
    pub fn handplanting(&self) -> bool {
        self.handplant.found || self.hand_plant.is_some()
    }

    /// What the graphs read of the hand plant (`IsHandPlanting`, `HasNewHandPlantPos`,
    /// `ShouldPlayHandPlantAnim`).
    pub fn handplant_world(&self) -> crate::graph::host::HandPlantWorld {
        let m = &self.handplant;
        let found = m.found && self.rolling();
        crate::graph::host::HandPlantWorld {
            found,
            air: self.hand_plant.is_some(),
            frontside: self.hand_plant.map_or(m.frontside, |p| p.frontside),
            new_pos: found && m.new_pos,
            time: self
                .hand_plant
                .map_or(if found { m.phase } else { f32::MAX }, |p| p.phase),
            thresholds: self.tuning.handplant.anim,
        }
    }

    /// Riding: with the world grab held (the motion graph's `GrabWorld`, else Z), the manager looks for a
    /// coping up the ramp; once its apex is near enough (committed, and with the graphs their antic
    /// committed too) the plant starts. Returns whether it did (this tick is the plant's).
    pub(super) fn update_handplant(
        &mut self,
        park: &dyn Terrain,
        events: &mut alloc::vec::Vec<super::Event>,
    ) -> bool {
        let grab = self.graph.grab_world;
        if !grab || self.manual.is_some() || self.slide.is_some() || self.ground_plant.is_some() {
            if self.handplant.found {
                self.handplant = Manager::default();
            }
            return false;
        }
        let t = self.tuning;
        let up = self.normal;
        let com = self.pos + up * self.com_height();
        let g = Ground {
            com,
            vel: self.vel,
            normal: self.contact_n,
            heading: self.fwd,
            side: up.cross(self.fwd).normalize_or(Vec3::ZERO),
            gravity: t.gravity,
            anim_committed: self.graph.handplant_committed,
        };
        self.handplant.query(&t.handplant, park.rails(), &g);
        let m = self.handplant;
        if !(m.found && m.committed && self.graph.handplant_committed) {
            return false;
        }
        let Some(p) = Plant::start(
            &t.handplant,
            park,
            &m,
            com,
            self.vel,
            self.fwd,
            self.contact_n,
        ) else {
            return false;
        };
        self.take_off(None);
        self.hand_plant = Some(p);
        self.hand_plant_name = None;
        events.push(super::Event::HandPlanted);
        true
    }

    /// One tick of the plant (SK8 physical state 600): the centre of mass on its path, the rider turned
    /// over the hand; once the hand lets go, scored and on into the fall back into the ramp.
    pub(super) fn in_hand_plant(
        &mut self,
        time: f32,
        dt: f32,
        events: &mut alloc::vec::Vec<super::Event>,
    ) {
        let Some(mut p) = self.hand_plant else {
            return;
        };
        let t = self.tuning.handplant;
        let (com, frame, released) = p.step(&t);
        if self.graph.handplant_name.is_some() {
            self.hand_plant_name = self.graph.handplant_name;
        }
        self.normal = frame.up;
        self.fwd = frame.fwd;
        let to = com - frame.up * self.com_height();
        self.vel = (to - self.pos) * (1.0 / dt.max(1e-4));
        self.pos = to;
        self.sync_heading();
        if released {
            let name = self
                .hand_plant_name
                .unwrap_or_else(|| name(p.frontside, [0.0; 2], Feet::Both));
            events.push(super::Event::Landed {
                name: display(name),
                points: 0,
                score: crate::scoring::Scored::Ground {
                    id: crate::scoring::catalog::id(name),
                    held: p.elapsed,
                },
            });
            events.push(super::Event::HandReleased);
            self.hand_plant = None;
            self.handplant = Manager::default();
            self.hand_plant_name = None;
            // Round to ride back down the ramp, falling on along the path.
            let r3 = p.rotations[3];
            self.normal = r3.up;
            self.fwd = r3.fwd;
            self.vel = p.velocity(&t);
            self.plan = None;
            self.selector = super::air::Selector::default();
            self.sync_heading();
        } else {
            self.hand_plant = Some(p);
        }
        self.air_time = time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rails::Rail;
    use alloc::vec;
    use alloc::vec::Vec;

    const G: f32 = 9.8;

    #[test]
    fn the_apex_is_over_the_coping_and_back_over_the_ramp() {
        // SK8 trajectory.rs `stock_handplant_apex_is_above_coping_and_back_towards_ramp`.
        let coping = v3(-7.0, 3.885_004_5, 5.598_797_3);
        let apex = apex_position(coping, v3(0.0, 0.0, 1.0), 0.8, -0.3);
        assert!((apex.y - 4.649_274).abs() < 1e-4, "{apex:?}");
        assert!((apex.z - 5.362_381).abs() < 1e-4, "{apex:?}");
        let start = v3(-6.929_105_3, 3.925_457, 4.956_236_4);
        let (f, t) = Flight::to_apex(start, apex, G);
        assert!(t > 0.0);
        let end = f.at(t);
        assert!((end - apex).length() < 1e-4, "{end:?}");
        assert!(f.vel_at(t).y.abs() < 1e-4);
    }

    #[test]
    fn the_clock_slows_over_the_apex() {
        let t = HandPlantTuning::BUILT_IN;
        // Its four samples reach the apex's slow part: the estimate is a little over the time itself.
        let far = estimate_apex(&t, 0.0, 3.0).unwrap();
        assert!(far > 3.0 && far < 3.7, "{far}");
        // Near it the clock runs at a third: half a tick of path takes longer to reach.
        let near = estimate_apex(&t, 0.0, 0.05).unwrap();
        assert!(near > 0.1, "{near}");
    }

    /// A ramp at 60 degrees rising towards +z, its coping 3 m up at z = 1.73 along x.
    fn coping() -> Vec<Rail> {
        vec![Rail::new(vec![v3(-5.0, 3.0, 1.73), v3(5.0, 3.0, 1.73)])]
    }

    #[test]
    fn the_search_finds_the_coping_within_the_window_only() {
        let t = HandPlantTuning::BUILT_IN;
        let n = v3(0.0, 0.5, -0.866);
        let up_ramp = v3(0.0, 0.866, 0.5);
        // Fast and near the coping's height: found, over it.
        let com = v3(0.3, 2.6, 0.9);
        let c = select(&t, G, &coping(), com, up_ramp * 6.0, n, 0).expect("found");
        assert!(
            (c.point.y - 3.0).abs() < 1e-3 && (c.point.z - 1.73).abs() < 1e-3,
            "{c:?}"
        );
        assert!(c.point.x.abs() < 1.0, "{c:?}");
        // Too slow to reach it from far below: not found.
        let low = v3(0.0, 0.5, -0.4);
        assert_eq!(select(&t, G, &coping(), low, up_ramp * 2.0, n, 0), None);
        // On flat ground, or coming down: never.
        assert_eq!(
            select(&t, G, &coping(), com, up_ramp * 6.0, Vec3::UP, 0),
            None
        );
        assert_eq!(select(&t, G, &coping(), com, -up_ramp * 6.0, n, 0), None);
        // A rail running up the ramp is no coping.
        let up = vec![Rail::new(vec![v3(0.0, 0.0, 0.0), v3(0.0, 3.0, 1.73)])];
        assert_eq!(select(&t, G, &up, com, up_ramp * 6.0, n, 0), None);
    }

    #[test]
    fn the_manager_flags_a_new_coping_and_commits_near_the_apex() {
        let t = HandPlantTuning::BUILT_IN;
        let mut m = Manager::default();
        let g = |com: Vec3, v: f32| Ground {
            com,
            vel: v3(0.0, 0.866, 0.5) * v,
            normal: v3(0.0, 0.5, -0.866),
            heading: v3(0.0, 0.866, 0.5),
            side: v3(1.0, 0.0, 0.0),
            gravity: G,
            anim_committed: false,
        };
        m.query(&t, &coping(), &g(v3(0.0, 2.6, 0.9), 6.0));
        assert!(m.found && m.new_pos, "{m:?}");
        assert!(
            m.apex.y > 3.5 && m.apex.z < 1.73,
            "over and back over the ramp: {m:?}"
        );
        assert!(m.phase > 0.0 && m.phase < 1.0, "{}", m.phase);
        m.query(&t, &coping(), &g(v3(0.0, 2.65, 0.92), 6.0));
        assert!(m.found && !m.new_pos, "the same coping: {m:?}");
        // Nearer the apex it is committed.
        assert!(m.phase < t.committed_time || !m.committed);
    }

    #[test]
    fn a_half_turn_blends_through_the_middle() {
        let a = Basis::of(v3(0.0, 0.0, 1.0), Vec3::UP);
        let b = Basis::of(v3(0.0, 0.0, -1.0), Vec3::UP);
        let m = a.blend(b, 0.5);
        assert!(m.fwd.z.abs() < 1e-3 && m.up.y > 0.999, "{m:?}");
        let c = Basis::of(v3(0.0, 0.0, 1.0), -Vec3::UP);
        let h = a.blend(c, 0.5);
        assert!(h.up.y.abs() < 1e-3, "{h:?}");
    }

    #[test]
    fn names_follow_the_tweak_and_the_feet() {
        assert_eq!(name(true, [0.0, 0.0], Feet::Both), "fs_handplant");
        assert_eq!(name(false, [0.0, 0.0], Feet::Both), "bs_handplant");
        assert_eq!(name(true, [0.0, 0.0], Feet::None), "fs_handplant_nofoot");
        assert_eq!(
            name(false, [0.0, 0.0], Feet::OneBack),
            "bs_handplant_onefoot_back"
        );
        assert_ne!(name(true, [1.0, 0.0], Feet::Both), "fs_handplant");
        assert_eq!(display("fs_handplant_up"), "FS HANDPLANT UP");
    }
}
