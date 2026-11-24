//! Contact rows: SK8 contact.rs (`generate_contact`) and solver/contact_build/ (TU3
//! `ContactBatchBuild`, 0x82AE10C8), and their iteration (solver/contact.rs, 0x82AE2914).
//!
//! A contact has three velocity lanes (the normal, then two friction axes, the first along the
//! sliding direction) and a fourth, position-only lane along the normal that pushes the bodies out of
//! penetration without giving them velocity. Body A is pushed along `+normal`.

use super::{Body, Reaction, STEP, UNBOUNDED};
use crate::math::{Vec3, v3};

/// What the narrow phase reports: the touching points on each body (world), the normal (facing body
/// A, out of B) and the combined material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactInput {
    pub point_a: Vec3,
    pub point_b: Vec3,
    pub normal: Vec3,
    pub restitution: f32,
    pub static_friction: f32,
    pub dynamic_friction: f32,
}

/// The friction coefficients of two touching materials: the larger frictions and the smaller
/// restitution (SK8 contact.rs `combine_contact_materials`).
pub fn combine(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2])]
}

/// A compiled contact row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactRow {
    pub a: Option<usize>,
    pub b: Option<usize>,
    pub arm_a: Vec3,
    pub arm_b: Vec3,
    /// The normal and the two friction axes.
    pub axes: [Vec3; 3],
    /// Per axis: one over the effective mass (the impulse that closes a unit of error).
    pub inverse_effective_mass: [f32; 3],
    /// Per axis and body: the angular correction a unit impulse gives.
    pub angular_a: [Vec3; 3],
    pub angular_b: [Vec3; 3],
    pub inverse_mass: [f32; 2],
    pub static_friction: f32,
    pub dynamic_friction: f32,
    /// The three velocity lanes' targets and the position lane's.
    pub target: [f32; 4],
    /// The accumulated impulses (the normal's is what the solve reports as its load).
    pub impulse: [f32; 4],
}

fn body_at(bodies: &[Body], i: Option<usize>) -> Option<&Body> {
    i.map(|i| &bodies[i])
}

impl ContactRow {
    /// Builds the row from the bodies' state this tick, after this tick's forces were added.
    pub fn build(
        bodies: &[Body],
        a: Option<usize>,
        b: Option<usize>,
        c: &ContactInput,
    ) -> ContactRow {
        let ba = body_at(bodies, a);
        let bb = body_at(bodies, b);
        let arm = |body: Option<&Body>, p: Vec3| body.map_or(Vec3::ZERO, |b| p - b.position);
        let arm_a = arm(ba, c.point_a);
        let arm_b = arm(bb, c.point_b);
        let velocity =
            |body: Option<&Body>, r: Vec3| body.map_or(Vec3::ZERO, |b| b.point_velocity(r));
        let relative = velocity(bb, arm_b) - velocity(ba, arm_a);
        // The first friction axis along the sliding (generate_contact): relative velocity x normal.
        let n = c.normal;
        let mut t = relative.cross(n);
        let mut sq = t.dot(t);
        if sq < 1.0e-30 {
            if 0.5 - n.x * n.x >= 0.0 {
                t = v3(1.0 - n.x * n.x, -n.y * n.x, -n.z * n.x);
                sq = t.x;
            } else {
                t = -v3(-n.x * n.z, -n.y * n.z, 1.0 - n.z * n.z);
                sq = 1.0 - n.z * n.z;
            }
        }
        let inv_len = 1.0 / libm::sqrtf(sq);
        let t0 = t * inv_len;
        let t1 = n.cross(t) * inv_len;
        let axes = [n, t0, t1];

        let active = |body: Option<&Body>| body.is_some_and(|b| b.active);
        let inverse_mass = [
            ba.map_or(0.0, |b| b.inverse_mass()),
            bb.map_or(0.0, |b| b.inverse_mass()),
        ];
        let response = |body: Option<&Body>, r: Vec3| {
            axes.map(|ax| match body {
                Some(b) if active(Some(b)) => b.inverse_inertia.apply(r.cross(ax)),
                _ => Vec3::ZERO,
            })
        };
        let angular_a = response(ba, arm_a);
        let angular_b = response(bb, arm_b);
        let inverse_effective_mass: [f32; 3] = core::array::from_fn(|k| {
            let ja = arm_a.cross(axes[k]);
            let jb = arm_b.cross(axes[k]);
            let m = ja.dot(angular_a[k]) + inverse_mass[0] + jb.dot(angular_b[k]) + inverse_mass[1];
            if m > 0.0 { 1.0 / m } else { 0.0 }
        });
        let accel = |body: Option<&Body>, r: Vec3| match body {
            Some(b) if b.active => b.point_acceleration(r),
            _ => Vec3::ZERO,
        };
        let separation = c.point_b - c.point_a;
        let relative_accel = accel(bb, arm_b) - accel(ba, arm_a);
        let scaled_velocity = relative * STEP;
        let predicted = scaled_velocity + separation + relative_accel * (STEP * STEP);
        let restitution = scaled_velocity * -c.restitution;
        let sep = n.dot(separation) * inverse_effective_mass[0];
        let rest = n.dot(restitution) * inverse_effective_mass[0];
        let pred: [f32; 3] =
            core::array::from_fn(|k| axes[k].dot(predicted) * inverse_effective_mass[k]);
        // Restitution only while approaching; a contact not yet touching closes only its predicted gap.
        let mut adjust = if rest >= 0.0 { 0.0 } else { sep + rest };
        if sep >= 0.0 {
            adjust = rest;
        }
        if 0.0 >= pred[0] {
            adjust = 0.0;
        }
        ContactRow {
            a,
            b,
            arm_a,
            arm_b,
            axes,
            inverse_effective_mass,
            angular_a,
            angular_b,
            inverse_mass,
            static_friction: c.static_friction,
            dynamic_friction: c.dynamic_friction,
            target: [pred[0] - adjust, pred[1], pred[2], sep],
            impulse: [0.0; 4],
        }
    }

