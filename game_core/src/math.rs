//! Small 3D math: just what the game needs, `no_std`, f32 only. Y is up, units are metres.

use core::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

pub use libm::{atan2f, cosf, expf, fabsf, sinf, sqrtf};

pub const PI: f32 = core::f32::consts::PI;
pub const TAU: f32 = core::f32::consts::TAU;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    pub const ZERO: Vec3 = v3(0.0, 0.0, 0.0);
    pub const UP: Vec3 = v3(0.0, 1.0, 0.0);

    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn length(self) -> f32 {
        sqrtf(self.dot(self))
    }
    /// Unit vector, or `fallback` when the length is ~0.
    pub fn normalize_or(self, fallback: Vec3) -> Vec3 {
        let l = self.length();
        if l > 1e-6 { self * (1.0 / l) } else { fallback }
    }
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
    pub fn horizontal(self) -> Vec3 {
        v3(self.x, 0.0, self.z)
    }
    pub fn get(self, i: usize) -> f32 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        v3(-self.x, -self.y, -self.z)
    }
}

/// Unit vector pointing along a yaw angle. Yaw 0 faces +Z; positive yaw turns towards +X.
pub fn yaw_dir(yaw: f32) -> Vec3 {
    v3(sinf(yaw), 0.0, cosf(yaw))
}

/// Wraps an angle into (-PI, PI].
pub fn wrap_angle(a: f32) -> f32 {
    let mut a = a % TAU;
    if a > PI {
        a -= TAU;
    } else if a <= -PI {
        a += TAU;
    }
    a
}

pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

/// Frame-rate independent smoothing factor for `x += (target - x) * factor`.
pub fn smooth(rate: f32, dt: f32) -> f32 {
    1.0 - expf(-rate * dt)
}

/// 3x3 rotation stored as columns (the local X, Y, Z axes in world space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub cols: [Vec3; 3],
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 {
        cols: [v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0), v3(0.0, 0.0, 1.0)],
    };

    pub fn from_cols(x: Vec3, y: Vec3, z: Vec3) -> Mat3 {
        Mat3 { cols: [x, y, z] }
    }
    pub fn rot_x(a: f32) -> Mat3 {
        let (s, c) = (sinf(a), cosf(a));
        Mat3::from_cols(v3(1.0, 0.0, 0.0), v3(0.0, c, s), v3(0.0, -s, c))
    }
    pub fn rot_y(a: f32) -> Mat3 {
        let (s, c) = (sinf(a), cosf(a));
        Mat3::from_cols(v3(c, 0.0, -s), v3(0.0, 1.0, 0.0), v3(s, 0.0, c))
    }
    pub fn rot_z(a: f32) -> Mat3 {
        let (s, c) = (sinf(a), cosf(a));
        Mat3::from_cols(v3(c, s, 0.0), v3(-s, c, 0.0), v3(0.0, 0.0, 1.0))
    }
    /// Basis whose Y axis is `up` and whose Z axis is `yaw` projected onto the plane of `up`.
    pub fn yaw_on_surface(yaw: f32, up: Vec3) -> Mat3 {
        let up = up.normalize_or(Vec3::UP);
        let f = yaw_dir(yaw);
        let fwd = (f - up * f.dot(up)).normalize_or(f);
        Mat3::from_cols(up.cross(fwd), up, fwd)
    }
    pub fn apply(&self, v: Vec3) -> Vec3 {
        self.cols[0] * v.x + self.cols[1] * v.y + self.cols[2] * v.z
    }
    pub fn mul(&self, o: &Mat3) -> Mat3 {
        Mat3::from_cols(
            self.apply(o.cols[0]),
            self.apply(o.cols[1]),
            self.apply(o.cols[2]),
        )
    }
    /// `self^T * o` (`o` in this frame, for a rotation).
    pub fn transpose_mul(&self, o: &Mat3) -> Mat3 {
        let t = |v: Vec3| Vec3 {
            x: self.cols[0].dot(v),
            y: self.cols[1].dot(v),
            z: self.cols[2].dot(v),
        };
        Mat3::from_cols(t(o.cols[0]), t(o.cols[1]), t(o.cols[2]))
    }
}

/// 3x4 row-major affine matrix, the same layout as libogc's `Mtx` (`f32[3][4]`, translation in column 3).
pub type Mat34 = [[f32; 4]; 3];

/// Rotation plus translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub rot: Mat3,
    pub pos: Vec3,
}

impl Xform {
    pub const IDENTITY: Xform = Xform {
        rot: Mat3::IDENTITY,
        pos: Vec3::ZERO,
    };

    pub fn new(rot: Mat3, pos: Vec3) -> Xform {
        Xform { rot, pos }
    }
    pub fn point(&self, p: Vec3) -> Vec3 {
        self.rot.apply(p) + self.pos
    }
    /// `self * o`: apply `o` first, then `self`.
    pub fn then(&self, o: &Xform) -> Xform {
        Xform::new(self.rot.mul(&o.rot), self.point(o.pos))
    }
    pub fn to_mat34(&self) -> Mat34 {
        let c = &self.rot.cols;
        let mut m = [[0.0; 4]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            *row = [c[0].get(r), c[1].get(r), c[2].get(r), self.pos.get(r)];
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn yaw_turns_towards_positive_x() {
        assert!(close(yaw_dir(0.0), v3(0.0, 0.0, 1.0)));
        assert!(close(yaw_dir(PI / 2.0), v3(1.0, 0.0, 0.0)));
        assert!(close(
            Mat3::rot_y(PI / 2.0).apply(v3(0.0, 0.0, 1.0)),
            yaw_dir(PI / 2.0)
        ));
    }

    #[test]
    fn wrap_angle_range() {
        assert!((wrap_angle(3.0 * PI) - PI).abs() < 1e-4);
        assert!((wrap_angle(-PI / 2.0 - TAU) + PI / 2.0).abs() < 1e-4);
    }

    #[test]
    fn surface_basis_is_orthonormal() {
        let m = Mat3::yaw_on_surface(0.7, v3(0.0, 1.0, -0.4));
        for i in 0..3 {
            assert!((m.cols[i].length() - 1.0).abs() < 1e-4);
            for j in (i + 1)..3 {
                assert!(m.cols[i].dot(m.cols[j]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn mat34_matches_point_transform() {
        let x = Xform::new(Mat3::rot_y(0.3).mul(&Mat3::rot_x(0.2)), v3(1.0, 2.0, 3.0));
        let m = x.to_mat34();
        let p = v3(0.5, -1.0, 2.0);
        let q = x.point(p);
        for (r, row) in m.iter().enumerate() {
            let got = row[0] * p.x + row[1] * p.y + row[2] * p.z + row[3];
            assert!((got - q.get(r)).abs() < 1e-4);
        }
    }
}
