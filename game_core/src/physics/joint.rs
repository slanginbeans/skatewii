//! Joints: SK8 joint_builder.rs (TU3 `JointJacobian::Build`, 0x82AE3BC8), joint_builder/arithmetic.rs
//! (the angular limit modes), constraint_frames.rs, and their iteration (solver/joint.rs,
//! 0x82AE2BC8). A joint holds a frame on body B to a frame on body A: three linear lanes along B's
//! linear frame, three angular lanes (twist about A's first axis, two swings), each locked, limited or
//! free; the limits are "preconditioned errors" so a lane inside its range takes no correction.

use super::{Body, Quat, Reaction, STEP, UNBOUNDED, transpose_apply};
use crate::math::{Mat3, Vec3};

/// SK8 joint_builder.rs `NEAR_PARALLEL`: the cone limit's singularity guard.
const NEAR_PARALLEL: f32 = 0.99999;

/// A joint's limits (SK8's 64-byte parameter record): the linear position and rate allowances per
/// axis, the twist and swing rate allowances (rad/s), the swing and twist thresholds (cosines) and
/// modes (swing: 0 locked, 1 cone, 2 and 3 limited about B's second axis, other free; twist: 0 locked,
/// 1 limited, other free).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointParams {
    pub position_allowance: Vec3,
    pub velocity_allowance: Vec3,
    pub twist_velocity: f32,
    pub swing_velocity: f32,
    pub swing_threshold: f32,
    pub twist_threshold: f32,
    pub swing_mode: u32,
    pub twist_mode: u32,
}

/// The joint's frames on each body (orientation, then anchor in the body's axes) and the frame on B
/// whose axes the linear lanes use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointFrames {
    pub orientation_a: Quat,
    pub anchor_a: Vec3,
    pub orientation_b: Quat,
    pub anchor_b: Vec3,
    pub linear_orientation_b: Quat,
}

/// The six lanes' geometry shared by joints and drives (SK8 solver/joint_data.rs
/// `ConstraintGeometry`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lanes {
    pub a: usize,
    pub b: usize,
    pub arm_a: Vec3,
    pub arm_b: Vec3,
    pub linear_axes: [Vec3; 3],
    pub angular_axes: [Vec3; 3],
    pub linear_inverse_mass: [f32; 3],
    pub angular_inverse_mass: [f32; 3],
    pub inertia_a: Mat3,
    pub inertia_b: Mat3,
    pub inverse_mass: [f32; 2],
    pub linear: [f32; 3],
    pub angular: [f32; 3],
}

impl Lanes {
    /// The lanes' candidate impulses from the corrections so far (both read before either applies).
    pub fn candidates(&self, ra: &Reaction, rb: &Reaction) -> ([f32; 3], [f32; 3]) {
        let point = rb.point(self.arm_b) - ra.point(self.arm_a);
        let angular = rb.angular - ra.angular;
        (
            core::array::from_fn(|k| {
                self.linear[k] + self.linear_inverse_mass[k] * self.linear_axes[k].dot(point)
            }),
            core::array::from_fn(|k| {
                self.angular[k] + self.angular_inverse_mass[k] * self.angular_axes[k].dot(angular)
            }),
        )
    }

    /// Stores the new impulses and applies their change to both bodies.
    pub fn apply(
        &mut self,
        ra: &mut Reaction,
        rb: &mut Reaction,
        linear: [f32; 3],
        angular: [f32; 3],
    ) {
        let mut lin = Vec3::ZERO;
        let mut ang = Vec3::ZERO;
        for k in 0..3 {
            lin += self.linear_axes[k] * (linear[k] - self.linear[k]);
            ang += self.angular_axes[k] * (angular[k] - self.angular[k]);
        }
        self.linear = linear;
        self.angular = angular;
        ra.linear += lin * self.inverse_mass[0];
        rb.linear -= lin * self.inverse_mass[1];
        ra.angular += self.inertia_a.apply(self.arm_a.cross(lin) + ang);
        rb.angular -= self.inertia_b.apply(self.arm_b.cross(lin) + ang);
    }
}

/// One over a linear lane's effective mass between the two bodies.
pub fn linear_inverse_mass(a: &Body, b: &Body, arm_a: Vec3, arm_b: Vec3, axis: Vec3) -> f32 {
    let mut m = 0.0;
    for (body, arm) in [(a, arm_a), (b, arm_b)] {
        if body.active {
            let j = arm.cross(axis);
            m += body.mass.inverse_mass + j.dot(body.inverse_inertia.apply(j));
        }
    }
    if m > 0.0 { 1.0 / m } else { 0.0 }
}

/// The effective-mass sum of an angular lane.
pub fn angular_mass_sum(a: &Body, b: &Body, axis: Vec3) -> f32 {
    let mut m = 0.0;
    for body in [a, b] {
        if body.active {
            m += axis.dot(body.inverse_inertia.apply(axis));
        }
    }
    m
}

