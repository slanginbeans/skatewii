//! The rider's body in a bail: a light ragdoll (Skate 3 hands the rider's 24 physical bones to its
//! rigid-body solver in a wipeout, SK8-ENGINE/skate-3-rust-engine, GPL-3.0:
//! `skate-game/src/physics/wipeout_states/ragdoll*`, `skate-core/src/player/wipeout_state/`).
//!
//! flatground's is cheaper, for the Wii's CPU: fifteen point masses at the main joints (Verlet
//! integration at the game's fixed tick), held together by distance links (the bones, braces that keep
//! the torso and hips rigid) and joint limits (minimum and maximum distances, knees bending forwards,
//! the head kept up off the chest), colliding with the park through its ground and ray queries (one
//! ground query and at most one wall ray per point a tick) as spheres with Skate 3's ragdoll friction
//! and restitution. Eleven body parts (pelvis, torso, head, upper and lower arms and legs) are framed
//! from the points: they draw the placeholder rider, place the imported skeleton's bones and take the
//! injuries (`skater::meat`). Drives pull the points towards a pose held in the pelvis's frame (the
//! animated pose at the bail, or the wipeout gesture's), Skate 3's drives on its physical skeleton.
//! Fixed-size arrays only: nothing is allocated while it runs.

// The points are fixed arrays indexed together (positions, masses, radii, contacts): index loops read
// clearest.
#![allow(clippy::needless_range_loop)]

use crate::math::{Vec3, v3};
use crate::park::Terrain;

/// The points: pelvis, chest, head; hips, shoulders; elbows, hands; knees, feet (left then right).
pub const POINTS: usize = 15;
pub const PELVIS: usize = 0;
pub const CHEST: usize = 1;
pub const HEAD: usize = 2;
pub const L_HIP: usize = 3;
pub const R_HIP: usize = 4;
pub const L_SHOULDER: usize = 5;
pub const R_SHOULDER: usize = 6;
pub const L_ELBOW: usize = 7;
pub const L_HAND: usize = 8;
pub const R_ELBOW: usize = 9;
pub const R_HAND: usize = 10;
pub const L_KNEE: usize = 11;
pub const L_FOOT: usize = 12;
pub const R_KNEE: usize = 13;
pub const R_FOOT: usize = 14;

/// The body parts (drawn, posing the imported bones, injured).
pub const PARTS: usize = 11;
pub const PART_PELVIS: usize = 0;
pub const PART_TORSO: usize = 1;
pub const PART_HEAD: usize = 2;

/// The rider's mass (kg, ours), and each point's share of it (ours, from Skate 3's part volumes in
/// `rider.rs` `COM_PARTS`: the legs' and the torso's shares folded onto the nearest point).
pub const BODY_MASS: f32 = 75.0;
pub const MASS: [f32; POINTS] = [
    0.14, 0.18, 0.08, 0.07, 0.07, 0.05, 0.05, 0.035, 0.015, 0.035, 0.015, 0.065, 0.03, 0.065, 0.03,
];
/// Each point's collision sphere (m, ours: about the body's thickness there).
pub const RADIUS: [f32; POINTS] = [
    0.12, 0.13, 0.11, 0.09, 0.09, 0.07, 0.07, 0.05, 0.05, 0.05, 0.05, 0.06, 0.06, 0.06, 0.06,
];

