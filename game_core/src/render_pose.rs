//! The drawn rider and board: Skate 3's physical pose job, after SK8-ENGINE/skate-3-rust-engine
//! (GPL-3.0): skate-game physics/render_pose.rs and skeleton_output.rs, skate-core
//! physics/skeleton_output/{mod,board,wobble}.rs, skeleton_root.rs and board_animation.rs (TU3
//! `0x82DB6698` / `0x82DB95E8`, `SkeletonRootFrames::update` `0x82BE0318`, the board's animation blend
//! `0x82C04368` / `0x82C041C0`, `SkateboardWobble` `0x82BF2E48..2FB8`).
//!
//! Skate 3 draws what its physics solved, not the clips: each of the skeleton's 24 physical parts puts
//! its animation bone where the part is (the part's frame times the inverse of its volume's frame on the
//! bone), the bones in between keep the clips' pose on their mapped parents, the board's bone is the
//! physical deck (with the landing and take-off wobble), its trucks tilt with the axles and sink with
//! the wheels' compression, and the wheels turn as their bodies turn. Everything is in the animation's
//! space, which the root frames place in the world: the animated board at the predicted physical board,
//! turned by the reckoning and by a heading alignment (re-taken every ground tick), so the posed board
//! lies on the physical one. When the clips own the board (in the air, a take-off from the ground, the
//! board in hand) its bone is the animated one; handing it back to the physics blends the rotation error
//! out on Skate 3's `PhysToAnim` curves instead of snapping.

use crate::math::{Mat3, Vec3, Xform, v3};
use crate::physics::Quat;
use crate::physics::rider_body::{PHYS_BONES, inverse};
use crate::physics::skeleton::ANIMATED_PARTS;
use alloc::vec::Vec;

/// TU3's `PointGraph`: piecewise linear, held past its ends (SK8 point_graph.rs `evaluate`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointGraph8 {
    pub x: [f32; 8],
    pub y: [f32; 8],
}

impl PointGraph8 {
    pub const fn new(v: [f32; 16]) -> PointGraph8 {
        let mut x = [0.0; 8];
        let mut y = [0.0; 8];
        let mut i = 0;
        while i < 8 {
            x[i] = v[i];
            y[i] = v[i + 8];
            i += 1;
        }
        PointGraph8 { x, y }
    }

    pub fn eval(&self, input: f32) -> f32 {
        if input < self.x[0] {
            return self.y[0];
        }
        if input >= self.x[7] || input.is_nan() {
            return self.y[7];
        }
        for upper in 1..8 {
            if input >= self.x[upper] {
                continue;
            }
            let lower = upper - 1;
            let width = self.x[upper] - self.x[lower];
            if width <= 0.0 {
                return self.y[upper];
            }
            let slope = (self.y[upper] - self.y[lower]) / width;
            return slope * (input - self.x[lower]) + self.y[lower];
        }
        self.y[0]
    }
}

