//! Camera shake, after Skate 3's (SK8-ENGINE/skate-3-rust-engine, GPL-3.0, crates/skate-core/src/camera/
//! shake.rs, shake_update.rs, shake_curve.rs, manager_output.rs): a hand-held sway read from a loop of
//! samples, its amplitude and speed through the loop growing with the camera's speed over the ground, and
//! a one-shot impulse on landings that decays away.
//!
//! Each tick (`ShakeEffect82E05800`):
//! - the shake fades in (out) at 5 a second while the rider isn't (is) wiping out;
//! - amplitude = Bezier(speed / `AmplitudeTopSkaterSpeed`) between `AmplitudeMin` and `AmplitudeMax`,
//!   plus the impulse; the frequency the same way, or `OneShotFrequency` while an impulse lasts;
//! - the loop advances `DataFPS` x frequency samples a second; the two samples around it are mixed;
//! - only a camera within 3.4 m of its anchor shakes (fully from 2.8 m): Skate 3's close low camera
//!   sways, the high camera's longer arm hardly does;
//! - rotation = sample x amplitude x `RotationMultiplier` (the roll also x `DutchMultiplier`), turning
//!   the view about its own axes; translation = sample x `TranslationMultiplier` x amplitude, along them;
//! - the impulse decays by `OneShotDecay` a tick and stops under 0.005.
//!
//! A landing (the camera's ground state coming on while it moved into the ground, `82E055D8`) sets the
//! impulse to `OneShotMagnitude` x the landing speed's share between `OneShotMinVelocity` and
//! `OneShotMaxVelocity` squared, weighted by how square to the ground it came down. Skate 3's numbers
//! make that share 1 for any landing (both speeds are 0). A wipeout fades the shake out (Skate 3 shakes
//! a wipeout only in its broken-bone slow motion, which flatground doesn't have).
//!
//! The samples are a data file on Skate 3's disc (`data/camera/1.shk`, brought in by `fg_import rules` as
//! [`ENTRY`]); without it flatground uses its own loop ([`Samples::built_in`]), of about the same size.

use alloc::vec::Vec;

use crate::math::{Vec3, sinf};
use crate::tuning::camera::ShakeTuning;
use libm::acosf;

pub const ENTRY: &str = "rules/camera/shake";
pub const MAGIC: [u8; 4] = *b"FGSK";
pub const VERSION: u32 = 1;

/// The loop of shake samples, each sequence centred on its mean: rotations (radians, about the view's
/// right, up and forward) and translations (m, along them).
#[derive(Clone, Debug, PartialEq)]
pub struct Samples {
    pub rotation: Vec<[f32; 3]>,
    pub translation: Vec<[f32; 3]>,
}

impl Samples {
    /// From rows of rotation then translation, centred (as Skate 3 loads them). None if empty or not all
    /// finite.
    pub fn from_rows(rows: &[[f32; 6]]) -> Option<Samples> {
        if rows.is_empty() || rows.iter().flatten().any(|v| !v.is_finite()) {
            return None;
        }
        let n = rows.len() as f32;
        let mut mean = [0.0f32; 6];
        for r in rows {
            for i in 0..6 {
                mean[i] += r[i] / n;
            }
        }
        Some(Samples {
            rotation: rows
                .iter()
                .map(|r| [r[0] - mean[0], r[1] - mean[1], r[2] - mean[2]])
                .collect(),
            translation: rows
                .iter()
                .map(|r| [r[3] - mean[3], r[4] - mean[4], r[5] - mean[5]])
                .collect(),
        })
    }

