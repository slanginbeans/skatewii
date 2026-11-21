//! Where the camera looks: the shot's look-at point (a mix of the body's reference points, lifted along
//! the body's up), turned by the shot's framing yaw (mirrored for the stance) and pitch, eased between
//! the previous and current shots; then followed by pan and tilt trackers, and rolled (dutch) towards
//! the shot's framing roll.
//!
//! After SK8-ENGINE/skate-3-rust-engine (GPL-3.0) crates/skate-core/src/camera/frame_composer.rs and
//! rig_orientation.rs.

use libm::asinf;

use crate::math::{Vec3, atan2f, cosf, sinf, v3, wrap_angle};
use crate::tuning::CamTuning;

use super::shot::{REFS, Shot};
use super::tracker::{Angle, Params, RAD};

/// The look-at point of a shot.
pub fn reference(shot: &Shot, refs: &[Vec3; REFS], up: Vec3, fallback: Vec3) -> Vec3 {
    let offset = up * shot.board_offset;
    let mut sum = Vec3::ZERO;
    let mut w = 0.0;
    for (p, &k) in refs.iter().zip(&shot.weights) {
        w += k;
        sum += (*p + offset) * k;
    }
    if w.abs() < 1e-4 {
        fallback
    } else {
        sum * (1.0 / w)
    }
}

/// The view direction for a look `dir` turned by framing `yaw` and `pitch` (pitch + looks down).
fn framed(dir: Vec3, yaw: f32, pitch: f32) -> Vec3 {
    let right = Vec3::UP.cross(dir).normalize_or(v3(1.0, 0.0, 0.0));
    let up = dir.cross(right).normalize_or(Vec3::UP);
    let (sy, cy) = (sinf(yaw), cosf(yaw));
    let (sp, cp) = (sinf(pitch), cosf(pitch));
    (right * (sy * cp) + up * (-sp) + dir * (cy * cp)).normalize_or(dir)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Composer {
    pub roll: f32,
}

impl Composer {
    /// The direction one shot frames from `camera`.
    #[allow(clippy::too_many_arguments)]
    fn single(
        shot: &Shot,
        fallback: Vec3,
        camera: Vec3,
        mirror: f32,
        refs: &[Vec3; REFS],
        up: Vec3,
        max_pitch: f32,
    ) -> Vec3 {
        let r = reference(shot, refs, up, fallback);
        let d = r - camera;
        let mut dir = if d.length() < 1e-6 {
            v3(0.0, 0.0, 1.0)
        } else {
            d.normalize_or(v3(0.0, 0.0, 1.0))
        };
        let ceiling = sinf(max_pitch * RAD);
        if dir.y > ceiling {
            dir.y = ceiling;
            dir = dir.normalize_or(v3(0.0, 0.0, 1.0));
        }
        framed(dir, shot.framing[1] * mirror, -shot.framing[2])
    }

    /// The direction to look this tick, from `from` to `to` by `k` (already eased); the roll eases
    /// towards `to`'s.
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        &mut self,
        dt: f32,
        k: f32,
        mirror: f32,
        from: &Shot,
        to: &Shot,
        fallback: Vec3,
        camera: Vec3,
        refs: &[Vec3; REFS],
        up: Vec3,
        t: &CamTuning,
    ) -> Vec3 {
        let b = Self::single(to, fallback, camera, mirror, refs, up, t.max_pitch);
        let a = Self::single(from, fallback, camera, mirror, refs, up, t.max_pitch);
        self.roll = if dt > 0.0 {
            self.roll + (to.framing[0] - self.roll) * t.dutch_speed * dt
        } else {
            to.framing[0]
        };
        (a + (b - a) * k).normalize_or(b)
    }
}

/// Pan and tilt trackers on the composed direction, and the roll.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orientation {
    pub yaw: Angle,
    /// + looks down.
    pub pitch: Angle,
    pub pan_smoothing: f32,
    pub tilt_smoothing: f32,
    pub roll: f32,
    pub framing_mirror: f32,
}

fn cap(v: f32) -> f32 {
    if v >= 1.0 { 1.0 } else { v.min(0.9) }
}

