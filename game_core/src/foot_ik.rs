//! Off the board, the feet on the ground under them: Skate 3's off-board foot placement and its skeleton
//! IK's leg solve, as SK8-ENGINE/skate-3-rust-engine reads them (GPL-3.0; skate-core
//! player/offboard/feet.rs and biped_air/feet.rs, TU3 82D773F8 / 82D77558 / 82D77AE8; skate-game
//! physics/biped_ground/feet.rs 82D77330 and physics/foot_ik_queries.rs 82DB63C0; skate-core
//! animation/foot_ik external.rs 82BEE390, status.rs, two_bone.rs 8296F8C0).
//!
//! - Each toe looks down for the ground (0.33 m over it to 1.5 m under). A foot whose ground is over
//!   0.3 m from it, and over 0.3 m from the other's, is ignored.
//! - A foot is low when the clip has it within 0.09 m of the ground plane, supported when it is within
//!   0.09 m of the ground found under it (0.2 m when low). On the ground, exactly one supported foot is
//!   the support: it stays where it was in the world (following the clip by at most 0.08 m a tick).
//! - The other foot follows the clip, kept at least 0.02 m over the ground under it (and at most 0.08
//!   m higher than the clip has it over its own ground plane), never raised more than 0.4 m or past
//!   0.46 m under the hips; let go as a support, it blends back over 0.1 s, and its correction eases
//!   in at 0.3 of the way, by at most 0.025 m a tick.
//! - A supported foot tilts to the ground (its normal kept within 0.9 of up, turned by at most 0.1 rad
//!   and blended in by 0.2 a tick, the tilt at most 0.71 rad); a foot is moved at most 0.7 m.
//! - The leg (thigh, shin) is solved to put the ankle where the foot goes, the knee kept between 20 and
//!   165 degrees (`IKMinAngle` / `IKMaxAngle`). Off the feet's ground (on the board) the correction
//!   fades by 0.15 a tick (`FeetIKMaxBlendDeltaOut`).

use crate::character::{Character, Rows};
use crate::math::{Vec3, Xform, cosf, sinf, sqrtf, v3};

/// The toe's ground test: from this far over the toe (0x3EA8F5C3) to this far under it (m).
pub const LINE_UP: f32 = 0.33;
pub const LINE_DOWN: f32 = 1.5;
/// A foot's ground this far from it, and from the other foot's, is ignored (m).
const DISAGREE: f32 = 0.3;
/// Low (the clip's foot near its ground plane), supported, supported when low (m).
const LOW: f32 = 0.09;
const SUPPORTED: f32 = 0.09;
const SUPPORTED_LOW: f32 = 0.2;
/// The support foot follows the clip by at most this a tick (m; 0x3BD1B717 is its square).
const FOLLOW: f32 = 0.08;
/// The swing foot over the ground: at least, and at most this much over that (m); the most it is
/// raised, and how far under the hips it must stay.
const CLEAR: f32 = 0.02;
const CLEAR_RANGE: f32 = 0.08;
const MOST_RAISED: f32 = 0.4;
const UNDER_HIPS: f32 = 0.46;
/// A foot let go as the support blends back over this (s), and a swing foot's correction eases in at
/// this weight, by at most this a tick (m).
const RELEASE: f32 = 0.1;
const LOCAL_WEIGHT: f32 = 0.3;
const LOCAL_STEP: f32 = 0.025;
/// The most a foot is moved (m), its ground normal's least up, and the tilt's most (rad), its turn a
/// tick (rad) and blend a tick.
const MOST_MOVED: f32 = 0.7;
const NORMAL_UP: f32 = 0.9;
const MOST_TILT: f32 = 0.71;
const NORMAL_TURN: f32 = 0.1;
const NORMAL_BLEND: f32 = 0.2;
/// Off the feet's ground the correction fades by this a tick (`physics_skeletonik/FeetIKMaxBlendDeltaOut`).
pub const BLEND_OUT: f32 = 0.15;
/// The knee's least and most angle (degrees: `physics_animation/IKMinAngle`, `IKMaxAngle`).
pub const KNEE_MIN: f32 = 20.0;
pub const KNEE_MAX: f32 = 165.0;
/// Skate 3's tick (the placement's timers count it).
const TICK: f32 = 1.0 / 60.0;
const DEG: f32 = crate::math::PI / 180.0;

