//! A rigid body and its integration: SK8 rigid_body.rs and rigid_body/dynamic_update.rs (TU3
//! `RigidBody::DynamicUpdate`, 0x82AE6590), point_force.rs and deck_angular_correction.rs.

use super::{Quat, STEP, world_tensor};
use crate::math::{Mat3, Vec3};

/// A body's mass properties (SK8 `RetailInertiaDynamics`): the inverse principal inertia in the body's
/// own axes, the inverse mass, the speed caps and the drags (per second, subtracted from the step's
/// frequency when the velocity is rebuilt from the tick's displacement).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mass {
    pub inverse_inertia: Vec3,
    pub inverse_mass: f32,
    pub max_speed: f32,
    pub max_spin: f32,
    pub linear_drag: f32,
    pub angular_drag: f32,
}

impl Mass {
    /// A solid sphere of `mass` (kg) and `radius` (m): SK8 construction/primitive_mass.rs.
    pub fn sphere(mass: f32, radius: f32, max_spin: f32) -> Mass {
        let i = 0.4 * mass * radius * radius;
        Mass {
            inverse_inertia: Vec3 {
                x: 1.0 / i,
                y: 1.0 / i,
                z: 1.0 / i,
            },
            inverse_mass: 1.0 / mass,
            max_speed: f32::MAX,
            max_spin,
            linear_drag: 0.0,
            angular_drag: 0.0,
        }
    }
}

/// The solver's four correction vectors for one body (SK8 `RetailReactionCorrections`): the linear
/// and angular displacements produce velocity; the position and orientation corrections (the
/// contacts' position-only lane) only move the body.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reaction {
    pub linear: Vec3,
    pub position: Vec3,
    pub angular: Vec3,
    pub orientation: Vec3,
}

impl Reaction {
    /// The velocity-producing correction of a point `arm` from the centre of mass.
    pub fn point(&self, arm: Vec3) -> Vec3 {
        self.linear + self.angular.cross(arm)
    }
    /// The position-only correction of a point `arm` from the centre of mass.
    pub fn point_position(&self, arm: Vec3) -> Vec3 {
        self.position + self.orientation.cross(arm)
    }
}

/// One rigid body (SK8 `BodySnapshot`: `RetailBodyRates` plus its inertia). `force` and `torque` are
/// accelerations (a force already divided by the mass, a torque already through the world inverse
/// inertia); integration consumes them and leaves gravity in `force` for the next tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub orientation: Quat,
    /// The orientation's basis (columns: the body's X, Y, Z axes).
    pub axes: Mat3,
    /// The centre of mass.
    pub position: Vec3,
    pub velocity: Vec3,
    pub spin: Vec3,
    pub force: Vec3,
    pub torque: Vec3,
    pub mass: Mass,
    /// The inverse inertia in world space.
    pub inverse_inertia: Mat3,
    /// Moved by the solver and integrated (SK8 state bit 4); an inactive body is immovable.
    pub active: bool,
}

impl Body {
    pub fn new(mass: Mass, position: Vec3, orientation: Quat, gravity: Vec3) -> Body {
        let orientation = orientation.normalize();
        let axes = orientation.to_mat3();
        Body {
            orientation,
            axes,
            position,
            velocity: Vec3::ZERO,
            spin: Vec3::ZERO,
            force: gravity,
            torque: Vec3::ZERO,
            mass,
            inverse_inertia: world_tensor(&axes, mass.inverse_inertia),
            active: true,
        }
    }

    /// Puts the body at `orientation` (its world inertia follows).
    pub fn set_orientation(&mut self, orientation: Quat) {
        self.orientation = orientation.normalize();
        self.axes = self.orientation.to_mat3();
        self.inverse_inertia = world_tensor(&self.axes, self.mass.inverse_inertia);
    }

    /// The body's inverse mass, zero when it doesn't move.
    pub fn inverse_mass(&self) -> f32 {
        if self.active {
            self.mass.inverse_mass
        } else {
            0.0
        }
    }

    /// The world inverse inertia, zero when the body doesn't move.
    pub fn inverse_inertia_or_zero(&self) -> Mat3 {
        if self.active {
            self.inverse_inertia
        } else {
            Mat3::from_cols(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO)
        }
    }

    /// The velocity of a point `arm` from the centre of mass.
    pub fn point_velocity(&self, arm: Vec3) -> Vec3 {
        self.velocity + self.spin.cross(arm)
    }

    /// The acceleration of a point `arm` from the centre of mass from the accumulated force and torque
    /// (no centripetal term, as Skate 3's builders).
    pub fn point_acceleration(&self, arm: Vec3) -> Vec3 {
        self.force + self.torque.cross(arm)
    }

    /// A world `force` (N) at the body-local `point` (SK8 point_force.rs `accumulate_point_force`).
    pub fn add_force_at(&mut self, force: Vec3, point: Vec3) {
        self.force += force * self.mass.inverse_mass;
        let arm = self.axes.apply(point);
        self.torque += self.inverse_inertia.apply(arm.cross(force));
    }

    /// Turns the body by the world rotation vector `displacement` (rad) this tick on top of its free
    /// motion: the angular acceleration that does it in one step (SK8 deck_angular_correction.rs
    /// `apply_angular_displacement`; the inertia it goes through and back cancels).
    pub fn add_angular_displacement(&mut self, displacement: Vec3) {
        self.torque += displacement * (1.0 / (STEP * STEP));
    }

