//! Skate 3 animation banks (`.abin`, "Andale" banks) to Wii animation data.
//!
//! `abin` reads the bank records (clips, poses, the skeleton hierarchy); `vbr` decodes the VBR SQT codec
//! (8-frame DCT blocks). Both are ported from SK8-ENGINE/skate-3-rust-engine (GPL-3.0). This module joins
//! the per-part streams into whole-skeleton frames and quantises them into the layouts documented in
//! `game_core::anim`.

#[allow(dead_code)] // the ported reader exposes more of the format than the converter uses
pub mod abin;
pub mod vbr;

use abin::{Bank, Codec, Hierarchy, PartEntry, Record, RecordData};
use game_core::anim::{ANIM_HEADER, FLAG_LOOP, FLAG_SCALE, NAME_LEN};

/// One bone's sample: scale xyz, rotation quaternion xyzw, translation xyz.
pub type Sqt = [f32; 10];

pub struct ClipFrames {
    pub fps: f32,
    pub looping: bool,
    /// `frames[frame][bone]`
    pub frames: Vec<Vec<Sqt>>,
}

pub fn decode_clip(bank: &Bank, record: &Record) -> Result<ClipFrames, String> {
    let RecordData::Clip(clip) = &record.data else {
        return Err("not a clip".into());
    };
    if record.header.codec() != Codec::Vbr {
        return Err(format!(
            "codec {:?} not supported (VBR only)",
            record.header.codec()
        ));
    }
    let count = f32::from_bits(clip.frame_count_bits);
    if !count.is_finite() || count < 1.0 || count.fract() != 0.0 {
        return Err("invalid frame count".into());
    }
    let fps = f32::from_bits(clip.fps_bits);
    if !(fps.is_finite() && fps > 0.0) {
        return Err("invalid frame rate".into());
    }
    Ok(ClipFrames {
        fps,
        looping: clip.looping(),
        frames: decode_parts(bank, &clip.parts, count as usize)?,
    })
}

/// Name of the pose every clip is relative to (SK8: SkaterAnim::AddBindPose with `RIG_TPOSE`).
pub const BIND_POSE: &str = "RIG_TPOSE";

fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0] * b[3] + b[0] * a[3] + a[1] * b[2] - a[2] * b[1],
        a[1] * b[3] + b[1] * a[3] + a[2] * b[0] - a[0] * b[2],
        a[2] * b[3] + b[2] * a[3] + a[0] * b[1] - a[1] * b[0],
        a[3] * b[3] - (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]),
    ]
}

fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let c = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let u = [q[0], q[1], q[2]];
    let first = c(u, v);
    let mid = std::array::from_fn(|i| q[3] * v[i] + first[i]);
    let second = c(u, mid);
    std::array::from_fn(|i| v[i] + 2.0 * second[i])
}

/// Clips store motion relative to the bind pose. Composes `bind` onto every frame (SK8 pose_add::add with
/// the clip as motion): rotation = bind * clip, translation = bind.rotate(clip.t) + bind.t, scale = product.
/// The result is plain local transforms per bone, ready to play.
pub fn add_bind_pose(clip: &mut ClipFrames, bind: &[Sqt]) {
    for frame in &mut clip.frames {
        for (s, b) in frame.iter_mut().zip(bind) {
            let (bq, cq) = ([b[3], b[4], b[5], b[6]], [s[3], s[4], s[5], s[6]]);
            let q = quat_mul(bq, cq);
            let t = quat_rotate(bq, [s[7], s[8], s[9]]);
            *s = [
                b[0] * s[0],
                b[1] * s[1],
                b[2] * s[2],
                q[0],
                q[1],
                q[2],
                q[3],
                t[0] + b[7],
                t[1] + b[8],
                t[2] + b[9],
            ];
        }
    }
}

/// The decoded bind pose of a bank, if it has one.
pub fn bind_pose(bank: &Bank) -> Option<Vec<Sqt>> {
    let record = bank
        .records()
        .iter()
        .find(|r| r.header.name == BIND_POSE && matches!(r.data, RecordData::Pose(_)))?;
    decode_pose(bank, record).ok()?.frames.into_iter().next()
}

/// A pose record (a single frame other animations reference, e.g. for additive blends), as a 1-frame clip.
pub fn decode_pose(bank: &Bank, record: &Record) -> Result<ClipFrames, String> {
    let RecordData::Pose(pose) = &record.data else {
        return Err("not a pose".into());
    };
    if record.header.codec() != Codec::Vbr {
        return Err(format!(
            "codec {:?} not supported (VBR only)",
            record.header.codec()
        ));
    }
    Ok(ClipFrames {
        fps: 30.0,
        looping: false,
        frames: decode_parts(bank, &pose.parts, 1)?,
    })
}