/// The links held at their length from the start of the bail: bones, and braces that keep the hips,
/// the torso and the neck from folding.
const LINKS: [(usize, usize); 26] = [
    (PELVIS, L_HIP),
    (PELVIS, R_HIP),
    (L_HIP, R_HIP),
    (PELVIS, CHEST),
    (CHEST, L_SHOULDER),
    (CHEST, R_SHOULDER),
    (L_SHOULDER, R_SHOULDER),
    (CHEST, HEAD),
    (L_SHOULDER, L_ELBOW),
    (L_ELBOW, L_HAND),
    (R_SHOULDER, R_ELBOW),
    (R_ELBOW, R_HAND),
    (L_HIP, L_KNEE),
    (L_KNEE, L_FOOT),
    (R_HIP, R_KNEE),
    (R_KNEE, R_FOOT),
    // The torso box.
    (L_HIP, L_SHOULDER),
    (R_HIP, R_SHOULDER),
    (L_HIP, R_SHOULDER),
    (R_HIP, L_SHOULDER),
    (PELVIS, L_SHOULDER),
    (PELVIS, R_SHOULDER),
    (CHEST, L_HIP),
    (CHEST, R_HIP),
    // The neck.
    (HEAD, L_SHOULDER),
    (HEAD, R_SHOULDER),
];
/// Joint limits: point pairs kept between these shares of their distance at the start of the bail
/// (ours: knees and elbows don't fold flat, the knees and hands don't go through the body, the chin
/// doesn't go into the chest), with an absolute least distance (m).
const LIMITS: [(usize, usize, f32, f32, f32); 14] = [
    (L_HIP, L_FOOT, 0.45, 1.05, 0.0),
    (R_HIP, R_FOOT, 0.45, 1.05, 0.0),
    (L_SHOULDER, L_HAND, 0.35, 1.05, 0.0),
    (R_SHOULDER, R_HAND, 0.35, 1.05, 0.0),
    (L_KNEE, CHEST, 0.0, 10.0, 0.28),
    (R_KNEE, CHEST, 0.0, 10.0, 0.28),
    (L_KNEE, R_KNEE, 0.0, 10.0, 0.13),
    (L_FOOT, R_FOOT, 0.0, 10.0, 0.12),
    (L_HAND, HEAD, 0.0, 10.0, 0.16),
    (R_HAND, HEAD, 0.0, 10.0, 0.16),
    (L_HAND, CHEST, 0.0, 10.0, 0.15),
    (R_HAND, CHEST, 0.0, 10.0, 0.15),
    (L_FOOT, PELVIS, 0.0, 10.0, 0.25),
    (R_FOOT, PELVIS, 0.0, 10.0, 0.25),
];

/// A body part's frame from the points: origin, its y along `y.0 -> y.1`, its x towards `x.0 -> x.1`.
#[derive(Clone, Copy, Debug)]
struct PartDef {
    origin: usize,
    y: (usize, usize),
    x: (usize, usize),
}

const fn part(origin: usize, y: (usize, usize), x: (usize, usize)) -> PartDef {
    PartDef { origin, y, x }
}

const PART_DEFS: [PartDef; PARTS] = [
    part(PELVIS, (PELVIS, CHEST), (L_HIP, R_HIP)),
    part(PELVIS, (PELVIS, CHEST), (L_SHOULDER, R_SHOULDER)),
    part(HEAD, (CHEST, HEAD), (L_SHOULDER, R_SHOULDER)),
    part(L_SHOULDER, (L_SHOULDER, L_ELBOW), (PELVIS, CHEST)),
    part(L_ELBOW, (L_ELBOW, L_HAND), (PELVIS, CHEST)),
    part(R_SHOULDER, (R_SHOULDER, R_ELBOW), (PELVIS, CHEST)),
    part(R_ELBOW, (R_ELBOW, R_HAND), (PELVIS, CHEST)),
    part(L_HIP, (L_HIP, L_KNEE), (L_HIP, R_HIP)),
    part(L_KNEE, (L_KNEE, L_FOOT), (L_HIP, R_HIP)),
    part(R_HIP, (R_HIP, R_KNEE), (L_HIP, R_HIP)),
    part(R_KNEE, (R_KNEE, R_FOOT), (L_HIP, R_HIP)),
];

/// The part each point's impacts injure.
pub const POINT_PART: [usize; POINTS] = [0, 1, 2, 0, 0, 1, 1, 3, 4, 5, 6, 7, 8, 9, 10];

/// Constraint passes a tick.
const ITERATIONS: usize = 6;
/// A point's ground is looked for this far over it (m).
const GROUND_REACH: f32 = 0.05;
/// A height-field rise past this over the point is a wall (m).
const WALL_RISE: f32 = 0.3;
/// Surfaces this steep are walls (the trimesh's own walkable limit).
const WALL_UP: f32 = 0.35;

