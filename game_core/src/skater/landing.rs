//! Landing quality (SK8-ENGINE/skate-3-rust-engine, GPL-3.0: `animation/landing_quality.rs`, Skate 3's
//! `CalcLandingQuality`): straight, sketchy (with a magnitude) or a spin landing, from how the board
//! meets its travel along the surface and how it is still spinning. The state graphs pick their landing
//! clips by it (`HasLandingType`); it decides nothing about bailing.

use crate::math::Vec3;
use crate::tuning::Tuning;

/// Skate 3's landing types: 0 straight, 1 sketchy, 2 spin, 3 slightly off (the graphs land it straight).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quality {
    pub kind: u8,
    /// Signed magnitude (0..1; the sign is the spin's).
    pub magnitude: f32,
    /// Speed across the board and along it (m/s).
    pub side_speed: f32,
    pub forward_speed: f32,
}

/// No landing is judged below this speed along the surface (m/s): it is straight.
const MIN_SPEED: f32 = 2.0;
/// Spinning slower than this (rad/s) isn't spinning.
const NO_SPIN: f32 = 0.1;
/// Board this far off its travel (as the sine of the angle) is sketchy; under `SLIGHT` it is straight.
const SKETCHY: f32 = 0.2;
const SLIGHT: f32 = 0.05;
/// A spin landing past this magnitude stays one however straight the board is.
const STRONG_SPIN: f32 = 0.3;

/// Classifies a landing: the velocity along the surface `vel`, the board's nose `fwd`, the surface's
/// normal `n` and the body's spin `spin` (rad/s, + turning the nose towards +X seen from above).
pub fn classify(t: &Tuning, vel: Vec3, fwd: Vec3, n: Vec3, spin: f32) -> Quality {
    let v = vel - n * vel.dot(n);
    let f = fwd - n * fwd.dot(n);
    let speed = v.length();
    let f_len = f.length();
    let mut q = Quality {
        kind: 0,
        magnitude: 0.0,
        side_speed: 0.0,
        forward_speed: 0.0,
    };
    if f_len * speed < 1e-5 {
        return q;
    }
    q.forward_speed = v.dot(f).abs() / f_len;
    q.side_speed = (f.cross(n).dot(vel) / f_len).abs();
    let vn = v * (1.0 / speed);
    let fnorm = f * (1.0 / f_len);
    // Landing fakie: the tail is the board's front.
    let flipped = fnorm.dot(vn) < 0.0;
    let mut orientation = n.dot(fnorm.cross(vn));
    if flipped {
        orientation = -orientation;
    }
    // Skate 3's spin is positive turning the other way.
    let angular = -spin;
    let twist = t
        .land_sketchy_twist
        .eval(((angular.abs() - NO_SPIN) / 6.9).min(1.0));
    let side = t
        .land_sketchy_side
        .eval(((q.side_speed - NO_SPIN) / 9.9).min(1.0));
    if speed < MIN_SPEED {
        return q;
    }
    let (kind, magnitude) = if angular.abs() <= NO_SPIN {
        (1, side)
    } else {
        // Still spinning towards the travel (finishing the rotation): a spin landing.
        let toward_travel = angular * orientation <= 0.0;
        (if toward_travel { 2 } else { 1 }, twist.max(side))
    };
    q.kind = kind;
    q.magnitude = if angular >= 0.0 {
        magnitude
    } else {
        -magnitude
    };
    if orientation.abs() < SKETCHY && (kind != 2 || q.magnitude.abs() <= STRONG_SPIN) {
        q.kind = if orientation.abs() >= SLIGHT { 3 } else { 0 };
        q.magnitude = 0.0;
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{cosf, sinf, v3};

    fn at(deg: f32, speed: f32, spin: f32) -> Quality {
        let a = deg * crate::math::PI / 180.0;
        let fwd = v3(sinf(a), 0.0, cosf(a));
        classify(&Tuning::BUILT_IN, v3(0.0, 0.0, speed), fwd, Vec3::UP, spin)
    }

    #[test]
    fn landings_are_straight_sketchy_or_spins() {
        assert_eq!(at(0.0, 5.0, 0.0).kind, 0);
        // Slow: straight whatever the angle.
        assert_eq!(at(40.0, 1.5, 0.0).kind, 0);
        // A little off: type 3 (the graphs land it straight); further: sketchy with a magnitude.
        assert_eq!(at(5.0, 5.0, 0.0).kind, 3);
        let s = at(30.0, 5.0, 0.0);
        assert_eq!(s.kind, 1);
        assert!(s.magnitude.abs() > 0.0 && s.side_speed > 2.0, "{s:?}");
        // Fakie counts from the tail.
        assert_eq!(at(180.0, 5.0, 0.0).kind, 0);
        // Still spinning towards the travel: a spin landing; spinning away: sketchy.
        assert_eq!(at(-30.0, 5.0, 3.0).kind, 2);
        assert_eq!(at(30.0, 5.0, 3.0).kind, 1);
    }
}
