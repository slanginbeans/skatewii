//! Skeletons and animation clips as stored in `.fgpak` entries (kinds `Skeleton` and `Animation`), read
//! in place. All integers and floats big-endian. Written on the PC by `tools/import`.
//!
//! Skeleton:
//! ```text
//! u16 bone count, u16 has trajectory (bone 0 is a ground-following root)
//! per bone: i16 parent (-1 = root), i16 mirror bone (-1 = none)
//! per bone: name, NAME_LEN bytes, NUL-padded ASCII
//! ```
//! Clip (`ANIM_HEADER` = 64-byte header, then `frames x bones` samples):
//! ```text
//! 0   u16 bones, u16 frames
//! 4   f32 frames per second
//! 8   u32 flags (FLAG_LOOP, FLAG_SCALE)
//! 12  f32 translation min xyz, f32 scale min xyz
//! 36  f32 translation step xyz, f32 scale step xyz      value = min + q * step
//! 60  u32 with FLAG_CHANNEL: offset of the per-bone channel weights (f32 each), else reserved
//! 64  samples: i16 rotation xyzw (Q15, unit quaternion, sign continuous over time),
//!     u16 translation xyz, and u16 scale xyz only if FLAG_SCALE
//! ```
//! With FLAG_PACKED (what `fg_import` writes now) the samples are smaller:
//! ```text
//! 64  per bone: u8 1 = its translation moves, 0 = it is constant; padded to 4 bytes
//!     per constant bone: u16 translation xyz; padded to 4 bytes
//!     samples, frame by frame, bone by bone: rotation as u16 x3 (the three smallest components of the
//!     unit quaternion, Q15 over +-1/sqrt 2; bit 15 of the first two holds which component was dropped,
//!     which is positive), then u16 translation xyz for moving bones, u16 scale xyz if FLAG_SCALE
//! ```

pub const NAME_LEN: usize = 24;
pub const ANIM_HEADER: usize = 64;
pub const FLAG_LOOP: u32 = 1;
pub const FLAG_SCALE: u32 = 2;
pub const FLAG_PACKED: u32 = 4;
/// A channel animation: it carries a weight per bone (how much of the bone a channel blend takes).
pub const FLAG_CHANNEL: u32 = 8;

/// Range of the three smallest components of a unit quaternion.
pub const SMALLEST_RANGE: f32 = core::f32::consts::FRAC_1_SQRT_2;

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn i16_at(b: &[u8], at: usize) -> i16 {
    u16_at(b, at) as i16
}
fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated;

pub struct Skeleton<'a> {
    data: &'a [u8],
    bones: usize,
}

impl<'a> Skeleton<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, Truncated> {
        let bones = u16_at(data.get(..4).ok_or(Truncated)?, 0) as usize;
        if data.len() < 4 + bones * (4 + NAME_LEN) {
            return Err(Truncated);
        }
        Ok(Skeleton { data, bones })
    }
    pub fn bones(&self) -> usize {
        self.bones
    }
    pub fn has_trajectory(&self) -> bool {
        u16_at(self.data, 2) != 0
    }
    pub fn parent(&self, bone: usize) -> i16 {
        i16_at(self.data, 4 + bone * 4)
    }
    pub fn mirror(&self, bone: usize) -> i16 {
        i16_at(self.data, 6 + bone * 4)
    }
    pub fn name(&self, bone: usize) -> &'a str {
        let at = 4 + self.bones * 4 + bone * NAME_LEN;
        let field = &self.data[at..at + NAME_LEN];
        let end = field.iter().position(|&c| c == 0).unwrap_or(NAME_LEN);
        core::str::from_utf8(&field[..end]).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

pub struct Clip<'a> {
    data: &'a [u8],
    bones: usize,
    frames: usize,
    /// Bytes per frame.
    stride: usize,
    /// FLAG_PACKED: per bone, its offset within a frame and where its constant translation is (0 = it
    /// moves).
    packed: Option<alloc::vec::Vec<(u16, u16)>>,
    samples: usize,
}

/// Unpacks a smallest-three rotation (see the module docs).
pub fn unpack_rotation(w: [u16; 3]) -> [f32; 4] {
    let dropped = (((w[0] >> 15) << 1) | (w[1] >> 15)) as usize;
    let c = |v: u16| (v & 0x7fff) as f32 * (2.0 * SMALLEST_RANGE / 32767.0) - SMALLEST_RANGE;
    let small = [c(w[0]), c(w[1]), c(w[2])];
    let big = crate::math::sqrtf((1.0 - small.iter().map(|v| v * v).sum::<f32>()).max(0.0));
    let mut q = [0.0; 4];
    let mut k = 0;
    for (i, out) in q.iter_mut().enumerate() {
        if i == dropped {
            *out = big;
        } else {
            *out = small[k];
            k += 1;
        }
    }
    q
}