/// A part's frame: three axis rows (x, y, z) then the origin, as `character::Rows`.
pub type Frame = [[f32; 3]; 4];

/// The standing rider the placeholder is drawn as, in its board space (goofy, facing +x, the left foot
/// at -z; m): where the points start without the imported skeleton.
pub const REST: [Vec3; POINTS] = [
    v3(0.0, 0.88, 0.0),
    v3(0.0, 1.33, 0.0),
    v3(0.0, 1.62, 0.0),
    v3(0.0, 0.88, -0.10),
    v3(0.0, 0.88, 0.10),
    v3(0.0, 1.37, -0.17),
    v3(0.0, 1.37, 0.17),
    v3(0.0, 1.34, -0.42),
    v3(0.0, 1.34, -0.66),
    v3(0.0, 1.34, 0.42),
    v3(0.0, 1.34, 0.66),
    v3(0.04, 0.50, -0.21),
    v3(0.0, 0.14, -0.22),
    v3(0.04, 0.50, 0.19),
    v3(0.0, 0.14, 0.20),
];

/// What the body touched in a tick: each point's ground or wall (a plane), how fast it came into it
/// (m/s), and whether it touches anything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contacts {
    pub touching: [bool; POINTS],
    pub impact: [f32; POINTS],
    pub normal: [Vec3; POINTS],
}

impl Default for Contacts {
    fn default() -> Self {
        Contacts {
            touching: [false; POINTS],
            impact: [0.0; POINTS],
            normal: [Vec3::UP; POINTS],
        }
    }
}

impl Contacts {
    pub fn any(&self) -> bool {
        self.touching.iter().any(|t| *t)
    }

    /// The touching points' surfaces' up, averaged (`Vec3::UP` touching nothing).
    pub fn normal(&self) -> Vec3 {
        let mut n = Vec3::ZERO;
        for i in 0..POINTS {
            if self.touching[i] {
                n += self.normal[i];
            }
        }
        n.normalize_or(Vec3::UP)
    }
}

/// The body's surroundings and settings for a tick.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub gravity: f32,
    /// Relative velocity damping (per second) about the centre of mass, and of the whole body.
    pub angular_drag: f32,
    pub linear_drag: f32,
    /// Sliding friction and restitution (the head's apart).
    pub friction: f32,
    pub restitution: f32,
    pub head_friction: f32,
    pub head_restitution: f32,
    /// How much of the gap to the drive targets is closed a tick (0..1).
    pub drive: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub pos: [Vec3; POINTS],
    /// Where each point was a tick ago (its velocity is `(pos - prev) / dt`).
    pub prev: [Vec3; POINTS],
    /// The positions after the tick before (for drawing between ticks).
    pub last: [Vec3; POINTS],
    rest: [f32; LINKS.len()],
    limit_rest: [f32; LIMITS.len()],
    /// +1 when the points' right x up is the body's front, -1 for a mirror-image skeleton.
    pub hand: f32,
    /// The drives' targets in the pelvis's frame (x right, y up, z forwards).
    pub target: [Vec3; POINTS],
    pub contacts: Contacts,
}

impl Default for Body {
    fn default() -> Self {
        Body::new(&REST, Vec3::ZERO, 1.0 / 60.0)
    }
}

fn inv_mass(i: usize) -> f32 {
    1.0 / MASS[i]
}