/// The legs' bones: thigh, shin, foot (ankle) and toe, left then right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Legs {
    pub thigh: [usize; 2],
    pub shin: [usize; 2],
    pub ankle: [usize; 2],
    pub toe: [usize; 2],
    pub hips: usize,
}

impl Legs {
    pub fn find(ch: &Character) -> Option<Legs> {
        let pair = |l: &str, r: &str| Some([ch.bone(l)?, ch.bone(r)?]);
        Some(Legs {
            thigh: pair("LEFTUPLEG", "RIGHTUPLEG")?,
            shin: pair("LEFTLEG", "RIGHTLEG")?,
            ankle: pair("LEFTFOOT", "RIGHTFOOT")?,
            toe: pair("LEFTTOEBASE", "RIGHTTOEBASE")?,
            hips: ch.bone("HIPS")?,
        })
    }
}

/// What the placement reads this tick: the character's placement in the world, the ground found under
/// each toe (point and normal, world: from the last pose's toes), whether the rider is in the air, and
/// his vertical speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub to_world: Xform,
    pub ground: [Option<(Vec3, Vec3)>; 2],
    pub air: bool,
    pub vel_y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Foot {
    /// Where it goes (world; SK8 vectors_0).
    target: Vec3,
    /// Blending back after being the support (s; scalars_96_100[1]).
    release: f32,
    /// The ground's slope under a low foot, raising it (scalars_96_100[0]).
    slope: f32,
    /// The eased correction of a swing foot (character space; external_target_local_delta).
    delta: Vec3,
    /// The last correction (character space), its blend, and the tilt to the ground.
    offset: Vec3,
    blend: f32,
    normal: Vec3,
    normal_blend: f32,
}

/// The placement's state between ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FeetIk {
    feet: [Foot; 2],
    /// 0 / 1 the left / right foot holds the ground, 2 neither, 3 in the air (SK8 word_300).
    support: u8,
    lock: Vec3,
    /// The character's origin height last tick (scalars_288_to_296[0]).
    root_y: f32,
    started: bool,
}

/// `v` turned about the unit `axis` by the angle of cosine `c`, sine `s` (Rodrigues).
fn turn(v: Vec3, axis: Vec3, c: f32, s: f32) -> Vec3 {
    v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
}

/// The turn taking the unit `a` to the unit `b`: axis, cosine, sine (`None` when they are the same way).
fn arc(a: Vec3, b: Vec3) -> Option<(Vec3, f32, f32)> {
    let k = a.cross(b);
    let s = k.length();
    (s > 1e-6).then(|| (k * (1.0 / s), a.dot(b).clamp(-1.0, 1.0), s))
}

fn rows_turned(m: &Rows, axis: Vec3, c: f32, s: f32, at: Vec3) -> Rows {
    let r = |i: usize| turn(v3(m[i][0], m[i][1], m[i][2]), axis, c, s);
    let (x, y, z) = (r(0), r(1), r(2));
    [
        [x.x, x.y, x.z],
        [y.x, y.y, y.z],
        [z.x, z.y, z.z],
        [at.x, at.y, at.z],
    ]
}

fn at(m: &Rows) -> Vec3 {
    v3(m[3][0], m[3][1], m[3][2])
}

fn limit(v: Vec3, most: f32) -> Vec3 {
    let l = v.length();
    if l > most { v * (most / l) } else { v }
}

/// The character-space vector of a world one (`to_world`'s rotation is orthonormal).
fn local_dir(x: &Xform, v: Vec3) -> Vec3 {
    let c = &x.rot.cols;
    v3(c[0].dot(v), c[1].dot(v), c[2].dot(v))
}

/// SK8 82D78260: a normal under 0.9 up is laid back to 0.9 up, its horizontal part scaled to keep it
/// unit length (sqrt of 0.19: 0x3E428F5F).
fn clamp_normal(n: Vec3) -> Vec3 {
    if n.y >= NORMAL_UP {
        return n;
    }
    let h = v3(n.x, 0.0, n.z).normalize_or(v3(0.0, 0.0, 1.0)) * sqrtf(0.19);
    v3(h.x, NORMAL_UP, h.z)
}