/// Packs a unit quaternion as its three smallest components.
pub fn pack_rotation(q: [f32; 4]) -> [u16; 3] {
    let mut dropped = 0;
    for i in 1..4 {
        if q[i].abs() > q[dropped].abs() {
            dropped = i;
        }
    }
    let sign = if q[dropped] < 0.0 { -1.0 } else { 1.0 };
    let mut out = [0u16; 3];
    let mut k = 0;
    for (i, &v) in q.iter().enumerate() {
        if i != dropped {
            let x = (v * sign).clamp(-SMALLEST_RANGE, SMALLEST_RANGE);
            out[k] = ((x + SMALLEST_RANGE) / (2.0 * SMALLEST_RANGE) * 32767.0 + 0.5) as u16;
            k += 1;
        }
    }
    out[0] |= ((dropped as u16) >> 1) << 15;
    out[1] |= ((dropped as u16) & 1) << 15;
    out
}

impl<'a> Clip<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, Truncated> {
        let head = data.get(..ANIM_HEADER).ok_or(Truncated)?;
        let (bones, frames) = (u16_at(head, 0) as usize, u16_at(head, 2) as usize);
        let flags = u32::from_be_bytes([head[8], head[9], head[10], head[11]]);
        let scale = if flags & FLAG_SCALE != 0 { 6 } else { 0 };
        if flags & FLAG_PACKED == 0 {
            let stride = (14 + scale) * bones;
            if data.len() < ANIM_HEADER + frames * stride {
                return Err(Truncated);
            }
            return Ok(Clip {
                data,
                bones,
                frames,
                stride,
                packed: None,
                samples: ANIM_HEADER,
            });
        }
        let moving = data
            .get(ANIM_HEADER..ANIM_HEADER + bones)
            .ok_or(Truncated)?;
        let mut constants = (ANIM_HEADER + bones).next_multiple_of(4);
        let mut table = alloc::vec::Vec::with_capacity(bones);
        let mut offset = 0usize;
        for &m in moving {
            if m & 1 != 0 {
                table.push((offset as u16, 0u16));
                offset += 12 + scale;
            } else {
                table.push((offset as u16, constants as u16));
                constants += 6;
                offset += 6 + scale;
            }
        }
        let samples = constants.next_multiple_of(4);
        if samples > u16::MAX as usize || data.len() < samples + frames * offset {
            return Err(Truncated);
        }
        Ok(Clip {
            data,
            bones,
            frames,
            stride: offset,
            packed: Some(table),
            samples,
        })
    }
    pub fn bones(&self) -> usize {
        self.bones
    }
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn fps(&self) -> f32 {
        f32_at(self.data, 4)
    }
    fn flags(&self) -> u32 {
        u32::from_be_bytes([self.data[8], self.data[9], self.data[10], self.data[11]])
    }
    pub fn looping(&self) -> bool {
        self.flags() & FLAG_LOOP != 0
    }
    /// A channel animation's weights live in the clip.
    pub fn is_channel(&self) -> bool {
        self.flags() & FLAG_CHANNEL != 0 && self.channel_at().is_some()
    }
    fn channel_at(&self) -> Option<usize> {
        let at = u32::from_be_bytes([self.data[60], self.data[61], self.data[62], self.data[63]]);
        let at = at as usize;
        (at >= ANIM_HEADER && at + 4 * self.bones <= self.data.len()).then_some(at)
    }
    /// The bone's channel weight: 1 except in a channel animation.
    pub fn channel_weight(&self, bone: usize) -> f32 {
        match self.channel_at() {
            Some(at) if self.flags() & FLAG_CHANNEL != 0 && bone < self.bones => {
                f32_at(self.data, at + 4 * bone)
            }
            _ => 1.0,
        }
    }
    /// Dequantised sample of `bone` at `frame` (both must be in range).
    pub fn sample(&self, frame: usize, bone: usize) -> Sample {
        let d = self.data;
        let deq = |q: u16, i: usize| f32_at(d, 12 + 4 * i) + q as f32 * f32_at(d, 36 + 4 * i);
        let scaled = self.flags() & FLAG_SCALE != 0;
        let Some(table) = &self.packed else {
            let at = self.samples + frame * self.stride + bone * (self.stride / self.bones);
            let rotation = core::array::from_fn(|i| i16_at(d, at + 2 * i) as f32 / 32767.0);
            let translation = core::array::from_fn(|i| deq(u16_at(d, at + 8 + 2 * i), i));
            let scale = if scaled {
                core::array::from_fn(|i| deq(u16_at(d, at + 14 + 2 * i), 3 + i))
            } else {
                [1.0; 3]
            };
            return Sample {
                rotation,
                translation,
                scale,
            };
        };
        let (offset, constant) = table[bone];
        let at = self.samples + frame * self.stride + offset as usize;
        let rotation = unpack_rotation(core::array::from_fn(|i| u16_at(d, at + 2 * i)));
        let (t_at, s_at) = if constant == 0 {
            (at + 6, at + 12)
        } else {
            (constant as usize, at + 6)
        };
        let translation = core::array::from_fn(|i| deq(u16_at(d, t_at + 2 * i), i));
        let scale = if scaled {
            core::array::from_fn(|i| deq(u16_at(d, s_at + 2 * i), 3 + i))
        } else {
            [1.0; 3]
        };
        Sample {
            rotation,
            translation,
            scale,
        }
    }
}

