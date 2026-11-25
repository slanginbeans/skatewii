//! Skate 3's rigid-body physics: bodies, their mass, and the constraint solve that moves them.
//!
//! A port of SK8-ENGINE/skate-3-rust-engine's reconstruction of Skate 3's RenderWare physics
//! (GPL-3.0; crates/skate-core/src/physics: rigid_body.rs, rigid_body/dynamic_update.rs, mass.rs,
//! contact.rs, solver/contact_build/, joint_builder.rs, drive_solver/, solver.rs and solver/,
//! constraint_frames.rs, point_force.rs, deck_angular_correction.rs). The formulas and their order are
//! SK8's; the arithmetic is plain f32 (SK8 keeps the Xenon's fused multiply-adds and reciprocal
//! estimates, which the Wii's CPU doesn't have) and rows keep named fields instead of SK8's packed
//! native records. See docs/graph-runner.md, "The rigid board".
//!
//! The solver works in displacements per tick, as Skate 3's does: every body carries four correction
//! vectors (a linear and an angular displacement that produce velocity, and a position and orientation
//! correction that only move the body), each constraint row turns its error into an "impulse" in those
//! units, and `Body::integrate` adds them to the tick's free motion.

pub mod board;
pub mod body;
pub mod contact;
pub mod drive;
pub mod joint;
pub mod rider_body;
pub mod skeleton;
pub mod solver;

pub use body::{Body, Reaction};
pub use contact::{ContactInput, ContactRow};
pub use drive::{DriveDynamics, DriveFrame, DriveKind, DriveParams, DriveRow};
pub use joint::{JointFrames, JointParams, JointRow};
pub use solver::solve;

use crate::math::{Mat3, Vec3, v3};

/// Skate 3's fixed simulation step (s): the binary32 nearest 1/60 that TU3 loads (SK8
/// rigid_body.rs `fixed_60_hz`).
pub const STEP: f32 = 1.0 / 60.0;

/// TU3's finite "unbounded" value (SK8 joint_builder.rs `FINITE_INFINITY`).
pub const UNBOUNDED: f32 = f32::MAX;

/// A rotation quaternion, RenderWare's `(x, y, z, w)` lanes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub fn vector(self) -> Vec3 {
        v3(self.x, self.y, self.z)
    }

    /// `self * o` (apply `o` first): SK8 constraint_frames.rs `compose`.
    pub fn compose(self, o: Quat) -> Quat {
        let c = self.vector().cross(o.vector());
        Quat {
            x: self.x * o.w + o.x * self.w + c.x,
            y: self.y * o.w + o.y * self.w + c.y,
            z: self.z * o.w + o.z * self.w + c.z,
            w: self.w * o.w - self.vector().dot(o.vector()),
        }
    }

    pub fn conjugate(self) -> Quat {
        Quat {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    pub fn normalize(self) -> Quat {
        let l = libm::sqrtf(self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w);
        if l < 1e-20 {
            return Quat::IDENTITY;
        }
        let k = 1.0 / l;
        Quat {
            x: self.x * k,
            y: self.y * k,
            z: self.z * k,
            w: self.w * k,
        }
    }

    /// Rotation by `angle` (rad) about the unit `axis`.
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
        let (s, c) = (libm::sinf(angle * 0.5), libm::cosf(angle * 0.5));
        Quat {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: c,
        }
    }

    /// The rotation's basis (columns: the local X, Y, Z axes): SK8 constraint_frames.rs `basis`.
    pub fn to_mat3(self) -> Mat3 {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        Mat3::from_cols(
            v3(
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y + w * z),
                2.0 * (x * z - w * y),
            ),
            v3(
                2.0 * (x * y - w * z),
                1.0 - 2.0 * (z * z + x * x),
                2.0 * (y * z + w * x),
            ),
            v3(
                2.0 * (x * z + w * y),
                2.0 * (y * z - w * x),
                1.0 - 2.0 * (x * x + y * y),
            ),
        )
    }

    /// The quaternion of a rotation basis: SK8 drive_frames.rs `retail_quaternion_from_basis`.
    pub fn from_mat3(m: &Mat3) -> Quat {
        let c = &m.cols;
        let (m00, m01, m02) = (c[0].x, c[1].x, c[2].x);
        let (m10, m11, m12) = (c[0].y, c[1].y, c[2].y);
        let (m20, m21, m22) = (c[0].z, c[1].z, c[2].z);
        let trace = m00 + m11 + m22;
        let pick = |sum: f32, lanes: [f32; 4], dominant: usize| {
            let root = libm::sqrtf(sum);
            let half_inverse = 0.5 / root;
            let mut q = lanes.map(|v| v * half_inverse);
            q[dominant] = root * 0.5;
            Quat {
                x: q[0],
                y: q[1],
                z: q[2],
                w: q[3],
            }
        };
        if trace > 0.0 {
            pick(trace + 1.0, [m21 - m12, m02 - m20, m10 - m01, 0.0], 3)
        } else if m00 > m11 && m00 > m22 {
            pick(
                m00 - m11 - m22 + 1.0,
                [0.0, m01 + m10, m02 + m20, m21 - m12],
                0,
            )
        } else if m11 > m22 {
            pick(
                m11 - m00 - m22 + 1.0,
                [m01 + m10, 0.0, m12 + m21, m02 - m20],
                1,
            )
        } else {
            pick(
                m22 - m00 - m11 + 1.0,
                [m02 + m20, m12 + m21, 0.0, m10 - m01],
                2,
            )
        }
    }

    /// `v` rotated by this quaternion.
    pub fn rotate(self, v: Vec3) -> Vec3 {
        self.to_mat3().apply(v)
    }
}

