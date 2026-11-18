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
//! 60  reserved
//! 64  samples: i16 rotation xyzw (Q15, unit quaternion, sign continuous over time),
//!     u16 translation xyz, and u16 scale xyz only if FLAG_SCALE
//! ```

pub const NAME_LEN: usize = 24;
pub const ANIM_HEADER: usize = 64;
pub const FLAG_LOOP: u32 = 1;
pub const FLAG_SCALE: u32 = 2;

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
    stride: usize,
}

impl<'a> Clip<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, Truncated> {
        let head = data.get(..ANIM_HEADER).ok_or(Truncated)?;
        let (bones, frames) = (u16_at(head, 0) as usize, u16_at(head, 2) as usize);
        let stride = if u32::from_be_bytes([head[8], head[9], head[10], head[11]]) & FLAG_SCALE != 0
        {
            20
        } else {
            14
        };
        if data.len() < ANIM_HEADER + bones * frames * stride {
            return Err(Truncated);
        }
        Ok(Clip {
            data,
            bones,
            frames,
            stride,
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
    /// Dequantised sample of `bone` at `frame` (both must be in range).
    pub fn sample(&self, frame: usize, bone: usize) -> Sample {
        let d = self.data;
        let at = ANIM_HEADER + (frame * self.bones + bone) * self.stride;
        let rotation = core::array::from_fn(|i| i16_at(d, at + 2 * i) as f32 / 32767.0);
        let deq = |q: u16, i: usize| f32_at(d, 12 + 4 * i) + q as f32 * f32_at(d, 36 + 4 * i);
        let translation = core::array::from_fn(|i| deq(u16_at(d, at + 8 + 2 * i), i));
        let scale = if self.stride == 20 {
            core::array::from_fn(|i| deq(u16_at(d, at + 14 + 2 * i), 3 + i))
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
}