/// Decodes every part of the hierarchy and places it at its bone range (SK8 animation_frames/native.rs).
fn decode_parts(bank: &Bank, parts: &[PartEntry], count: usize) -> Result<Vec<Vec<Sqt>>, String> {
    let hierarchy = bank.hierarchy().ok_or("bank has no hierarchy")?;
    let bones = hierarchy.bone_count as usize;
    if parts.len() != hierarchy.parts.len() {
        return Err("part count differs from the hierarchy".into());
    }
    let mut frames = vec![vec![[0f32; 10]; bones]; count];
    let mut assigned = vec![false; bones];
    for (entry, layout) in parts.iter().zip(&hierarchy.parts) {
        let part = entry.part.as_ref().ok_or("animation part is absent")?;
        let mut decoder = vbr::VbrDecoder::new(
            bank.bytes(),
            part.offset,
            part.compression_header_relative as usize,
            part.compressed_data_relative as usize,
        )?;
        if decoder.frame_count() != count || decoder.channel_count() != layout.bone_count as usize {
            return Err(format!(
                "{}: codec frame/channel count disagrees with the bank",
                layout.name
            ));
        }
        let start = usize::try_from(layout.sqt_offset).map_err(|_| "negative part offset")?;
        let end = start + layout.bone_count as usize;
        if end > bones || assigned[start..end].iter().any(|b| *b) {
            return Err("overlapping animation parts".into());
        }
        assigned[start..end].fill(true);
        for (index, frame) in frames.iter_mut().enumerate() {
            for (dst, words) in frame[start..end]
                .iter_mut()
                .zip(decoder.decode_frame(index)?)
            {
                *dst = words.map(f32::from_bits);
                if dst.iter().any(|v| !v.is_finite()) {
                    return Err("non-finite sample".into());
                }
            }
        }
    }
    if assigned.iter().any(|v| !*v) {
        return Err("hierarchy channels not fully covered".into());
    }
    Ok(frames)
}

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Skeleton entry (`game_core::anim::Skeleton`).
pub fn encode_skeleton(h: &Hierarchy) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&h.bone_count.to_be_bytes());
    out.extend_from_slice(&(h.has_trajectory as u16).to_be_bytes());
    for (p, m) in h.parents.iter().zip(&h.mirrors) {
        out.extend_from_slice(&(*p as i16).to_be_bytes());
        out.extend_from_slice(&(*m as i16).to_be_bytes());
    }
    for name in &h.bone_names {
        let mut field = [0u8; NAME_LEN];
        let n = name.len().min(NAME_LEN - 1);
        field[..n].copy_from_slice(&name.as_bytes()[..n]);
        out.extend_from_slice(&field);
    }
    out
}

/// (min, step) so that `min + q * step` covers `values` with q in 0..=65535.
fn range(values: impl Iterator<Item = f32>) -> (f32, f32) {
    let (lo, hi) = values.fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if lo > hi {
        (0.0, 0.0)
    } else {
        (lo, (hi - lo) / 65535.0)
    }
}

fn quantise(v: f32, (min, step): (f32, f32)) -> u16 {
    if step == 0.0 {
        0
    } else {
        ((v - min) / step).round().clamp(0.0, 65535.0) as u16
    }
}