impl Body {
    /// A body at `points` (world), all moving at `vel`.
    pub fn new(points: &[Vec3; POINTS], vel: Vec3, dt: f32) -> Body {
        let mut b = Body {
            pos: *points,
            prev: [Vec3::ZERO; POINTS],
            last: *points,
            rest: [0.0; LINKS.len()],
            limit_rest: [0.0; LIMITS.len()],
            hand: 1.0,
            target: [Vec3::ZERO; POINTS],
            contacts: Contacts::default(),
        };
        for (i, p) in b.prev.iter_mut().enumerate() {
            *p = points[i] - vel * dt;
        }
        for (k, &(a, c)) in LINKS.iter().enumerate() {
            b.rest[k] = (points[a] - points[c]).length();
        }
        for (k, &(a, c, ..)) in LIMITS.iter().enumerate() {
            b.limit_rest[k] = (points[a] - points[c]).length();
        }
        // Which way the knees bend tells the front from the back (a mirrored skeleton's right x up
        // points backwards).
        let [r, u, f] = b.axes_with(1.0);
        let _ = (r, u);
        let mut bend = 0.0;
        for (hip, knee, foot) in [(L_HIP, L_KNEE, L_FOOT), (R_HIP, R_KNEE, R_FOOT)] {
            let mid = (points[hip] + points[foot]) * 0.5;
            bend += (points[knee] - mid).dot(f);
        }
        b.hand = if bend < 0.0 { -1.0 } else { 1.0 };
        b.hold_pose();
        b
    }

    /// The drives hold the body as it is now.
    pub fn hold_pose(&mut self) {
        let pos = self.pos;
        self.set_targets(&pos);
    }

    /// The drives' targets: a pose (world, any place) taken into the pelvis's frame.
    pub fn set_targets(&mut self, pose: &[Vec3; POINTS]) {
        let [r, u, f] = axes_of(pose, self.hand);
        let o = pose[PELVIS];
        for i in 0..POINTS {
            let d = pose[i] - o;
            self.target[i] = v3(d.dot(r), d.dot(u), d.dot(f));
        }
    }

    /// The drives' targets in the pelvis's frame directly (x right, y up, z forwards).
    pub fn set_local_targets(&mut self, local: &[Vec3; POINTS]) {
        self.target = *local;
    }

    /// The pelvis's right, up and front.
    pub fn axes(&self) -> [Vec3; 3] {
        self.axes_with(self.hand)
    }

    fn axes_with(&self, hand: f32) -> [Vec3; 3] {
        axes_of(&self.pos, hand)
    }

    /// The centre of mass and its velocity.
    pub fn com(&self) -> Vec3 {
        weighted(&self.pos)
    }

    pub fn com_vel(&self, dt: f32) -> Vec3 {
        let mut v = Vec3::ZERO;
        let mut m = 0.0;
        for i in 0..POINTS {
            v += (self.pos[i] - self.prev[i]) * MASS[i];
            m += MASS[i];
        }
        v * (1.0 / (m * dt))
    }

    pub fn vel(&self, i: usize, dt: f32) -> Vec3 {
        (self.pos[i] - self.prev[i]) * (1.0 / dt)
    }

    /// Adds to every point's velocity.
    pub fn add_velocity(&mut self, dv: Vec3, dt: f32) {
        for p in &mut self.prev {
            *p -= dv * dt;
        }
    }

    /// Caps every point's speed (SK8 `limit_velocity`).
    pub fn limit_speed(&mut self, max: f32, dt: f32) {
        for i in 0..POINTS {
            let v = self.pos[i] - self.prev[i];
            let l = v.length() / dt;
            if l > max {
                self.prev[i] = self.pos[i] - v * (max / l);
            }
        }
    }

    /// The body's angular velocity about its centre of mass: its angular momentum through its inertia
    /// (SK8's `torque::angular_velocity` estimates the same).
    pub fn angular_velocity(&self, dt: f32) -> Vec3 {
        let c = self.com();
        let cv = self.com_vel(dt);
        let mut l = Vec3::ZERO;
        // The inertia tensor (symmetric): xx, yy, zz, xy, xz, yz.
        let mut t = [0.0f32; 6];
        for i in 0..POINTS {
            let m = MASS[i];
            let r = self.pos[i] - c;
            l += r.cross(self.vel(i, dt) - cv) * m;
            let rr = r.dot(r) + RADIUS[i] * RADIUS[i] * 0.4;
            t[0] += m * (rr - r.x * r.x);
            t[1] += m * (rr - r.y * r.y);
            t[2] += m * (rr - r.z * r.z);
            t[3] -= m * r.x * r.y;
            t[4] -= m * r.x * r.z;
            t[5] -= m * r.y * r.z;
        }
        solve3(t, l)
    }

