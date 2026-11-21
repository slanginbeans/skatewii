//! Body flips (front and back flips) and the upside-down bails that come with them, after SK8's
//! known/body.rs (`CalculateBodyFlipSpeed`), known/trajectory.rs (the flip's speed from the flight),
//! body_flip.rs (`UpdateBodyFlip`), physics/known_air/flip.rs (`BeginBodyFlip`, `ResetBodyFlip`) and
//! known/post_physics.rs (`CheckUpsideDownFallingWipeout`) (SK8-ENGINE/skate-3-rust-engine, GPL-3.0).
//!
//! A flip starts in the air with a flip asked for (the state graphs' `FrontFlip` / `BackFlip`) and a
//! grab held, once the known flight is longer than the tuning's start time for the landing surface. It
//! turns the whole body (board and rider) about an axis along the board, towards the toe side (front)
//! or the heel side (back), as fast as `flip_scalar` turns over the flight asks (smoothed, at most
//! `flip_max_speed`), while the grab is held or early in the flight; let go, it slows to a stop. The
//! turn is drawn about the centre of mass and baked into the board's up at the landing: landing off
//! it bails, and so does falling upside down without the grab.

use crate::math::Vec3;
use crate::tuning::moves::MoveTuning;

use super::rotate;

/// Height of the turn's centre over the board's contact point (m, ours).
pub const PIVOT: f32 = 0.85;
/// A landing whose flipped up is further than this from the surface's up (cosine) bails (ours).
pub const LAND_MIN_DOT: f32 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyFlip {
    /// Unit axis the body turns about (positive angle), the angle so far and the turn speed (rad/s).
    pub axis: Vec3,
    pub angle: f32,
    pub speed: f32,
    /// A front flip (towards the toe side), else a back flip.
    pub front: bool,
}

impl BodyFlip {
    /// Starts a flip: about the board's nose axis, so the head goes over the toe side (`toe`, the side
    /// the rider faces) for a front flip and over the heel side for a back flip.
    pub fn begin(up: Vec3, toe: Vec3, front: bool) -> BodyFlip {
        // d(up)/d(angle) = axis x up: towards the toe side for a front flip.
        let axis = up.cross(toe).normalize_or(Vec3::ZERO);
        BodyFlip {
            axis: if front { axis } else { -axis },
            angle: 0.0,
            speed: 0.0,
            front,
        }
    }

    /// The wanted speed for a flight of `flight` seconds (SK8: `FlipScalar / collision time x 2 pi`).
    pub fn target_speed(t: &MoveTuning, flight: f32) -> f32 {
        t.flip_scalar / flight.max(0.1) * crate::math::TAU
    }

    /// One tick towards `requested` (rad/s): smoothed, capped, turned.
    pub fn update(&mut self, t: &MoveTuning, requested: f32, dt: f32) {
        let speed = self.speed + (requested - self.speed) * t.flip_smoothing;
        self.speed = speed.clamp(-t.flip_max_speed, t.flip_max_speed);
        self.angle += self.speed * dt;
    }

    /// `v` turned by the flip.
    pub fn apply(&self, v: Vec3) -> Vec3 {
        rotate(v, self.axis, self.angle)
    }

    /// Whole turns done (rounded down) and how far into the next one (rad).
    pub fn turns(&self) -> (u32, f32) {
        let a = self.angle.abs();
        let n = (a / crate::math::TAU) as u32;
        (n, a - n as f32 * crate::math::TAU)
    }
}

/// Flipped up `up` against the landing surface `n` with the grab let go: Skate 3's upside-down falling
/// wipeout.
pub fn falling_upside_down(t: &MoveTuning, up: Vec3, n: Vec3) -> bool {
    up.y < t.fall_min_up_y && up.dot(n) < t.fall_max_dot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    #[test]
    fn a_back_flip_turns_the_head_over_the_heels() {
        let up = Vec3::UP;
        let toe = v3(1.0, 0.0, 0.0);
        let mut f = BodyFlip::begin(up, toe, false);
        f.angle = 0.3;
        // A little way in, the head has moved to the heel side.
        assert!(f.apply(up).x < -0.2, "{:?}", f.apply(up));
        let mut f = BodyFlip::begin(up, toe, true);
        f.angle = 0.3;
        assert!(f.apply(up).x > 0.2);
        // A full turn brings it back upright.
        f.angle = crate::math::TAU;
        assert!((f.apply(up) - up).length() < 1e-4);
        assert_eq!(f.turns().0, 1);
    }

    #[test]
    fn the_speed_eases_in_and_is_capped() {
        let t = MoveTuning::BUILT_IN;
        let mut f = BodyFlip::begin(Vec3::UP, v3(1.0, 0.0, 0.0), true);
        f.update(&t, 100.0, 1.0 / 60.0);
        assert!(f.speed > 0.0 && f.speed <= t.flip_max_speed);
        for _ in 0..200 {
            f.update(&t, 100.0, 1.0 / 60.0);
        }
        assert!((f.speed - t.flip_max_speed).abs() < 1e-4);
        // Let go: it slows.
        let s = f.speed;
        f.update(&t, 0.0, 1.0 / 60.0);
        assert!(f.speed < s);
        // Upside down falling: off unless the landing surface faces it.
        assert!(falling_upside_down(&t, -Vec3::UP, Vec3::UP));
        assert!(!falling_upside_down(&t, Vec3::UP, Vec3::UP));
    }
}