/// `physics_skeleton/default` `TruckTiltScalar`, `TruckTiltMaxAngle` (rad), `TruckTiltWobbleScalar`,
/// `TruckDisplacementMax` (m), read off the disc.
pub const TRUCK_TILT_SCALAR: f32 = 0.9;
pub const TRUCK_TILT_MAX: f32 = 0.25;
pub const TRUCK_TILT_WOBBLE: f32 = 0.1;
pub const TRUCK_DISPLACEMENT_MAX: f32 = 0.01;
/// `physicstrucks/default/TruckYPos`: the wheels' compression is measured from it (m).
pub const COMPRESSION_REST: f32 = crate::physics::board::TRUCK_Y;
/// `physics_deck_wobble/default`: the tilt (rad) and squish (m) against time after a take-off and a
/// landing, and how long a wobble lasts (s).
pub const WOBBLE_TAKEOFF_TILT: PointGraph8 = PointGraph8::new([
    0.0,
    0.01547231,
    0.06758957,
    0.1237785,
    0.1490228,
    0.2003257,
    0.223127,
    0.245114,
    0.05214285,
    0.01785714,
    -0.05642857,
    0.004999995,
    0.02642857,
    0.01571429,
    0.005714286,
    0.0,
]);
pub const WOBBLE_LANDING_TILT: PointGraph8 = PointGraph8::new([
    0.0,
    0.08306189,
    0.1368078,
    0.1758958,
    0.2410423,
    0.3143322,
    0.4315961,
    0.5789902,
    -0.03714276,
    -0.11625,
    -0.08571428,
    0.04071429,
    0.05571429,
    0.03571429,
    -0.02428571,
    -0.001428568,
]);
pub const WOBBLE_TAKEOFF_SQUISH: PointGraph8 = PointGraph8::new([
    0.0,
    0.02117264,
    0.04275244,
    0.06718241,
    0.08021173,
    0.09486971,
    0.1180782,
    0.1392508,
    -0.003642857,
    -0.009642857,
    -0.008357143,
    -0.001428571,
    0.002071428,
    0.006,
    0.003857143,
    0.0,
]);
pub const WOBBLE_LANDING_SQUISH: PointGraph8 = PointGraph8::new([
    0.0,
    0.03013029,
    0.07899023,
    0.1062704,
    0.1205212,
    0.1644951,
    0.1909609,
    0.21875,
    0.0,
    -0.004642857,
    -0.009714286,
    -0.0075,
    -0.002857143,
    0.0008571434,
    7.142901e-5,
    0.0,
]);
pub const WOBBLE_MAX_TIME: f32 = 1.0;
/// `physics_airstates/default` `PhysToAnimSlow` and `PhysToAnimFast`: the share of the board's rotation
/// error blended out a tick against the error's angle (rad).
pub const PHYS_TO_ANIM_SLOW: PointGraph8 = PointGraph8::new([
    0.0,
    0.006351792,
    0.04592834,
    0.08843648,
    0.128013,
    0.1724756,
    0.2188925,
    0.3,
    1.0,
    0.825,
    0.2964286,
    0.1571429,
    0.1107143,
    0.08928572,
    0.07142857,
    0.04,
]);
pub const PHYS_TO_ANIM_FAST: PointGraph8 = PointGraph8::new([
    0.0, 0.01856678, 0.04592835, 0.08648209, 0.1201955, 0.1631922, 0.2052117, 0.2990228, 1.0, 1.0,
    0.9214286, 0.7607143, 0.6178572, 0.4785714, 0.4, 0.2678571,
]);

/// The deck's wobble on a landing or a take-off (SK8 skeleton_output/wobble.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wobble {
    pub active: bool,
    pub landing: bool,
    pub time: f32,
    pub direction: f32,
    /// This tick's tilt (rad, about the deck's z) and squish (m, along its up).
    pub tilt: f32,
    pub squish: f32,
}

impl Wobble {
    /// `0x82BF2F18`: starts one (`reverse`: the stance's way round).
    pub fn trigger(&mut self, landing: bool, reverse: bool) {
        self.active = true;
        self.landing = landing;
        self.time = 0.0;
        self.direction = if reverse { -1.0 } else { 1.0 };
    }

    /// `0x82BF2FB8`, a tick: this tick's tilt and squish; whether it was sampled.
    pub fn update(&mut self) -> bool {
        if !self.active {
            self.tilt = 0.0;
            self.squish = 0.0;
            return false;
        }
        let (tilt, squish) = if self.landing {
            (WOBBLE_LANDING_TILT, WOBBLE_LANDING_SQUISH)
        } else {
            (WOBBLE_TAKEOFF_TILT, WOBBLE_TAKEOFF_SQUISH)
        };
        self.squish = squish.eval(self.time);
        self.tilt = tilt.eval(self.time) * self.direction;
        self.time += 1.0 / 60.0;
        if self.time > WOBBLE_MAX_TIME {
            self.active = false;
            self.time = 0.0;
        }
        true
    }