    /// Turns the body's velocity about its centre of mass by `dw` (rad/s; SK8 `torque::apply`).
    pub fn add_spin(&mut self, dw: Vec3, dt: f32) {
        let c = self.com();
        for i in 0..POINTS {
            let dv = dw.cross(self.pos[i] - c);
            self.prev[i] -= dv * dt;
        }
    }

    /// One tick.
    pub fn step(&mut self, park: &dyn Terrain, s: &Settings, dt: f32) {
        self.last = self.pos;
        // Velocities, damped about the centre of mass (angular drag) and as a whole.
        let cv = self.com_vel(dt);
        let rel = (1.0 - s.angular_drag * dt).clamp(0.0, 1.0);
        let lin = (1.0 - s.linear_drag * dt).clamp(0.0, 1.0);
        let mut moved = [Vec3::ZERO; POINTS];
        for (i, m) in moved.iter_mut().enumerate() {
            let v = self.vel(i, dt);
            let v = (cv + (v - cv) * rel) * lin;
            *m = v3(v.x, v.y - s.gravity * dt, v.z) * dt;
            self.prev[i] = self.pos[i];
            self.pos[i] += *m;
        }
        // The drives: towards the pose held in the pelvis's frame, with the mean taken out so they
        // move no mass as a whole.
        if s.drive > 0.0 {
            let [r, u, f] = self.axes();
            let o = self.pos[PELVIS];
            let mut d = [Vec3::ZERO; POINTS];
            let mut mean = Vec3::ZERO;
            let mut m = 0.0;
            for i in 0..POINTS {
                let t = self.target[i];
                let goal = o + r * t.x + u * t.y + f * t.z;
                d[i] = (goal - self.pos[i]) * s.drive;
                mean += d[i] * MASS[i];
                m += MASS[i];
            }
            let mean = mean * (1.0 / m);
            for i in 0..POINTS {
                let c = d[i] - mean;
                self.pos[i] += c;
                // Half of it as velocity: the drives push, they don't only place.
                self.prev[i] += c * 0.5;
            }
        }
        // Collision planes for the tick: each point's ground, and a wall it moved into.
        let mut ground = [(Vec3::ZERO, Vec3::UP, false); POINTS];
        let mut wall = [(Vec3::ZERO, Vec3::UP, false); POINTS];
        for i in 0..POINTS {
            let r = RADIUS[i];
            let (a, mut b) = (self.prev[i], self.pos[i]);
            let h = b - a;
            let across = v3(h.x, 0.0, h.z).length();
            if across > 1e-3 {
                let dir = h * (1.0 / h.length());
                if let Some(hit) = park.raycast(a, dir, h.length() + r)
                    && hit.normal.y.abs() < WALL_UP
                {
                    let n = v3(hit.normal.x, 0.0, hit.normal.z).normalize_or(-dir);
                    wall[i] = (hit.point, n, true);
                }
            }
            let top = a.y.max(b.y) + r + GROUND_REACH;
            let mut g = park.ground(b.x, b.z, top);
            if g.height > top + WALL_RISE - GROUND_REACH {
                // A height field's wall: back where it was across.
                b = v3(a.x, b.y, a.z);
                self.pos[i] = b;
                self.prev[i] = v3(b.x, self.prev[i].y, b.z);
                g = park.ground(b.x, b.z, top);
            }
            if g.height > f32::MIN / 2.0 {
                ground[i] = (
                    v3(b.x, g.height, b.z),
                    g.normal.normalize_or(Vec3::UP),
                    true,
                );
            }
        }
        let mut contacts = Contacts::default();
        let mut push = [0.0f32; POINTS];
        for it in 0..ITERATIONS {
            for (k, &(a, b)) in LINKS.iter().enumerate() {
                self.hold(a, b, self.rest[k], self.rest[k]);
            }
            for (k, &(a, b, lo, hi, least)) in LIMITS.iter().enumerate() {
                let rest = self.limit_rest[k];
                self.hold(a, b, (rest * lo).max(least), rest * hi);
            }
            self.bend_limits();
            for i in 0..POINTS {
                for (q, n, on) in [ground[i], wall[i]] {
                    if !on {
                        continue;
                    }
                    let depth = RADIUS[i] - (self.pos[i] - q).dot(n);
                    if depth > 0.0 {
                        if !contacts.touching[i] {
                            // How fast it came in, before any pass pushed it out.
                            let v = moved[i] * (1.0 / dt);
                            contacts.impact[i] = contacts.impact[i].max(-v.dot(n));
                        }
                        contacts.touching[i] = true;
                        contacts.normal[i] = n;
                        self.pos[i] += n * depth;
                        push[i] += depth;
                    }
                }
            }
            let _ = it;
        }
        // Friction and restitution where it touched: the sliding along the surface loses up to
        // `friction` x the push out, the speed out of the surface is the speed in x restitution.
        for i in 0..POINTS {
            if !contacts.touching[i] {
                continue;
            }
            let (mu, e) = if i == HEAD {
                (s.head_friction, s.head_restitution)
            } else {
                (s.friction, s.restitution)
            };
            let n = contacts.normal[i];
            let d = self.pos[i] - self.prev[i];
            let dn = d.dot(n);
            let dt_ = d - n * dn;
            let slide = dt_.length();
            let keep = if slide > 1e-6 {
                (1.0 - mu * push[i] / slide).max(0.0)
            } else {
                0.0
            };
            let out = if dn > 0.0 {
                (contacts.impact[i] * dt * e).min(dn)
            } else {
                dn
            };
            self.prev[i] = self.pos[i] - dt_ * keep - n * out;
        }
        self.contacts = contacts;
    }