    /// Turns the body by `requested` this tick less what it already turns that way, and not against it
    /// (SK8 deck_angular_correction.rs `apply_limited_displacement`, Skate 3's 82C07000: the heading,
    /// straighten and collision turns).
    pub fn add_limited_displacement(&mut self, requested: Vec3) {
        let length = requested.length();
        let direction = if length > 1e-6 {
            requested * (1.0 / length)
        } else {
            Vec3::ZERO
        };
        let existing = direction * direction.dot(self.spin * STEP);
        let mut remainder = requested - existing;
        if remainder.dot(requested) < 0.0 {
            remainder = Vec3::ZERO;
        }
        let selected = if existing.dot(requested) < 0.0 {
            requested
        } else {
            remainder
        };
        self.add_angular_displacement(selected);
    }

    /// One step of Skate 3's `DynamicUpdate` with the solver's `reaction`: the accelerations make a
    /// candidate velocity, that times the step plus the corrections is the tick's displacement, the
    /// velocity is rebuilt from it less the drag and capped, and `gravity` is the next tick's force.
    pub fn integrate(&mut self, reaction: &Reaction, gravity: Vec3) {
        if !self.active {
            return;
        }
        let dt = STEP;
        let frequency = 1.0 / dt;
        let linear = (self.force * dt + self.velocity) * dt + reaction.linear;
        let angular = (self.torque * dt + self.spin) * dt + reaction.angular;
        self.position += reaction.position + linear;
        let rotation = angular + reaction.orientation;
        self.orientation = integrate_orientation(self.orientation, rotation);
        self.axes = self.orientation.to_mat3();
        self.inverse_inertia = world_tensor(&self.axes, self.mass.inverse_inertia);
        let rate = |drag: f32| (frequency - drag).max(0.0);
        self.spin = cap(angular * rate(self.mass.angular_drag), self.mass.max_spin);
        self.velocity = cap(linear * rate(self.mass.linear_drag), self.mass.max_speed);
        self.force = gravity;
        self.torque = Vec3::ZERO;
    }
}

/// `q + 0.5 (rotation, 0) q`, normalised: the rotation pre-multiplies (SK8 rigid_body.rs
/// `integrate_orientation`).
pub fn integrate_orientation(q: Quat, rotation: Vec3) -> Quat {
    let v = q.vector();
    let c = rotation.cross(v);
    let inc = (rotation * q.w + c) * 0.5;
    Quat {
        x: q.x + inc.x,
        y: q.y + inc.y,
        z: q.z + inc.z,
        w: q.w - 0.5 * rotation.dot(v),
    }
    .normalize()
}

fn cap(v: Vec3, max: f32) -> Vec3 {
    let sq = v.dot(v);
    if max < f32::MAX && sq > max * max {
        v * (max / libm::sqrtf(sq))
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    const G: Vec3 = v3(0.0, -9.8, 0.0);

    #[test]
    fn a_free_body_falls_and_spins_as_skate_3_integrates() {
        let mut b = Body::new(
            Mass::sphere(1.0, 0.1, f32::MAX),
            Vec3::ZERO,
            Quat::IDENTITY,
            G,
        );
        b.spin = v3(0.0, 2.0, 0.0);
        for _ in 0..60 {
            b.integrate(&Reaction::default(), G);
        }
        // Semi-implicit: v = g t, y = -g dt^2 (1 + 2 + ... + 60).
        assert!((b.velocity.y + 9.8).abs() < 1e-3, "{:?}", b.velocity);
        assert!((b.position.y + 9.8 * STEP * STEP * 1830.0).abs() < 1e-3);
        // Two radians about +Y: the nose (+Z) swings towards +X.
        let nose = b.axes.cols[2];
        assert!(
            (libm::atan2f(nose.x, nose.z) - 2.0).abs() < 0.02,
            "{nose:?}"
        );
    }

    #[test]
    fn drag_takes_its_share_of_the_spin_every_tick() {
        // The stock deck's angular drag (0.45 x 60): the spin rebuilt from a tick's turn at 33/s.
        let mut mass = Mass::sphere(6.0, 0.2, f32::MAX);
        mass.angular_drag = 27.0;
        let mut b = Body::new(mass, Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO);
        b.spin = v3(0.0, 1.0, 0.0);
        b.integrate(&Reaction::default(), Vec3::ZERO);
        assert!((b.spin.y - 33.0 / 60.0).abs() < 1e-4);
    }

    #[test]
    fn displacements_and_forces_reach_the_tick() {
        let mut b = Body::new(
            Mass::sphere(2.0, 0.1, f32::MAX),
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        b.add_angular_displacement(v3(0.0, 0.1, 0.0));
        b.add_force_at(v3(2.0, 0.0, 0.0), Vec3::ZERO);
        b.integrate(&Reaction::default(), Vec3::ZERO);
        let nose = b.axes.cols[2];
        assert!((libm::atan2f(nose.x, nose.z) - 0.1).abs() < 1e-3);
        assert!((b.velocity.x - STEP).abs() < 1e-5);
        // A limited turn already under way adds only what is missing.
        let mut b = Body::new(
            Mass::sphere(2.0, 0.1, f32::MAX),
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        b.spin = v3(0.0, 3.0, 0.0);
        b.add_limited_displacement(v3(0.0, 0.1, 0.0));
        assert!((b.torque.y * STEP * STEP - 0.05).abs() < 1e-5);
    }
}