/// `m^T v`: a world vector in the frame whose axes are `m`'s columns.
pub fn transpose_apply(m: &Mat3, v: Vec3) -> Vec3 {
    v3(m.cols[0].dot(v), m.cols[1].dot(v), m.cols[2].dot(v))
}

/// `R diag(d) R^T`: a principal-axis tensor in world space (SK8 dynamic_update.rs
/// `inverse_inertia`).
pub fn world_tensor(r: &Mat3, d: Vec3) -> Mat3 {
    let c = &r.cols;
    let col = |j: usize| {
        c[0] * (d.x * c[0].get(j)) + c[1] * (d.y * c[1].get(j)) + c[2] * (d.z * c[2].get(j))
    };
    Mat3::from_cols(col(0), col(1), col(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn quaternions_round_trip_through_their_basis() {
        let q = Quat::from_axis_angle(v3(0.3, 0.8, -0.52).normalize_or(Vec3::UP), 2.1);
        let m = q.to_mat3();
        let back = Quat::from_mat3(&m);
        // q and -q are the same rotation.
        let same = (back.w - q.w).abs() < 1e-5 || (back.w + q.w).abs() < 1e-5;
        assert!(same, "{q:?} {back:?}");
        assert!(close(back.to_mat3().cols[2], m.cols[2]));
        // Composition applies the right-hand rotation first.
        let a = Quat::from_axis_angle(Vec3::UP, 0.5);
        let b = Quat::from_axis_angle(v3(1.0, 0.0, 0.0), 0.25);
        let v = v3(0.2, -0.4, 1.0);
        assert!(close(a.compose(b).rotate(v), a.rotate(b.rotate(v))));
        assert!(close(a.conjugate().rotate(a.rotate(v)), v));
    }

    #[test]
    fn a_world_tensor_turns_with_its_body() {
        let r = Quat::from_axis_angle(Vec3::UP, 0.7).to_mat3();
        let d = v3(2.0, 3.0, 5.0);
        let t = world_tensor(&r, d);
        // Along a body axis the tensor scales by that axis' value.
        assert!(close(t.apply(r.cols[2]), r.cols[2] * 5.0));
        assert!(close(t.apply(r.cols[0]), r.cols[0] * 2.0));
    }
}