impl FeetIk {
    /// Places the posed feet (`ch`'s bones in character space), or with `None` fades the correction out.
    pub fn update(&mut self, ch: &mut Character, legs: &Legs, frame: Option<&Frame>) {
        let Some(f) = frame else {
            self.started = false;
            for i in 0..2 {
                let foot = &mut self.feet[i];
                foot.blend = (foot.blend - BLEND_OUT).max(0.0);
                foot.normal_blend = (foot.normal_blend - NORMAL_BLEND).max(0.0);
                if foot.blend > 0.0 {
                    let (offset, normal, k) = (foot.offset, foot.normal, foot.blend);
                    solve_leg(ch, legs, i, offset * k, normal, foot.normal_blend * k);
                }
            }
            return;
        };
        let x = f.to_world;
        let world = ch.world();
        let toe_c = legs.toe.map(|b| at(&world[b]));
        // The foot's two volumes (SK8 parts 15 and 16): the toe, and the middle of the foot (between the
        // ankle and the toe: the skeleton has no foot volume, whose centre sits low in the shoe).
        let mid_c: [Vec3; 2] =
            core::array::from_fn(|i| (at(&world[legs.ankle[i]]) + toe_c[i]) * 0.5);
        let hips_w = x.point(at(&world[legs.hips]));
        let toe_w = toe_c.map(|p| x.point(p));
        if !self.started {
            self.started = true;
            self.support = 2;
            self.root_y = x.pos.y;
            for (foot, toe) in self.feet.iter_mut().zip(toe_w) {
                foot.target = toe;
            }
        }
        // Each toe's ground; one far off it and off the other's is ignored (82D77330).
        let mut ground = f.ground;
        if let [Some(a), Some(b)] = ground {
            let d = [a.0.y - toe_w[0].y, b.0.y - toe_w[1].y];
            for i in 0..2 {
                if d[i].abs() > DISAGREE && (d[i] - d[1 - i]).abs() > DISAGREE {
                    ground[i] = None;
                }
            }
        }
        // Low and supported (82D783A8, the old support mode read first).
        let mut low = [false; 2];
        let mut supported = [false; 2];
        let mut normals = [Vec3::UP; 2];
        for i in 0..2 {
            low[i] = self.support != 3 && toe_c[i].y < LOW && mid_c[i].y < LOW;
            supported[i] = match ground[i] {
                Some((p, n)) => {
                    let n = clamp_normal(n);
                    normals[i] = n;
                    let mid_w = x.point(mid_c[i]);
                    let d = n.dot(toe_w[i] - p).max(n.dot(mid_w - p));
                    d < SUPPORTED || (low[i] && d < SUPPORTED_LOW)
                }
                None => low[i],
            };
        }
        let support = if f.air {
            3
        } else {
            match supported {
                [true, false] => 0,
                [false, true] => 1,
                _ => 2,
            }
        };
        if support != self.support {
            self.support = support;
            self.lock = if support < 2 {
                toe_w[support as usize]
            } else {
                Vec3::ZERO
            };
        }
        for i in 0..2 {
            let mut desired = toe_w[i];
            let foot = &mut self.feet[i];
            if self.support == i as u8 {
                // The support stays put, following the clip by at most 0.08 m a tick.
                let flat = v3(self.lock.x - desired.x, 0.0, self.lock.z - desired.z);
                if flat.dot(flat) < FOLLOW * FOLLOW {
                    desired = self.lock;
                } else {
                    let y = self.lock.y;
                    desired += flat * (FOLLOW / flat.length());
                    desired.y = y;
                    self.lock = desired;
                }
                foot.target = desired;
                foot.release = RELEASE;
            } else {
                if let Some((p, _)) = ground[i] {
                    desired.y = foot_height(
                        self.root_y,
                        p.y,
                        desired.y - foot.slope.max(0.0),
                        hips_w.y + f.vel_y * TICK,
                    );
                }
                foot.release = (foot.release - TICK).max(0.0);
                let w = 1.0 - foot.release * 10.0;
                foot.target = desired * w + foot.target * (1.0 - w);
            }
        }
        self.root_y = x.pos.y;
        // The ground's slope under a low foot (height_correction's side effect, 82D77AE8).
        let side = x.rot.cols[0];
        let ahead = x.rot.cols[2];
        for i in 0..2 {
            self.feet[i].slope = 0.0;
            if let (Some((_, n)), true) = (ground[i], low[i]) {
                let n = (n - side * side.dot(n)).normalize_or(Vec3::ZERO);
                self.feet[i].slope = n.dot(ahead) * 0.2;
            }
        }
        // The corrections (82BEE390) and the legs.
        for i in 0..2 {
            let world_target = self.support == i as u8;
            let foot = &mut self.feet[i];
            let offset_w = if world_target {
                foot.delta = Vec3::ZERO;
                foot.target - toe_w[i]
            } else {
                let want = local_dir(&x, foot.target - toe_w[i]);
                let desired = want * LOCAL_WEIGHT + foot.delta * (1.0 - LOCAL_WEIGHT);
                foot.delta = foot.delta + limit(desired - foot.delta, LOCAL_STEP);
                x.rot.apply(desired)
            };
            foot.offset = local_dir(&x, limit(offset_w, MOST_MOVED));
            foot.blend = 1.0;
            let n = (supported[i] && ground[i].is_some()).then_some(normals[i]);
            match n {
                Some(n) => {
                    let n = local_dir(&x, n);
                    foot.normal = if foot.normal_blend > 0.0 {
                        limit_angle(n, foot.normal, NORMAL_TURN)
                    } else {
                        n
                    };
                    foot.normal_blend = (foot.normal_blend + NORMAL_BLEND).min(1.0);
                }
                None => foot.normal_blend = (foot.normal_blend - NORMAL_BLEND).max(0.0),
            }
            let (offset, normal, nb) = (foot.offset, foot.normal, foot.normal_blend);
            solve_leg(ch, legs, i, offset, normal, nb);
        }
    }