/// The rows of a quaternion's relative rotation (SK8 constraint_frames.rs `quaternion_rows`, TU3
/// 0x82AE0DB0): three axes whose small rotations change the relative quaternion's x, y, z, and the
/// relative quaternion conj(a) * b itself.
pub fn quaternion_rows(a: Quat, b: Quat) -> ([Vec3; 3], Quat) {
    let (xx, xy, xz, xw) = (a.x * b.x, a.x * b.y, a.x * b.z, a.x * b.w);
    let (yx, yy, yz, yw) = (a.y * b.x, a.y * b.y, a.y * b.z, a.y * b.w);
    let (zx, zy, zz, zw) = (a.z * b.x, a.z * b.y, a.z * b.z, a.z * b.w);
    let (wx, wy, wz, ww) = (a.w * b.x, a.w * b.y, a.w * b.z, a.w * b.w);
    (
        [
            Vec3 {
                x: ww + xx - yy - zz,
                y: yx + xy + wz + zw,
                z: xz + zx - wy - yw,
            },
            Vec3 {
                x: yx + xy - wz - zw,
                y: ww + yy - zz - xx,
                z: yz + zy + wx + xw,
            },
            Vec3 {
                x: xz + zx + yw + wy,
                y: zy + yz - wx - xw,
                z: ww + zz - xx - yy,
            },
        ],
        Quat {
            x: wx - xw + zy - yz,
            y: wy - yw + xz - zx,
            z: wz - zw + yx - xy,
            w: ww + zz + yy + xx,
        },
    )
}

/// A compiled joint row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointRow {
    pub lanes: Lanes,
    pub linear_low: [f32; 3],
    pub linear_high: [f32; 3],
    pub angular_low: [f32; 3],
    pub angular_high: [f32; 3],
}

/// The angular lanes and their raw limits (SK8 joint_builder/arithmetic.rs `angular_rows`).
fn angular_rows(
    p: &JointParams,
    ba: &Mat3,
    bb: &Mat3,
    rel: &[[f32; 3]; 3],
    rqd: &([Vec3; 3], Quat),
) -> ([Vec3; 3], [f32; 3], [f32; 3]) {
    let mut axes = bb.cols;
    let mut low = [-UNBOUNDED; 3];
    let mut high = [UNBOUNDED; 3];
    let (rq_axes, rq) = rqd;
    match p.swing_mode {
        0 => {
            axes[1] = rq_axes[1];
            axes[2] = rq_axes[2];
            low[1] = 2.0 * rq.y;
            high[1] = low[1];
            low[2] = 2.0 * rq.z;
            high[2] = low[2];
        }
        1 => {
            if rel[0][0] < NEAR_PARALLEL {
                let (py, pz) = (rel[0][1], rel[0][2]);
                let root = libm::sqrtf(py * py + pz * pz);
                let inv = if root > 0.0 { 1.0 / root } else { 0.0 };
                axes[1] = ba.cols[0].cross(bb.cols[0]) * inv;
                axes[2] = axes[1].cross(ba.cols[0]);
                low[1] = (p.swing_threshold - rel[0][0]) * inv;
            }
        }
        2 | 3 => {
            axes[1] = bb.cols[1];
            if p.twist_mode == 0 {
                axes[2] = rq_axes[2];
                low[2] = 2.0 * rq.z;
                high[2] = low[2];
            } else {
                axes[2] = ba.cols[0].cross(axes[1]);
                low[2] = -rel[0][1];
                high[2] = low[2];
            }
            if p.swing_mode == 2 {
                let d = rel[0][2];
                if d != 0.0 {
                    let c = (p.swing_threshold - rel[0][0]) / d;
                    if d < 0.0 {
                        high[1] = c;
                    } else {
                        low[1] = c;
                    }
                }
            }
        }
        _ => {
            axes[1] = bb.cols[1];
            axes[2] = bb.cols[2];
        }
    }
    match p.twist_mode {
        0 => {
            axes[0] = rq_axes[0];
            low[0] = 2.0 * rq.x;
            high[0] = low[0];
        }
        1 => {
            axes[0] = ba.cols[0];
            let d = rel[2][1] - rel[1][2];
            if d != 0.0 {
                let c = ((1.0 + rel[0][0]) * p.twist_threshold - (rel[1][1] + rel[2][2])) / d;
                if d < 0.0 {
                    high[0] = c;
                } else {
                    low[0] = c;
                }
            }
        }
        _ => axes[0] = bb.cols[0],
    }
    (axes, low, high)
}