    /// flatground's own loop: 128 samples of a few sines a loop on each axis (seamless round the loop),
    /// sized like Skate 3's (about 0.003 to 0.006 rad and 0.3 to 0.7 m standard deviation; the
    /// translation multiplier scales it down to centimetres).
    pub fn built_in() -> Samples {
        const N: usize = 128;
        // Per axis: (cycles a loop, weight, phase) of three sines, and the axis' standard deviation.
        type Wave = (f32, f32, f32);
        const AXES: [([Wave; 3], f32); 6] = [
            (
                [(3.0, 0.8, 0.4), (7.0, 0.45, 1.9), (13.0, 0.25, 4.1)],
                0.0028,
            ),
            (
                [(2.0, 0.8, 2.2), (5.0, 0.45, 0.3), (11.0, 0.25, 5.0)],
                0.0036,
            ),
            (
                [(4.0, 0.8, 1.1), (9.0, 0.45, 3.3), (17.0, 0.25, 0.7)],
                0.0064,
            ),
            ([(1.0, 0.8, 0.9), (4.0, 0.45, 2.6), (9.0, 0.25, 1.4)], 0.71),
            ([(2.0, 0.8, 3.7), (6.0, 0.45, 0.2), (11.0, 0.25, 2.9)], 0.40),
            ([(3.0, 0.8, 5.3), (5.0, 0.45, 1.7), (13.0, 0.25, 0.5)], 0.27),
        ];
        // The three weights' standard deviation: sqrt(sum of w^2 / 2).
        let unit = libm::sqrtf((0.8f32 * 0.8 + 0.45 * 0.45 + 0.25 * 0.25) / 2.0);
        let rows: Vec<[f32; 6]> = (0..N)
            .map(|i| {
                let a = core::f32::consts::TAU * i as f32 / N as f32;
                let mut r = [0.0; 6];
                for (k, (waves, sd)) in AXES.iter().enumerate() {
                    let v: f32 = waves.iter().map(|(c, w, p)| w * sinf(c * a + p)).sum();
                    r[k] = v * sd / unit;
                }
                r
            })
            .collect();
        Samples::from_rows(&rows).unwrap_or(Samples {
            rotation: alloc::vec![[0.0; 3]],
            translation: alloc::vec![[0.0; 3]],
        })
    }

    /// The rules pack entry: `"FGSK"`, u32 version, u32 count, then per sample six little-endian f32
    /// (rotation x y z, translation x y z), as centred.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.rotation.len() * 24);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.rotation.len() as u32).to_le_bytes());
        for (r, t) in self.rotation.iter().zip(&self.translation) {
            for v in r.iter().chain(t) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    pub fn parse(b: &[u8]) -> Option<Samples> {
        if b.len() < 12 || b[..4] != MAGIC {
            return None;
        }
        let word = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        if word(4) != VERSION {
            return None;
        }
        let n = word(8) as usize;
        if n == 0 || n > 4096 || b.len() != 12 + n * 24 {
            return None;
        }
        let f = |at: usize| f32::from_bits(word(at));
        let rows: Vec<[f32; 6]> = (0..n)
            .map(|i| core::array::from_fn(|k| f(12 + i * 24 + k * 4)))
            .collect();
        Samples::from_rows(&rows)
    }
}

/// Skate 3's shake curve (`8296F530` / `8296F388`): a cubic Bezier through four (x, y) control points,
/// x inverted by Newton's method from t = 0.5 (ten steps at most, stopping within 0.01), y returned.
pub fn curve(points: &[[f32; 4]; 4], input: f32) -> f32 {
    const BASIS: [[f32; 4]; 4] = [
        [-1.0, 3.0, -3.0, 1.0],
        [3.0, -6.0, 3.0, 0.0],
        [-3.0, 3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
    ];
    let c: [[f32; 2]; 4] = core::array::from_fn(|r| {
        core::array::from_fn(|lane| (0..4).map(|k| points[k][lane] * BASIS[r][k]).sum())
    });
    let eval = |p: [f32; 4], lane: usize| (0..4).map(|r| c[r][lane] * p[r]).sum::<f32>();
    let mut t = 0.5f32;
    for i in 0..10 {
        let sq = t * t;
        let x = eval([sq * t, sq, t, 1.0], 0);
        let y = eval([sq * t, sq, t, 1.0], 1);
        let dx = eval([sq * 3.0, t * 2.0, 1.0, 0.0], 0);
        let residual = input - x;
        if i == 9 || !(residual.abs() > f32::from_bits(0x3c23_d70a) && dx.abs() > 1.192_092_9e-7) {
            return y;
        }
        t += residual / dx;
    }
    0.0
}

/// What the shake does to the view this tick: rotations (radians) about its right, up and forward, and
/// a translation (m) along them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offset {
    pub rotation: [f32; 3],
    pub translation: [f32; 3],
}

impl Offset {
    pub fn is_zero(&self) -> bool {
        self.rotation == [0.0; 3] && self.translation == [0.0; 3]
    }
}

/// One shake (Skate 3's `ShakeEffect`; the gameplay camera shows the first of its two).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shake {
    /// Samples into the loop.
    pub time: f32,
    pub impulse: f32,
    /// Fades in and out (0..1).
    pub weight: f32,
    /// This tick's offset.
    pub offset: Offset,
}