    /// PostWipeoutCheck `0x82BD857C`: the drawn deck tilted about its z and pushed along its up.
    pub fn apply(&self, deck: &Xform) -> Xform {
        let (s, c) = (libm::sinf(self.tilt), libm::cosf(self.tilt));
        let rot = Mat3::from_cols(v3(c, s, 0.0), v3(-s, c, 0.0), v3(0.0, 0.0, 1.0));
        let mut out = Xform::new(deck.rot.mul(&rot), deck.pos);
        out.pos += out.rot.cols[1] * self.squish;
        out
    }
}

/// The board's rotation error between the physics and the animation, blended out (SK8
/// board_animation.rs: `0x82C04368` captures it, `0x82C041C0` applies it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoardBlend {
    pub error: Quat,
    pub blending: bool,
}

impl Default for BoardBlend {
    fn default() -> Self {
        BoardBlend {
            error: Quat::IDENTITY,
            blending: false,
        }
    }
}

impl BoardBlend {
    /// The error of `actual` (the board drawn now) against `target` (the one taking over).
    pub fn capture(&mut self, target: &Xform, actual: &Xform) {
        let rel = inverse(target).then(actual);
        self.error = Quat::from_mat3(&orthonormal(rel.rot)).normalize();
        self.blending = true;
    }

    /// `target` with what is left of the error, a share of it blended out this tick (the slow or fast
    /// curve against the error's angle); the error is gone once the curve says 0.95 of it.
    pub fn apply(&mut self, target: &Xform, fast: bool) -> Xform {
        if !self.blending {
            return *target;
        }
        let q = self.error;
        let q = if q.w < 0.0 {
            Quat {
                x: -q.x,
                y: -q.y,
                z: -q.z,
                w: -q.w,
            }
        } else {
            q
        };
        let angle = 2.0 * libm::acosf(q.w.clamp(-1.0, 1.0));
        let graph = if fast {
            PHYS_TO_ANIM_FAST
        } else {
            PHYS_TO_ANIM_SLOW
        };
        let fraction = graph.eval(crate::math::wrap_angle(angle).abs());
        if fraction >= 0.95 || fraction.is_nan() {
            self.blending = false;
            return *target;
        }
        // Towards the identity by `fraction` (a slerp; nearly there, a normalised lerp).
        let next = if q.w > 0.9998 {
            Quat {
                x: q.x * (1.0 - fraction),
                y: q.y * (1.0 - fraction),
                z: q.z * (1.0 - fraction),
                w: q.w + (1.0 - q.w) * fraction,
            }
            .normalize()
        } else {
            let theta = libm::acosf(q.w.clamp(-1.0, 1.0));
            let st = libm::sinf(theta);
            let a = libm::sinf((1.0 - fraction) * theta) / st;
            let b = libm::sinf(fraction * theta) / st;
            Quat {
                x: q.x * a,
                y: q.y * a,
                z: q.z * a,
                w: q.w * a + b,
            }
        };
        self.error = next;
        Xform::new(target.rot.mul(&next.to_mat3()), target.pos)
    }
}

/// Gram-Schmidt from the last axis (SK8 skeleton_root.rs `orthonormalize`).
fn orthonormal(m: Mat3) -> Mat3 {
    let z = m.cols[2].normalize_or(v3(0.0, 0.0, 1.0));
    let y = (m.cols[1] - z * z.dot(m.cols[1])).normalize_or(v3(0.0, 1.0, 0.0));
    let x = y.cross(z);
    let x = if x.dot(m.cols[0]) < 0.0 { -x } else { x };
    Mat3::from_cols(x, y, z)
}

/// SK8 `SkeletonRootFrames` (TU3 `Skeleton::UpdateRootTransforms`): the animation's frame in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootFrames {
    /// The heading alignment (a turn about the animation's up that points the animated board's nose
    /// along +z), and whether the next update takes it afresh.
    pub heading: Mat3,
    pub initialize_heading: bool,
    pub animation_to_world: Xform,
}

