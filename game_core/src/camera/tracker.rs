//! The camera's trackers: a value chasing a target with clamped acceleration and speed, then smoothed.
//!
//! After SK8-ENGINE/skate-3-rust-engine (GPL-3.0) crates/skate-core/src/camera/tracker.rs,
//! angle_tracker.rs and vector_tracker.rs: each update the tracker wants to reach the target in one step,
//! its acceleration is clamped (more freely the closer it is, between `accel_far` and `accel_near` over
//! the `umbra`..`penumbra` error band), its speed clamped, and then the position only moves `1 -
//! smoothing` of the way to where that speed takes it. Skate 3 runs them once per 1/60 s tick.

use crate::math::{PI, Vec3, wrap_angle};

/// How a tracker moves (angles in radians).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub umbra: f32,
    pub penumbra: f32,
    pub speed: f32,
    /// Acceleration clamp outside the penumbra (far) and inside the umbra (near).
    pub accel_far: f32,
    pub accel_near: f32,
    /// Smoothing far from and near to the target.
    pub smooth_far: f32,
    pub smooth_near: f32,
    /// Moving away from the target zeroes the speed.
    pub overshoot_stops: bool,
}

impl Params {
    pub const fn fixed(speed: f32, accel: f32, smoothing: f32) -> Params {
        Params {
            umbra: 0.0,
            penumbra: 0.0,
            speed,
            accel_far: accel,
            accel_near: accel,
            smooth_far: smoothing,
            smooth_near: smoothing,
            overshoot_stops: true,
        }
    }
}

/// `far` to `near` by `k` (1: near).
fn mix(far: f32, near: f32, k: f32) -> f32 {
    if far == near {
        near
    } else {
        far + (near - far) * k
    }
}

/// 1 inside the umbra, 0 outside the penumbra, linear between.
fn nearness(err: f32, p: &Params) -> f32 {
    if err < p.umbra {
        1.0
    } else if err < p.penumbra {
        1.0 - (err - p.umbra) / (p.penumbra - p.umbra)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scalar {
    pub target: f32,
    pub pos: f32,
    pub vel: f32,
    pub accel: f32,
}

impl Scalar {
    pub fn at(v: f32) -> Scalar {
        Scalar {
            target: v,
            pos: v,
            vel: 0.0,
            accel: 0.0,
        }
    }

    pub fn snap(&mut self, v: f32) {
        *self = Scalar::at(v);
    }

    pub fn update(&mut self, dt: f32, target: f32, p: &Params) {
        let delta = target - self.pos;
        self.step(dt, target, delta, p, false);
    }

    fn step(&mut self, dt: f32, target: f32, delta: f32, p: &Params, wrap: bool) {
        self.target = target;
        if dt <= 0.0 {
            return;
        }
        if p.overshoot_stops && self.vel * delta < 0.0 {
            self.vel = 0.0;
        }
        let inv = 1.0 / dt;
        self.accel = (delta * inv - self.vel) * inv;
        let k = nearness(delta.abs(), p);
        if self.accel * delta > 0.0 {
            let lim = mix(p.accel_far, p.accel_near, k);
            if self.accel.abs() > lim {
                self.accel = if self.accel < 0.0 { -lim } else { lim };
            }
        }
        self.vel += self.accel * dt;
        if self.vel.abs() > p.speed {
            self.vel = if self.vel < 0.0 { -p.speed } else { p.speed };
        }
        let s = mix(p.smooth_far, p.smooth_near, k);
        let step = self.vel * dt * (1.0 - s);
        self.pos = if wrap {
            wrap_angle(self.pos + wrap_angle(step))
        } else {
            self.pos + step
        };
    }
}

/// A [`Scalar`] holding an angle: errors and positions wrap the short way round.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Angle(pub Scalar);

impl Angle {
    pub fn at(a: f32) -> Angle {
        Angle(Scalar::at(wrap_angle(a)))
    }

    pub fn pos(&self) -> f32 {
        self.0.pos
    }

    pub fn snap(&mut self, a: f32) {
        self.0.snap(wrap_angle(a));
    }

    pub fn update(&mut self, dt: f32, target: f32, p: &Params) {
        let target = wrap_angle(target);
        let delta = wrap_angle(target - self.0.pos);
        self.0.step(dt, target, delta, p, true);
    }
}

/// A position chasing a target point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub pos: Vec3,
    pub vel: Vec3,
}

impl Point {
    pub fn at(p: Vec3) -> Point {
        Point {
            pos: p,
            vel: Vec3::ZERO,
        }
    }

    pub fn update(&mut self, dt: f32, target: Vec3, p: &Params) {
        if dt <= 0.0 {
            return;
        }
        let delta = target - self.pos;
        if p.overshoot_stops && delta.dot(self.vel) < 0.0 {
            self.vel = Vec3::ZERO;
        }
        let inv = 1.0 / dt;
        let mut accel = (delta * inv - self.vel) * inv;
        let k = nearness(delta.length(), p);
        if accel.dot(delta) > 0.0 {
            accel = clamp_len(accel, mix(p.accel_far, p.accel_near, k));
        }
        self.vel = clamp_len(self.vel + accel * dt, p.speed);
        let s = mix(p.smooth_far, p.smooth_near, k);
        self.pos += self.vel * (dt * (1.0 - s));
    }
}

pub fn clamp_len(v: Vec3, max: f32) -> Vec3 {
    let l = v.length();
    if l > max && l > 0.0 { v * (max / l) } else { v }
}

/// `a` towards `b` by `k`, the short way round.
pub fn blend_angle(a: f32, b: f32, k: f32) -> f32 {
    wrap_angle(a + wrap_angle(b - a) * k)
}

/// Degrees to radians.
pub const RAD: f32 = PI / 180.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_slows_the_approach_and_clamps_hold() {
        let p = Params::fixed(1000.0, 1.0e6, 0.9);
        let mut t = Scalar::at(0.0);
        t.update(1.0 / 60.0, 1.0, &p);
        // Wants all the way, moves a tenth of it.
        assert!((t.pos - 0.1).abs() < 1e-4, "{}", t.pos);
        let slow = Params::fixed(1.0, 1.0e6, 0.0);
        let mut t = Scalar::at(0.0);
        t.update(1.0 / 60.0, 1.0, &slow);
        assert!((t.pos - 1.0 / 60.0).abs() < 1e-5);
    }

    #[test]
    fn angles_take_the_short_way() {
        let p = Params::fixed(1000.0, 1.0e6, 0.5);
        let mut a = Angle::at(3.0);
        a.update(1.0 / 60.0, -3.0, &p);
        // Across the +-pi seam: 0.28 rad apart, half of it.
        assert!(a.pos() > 3.0 || a.pos() < -3.0, "{}", a.pos());
    }
}