    /// One pass over the row (solver/contact.rs): friction is bounded by the normal impulse from
    /// before this pass; past the static limit it takes the dynamic one.
    pub fn solve(&mut self, ra: &mut Reaction, rb: &mut Reaction) {
        let position_error = rb.point_position(self.arm_b) - ra.point_position(self.arm_a);
        let total_error = rb.point(self.arm_b) - ra.point(self.arm_a) + position_error;
        let candidate: [f32; 4] = core::array::from_fn(|lane| {
            if lane == 3 {
                self.impulse[3]
                    + self.target[3]
                    + self.inverse_effective_mass[0] * self.axes[0].dot(position_error)
            } else {
                self.impulse[lane]
                    + self.target[lane]
                    + self.inverse_effective_mass[lane] * self.axes[lane].dot(total_error)
            }
        });
        let next: [f32; 4] = core::array::from_fn(|lane| {
            let c = candidate[lane];
            if lane == 0 || lane == 3 {
                return if c >= 0.0 { c.min(UNBOUNDED) } else { 0.0 };
            }
            let normal = self.impulse[0];
            let (s, d) = (
                self.static_friction * normal,
                self.dynamic_friction * normal,
            );
            if c < -s {
                -d
            } else if c > s {
                d
            } else {
                c
            }
        });
        let change: [f32; 4] = core::array::from_fn(|i| next[i] - self.impulse[i]);
        self.impulse = next;
        let linear = self.axes[0] * change[0] + self.axes[1] * change[1] + self.axes[2] * change[2];
        let push = self.axes[0] * change[3];
        ra.angular += self.angular_a[0] * change[0]
            + self.angular_a[1] * change[1]
            + self.angular_a[2] * change[2];
        ra.orientation += self.angular_a[0] * change[3];
        ra.linear += linear * self.inverse_mass[0];
        ra.position += push * self.inverse_mass[0];
        rb.angular -= self.angular_b[0] * change[0]
            + self.angular_b[1] * change[1]
            + self.angular_b[2] * change[2];
        rb.orientation -= self.angular_b[0] * change[3];
        rb.linear -= linear * self.inverse_mass[1];
        rb.position -= push * self.inverse_mass[1];
    }

    /// The normal force the row carried this tick (N), from its accumulated displacement impulse.
    pub fn normal_force(&self) -> f32 {
        self.impulse[0] / (STEP * STEP)
    }

    /// The friction force on body A (N, world).
    pub fn friction_force(&self) -> Vec3 {
        (self.axes[1] * self.impulse[1] + self.axes[2] * self.impulse[2]) * (1.0 / (STEP * STEP))
    }
}