impl Default for RootFrames {
    fn default() -> Self {
        RootFrames {
            heading: Mat3::IDENTITY,
            initialize_heading: true,
            animation_to_world: Xform::IDENTITY,
        }
    }
}

impl RootFrames {
    /// The animated board (`animation_board`, in the animation's space) at the predicted physical board
    /// position `board`, turned by the reckoning's axes `reckoning` and the heading alignment.
    pub fn update(&mut self, board: Vec3, reckoning: Mat3, animation_board: &Xform) {
        if self.initialize_heading {
            let z = animation_board.rot.cols[2];
            let at = v3(z.x, 0.0, z.z);
            if at.length() > 1e-4 {
                let yaw = crate::math::atan2f(at.x, at.z);
                self.heading = Mat3::rot_y(-yaw);
            }
            self.initialize_heading = false;
        }
        self.animation_to_world =
            Xform::new(reckoning, board).then(&self.on_board(animation_board));
    }

    /// The animation in the reckoning's frame at the board: the heading alignment with the animated
    /// board's centre at the origin (SK8 `animation_to_board`).
    pub fn on_board(&self, animation_board: &Xform) -> Xform {
        Xform::new(self.heading, Vec3::ZERO).then(&Xform::new(Mat3::IDENTITY, -animation_board.pos))
    }
}

/// The skateboard's bones (`Truck_Front` ... `Right_WheelBack`), in the animation's skeleton.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoardBones {
    pub front_truck: usize,
    pub back_truck: usize,
    /// Front left, front right, back left, back right: the physical wheels 0..3 (SK8: the native body
    /// order and the animation names' differ).
    pub wheels: [usize; 4],
}

/// The physical board this tick (world): its six wheel and truck bodies (`physics::board` order: the
/// wheels, then the front and back trucks) and the deck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalBoard {
    pub bodies: [Xform; 6],
    pub deck: Xform,
}

/// The mapping from the physical skeleton to an animation skeleton, and the buffers.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapping {
    /// Each physical part's animation bone, and the nearest physical ancestor part.
    pub bones: [usize; ANIMATED_PARTS],
    pub parents: [Option<usize>; ANIMATED_PARTS],
    pub board: Option<BoardBones>,
    /// The inverse of each part's volume frame on its bone.
    inverse_volume: [Xform; ANIMATED_PARTS],
    locals: Vec<Xform>,
    globals: Vec<Xform>,
}

impl Mapping {
    /// `bone_parents`: the animation skeleton's parent table (-1 at a root; parents before children).
    pub fn new(
        bones: [usize; ANIMATED_PARTS],
        bone_parents: &[i16],
        board: Option<BoardBones>,
    ) -> Mapping {
        let parents = core::array::from_fn(|part| {
            let mut b = bone_parents.get(bones[part]).copied().unwrap_or(-1);
            let mut guard = 0;
            while b >= 0 && guard < bone_parents.len() {
                if let Some(p) = bones.iter().position(|&x| x == b as usize) {
                    return Some(p);
                }
                b = bone_parents[b as usize];
                guard += 1;
            }
            None
        });
        Mapping {
            bones,
            parents,
            board,
            inverse_volume: core::array::from_fn(|p| inverse(&PHYS_BONES[p].volume())),
            locals: Vec::new(),
            globals: Vec::new(),
        }
    }