/// Mirrors a whole pose left for right, as Skate 3 does for a mirrored animation tree (SK8-ENGINE
/// skate-core pose_mirror.rs, GPL-3.0): `partners` is the skeleton's mirror table (-1 = no partner, mirror
/// in place); paired bones swap. Mode 2 reflects every bone; mode 1 handles the root and its children
/// (the trajectory) by turning them round instead. Poses are local (parent-relative) samples.
pub fn mirror_pose(pose: &mut [Sample], parents: &[i16], partners: &[i16], mode: u32) {
    fn reflect(s: &mut Sample) {
        s.rotation[0] = -s.rotation[0];
        s.rotation[1] = -s.rotation[1];
        s.translation[2] = -s.translation[2];
    }
    let n = pose.len().min(parents.len()).min(partners.len());
    for i in 0..n {
        let partner = partners[i];
        if partner >= n as i16 || (partner >= 0 && partner < i as i16) {
            continue;
        }
        if mode == 1 && (i == 0 || parents[i] == 0) {
            if i != 0 {
                // Turn half round about y: q * (0, 1, 0, 0).
                let [x, y, z, w] = pose[i].rotation;
                pose[i].rotation = [-z, w, x, -y];
            }
            pose[i].rotation[1] = -pose[i].rotation[1];
            pose[i].rotation[2] = -pose[i].rotation[2];
            pose[i].translation[0] = -pose[i].translation[0];
        } else {
            reflect(&mut pose[i]);
        }
        if partner > i as i16 {
            pose.swap(i, partner as usize);
            reflect(&mut pose[i]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_data() {
        assert!(Skeleton::parse(&[0, 5, 0, 0]).is_err());
        assert!(Clip::parse(&[0; 10]).is_err());
        let mut h = [0u8; ANIM_HEADER];
        h[1] = 1;
        h[3] = 1; // 1 bone x 1 frame, no samples present
        assert!(Clip::parse(&h).is_err());
    }

    #[test]
    fn smallest_three_round_trips() {
        let n = |q: [f32; 4]| {
            let l = crate::math::sqrtf(q.iter().map(|v| v * v).sum());
            q.map(|v| v / l)
        };
        for q in [
            [0.0, 0.0, 0.0, 1.0],
            n([0.1, -0.7, 0.2, 0.3]),
            n([-0.9, 0.1, 0.1, -0.2]),
            n([0.5, 0.5, -0.5, 0.5]),
        ] {
            let r = unpack_rotation(pack_rotation(q));
            let dot: f32 = q.iter().zip(&r).map(|(a, b)| a * b).sum();
            assert!(dot.abs() > 0.99999, "{q:?} -> {r:?}");
        }
    }

    #[test]
    fn mirror_swaps_partners_and_reflects() {
        let s = |r: [f32; 4], t: [f32; 3]| Sample {
            rotation: r,
            translation: t,
            scale: [1.0; 3],
        };
        let mut pose = [
            s([0.1, 0.2, 0.3, 0.9], [1.0, 2.0, 3.0]),
            s([0.5, 0.0, 0.0, 0.8], [0.0, 0.0, 1.0]),
            s([0.0, 0.5, 0.0, 0.8], [0.0, 0.0, -2.0]),
        ];
        let parents = [-1, 0, 0];
        let partners = [-1, 2, 1];
        let before = pose;
        mirror_pose(&mut pose, &parents, &partners, 2);
        assert_eq!(pose[0].rotation, [-0.1, -0.2, 0.3, 0.9]);
        assert_eq!(pose[0].translation, [1.0, 2.0, -3.0]);
        // Bone 1 now holds bone 2's sample, reflected, and the other way round.
        assert_eq!(pose[1].rotation, [-0.0, -0.5, 0.0, 0.8]);
        assert_eq!(pose[1].translation[2], 2.0);
        assert_eq!(pose[2].rotation, [-0.5, -0.0, 0.0, 0.8]);
        // Twice is the identity.
        mirror_pose(&mut pose, &parents, &partners, 2);
        assert_eq!(pose, before);
    }
}