    /// Holds points `a` and `b` between `lo` and `hi` apart (moved by their inverse masses).
    fn hold(&mut self, a: usize, b: usize, lo: f32, hi: f32) {
        let d = self.pos[b] - self.pos[a];
        let l = d.length();
        if l < 1e-6 {
            return;
        }
        let want = l.clamp(lo, hi);
        if want == l {
            return;
        }
        let (wa, wb) = (inv_mass(a), inv_mass(b));
        let k = (l - want) / (l * (wa + wb));
        self.pos[a] += d * (k * wa);
        self.pos[b] -= d * (k * wb);
    }

    /// The knees bend forwards and the head stays up off the chest.
    fn bend_limits(&mut self) {
        let [_, u, f] = self.axes();
        for (hip, knee, foot) in [(L_HIP, L_KNEE, L_FOOT), (R_HIP, R_KNEE, R_FOOT)] {
            let mid = (self.pos[hip] + self.pos[foot]) * 0.5;
            let d = (self.pos[knee] - mid).dot(f);
            if d < -0.02 {
                let c = f * (-0.02 - d);
                let w = inv_mass(knee) / (inv_mass(knee) + inv_mass(hip));
                self.pos[knee] += c * w;
                self.pos[hip] -= c * ((1.0 - w) * 0.5);
                self.pos[foot] -= c * ((1.0 - w) * 0.5);
            }
        }
        let neck = self.pos[HEAD] - self.pos[CHEST];
        let l = neck.length();
        let up = neck.dot(u);
        if l > 1e-4 && up < 0.5 * l {
            let c = u * ((0.5 * l - up) * 0.5);
            self.pos[HEAD] += c;
        }
    }

    /// The points between the last two ticks (`k` 0: a tick ago, 1: now).
    pub fn between(&self, k: f32) -> [Vec3; POINTS] {
        let mut out = self.pos;
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.last[i].lerp(self.pos[i], k);
        }
        out
    }

    /// The body parts' frames from `points`.
    pub fn frames(&self, points: &[Vec3; POINTS]) -> [Frame; PARTS] {
        let mut out = [[[0.0; 3]; 4]; PARTS];
        let [_, _, front] = axes_of(points, self.hand);
        for (k, d) in PART_DEFS.iter().enumerate() {
            let y = (points[d.y.1] - points[d.y.0]).normalize_or(Vec3::UP);
            let side = points[d.x.1] - points[d.x.0];
            let mut x = side - y * side.dot(y);
            if x.length() < 1e-3 {
                x = front.cross(y);
            }
            let x = x.normalize_or(v3(1.0, 0.0, 0.0));
            let z = x.cross(y);
            let o = points[d.origin];
            out[k] = [
                [x.x, x.y, x.z],
                [y.x, y.y, y.z],
                [z.x, z.y, z.z],
                [o.x, o.y, o.z],
            ];
        }
        out
    }
}