    /// The drawn pose (`0x82DB95E8`): `anim` is this frame's animated pose (each bone's frame in the
    /// animation's space), `parts` the physical parts' frames (world; part 0 the board), `to_world` the
    /// animation's frame in the world. Returns each bone's frame in the animation's space.
    pub fn publish(
        &mut self,
        anim: &[Xform],
        bone_parents: &[i16],
        parts: &[Xform; ANIMATED_PARTS],
        to_world: &Xform,
        board: Option<(&PhysicalBoard, &Wobble)>,
    ) -> &[Xform] {
        let n = anim.len();
        let to_animation = inverse(to_world);
        // The animation's locals: each bone on its parent.
        self.locals.clear();
        self.locals
            .extend((0..n).map(|b| match bone_parents.get(b) {
                Some(&p) if p >= 0 && (p as usize) < n => inverse(&anim[p as usize]).then(&anim[b]),
                _ => anim[b],
            }));
        self.globals.clear();
        self.globals.extend_from_slice(anim);
        // `0x82DB96B8`: each part's bone where the part is (the part times its inverse volume frame).
        for (part, frame) in parts.iter().enumerate() {
            let b = self.bones[part];
            if b < n {
                self.globals[b] = to_animation.then(&frame.then(&self.inverse_volume[part]));
            }
        }
        // `0x82DB97F4`: their locals on the nearest physical ancestor's bone.
        for part in (0..ANIMATED_PARTS).rev() {
            let b = self.bones[part];
            if b >= n {
                continue;
            }
            self.locals[b] = match self.parents[part] {
                Some(p) => inverse(&self.globals[self.bones[p]]).then(&self.globals[b]),
                None => self.globals[b],
            };
        }
        if let (Some(bones), Some((pb, wobble))) = (self.board, board) {
            board_locals(&mut self.locals, bones, pb, &parts[0], wobble);
        }
        // The whole hierarchy again from the locals (render_pose.rs `compose_hierarchy_in_place`).
        for b in 0..n {
            self.globals[b] = match bone_parents.get(b) {
                Some(&p) if p >= 0 && (p as usize) < b => {
                    self.globals[p as usize].then(&self.locals[b])
                }
                _ => self.locals[b],
            };
        }
        &self.globals
    }
}