impl JointRow {
    pub fn build(
        bodies: &[Body],
        a: usize,
        b: usize,
        p: &JointParams,
        f: &JointFrames,
    ) -> JointRow {
        let (ba, bb) = (&bodies[a], &bodies[b]);
        let dt = STEP;
        let qa = ba.orientation.compose(f.orientation_a);
        let qb = bb.orientation.compose(f.orientation_b);
        let ql = bb.orientation.compose(f.linear_orientation_b);
        let (basis_a, basis_b, linear_basis) = (qa.to_mat3(), qb.to_mat3(), ql.to_mat3());
        let arm_a = ba.axes.apply(f.anchor_a);
        let arm_b = bb.axes.apply(f.anchor_b);
        let linear_axes = linear_basis.cols;
        let rel: [[f32; 3]; 3] = core::array::from_fn(|r| {
            core::array::from_fn(|c| basis_a.cols[r].dot(basis_b.cols[c]))
        });
        let rqd = quaternion_rows(qa, qb);
        let (angular_axes, raw_low, raw_high) = angular_rows(p, &basis_a, &basis_b, &rel, &rqd);

        let linear_inverse_mass =
            linear_axes.map(|ax| linear_inverse_mass(ba, bb, arm_a, arm_b, ax));
        // The angular lanes' errors are twice the relative quaternion's parts: half the impulse.
        let angular_inverse_mass = angular_axes.map(|ax| {
            let m = angular_mass_sum(ba, bb, ax);
            if m > 0.0 { 0.5 / m } else { 0.0 }
        });

        let displacement = |body: &Body, arm: Vec3| {
            (body.point_acceleration(arm) * dt + body.point_velocity(arm)) * dt
        };
        let da = displacement(ba, arm_a);
        let db = displacement(bb, arm_b);
        let predicted = (bb.position + arm_b + db) - (ba.position + arm_a + da);
        let rate = transpose_apply(
            &Mat3::from_cols(linear_axes[0], linear_axes[1], linear_axes[2]),
            db - da,
        );
        let sep = transpose_apply(
            &Mat3::from_cols(linear_axes[0], linear_axes[1], linear_axes[2]),
            predicted,
        );
        let mut linear_low = [0.0; 3];
        let mut linear_high = [0.0; 3];
        for k in 0..3 {
            let (pa, va) = (p.position_allowance.get(k), p.velocity_allowance.get(k));
            let (dl, dh) = (sep.get(k) - pa, sep.get(k) + pa);
            let (vl, vh) = (rate.get(k) - va * dt, rate.get(k) + va * dt);
            linear_low[k] = dl.max(vl.min(dh)) * linear_inverse_mass[k];
            linear_high[k] = dh.min(vh.max(dl)) * linear_inverse_mass[k];
        }
        let relative_rate = (bb.torque * dt + (bb.spin - ba.spin)) - ba.torque * dt;
        let allowance = [p.twist_velocity, p.swing_velocity, p.swing_velocity];
        let mut angular_low = [0.0; 3];
        let mut angular_high = [0.0; 3];
        for k in 0..3 {
            let disp = angular_axes[k].dot(relative_rate * dt);
            let (pl, ph) = (raw_low[k] + disp, raw_high[k] + disp);
            let (vl, vh) = (disp - allowance[k] * dt, disp + allowance[k] * dt);
            angular_low[k] = precondition(pl.max(vl.min(ph)), angular_inverse_mass[k]);
            angular_high[k] = precondition(ph.min(vh.max(pl)), angular_inverse_mass[k]);
        }
        JointRow {
            lanes: Lanes {
                a,
                b,
                arm_a,
                arm_b,
                linear_axes,
                angular_axes,
                linear_inverse_mass,
                angular_inverse_mass,
                inertia_a: ba.inverse_inertia_or_zero(),
                inertia_b: bb.inverse_inertia_or_zero(),
                inverse_mass: [ba.inverse_mass(), bb.inverse_mass()],
                linear: [0.0; 3],
                angular: [0.0; 3],
            },
            linear_low,
            linear_high,
            angular_low,
            angular_high,
        }
    }

    /// One pass (solver/joint.rs): each lane's correction is what takes it back inside its limits.
    pub fn solve(&mut self, ra: &mut Reaction, rb: &mut Reaction) {
        let (lc, ac) = self.lanes.candidates(ra, rb);
        let limit = |c: f32, low: f32, high: f32| (low + c).max(0.0) + (high + c).min(0.0);
        let linear =
            core::array::from_fn(|k| limit(lc[k], self.linear_low[k], self.linear_high[k]));
        let angular =
            core::array::from_fn(|k| limit(ac[k], self.angular_low[k], self.angular_high[k]));
        self.lanes.apply(ra, rb, linear, angular);
    }
}

/// A raw limit times the lane's inverse mass; a lane between two immovable bodies has no mass and no
/// limit (never NaN).
fn precondition(raw: f32, inverse_mass: f32) -> f32 {
    if inverse_mass == 0.0 {
        0.0
    } else {
        raw * inverse_mass
    }
}