/// Animation entry (`game_core::anim::Clip`). Rotations are normalised; quaternion sign is made
/// continuous along time so the game can lerp neighbouring frames.
pub fn encode_clip(c: &ClipFrames) -> Vec<u8> {
    let bones = c.frames.first().map_or(0, Vec::len);
    let samples = || c.frames.iter().flatten();
    let has_scale = samples().any(|s| s[..3].iter().any(|v| (v - 1.0).abs() > 1e-4));
    let t: [(f32, f32); 3] = std::array::from_fn(|i| range(samples().map(|s| s[7 + i])));
    let sc: [(f32, f32); 3] = std::array::from_fn(|i| {
        if has_scale {
            range(samples().map(|s| s[i]))
        } else {
            (1.0, 0.0)
        }
    });
    let mut out = Vec::with_capacity(ANIM_HEADER + c.frames.len() * bones * 20);
    out.extend_from_slice(&(bones as u16).to_be_bytes());
    out.extend_from_slice(&(c.frames.len() as u16).to_be_bytes());
    put_f32(&mut out, c.fps);
    let flags = if c.looping { FLAG_LOOP } else { 0 } | if has_scale { FLAG_SCALE } else { 0 };
    out.extend_from_slice(&flags.to_be_bytes());
    for r in t.iter().chain(&sc) {
        put_f32(&mut out, r.0);
    }
    for r in t.iter().chain(&sc) {
        put_f32(&mut out, r.1);
    }
    out.resize(ANIM_HEADER, 0);
    let mut previous: Vec<[f32; 4]> = vec![[0.0, 0.0, 0.0, 1.0]; bones];
    for frame in &c.frames {
        for (bone, s) in frame.iter().enumerate() {
            let mut q = [s[3], s[4], s[5], s[6]];
            let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
            q = if len > 1e-6 {
                q.map(|v| v / len)
            } else {
                [0.0, 0.0, 0.0, 1.0]
            };
            if q.iter()
                .zip(&previous[bone])
                .map(|(a, b)| a * b)
                .sum::<f32>()
                < 0.0
            {
                q = q.map(|v| -v);
            }
            previous[bone] = q;
            for v in q {
                out.extend_from_slice(&((v * 32767.0).round() as i16).to_be_bytes());
            }
            for i in 0..3 {
                out.extend_from_slice(&quantise(s[7 + i], t[i]).to_be_bytes());
            }
            if has_scale {
                for i in 0..3 {
                    out.extend_from_slice(&quantise(s[i], sc[i]).to_be_bytes());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_core::anim::{Clip, Skeleton};

    #[test]
    fn clip_round_trips_through_the_game_reader() {
        let a = 0.6f32;
        let frame = |k: f32| -> Vec<Sqt> {
            vec![
                [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, k, 0.0],
                // Second bone flips quaternion sign between frames; the encoder must make it continuous.
                [
                    1.0,
                    1.0,
                    1.0,
                    0.0,
                    a * k.signum(),
                    0.0,
                    0.8 * k.signum(),
                    1.0,
                    2.0,
                    -3.0 * k,
                ],
            ]
        };
        let c = ClipFrames {
            fps: 30.0,
            looping: true,
            frames: vec![frame(1.0), frame(-1.0)],
        };
        let bytes = encode_clip(&c);
        let clip = Clip::parse(&bytes).unwrap();
        assert_eq!(
            (clip.bones(), clip.frames(), clip.fps(), clip.looping()),
            (2, 2, 30.0, true)
        );
        let s = clip.sample(1, 1);
        assert!((s.translation[2] - 3.0).abs() < 1e-3);
        assert!((s.translation[1] - 2.0).abs() < 1e-3);
        assert_eq!(s.scale, [1.0; 3]);
        let s0 = clip.sample(0, 1);
        let dot: f32 = s0
            .rotation
            .iter()
            .zip(&s.rotation)
            .map(|(x, y)| x * y)
            .sum();
        assert!(dot > 0.99, "sign kept continuous: {dot}");
        assert!((clip.sample(1, 0).translation[1] + 1.0).abs() < 1e-3);
    }

    #[test]
    fn bind_pose_composes_like_add_sqt() {
        let h = (0.5f32).sqrt();
        // Bind: 90 degrees about Z, at (0, 0, 5). Clip: identity rotation, translation (1, 0, 0).
        let bind = [[1.0, 1.0, 1.0, 0.0, 0.0, h, h, 0.0, 0.0, 5.0]];
        let mut c = ClipFrames {
            fps: 30.0,
            looping: false,
            frames: vec![vec![[2.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]]],
        };
        add_bind_pose(&mut c, &bind);
        let s = c.frames[0][0];
        assert_eq!(s[0], 2.0);
        assert!(
            (s[5] - h).abs() < 1e-6 && (s[6] - h).abs() < 1e-6,
            "rotation is the bind's: {s:?}"
        );
        assert!(
            s[7].abs() < 1e-6 && (s[8] - 1.0).abs() < 1e-6 && (s[9] - 5.0).abs() < 1e-6,
            "translation rotated then offset: {s:?}"
        );
        // Order matters: bind * clip, not clip * bind.
        let q = quat_mul([0.0, 0.0, h, h], [h, 0.0, 0.0, h]);
        assert!(q.iter().all(|v| (v - 0.5).abs() < 1e-6), "{q:?}");
    }

    #[test]
    fn skeleton_round_trips() {
        let h = Hierarchy {
            offset: 0,
            bone_count: 2,
            has_trajectory: true,
            parents: vec![-1, 0],
            mirrors: vec![-1, 1],
            bone_names: vec!["ROOT".into(), "SPINE_1".into()],
            parts: vec![],
        };
        let bytes = encode_skeleton(&h);
        let s = Skeleton::parse(&bytes).unwrap();
        assert_eq!((s.bones(), s.has_trajectory()), (2, true));
        assert_eq!((s.parent(1), s.mirror(0), s.name(1)), (0, -1, "SPINE_1"));
    }
}