/// The board's bones (`0x82DB993C..A55C`, SK8 skeleton_output/board.rs): the wheels turned as their
/// bodies are on their trucks, the trucks tilted with their axles (and the wobble) and sunk by the
/// wheels' compression (less the deck's wobble squish).
fn board_locals(
    locals: &mut [Xform],
    bones: BoardBones,
    pb: &PhysicalBoard,
    skeleton_board: &Xform,
    deck_wobble: &Wobble,
) {
    let n = locals.len();
    let y = v3(0.0, 1.0, 0.0);
    let z = v3(0.0, 0.0, 1.0);
    let wheels: [Mat3; 4] = core::array::from_fn(|w| {
        let truck = &pb.bodies[4 + w / 2];
        let up = transpose_apply(&truck.rot, pb.bodies[w].rot.cols[1]);
        Mat3::rot_x(signed_angle(up, y, z))
    });
    // In the drawn board's frame (which is the deck's, or the deck's turned end for end when the clips'
    // board lies the other way on it: `turn`), the bone's front truck is the physical truck ahead.
    let to_board = |v: Vec3| transpose_apply(&skeleton_board.rot, v);
    let ahead = to_board(pb.bodies[4].pos - skeleton_board.pos).z >= 0.0;
    // Each truck bone's physical truck and wheel pair (SK8: front 0 less 1, back 3 less 2, both
    // about -x at rest; turned round, the other pair and the other order).
    let (front_pair, back_pair, front_truck, back_truck) = if ahead {
        ((0, 1), (3, 2), 4, 5)
    } else {
        ((2, 3), (1, 0), 5, 4)
    };
    let along = |(a, b): (usize, usize)| {
        let d = pb.bodies[a].pos - pb.bodies[b].pos;
        if d.length() > 1e-6 {
            to_board(d * (1.0 / d.length()))
        } else {
            Vec3::ZERO
        }
    };
    // The trucks' tilt axes in the drawn board's frame. flatground's truck bodies' x points out from
    // the board's middle, the imported truck bones' (SK8's truck frames') in: the bones' is the
    // reverse of the bodies'. With it the hangers turn against the deck's roll over the axles
    // (a carve, the wobble), keeping the wheels on the ground.
    let axis = |t: usize| -to_board(pb.bodies[t].rot.cols[0]);
    let mut front = along(front_pair);
    let mut back = along(back_pair);
    if front.dot(front) < 0.5 {
        front = to_board(pb.bodies[front_truck].rot.cols[2]);
    }
    if back.dot(back) < 0.5 {
        back = -to_board(pb.bodies[back_truck].rot.cols[2]);
    }
    let neg_x = v3(-1.0, 0.0, 0.0);
    let back_angle = crate::math::wrap_angle(signed_angle(back, neg_x, axis(back_truck)));
    let front_angle = crate::math::wrap_angle(signed_angle(front, neg_x, axis(front_truck)));
    let wobble = deck_wobble.tilt * TRUCK_TILT_WOBBLE;
    let back_tilt =
        (-TRUCK_TILT_SCALAR * back_angle - wobble).clamp(-TRUCK_TILT_MAX, TRUCK_TILT_MAX);
    let front_tilt =
        (-TRUCK_TILT_SCALAR * front_angle + wobble).clamp(-TRUCK_TILT_MAX, TRUCK_TILT_MAX);
    let compression = average_compression(&pb.deck, &pb.bodies);
    for (side, bone, tilt) in [
        (0, bones.front_truck, front_tilt),
        (1, bones.back_truck, back_tilt),
    ] {
        if bone >= n {
            continue;
        }
        let c = if compression[side] < 0.001 {
            0.0
        } else {
            compression[side]
        };
        // The deck squished down by the wobble leaves the trucks where they were (`0x82DBA0F0`).
        let displacement = (c - deck_wobble.squish).clamp(0.0, TRUCK_DISPLACEMENT_MAX);
        let rotated = locals[bone].then(&Xform::new(Mat3::rot_x(tilt), Vec3::ZERO));
        locals[bone] = Xform::new(Mat3::IDENTITY, v3(0.0, displacement, 0.0)).then(&rotated);
    }
    for (w, &bone) in bones.wheels.iter().enumerate() {
        if bone < n {
            locals[bone] = locals[bone].then(&Xform::new(wheels[w], Vec3::ZERO));
        }
    }
}

/// `CalculateAverageWheelCompressions` `0x82C08968`: the wheels' heights on the deck, front and back
/// pairs, over the trucks' rest height.
pub fn average_compression(deck: &Xform, bodies: &[Xform; 6]) -> [f32; 2] {
    let h = |w: usize| transpose_apply(&deck.rot, bodies[w].pos - deck.pos).y;
    [
        (h(0) + h(1)) * 0.5 - COMPRESSION_REST,
        (h(2) + h(3)) * 0.5 - COMPRESSION_REST,
    ]
}

fn transpose_apply(m: &Mat3, v: Vec3) -> Vec3 {
    v3(m.cols[0].dot(v), m.cols[1].dot(v), m.cols[2].dot(v))
}