impl Shake {
    /// A landing moving `vel` (m/s) into ground facing `normal` (`82E055D8`). Returns whether the
    /// impulse grew.
    pub fn landing(&mut self, normal: Vec3, vel: Vec3, t: &ShakeTuning) -> bool {
        let speed = vel.length();
        if f32::from_bits(0x3727_c5ac) > speed {
            return false;
        }
        let dir = vel * (1.0 / speed);
        let cos = (-normal).dot(dir).clamp(-1.0, 1.0);
        let angle = acosf(cos) * f32::from_bits(0x3f22_f983);
        let weighted = speed * (1.0 - angle.clamp(0.0, 1.0));
        if weighted <= 0.0 {
            return false;
        }
        // Skate 3's numbers have both speeds 0: any landing is a full share (x / 0 clamps to 1).
        let share = ((weighted - t.impulse_min_speed)
            / (t.impulse_max_speed - t.impulse_min_speed))
            .clamp(0.0, 1.0);
        let share = if share.is_nan() { 1.0 } else { share };
        let impulse = share * share * t.impulse_magnitude;
        if impulse > self.impulse {
            self.impulse = impulse;
            true
        } else {
            false
        }
    }

    /// One tick (`dt` s): `speed` the camera's over the ground (m/s), `distance` from its anchor (m),
    /// `enabled` while the rider isn't wiping out.
    pub fn update(
        &mut self,
        dt: f32,
        speed: f32,
        distance: f32,
        enabled: bool,
        samples: &Samples,
        t: &ShakeTuning,
    ) -> Offset {
        self.weight = if enabled {
            self.weight + dt * 5.0
        } else {
            self.weight - dt * 5.0
        }
        .clamp(0.0, 1.0);
        let along = |top: f32, curve_points: &[[f32; 4]; 4], lo: f32, hi: f32| {
            if top != 0.0 {
                curve(curve_points, (speed / top).clamp(0.0, 1.0)) * (hi - lo) + lo
            } else {
                lo
            }
        };
        let amplitude = along(
            t.amplitude_top_speed,
            &t.amplitude_curve,
            t.amplitude_min,
            t.amplitude_max,
        );
        let amplitude = (self.impulse + amplitude) * self.weight;
        let mut frequency = along(
            t.frequency_top_speed,
            &t.frequency_curve,
            t.frequency_min,
            t.frequency_max,
        );
        if self.impulse > 0.0 {
            frequency = t.impulse_frequency;
        }
        self.time += t.samples_per_second * frequency * dt;
        let n = samples.rotation.len().min(samples.translation.len());
        if n == 0 || !self.time.is_finite() {
            self.time = 0.0;
            self.offset = Offset::default();
            return self.offset;
        }
        // Keep the loop's time small (exactly, a whole loop at a time).
        if self.time >= n as f32 {
            self.time -= libm::floorf(self.time / n as f32) * n as f32;
        }
        let whole = libm::floorf(self.time);
        let frac = self.time - whole;
        let a = (whole as usize) % n;
        let b = (a + 1) % n;
        let near = ((f32::from_bits(0x4059_999a) - distance) * f32::from_bits(0x3fd5_5552))
            .clamp(0.0, 1.0);
        let mix = |s: &[[f32; 3]], i: usize| s[a][i] * (1.0 - frac) + s[b][i] * frac;
        let mut rotation: [f32; 3] =
            core::array::from_fn(|i| mix(&samples.rotation, i) * amplitude * near * t.rotation);
        rotation[2] *= t.dutch;
        let translation: [f32; 3] = core::array::from_fn(|i| {
            mix(&samples.translation, i) * t.translation * amplitude * near
        });
        self.impulse *= 1.0 - t.impulse_decay;
        if self.impulse < f32::from_bits(0x3ba3_d70a) {
            self.impulse = 0.0;
        }
        self.offset = Offset {
            rotation,
            translation,
        };
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    const T: ShakeTuning = ShakeTuning::BUILT_IN;

    #[test]
    fn the_curve_runs_from_the_least_to_the_most() {
        // Skate 3's amplitude curve: 0 at a standstill, 1 at the top speed, rising between.
        let c = &T.amplitude_curve;
        assert!(curve(c, 0.0).abs() < 0.02, "{}", curve(c, 0.0));
        assert!((curve(c, 1.0) - 1.0).abs() < 0.02, "{}", curve(c, 1.0));
        let mut last = -1.0;
        for i in 0..=10 {
            let y = curve(c, i as f32 / 10.0);
            assert!(y >= last - 0.02, "{i}: {y} after {last}");
            last = y;
        }
    }

    #[test]
    fn samples_are_centred_and_round_trip() {
        let s = Samples::built_in();
        assert_eq!(s.rotation.len(), 128);
        for k in 0..3 {
            let m: f32 = s.rotation.iter().map(|r| r[k]).sum::<f32>() / 128.0;
            assert!(m.abs() < 1e-6);
        }
        let back = Samples::parse(&s.encode()).unwrap();
        assert_eq!(back.rotation.len(), s.rotation.len());
        for (a, b) in back.translation.iter().zip(&s.translation) {
            for k in 0..3 {
                assert!((a[k] - b[k]).abs() < 1e-5);
            }
        }
        assert!(Samples::parse(b"FGSK").is_none());
        assert!(Samples::from_rows(&[]).is_none());
    }

    #[test]
    fn any_landing_gives_the_full_impulse_square_ones_most() {
        let mut s = Shake::default();
        assert!(s.landing(Vec3::UP, v3(0.0, -6.0, 4.0), &T));
        assert_eq!(s.impulse, 0.5);
        // Moving away from the ground: none.
        let mut g = Shake::default();
        assert!(!g.landing(Vec3::UP, v3(5.0, 0.5, 0.0), &T));
        assert!(!g.landing(Vec3::UP, v3(0.0, 2.0, 0.0), &T));
        assert_eq!(g.impulse, 0.0);
    }

    #[test]
    fn the_impulse_shakes_harder_then_decays_and_a_far_camera_is_still() {
        let samples = Samples::built_in();
        let size = |o: Offset| o.rotation.iter().map(|r| r.abs()).sum::<f32>();
        let mut calm = Shake::default();
        let mut hit = Shake {
            impulse: 0.5,
            ..Shake::default()
        };
        let (mut a, mut b) = (0.0, 0.0);
        for _ in 0..60 {
            a += size(calm.update(1.0 / 60.0, 1.0, 2.0, true, &samples, &T));
            b += size(hit.update(1.0 / 60.0, 1.0, 2.0, true, &samples, &T));
        }
        assert!(b > a * 1.5, "impulse {b} against {a}");
        for _ in 0..600 {
            hit.update(1.0 / 60.0, 1.0, 2.0, true, &samples, &T);
        }
        assert_eq!(hit.impulse, 0.0, "decayed");
        let mut far = Shake {
            impulse: 0.5,
            ..Shake::default()
        };
        for _ in 0..30 {
            assert!(
                far.update(1.0 / 60.0, 8.0, 3.5, true, &samples, &T)
                    .is_zero()
            );
        }
        // A wipeout fades it out at 5 a second.
        let mut w = Shake {
            weight: 1.0,
            ..Shake::default()
        };
        for _ in 0..13 {
            w.update(1.0 / 60.0, 8.0, 2.0, false, &samples, &T);
        }
        assert_eq!(w.weight, 0.0);
        assert!(w.offset.is_zero());
    }
}