/// Solves the symmetric 3x3 system `t` (xx, yy, zz, xy, xz, yz) for `b` (zero when singular).
fn solve3(t: [f32; 6], b: Vec3) -> Vec3 {
    let [a, e, i, b2, c, f] = t;
    let (d1, d2, d3) = (e * i - f * f, c * f - b2 * i, b2 * f - c * e);
    let det = a * d1 + b2 * d2 + c * d3;
    if det.abs() < 1e-9 {
        return Vec3::ZERO;
    }
    let inv = 1.0 / det;
    let (d5, d6, d9) = (a * i - c * c, b2 * c - a * f, a * e - b2 * b2);
    v3(
        (d1 * b.x + d2 * b.y + d3 * b.z) * inv,
        (d2 * b.x + d5 * b.y + d6 * b.z) * inv,
        (d3 * b.x + d6 * b.y + d9 * b.z) * inv,
    )
}

/// The mass-weighted centre of `points`.
pub fn weighted(points: &[Vec3; POINTS]) -> Vec3 {
    let mut c = Vec3::ZERO;
    let mut m = 0.0;
    for i in 0..POINTS {
        c += points[i] * MASS[i];
        m += MASS[i];
    }
    c * (1.0 / m)
}

/// A pose's pelvis frame: right (hip to hip), up (pelvis to chest, square to the right) and front
/// (right x up, turned round for a mirror-image skeleton).
fn axes_of(p: &[Vec3; POINTS], hand: f32) -> [Vec3; 3] {
    let r = (p[R_HIP] - p[L_HIP]).normalize_or(v3(1.0, 0.0, 0.0));
    let up = p[CHEST] - p[PELVIS];
    let u = (up - r * up.dot(r)).normalize_or(Vec3::UP);
    let f = r.cross(u) * hand;
    [r, u, f]
}