/// The signed angle from `a` to `b` about `axis` (SK8 `signed_angle`).
fn signed_angle(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    let a = a.normalize_or(Vec3::ZERO);
    let b = b.normalize_or(Vec3::ZERO);
    let cos = a.dot(b).clamp(-1.0, 1.0);
    let angle = libm::acosf(cos);
    if a.cross(b).dot(axis) < 0.0 {
        -angle
    } else {
        angle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn point_graphs_hold_past_their_ends() {
        let g = PHYS_TO_ANIM_SLOW;
        assert_eq!(g.eval(-1.0), 1.0);
        assert_eq!(g.eval(5.0), 0.04);
        assert!((g.eval(0.003175896) - 0.9125).abs() < 1e-4);
    }

    #[test]
    fn a_landing_wobble_tilts_then_settles_within_its_second() {
        let mut w = Wobble::default();
        assert!(!w.update());
        w.trigger(true, false);
        let mut most: f32 = 0.0;
        let mut ticks = 0;
        while w.update() {
            most = most.max(w.tilt.abs());
            ticks += 1;
        }
        assert!(most > 0.1 && most < 0.13, "{most}");
        assert!((59..=61).contains(&ticks), "{ticks}");
        assert_eq!((w.tilt, w.squish), (0.0, 0.0));
    }

    #[test]
    fn the_blend_takes_the_error_out_without_a_snap() {
        let mut b = BoardBlend::default();
        let target = Xform::new(Mat3::IDENTITY, v3(0.0, 1.0, 0.0));
        let actual = Xform::new(Mat3::rot_y(0.6), v3(0.0, 1.0, 0.0));
        b.capture(&target, &actual);
        let first = b.apply(&target, false);
        // The first tick is still almost where the board was.
        let yaw = |x: &Xform| crate::math::atan2f(x.rot.cols[2].x, x.rot.cols[2].z);
        assert!((yaw(&first) - 0.6).abs() < 0.12, "{}", yaw(&first));
        let mut last = yaw(&first);
        let mut ticks = 1;
        while b.blending && ticks < 200 {
            let now = yaw(&b.apply(&target, false));
            assert!(now <= last + 1e-5 && last - now < 0.12, "{last} -> {now}");
            last = now;
            ticks += 1;
        }
        assert!(!b.blending && last.abs() < 0.05, "{last} after {ticks}");
    }

    #[test]
    fn the_root_puts_the_animated_board_on_the_physical_one() {
        let mut r = RootFrames::default();
        // The animated board 0.1 m up, its nose 90 degrees round.
        let anim = Xform::new(Mat3::rot_y(core::f32::consts::FRAC_PI_2), v3(0.2, 0.1, 0.0));
        r.update(v3(5.0, 0.3, 2.0), Mat3::IDENTITY, &anim);
        let placed = r.animation_to_world.then(&anim);
        assert!((placed.pos - v3(5.0, 0.3, 2.0)).length() < 1e-5);
        assert!((placed.rot.cols[2] - v3(0.0, 0.0, 1.0)).length() < 1e-4);
        // Kept until it is taken afresh.
        let turned = Xform::new(Mat3::rot_y(0.3), anim.pos);
        r.update(v3(5.0, 0.3, 2.0), Mat3::IDENTITY, &turned);
        let placed = r.animation_to_world.then(&turned);
        assert!((placed.rot.cols[2] - v3(0.0, 0.0, 1.0)).length() > 0.1);
    }

    #[test]
    fn mapped_bones_go_where_their_parts_are_and_the_rest_ride_along() {
        // Bone 0 root, bone 1 (part 0's) under it, bone 2 (unmapped) under bone 1; the other parts on
        // bones past the end (unmapped here).
        let parents = vec![-1i16, 0, 1];
        let mut bones = [99usize; ANIMATED_PARTS];
        bones[0] = 1;
        let mut m = Mapping::new(bones, &parents, None);
        let anim = vec![
            Xform::IDENTITY,
            Xform::new(Mat3::IDENTITY, v3(0.0, 1.0, 0.0)),
            Xform::new(Mat3::IDENTITY, v3(0.0, 1.5, 0.0)),
        ];
        let mut parts = [Xform::IDENTITY; ANIMATED_PARTS];
        parts[0] = Xform::new(Mat3::rot_y(0.5), v3(3.0, 1.0, 0.0)).then(&PHYS_BONES[0].volume());
        let out = m.publish(&anim, &parents, &parts, &Xform::IDENTITY, None);
        assert!((out[1].pos - v3(3.0, 1.0, 0.0)).length() < 1e-5);
        // The unmapped child keeps its 0.5 m on its parent.
        assert!((out[2].pos - v3(3.0, 1.5, 0.0)).length() < 1e-5);
        assert!((out[0].pos).length() < 1e-6);
    }
}