    /// Which foot holds the ground: 0 left, 1 right, 2 neither, 3 in the air.
    pub fn support(&self) -> u8 {
        self.support
    }
}

/// SK8 foot_height: the swing foot at least 0.02 m over the ground under it, and no more than 0.08 m
/// above that than the clip has it over its own ground plane; never raised over 0.4 m or past 0.46 m
/// under the hips (`hips_y` where they will be next tick).
fn foot_height(root_y: f32, ground: f32, desired: f32, hips_y: f32) -> f32 {
    let base = ground + CLEAR;
    let proposed = desired - root_y + ground;
    let limited = proposed.min(base + CLEAR_RANGE).max(base);
    let out = desired.max(limited);
    if out - desired > MOST_RAISED || out > hips_y - UNDER_HIPS {
        desired
    } else {
        out
    }
}

/// `n` turned towards `from` so it is at most `most` (rad) from it.
fn limit_angle(n: Vec3, from: Vec3, most: f32) -> Vec3 {
    match arc(from, n) {
        Some((axis, c, s)) if libm::atan2f(s, c) > most => turn(from, axis, cosf(most), sinf(most)),
        _ => n,
    }
}

/// Moves the toe by `offset` (character space), tilted from up towards `normal` by `tilt` of the way
/// (at most [`MOST_TILT`]), solving the leg so the ankle follows: the thigh turns about the hip and the
/// shin about the knee, the knee bent in its own plane within [`KNEE_MIN`]..[`KNEE_MAX`] (SK8 two_bone.rs).
fn solve_leg(ch: &mut Character, legs: &Legs, i: usize, offset: Vec3, normal: Vec3, tilt: f32) {
    let world = ch.world();
    let (hip, knee, ankle, toe) = (
        at(&world[legs.thigh[i]]),
        at(&world[legs.shin[i]]),
        at(&world[legs.ankle[i]]),
        at(&world[legs.toe[i]]),
    );
    let foot_rows = world[legs.ankle[i]];
    // The foot's turn to the ground about the toe.
    let tilt_turn = arc(Vec3::UP, normal.normalize_or(Vec3::UP)).map(|(axis, c, s)| {
        let a = libm::atan2f(s, c).min(MOST_TILT) * tilt.clamp(0.0, 1.0);
        (axis, cosf(a), sinf(a))
    });
    let toe_to = toe + offset;
    let ankle_rel = ankle - toe;
    let target = toe_to + tilt_turn.map_or(ankle_rel, |(axis, c, s)| turn(ankle_rel, axis, c, s));
    let (l1, l2) = ((knee - hip).length(), (ankle - knee).length());
    let reach = target - hip;
    let d0 = reach.length();
    if l1 < 1e-3 || l2 < 1e-3 || d0 < 1e-3 {
        return;
    }
    let dir = reach * (1.0 / d0);
    let span = |deg: f32| sqrtf((l1 * l1 + l2 * l2 - 2.0 * l1 * l2 * cosf(deg * DEG)).max(0.0));
    let d = d0.clamp(span(KNEE_MIN), span(KNEE_MAX));
    // The knee bends the way it did.
    let bent = (knee - hip) - dir * (knee - hip).dot(dir);
    let pole = bent.normalize_or(
        (ankle - hip)
            .cross(v3(1.0, 0.0, 0.0))
            .normalize_or(Vec3::UP),
    );
    let along = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let knee_to = hip + dir * along + pole * sqrtf((l1 * l1 - along * along).max(0.0));
    let ankle_to = hip + dir * d;
    if let Some((axis, c, s)) = arc(
        (knee - hip).normalize_or(dir),
        (knee_to - hip).normalize_or(dir),
    ) {
        let rows = rows_turned(&ch.world()[legs.thigh[i]], axis, c, s, hip);
        ch.place_subtree(legs.thigh[i], &rows);
    }
    let knee_now = at(&ch.world()[legs.shin[i]]);
    let ankle_now = at(&ch.world()[legs.ankle[i]]);
    if let Some((axis, c, s)) = arc(
        (ankle_now - knee_now).normalize_or(dir),
        (ankle_to - knee_now).normalize_or(dir),
    ) {
        let rows = rows_turned(&ch.world()[legs.shin[i]], axis, c, s, knee_now);
        ch.place_subtree(legs.shin[i], &rows);
    }
    // The foot keeps its own turn (tilted to the ground) at the solved ankle.
    let ankle_now = at(&ch.world()[legs.ankle[i]]);
    let rows = match tilt_turn {
        Some((axis, c, s)) => rows_turned(&foot_rows, axis, c, s, ankle_now),
        None => {
            let mut r = foot_rows;
            r[3] = [ankle_now.x, ankle_now.y, ankle_now.z];
            r
        }
    };
    ch.place_subtree(legs.ankle[i], &rows);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_swing_foot_clears_the_ground_under_it() {
        // The clip's foot 0.05 m over its plane at 0; the ground under it 0.3 m up: raised to 0.32 at
        // least, 0.4 at most over the clip's own clearance.
        let h = foot_height(0.0, 0.3, 0.05, 2.0);
        assert!((h - 0.35).abs() < 1e-5, "{h}");
        let h = foot_height(0.0, 0.3, 0.0, 2.0);
        assert!((h - 0.32).abs() < 1e-5, "{h}");
        // Already over it: kept.
        assert_eq!(foot_height(0.0, 0.0, 0.3, 2.0), 0.3);
        // More than 0.4 m to raise, or past 0.46 m under the hips: left as it is.
        assert_eq!(foot_height(0.0, 0.6, 0.0, 2.0), 0.0);
        assert_eq!(foot_height(0.0, 0.3, 0.0, 0.7), 0.0);
    }

    #[test]
    fn a_steep_normal_is_laid_back_to_point_nine_up() {
        let n = clamp_normal(v3(0.0, 0.5, 0.866));
        assert!(
            (n.y - 0.9).abs() < 1e-6 && (n.length() - 1.0).abs() < 1e-4,
            "{n:?}"
        );
        assert_eq!(clamp_normal(Vec3::UP), Vec3::UP);
    }

    #[test]
    fn limit_angle_turns_at_most_so_far() {
        let a = limit_angle(v3(1.0, 0.0, 0.0), Vec3::UP, 0.1);
        assert!((libm::acosf(a.dot(Vec3::UP)) - 0.1).abs() < 1e-4, "{a:?}");
        let b = v3(0.05, 1.0, 0.0).normalize_or(Vec3::UP);
        assert_eq!(limit_angle(b, Vec3::UP, 0.1), b);
    }
}