/// The rest pose in the pelvis's frame (x right, y up, z forwards): where the gesture poses are drawn
/// from (`skater::wipeout`).
pub fn rest_local() -> [Vec3; POINTS] {
    // The placeholder faces +x with its left at -z: right is +z, front +x.
    let o = REST[PELVIS];
    let mut out = [Vec3::ZERO; POINTS];
    for (i, o2) in out.iter_mut().enumerate() {
        let d = REST[i] - o;
        *o2 = v3(d.z, d.y, d.x);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::Park;

    const DT: f32 = 1.0 / 60.0;

    fn settings() -> Settings {
        Settings {
            gravity: 9.8,
            angular_drag: 0.5,
            linear_drag: 0.0,
            friction: 0.9,
            restitution: 0.0,
            head_friction: 0.9,
            head_restitution: 0.0,
            drive: 0.0,
        }
    }

    /// The rest pose standing at `at` (board space as the world).
    fn standing(at: Vec3) -> [Vec3; POINTS] {
        let mut p = REST;
        for q in &mut p {
            *q += at;
        }
        p
    }

    struct Flat;
    impl Terrain for Flat {
        fn ground(&self, _x: f32, _z: f32, _max: f32) -> crate::park::Surface {
            crate::park::Surface {
                height: 0.0,
                normal: Vec3::UP,
            }
        }
        fn spawn(&self) -> (Vec3, f32) {
            (Vec3::ZERO, 0.0)
        }
    }

    #[test]
    fn the_links_hold_their_lengths_through_a_fall() {
        let mut b = Body::new(&standing(v3(0.0, 3.0, 0.0)), v3(2.0, 0.0, 1.0), DT);
        let rest = b.rest;
        for _ in 0..240 {
            b.step(&Flat, &settings(), DT);
        }
        for (k, &(a, c)) in LINKS.iter().enumerate() {
            let l = (b.pos[a] - b.pos[c]).length();
            assert!(
                (l - rest[k]).abs() < 0.03 * rest[k] + 0.01,
                "{a}-{c}: {l} vs {}",
                rest[k]
            );
        }
        // On the ground, not in it, at rest.
        for i in 0..POINTS {
            assert!(b.pos[i].y >= RADIUS[i] - 0.02, "{i}: {}", b.pos[i].y);
        }
        assert!(b.com_vel(DT).length() < 0.3, "{:?}", b.com_vel(DT));
        assert!(b.contacts.any());
    }

    #[test]
    fn it_is_deterministic() {
        let run = || {
            let park = Park::demo();
            let (spawn, _) = park.spawn();
            let mut b = Body::new(&standing(spawn + v3(0.0, 1.0, 0.0)), v3(4.0, 1.0, 3.0), DT);
            for _ in 0..180 {
                b.step(&park, &settings(), DT);
            }
            b
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_hard_landing_reports_the_impact_on_the_points_that_hit() {
        let mut b = Body::new(&standing(v3(0.0, 0.5, 0.0)), v3(0.0, -8.0, 0.0), DT);
        let mut hit = [0.0f32; POINTS];
        for _ in 0..30 {
            b.step(&Flat, &settings(), DT);
            for i in 0..POINTS {
                hit[i] = hit[i].max(b.contacts.impact[i]);
            }
        }
        assert!(hit[L_FOOT] > 7.0 && hit[R_FOOT] > 7.0, "{hit:?}");
        // Restitution 0: it doesn't bounce.
        assert!(b.pos[PELVIS].y < 1.4, "{}", b.pos[PELVIS].y);
    }

    #[test]
    fn walls_stop_the_body() {
        // A wall facing -x at x = 1 (a height field rising 2 m).
        struct Wall;
        impl Terrain for Wall {
            fn ground(&self, x: f32, _z: f32, _max: f32) -> crate::park::Surface {
                crate::park::Surface {
                    height: if x >= 1.0 { 2.0 } else { 0.0 },
                    normal: Vec3::UP,
                }
            }
            fn spawn(&self) -> (Vec3, f32) {
                (Vec3::ZERO, 0.0)
            }
        }
        let mut b = Body::new(&standing(v3(0.0, 0.0, 0.0)), v3(6.0, 0.0, 0.0), DT);
        for _ in 0..120 {
            b.step(&Wall, &settings(), DT);
        }
        for i in 0..POINTS {
            assert!(b.pos[i].x < 1.0 + 1e-3, "{i}: {:?}", b.pos[i]);
        }
    }

    #[test]
    fn spin_turns_the_body_and_drives_hold_a_pose() {
        let mut b = Body::new(&standing(v3(0.0, 20.0, 0.0)), Vec3::ZERO, DT);
        b.add_spin(v3(0.0, 3.0, 0.0), DT);
        let w = b.angular_velocity(DT);
        assert!((w.y - 3.0).abs() < 0.5, "{w:?}");
        // Held by the drives, the knee comes up to a tucked target.
        let mut tuck = rest_local();
        tuck[L_KNEE] = v3(-0.1, 0.1, 0.35);
        tuck[L_FOOT] = v3(-0.1, -0.2, 0.3);
        b.set_local_targets(&tuck);
        let s = Settings {
            drive: 0.3,
            ..settings()
        };
        let before = (b.pos[L_KNEE] - b.pos[PELVIS]).dot(b.axes()[1]);
        for _ in 0..30 {
            b.step(&Flat, &s, DT);
        }
        let after = (b.pos[L_KNEE] - b.pos[PELVIS]).dot(b.axes()[1]);
        assert!(after > before + 0.2, "{before} -> {after}");
        // The parts' frames are right-handed and unit.
        for f in b.frames(&b.pos) {
            let x = v3(f[0][0], f[0][1], f[0][2]);
            let y = v3(f[1][0], f[1][1], f[1][2]);
            let z = v3(f[2][0], f[2][1], f[2][2]);
            assert!((x.cross(y) - z).length() < 1e-4);
            assert!(x.dot(y).abs() < 1e-4);
        }
    }
}
