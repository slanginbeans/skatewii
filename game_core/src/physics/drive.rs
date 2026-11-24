//! Drives: SK8 drive_solver/builder.rs (TU3 `DriveJacobian::Build`, 0x82AE1AE8), drive_parameters.rs
//! and their iteration (solver/drive.rs, 0x82AE2E78). A drive pulls a frame on body A towards a frame on
//! body B, linearly and angularly, as a soft spring-damper or a hard drive (at most a speed), within a
//! strength.

use super::joint::{Lanes, angular_mass_sum, linear_inverse_mass, quaternion_rows};
use super::{Body, Quat, Reaction, STEP, transpose_apply};
use crate::math::{Mat3, Vec3, v3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveKind {
    None,
    Soft,
    Hard,
}

/// One half of a drive (SK8 `RetailDriveParams`): the spring (soft) or the most speed (hard, per
/// second), the damping and the strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveParams {
    pub spring_or_max_velocity: f32,
    pub damping: f32,
    pub max_strength: f32,
    pub kind: DriveKind,
}

impl DriveParams {
    pub const OFF: DriveParams = DriveParams {
        spring_or_max_velocity: 0.0,
        damping: 0.0,
        max_strength: 0.0,
        kind: DriveKind::None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveDynamics {
    pub linear: DriveParams,
    pub angular: DriveParams,
}

/// A frame on a body: its orientation and its origin, in the body's axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveFrame {
    pub orientation: Quat,
    pub translation: Vec3,
}

/// A compiled drive row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveRow {
    pub lanes: Lanes,
    pub linear_softness: f32,
    pub angular_softness: f32,
    pub linear_target: [f32; 3],
    pub angular_target: [f32; 3],
    pub linear_max: [f32; 3],
    pub angular_max: [f32; 3],
}

struct Coefficients {
    softness: f32,
    target: [f32; 3],
    strength: f32,
}

/// SK8 drive_solver/builder.rs `build_coefficients`: the position, rate and acceleration weights.
fn coefficients(
    p: &DriveParams,
    mut position: [f32; 3],
    rate: [f32; 3],
    accel: [f32; 3],
) -> Coefficients {
    let dt = STEP;
    let damping = dt * p.damping;
    let (pw, vw, aw) = if p.kind == DriveKind::Soft {
        let spring = dt * p.spring_or_max_velocity * dt;
        let inv = 1.0 / (1.0 + spring + damping);
        (inv * spring, inv * damping, inv * (spring + damping))
    } else {
        (1.0 / (1.0 + damping), 1.0, 1.0)
    };
    if p.kind == DriveKind::Hard {
        let sq = position[0] * position[0] + position[1] * position[1] + position[2] * position[2];
        let max = dt * p.spring_or_max_velocity;
        if sq > max * max {
            let k = max / libm::sqrtf(sq);
            position = position.map(|v| v * k);
        }
    }
    Coefficients {
        softness: aw,
        target: core::array::from_fn(|i| accel[i] * aw + position[i] * pw + rate[i] * vw),
        strength: dt * p.max_strength * dt,
    }
}

impl DriveRow {
    /// Frame A on body `a`, frame B on body `b`; the error is B's frame less A's, so A is pulled.
    pub fn build(
        bodies: &[Body],
        a: usize,
        b: usize,
        fa: &DriveFrame,
        fb: &DriveFrame,
        d: &DriveDynamics,
    ) -> DriveRow {
        let dt = STEP;
        let (ba, bb) = (&bodies[a], &bodies[b]);
        let arm_a = ba.axes.apply(fa.translation);
        let arm_b = bb.axes.apply(fb.translation);
        let qa = ba.orientation.compose(fa.orientation);
        let qb = bb.orientation.compose(fb.orientation);
        let linear_axes = qb.to_mat3().cols;
        let (raw_axes, rel) = quaternion_rows(qa, qb);
        let c = [rel.x, rel.y, rel.z];
        let sq = c.map(|c| (c * c).clamp(0.0, 1.0));
        let singular = sq.contains(&1.0);
        let inv_len = if singular {
            [1.0; 3]
        } else {
            sq.map(|c| 1.0 / libm::sqrtf(1.0 - c))
        };
        let angular_axes: [Vec3; 3] =
            core::array::from_fn(|i| if singular { linear_axes[i] } else { raw_axes[i] } * inv_len[i]);
        let angular_error: [f32; 3] = core::array::from_fn(|i| c[i] * inv_len[i] * 2.0);

        let position_error = (bb.position + arm_b) - (ba.position + arm_a);
        let rate = (bb.point_velocity(arm_b) - ba.point_velocity(arm_a)) * dt;
        let accel = (bb.point_acceleration(arm_b) - ba.point_acceleration(arm_a)) * (dt * dt);
        let spin_rate = (bb.spin - ba.spin) * dt;
        let spin_accel = (bb.torque - ba.torque) * (dt * dt);
        let linear_inverse_mass =
            linear_axes.map(|ax| linear_inverse_mass(ba, bb, arm_a, arm_b, ax));
        let angular_inverse_mass = angular_axes.map(|ax| {
            let m = angular_mass_sum(ba, bb, ax);
            if m > 0.0 { 1.0 / m } else { 0.0 }
        });
        let arr = |v: Vec3| [v.x, v.y, v.z];
        let lin = coefficients(&d.linear, arr(position_error), arr(rate), arr(accel));
        // The linear weights combine in world space, then project onto B's axes.
        let lt = transpose_apply(
            &Mat3::from_cols(linear_axes[0], linear_axes[1], linear_axes[2]),
            v3(lin.target[0], lin.target[1], lin.target[2]),
        );
        let project = |v: Vec3| angular_axes.map(|ax| ax.dot(v));
        let ang = coefficients(
            &d.angular,
            angular_error,
            project(spin_rate),
            project(spin_accel),
        );
        DriveRow {
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
            linear_softness: lin.softness,
            angular_softness: ang.softness,
            linear_target: core::array::from_fn(|i| lt.get(i) * linear_inverse_mass[i]),
            angular_target: core::array::from_fn(|i| ang.target[i] * angular_inverse_mass[i]),
            linear_max: linear_inverse_mass.map(|m| m * lin.strength),
            angular_max: angular_inverse_mass.map(|m| m * ang.strength),
        }
    }

    /// One pass (solver/drive.rs): the candidate is the softened response plus the target, within the
    /// strength either way.
    pub fn solve(&mut self, ra: &mut Reaction, rb: &mut Reaction) {
        let (lc, ac) = self.lanes.candidates(ra, rb);
        let linear = core::array::from_fn(|k| {
            (lc[k] * self.linear_softness + self.linear_target[k])
                .clamp(-self.linear_max[k], self.linear_max[k])
        });
        let angular = core::array::from_fn(|k| {
            (ac[k] * self.angular_softness + self.angular_target[k])
                .clamp(-self.angular_max[k], self.angular_max[k])
        });
        self.lanes.apply(ra, rb, linear, angular);
    }
}
