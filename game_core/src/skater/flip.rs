//! Body flips (front and back flips) and the upside-down bails that come with them, after SK8's
//! known/body.rs (`CalculateBodyFlipSpeed`), known/trajectory.rs (the flip's speed from the flight),
//! body_flip.rs (`UpdateBodyFlip`), physics/known_air/flip.rs (`BeginBodyFlip`, `ResetBodyFlip`) and
//! known/post_physics.rs (`CheckUpsideDownFallingWipeout`) (SK8-ENGINE/skate-3-rust-engine, GPL-3.0).
//!
//! A flip starts in the air with a flip asked for (the state graphs' `FrontFlip` / `BackFlip`) and a
//! grab held, once the known flight is longer than the tuning's start time for the landing surface. It
//! turns the whole body (board and rider) end over end about the board's lateral axis (SK8 BeginBodyFlip:
//! the reckoning system frame's right, turned round riding the other way), the head going over towards
//! the travel (front) or back against it (back), as fast as `flip_scalar` turns over the flight asks (smoothed, at most
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
/// A perfect body flip's turn over the flight (SK8 known/trajectory.rs `0xC0F1463B`, -2.4 pi; the
/// sign is the axis's here) and its fixed tick (`0x3C888889`, 1/60 s).
const PERFECT_TURN: f32 = 2.4 * core::f32::consts::PI;
const PERFECT_STEP: f32 = 1.0 / 60.0;

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
    /// Starts a flip: end over end about the board's lateral axis (square to `up` and `ahead`, the
    /// board's long axis pointed the way it travels), so the head goes over forwards (towards `ahead`)
    /// for a front flip and backwards for a back flip, whatever the stance (SK8 BeginBodyFlip turns
    /// the axis round only for the direction of travel).
    pub fn begin(up: Vec3, ahead: Vec3, front: bool) -> BodyFlip {
        // d(up)/d(angle) = axis x up = (up x ahead) x up: towards `ahead` for a front flip.
        let axis = up.cross(ahead).normalize_or(Vec3::ZERO);
        BodyFlip {
            axis: if front { axis } else { -axis },
            angle: 0.0,
            speed: 0.0,
            front,
        }
    }

    /// The wanted speed for a flight of `flight` seconds (SK8 InitTrajectoryInfo: `FlipScalar /
    /// collision time x 2 pi`; with the mode's perfect body flips `2.4 pi / collision time`).
    pub fn target_speed(t: &MoveTuning, flight: f32) -> f32 {
        let flight = flight.max(0.1);
        if t.perfect_body_flips {
            PERFECT_TURN / flight
        } else {
            t.flip_scalar / flight * crate::math::TAU
        }
    }

    /// One tick towards `requested` (rad/s): smoothed, capped, turned. With the mode's perfect body
    /// flips (SK8 UpdateBodyFlip) the cap is what is left of one turn x 12 instead, the turn a fixed
    /// tick, and it stops at one turn.
    pub fn update(&mut self, t: &MoveTuning, requested: f32, dt: f32) {
        let mut speed = self.speed + (requested - self.speed) * t.flip_smoothing;
        if t.perfect_body_flips {
            let tau = crate::math::TAU;
            let left = (tau - self.angle.abs()) * 12.0;
            let sign = if speed > 0.0 { 1.0 } else { -1.0 };
            if speed.abs() >= left {
                speed = sign * left;
            }
            self.speed = speed;
            self.angle = (self.angle + speed * PERFECT_STEP).clamp(-tau, tau);
            return;
        }
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
    fn a_back_flip_turns_the_head_back_end_over_end() {
        let up = Vec3::UP;
        let ahead = v3(0.0, 0.0, 1.0);
        let mut f = BodyFlip::begin(up, ahead, false);
        f.angle = 0.3;
        // A little way in, the head has moved back against the travel, not to a side.
        let head = f.apply(up);
        assert!(head.z < -0.2 && head.x.abs() < 1e-4, "{head:?}");
        let mut f = BodyFlip::begin(up, ahead, true);
        f.angle = 0.3;
        let head = f.apply(up);
        assert!(head.z > 0.2 && head.x.abs() < 1e-4, "{head:?}");
        // A full turn brings it back upright.
        f.angle = crate::math::TAU;
        assert!((f.apply(up) - up).length() < 1e-4);
    }

    #[test]
    fn a_perfect_body_flip_ends_exactly_one_turn_round() {
        // SK8 body_flip.rs `update` with `PerfectBodyFlips`: no speed cap but what is left of the turn
        // x 12, the angle stopping at 2 pi; the target is 2.4 pi over the flight.
        let mut t = MoveTuning::BUILT_IN;
        t.perfect_body_flips = true;
        assert!((BodyFlip::target_speed(&t, 1.2) - 2.0 * core::f32::consts::PI).abs() < 1e-4);
        let mut f = BodyFlip::begin(Vec3::UP, v3(0.0, 0.0, 1.0), true);
        let mut fastest: f32 = 0.0;
        for _ in 0..600 {
            f.update(&t, 100.0, 1.0 / 60.0);
            fastest = fastest.max(f.speed);
        }
        assert!(
            fastest > t.flip_max_speed,
            "not capped by FlipMaxSpeed: {fastest}"
        );
        assert!((f.angle - crate::math::TAU).abs() < 1e-3, "{}", f.angle);
        // Without it the cap holds and the flip turns on past one turn.
        t.perfect_body_flips = false;
        let mut f = BodyFlip::begin(Vec3::UP, v3(0.0, 0.0, 1.0), true);
        for _ in 0..120 {
            f.update(&t, 100.0, 1.0 / 60.0);
        }
        assert!(f.speed <= t.flip_max_speed && f.angle > crate::math::TAU);
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