fn params(t: &crate::tuning::camera::AngleTrack, smoothing: f32, clamp_accel: bool) -> Params {
    let (near, far) = if smoothing < 1.0 {
        (smoothing, smoothing.min(t.smoothing_min))
    } else {
        (1.0, 1.0)
    };
    Params {
        umbra: t.umbra * RAD,
        penumbra: t.penumbra * RAD,
        speed: f32::MAX,
        accel_far: if clamp_accel {
            t.accel_far * RAD
        } else {
            f32::MAX
        },
        accel_near: if clamp_accel {
            t.accel_near * RAD
        } else {
            f32::MAX
        },
        smooth_far: far,
        smooth_near: near,
        overshoot_stops: true,
    }
}

/// Yaw and downward pitch of a direction.
pub fn yaw_pitch(d: Vec3) -> (f32, f32) {
    (atan2f(d.x, d.z), -asinf(d.y.clamp(-1.0, 1.0)))
}

/// The direction of yaw and downward pitch.
pub fn direction(yaw: f32, pitch: f32) -> Vec3 {
    v3(
        sinf(yaw) * cosf(pitch),
        -sinf(pitch),
        cosf(yaw) * cosf(pitch),
    )
}

impl Orientation {
    /// One tick towards `target`. `snap` jumps there; `tilt` lets the pitch follow (shots that follow
    /// the board into the air).
    pub fn update(&mut self, dt: f32, target: Vec3, t: &CamTuning, snap: bool, tilt: bool) {
        let (yaw, pitch) = yaw_pitch(target);
        let err = wrap_angle(yaw - self.yaw.pos()).abs();
        let (u, p) = (t.pan_umbra * RAD, t.pan_penumbra * RAD);
        let near = if err < u {
            1.0
        } else if err < p {
            1.0 - (err - u) / (p - u)
        } else {
            0.0
        };
        let response = t.pan_curve.eval(near);
        let m = 1.0 - t.pan_min_smoothing;
        let least = 1.0 - m * m;
        let pan = if self.pan_smoothing >= least {
            (self.pan_smoothing - least) * response + least
        } else {
            self.pan_smoothing
        };
        let pan = cap(pan);
        self.tilt_smoothing = cap(self.tilt_smoothing);
        if snap {
            self.pitch.snap(pitch);
            self.yaw.snap(yaw);
        }
        if tilt {
            self.pitch
                .update(dt, pitch, &params(&t.tilt, self.tilt_smoothing, true));
        }
        self.yaw.update(dt, yaw, &params(&t.pan, pan, true));
    }

    /// The view: direction and up (rolled about the view by the roll, mirrored for the stance).
    pub fn view(&self) -> (Vec3, Vec3) {
        let at = direction(self.yaw.pos(), self.pitch.pos());
        let right = Vec3::UP.cross(at).normalize_or(v3(1.0, 0.0, 0.0));
        let up = at.cross(right).normalize_or(Vec3::UP);
        let a = self.roll * self.framing_mirror;
        (at, up * cosf(a) - right * sinf(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_yaw_turns_the_view_to_its_right() {
        let d = v3(0.0, 0.0, 1.0);
        let f = framed(d, 0.3, 0.0);
        // Right of a view along +z is +x (up x at).
        assert!(f.x > 0.25, "{f:?}");
        // Positive pitch looks down.
        assert!(framed(d, 0.0, 0.2).y < -0.15);
    }

    #[test]
    fn view_up_is_upright_without_roll() {
        let o = Orientation {
            framing_mirror: 1.0,
            ..Default::default()
        };
        let (at, up) = o.view();
        assert!(at.z > 0.99 && up.y > 0.99);
        let rolled = Orientation {
            roll: 0.2,
            framing_mirror: 1.0,
            ..Default::default()
        };
        let (_, up2) = rolled.view();
        let mirrored = Orientation {
            roll: 0.2,
            framing_mirror: -1.0,
            ..Default::default()
        };
        let (_, up3) = mirrored.view();
        assert!(up2.x * up3.x < 0.0, "mirror flips the roll");
    }
}
