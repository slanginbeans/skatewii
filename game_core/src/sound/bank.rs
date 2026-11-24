//! The sounds: clips by [`Cue`], from a sound pack (`fg_import`, the player's own disc) or the built-in
//! placeholders.
//!
//! A pack's clips are entries `sound/<cue>/<n>` (kind [`crate::pack::Kind::Sound`], `n` from 0 with no
//! gaps), all integers big-endian:
//!
//! ```text
//! 0x00 u32 sample rate (Hz)
//! 0x04 u32 samples
//! 0x08 u32 loop start (sample; 0xFFFFFFFF: plays once)
//! 0x0C u16 format (1: DSP-ADPCM, 0: PCM16)
//! 0x0E u16 reserved
//! 0x10 i16 coefficients[16] (ADPCM)
//! 0x30 i16 history at the start [h1, h2]
//! 0x34 i16 history at the loop start [h1, h2]
//! 0x38 u32 peak (largest |sample|)
//! 0x3C u32 reserved
//! 0x40 data: ADPCM frames (8 bytes / 14 samples), or PCM16
//! ```
//!
//! Clips are read in place (the pack stays loaded). A cue the pack lacks keeps its placeholder.

use alloc::vec::Vec;

use super::{Cue, adpcm};
use crate::pack::{Kind, Pack};

pub const HEADER: usize = 0x40;
pub const FORMAT_PCM: u16 = 0;
pub const FORMAT_ADPCM: u16 = 1;
pub const NO_LOOP: u32 = u32::MAX;

/// A clip's samples.
#[derive(Clone, Debug, PartialEq)]
pub enum Samples {
    /// DSP-ADPCM in place in a pack.
    Adpcm {
        coefs: [i16; 16],
        data: &'static [u8],
    },
    /// PCM16 (the placeholders).
    Pcm(Vec<i16>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub rate: u32,
    pub frames: u32,
    /// Where it loops back to (None: plays once).
    pub loop_start: Option<u32>,
    pub hist: [i16; 2],
    pub loop_hist: [i16; 2],
    pub peak: u16,
    pub samples: Samples,
}

impl Clip {
    pub fn pcm(rate: u32, pcm: Vec<i16>, looped: bool) -> Clip {
        let n = pcm.len();
        let peak = pcm
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0)
            .min(32767);
        let loop_hist = if looped && n >= 2 {
            [pcm[n - 1], pcm[n - 2]]
        } else {
            [0, 0]
        };
        Clip {
            rate,
            frames: n as u32,
            loop_start: looped.then_some(0),
            hist: [0, 0],
            loop_hist,
            peak,
            samples: Samples::Pcm(pcm),
        }
    }

    /// Seconds it lasts (once through).
    pub fn seconds(&self) -> f32 {
        self.frames as f32 / self.rate.max(1) as f32
    }

    /// Bytes of sample data it holds.
    pub fn bytes(&self) -> usize {
        match &self.samples {
            Samples::Adpcm { data, .. } => data.len(),
            Samples::Pcm(p) => p.len() * 2,
        }
    }

    /// Parses a pack entry's data (layout in the module docs).
    pub fn parse(d: &'static [u8]) -> Option<Clip> {
        let be32 = |at: usize| Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?));
        let be16 = |at: usize| Some(i16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?));
        let rate = be32(0)?;
        let frames = be32(4)?;
        let ls = be32(8)?;
        let format = be16(12)? as u16;
        if rate == 0 || rate > 96_000 || frames == 0 {
            return None;
        }
        let loop_start = (ls != NO_LOOP).then_some(ls);
        if loop_start.is_some_and(|l| l >= frames) {
            return None;
        }
        let mut coefs = [0i16; 16];
        for (i, c) in coefs.iter_mut().enumerate() {
            *c = be16(0x10 + i * 2)?;
        }
        let hist = [be16(0x30)?, be16(0x32)?];
        let loop_hist = [be16(0x34)?, be16(0x36)?];
        let peak = be32(0x38)?.min(32767) as u16;
        let body = d.get(HEADER..)?;
        let samples = match format {
            FORMAT_ADPCM => {
                let need = adpcm::bytes_for(frames as usize);
                Samples::Adpcm {
                    coefs,
                    data: body.get(..need)?,
                }
            }
            FORMAT_PCM => {
                let need = frames as usize * 2;
                let b = body.get(..need)?;
                Samples::Pcm(
                    b.chunks_exact(2)
                        .map(|c| i16::from_be_bytes([c[0], c[1]]))
                        .collect(),
                )
            }
            _ => return None,
        };
        Some(Clip {
            rate,
            frames,
            loop_start,
            hist,
            loop_hist,
            peak,
            samples,
        })
    }
}

/// Writes a clip entry's data (the importer; tests). `pcm` must be PCM16 when `format` is
/// [`FORMAT_PCM`]; `adpcm` is the encoded data for [`FORMAT_ADPCM`].
#[allow(clippy::too_many_arguments)]
pub fn write_clip(
    rate: u32,
    frames: u32,
    loop_start: Option<u32>,
    format: u16,
    coefs: &[i16; 16],
    hist: [i16; 2],
    loop_hist: [i16; 2],
    peak: u16,
    data: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + data.len());
    out.extend_from_slice(&rate.to_be_bytes());
    out.extend_from_slice(&frames.to_be_bytes());
    out.extend_from_slice(&loop_start.unwrap_or(NO_LOOP).to_be_bytes());
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    for c in coefs {
        out.extend_from_slice(&c.to_be_bytes());
    }
    for h in hist.iter().chain(&loop_hist) {
        out.extend_from_slice(&h.to_be_bytes());
    }
    out.extend_from_slice(&(peak as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(data);
    out
}

/// Where a cue's clips came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Placeholder,
    Imported,
}

/// Every clip, and which clips each cue plays (cues may share clips: the placeholders' rolling bands).
#[derive(Clone, Debug, PartialEq)]
pub struct Bank {
    pub clips: Vec<Clip>,
    cues: [Vec<u16>; Cue::COUNT],
    sources: [Source; Cue::COUNT],
}

impl Default for Bank {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Bank {
    /// No sounds at all.
    pub fn empty() -> Bank {
        Bank {
            clips: Vec::new(),
            cues: core::array::from_fn(|_| Vec::new()),
            sources: [Source::Placeholder; Cue::COUNT],
        }
    }

    /// The built-in placeholders (`synth`): every cue has something.
    pub fn builtin() -> Bank {
        let mut b = Bank::empty();
        super::synth::fill(&mut b);
        b
    }

    /// Adds a clip for `cue` (after its others); returns its index.
    pub fn add(&mut self, cue: Cue, clip: Clip) -> u16 {
        let i = self.clips.len() as u16;
        self.clips.push(clip);
        self.cues[cue as usize].push(i);
        i
    }

    /// Lets `cue` play the clips `other` already has.
    pub fn share(&mut self, cue: Cue, other: Cue) {
        self.cues[cue as usize] = self.cues[other as usize].clone();
    }

    /// The clip indices of `cue`.
    pub fn of(&self, cue: Cue) -> &[u16] {
        &self.cues[cue as usize]
    }

    pub fn clip(&self, i: u16) -> Option<&Clip> {
        self.clips.get(i as usize)
    }

    pub fn source(&self, cue: Cue) -> Source {
        self.sources[cue as usize]
    }

    /// Cues with imported sounds.
    pub fn imported(&self) -> usize {
        self.sources
            .iter()
            .filter(|s| **s == Source::Imported)
            .count()
    }

    /// Bytes of sample data held.
    pub fn bytes(&self) -> usize {
        self.clips.iter().map(Clip::bytes).sum()
    }

    /// Replaces the placeholders with every cue a sound pack has (`sound/<cue>/<n>`). Returns how many
    /// cues and clips it supplied; a malformed clip is an error, and nothing is changed then.
    pub fn load(&mut self, pack: &'static [u8]) -> Result<(usize, usize), crate::pack::Error> {
        let p = Pack::parse(pack)?;
        let mut found: Vec<(Cue, Vec<Clip>)> = Vec::new();
        for cue in Cue::ALL {
            let mut clips = Vec::new();
            loop {
                let name = alloc::format!("sound/{}/{}", cue.name(), clips.len());
                let Some(e) = p.find(&name) else { break };
                if e.kind != Kind::Sound as u32 {
                    return Err(crate::pack::Error::Truncated);
                }
                clips.push(Clip::parse(e.data).ok_or(crate::pack::Error::Truncated)?);
            }
            if !clips.is_empty() {
                found.push((cue, clips));
            }
        }
        let (mut cues, mut n) = (0, 0);
        for (cue, clips) in found {
            cues += 1;
            n += clips.len();
            self.cues[cue as usize].clear();
            self.sources[cue as usize] = Source::Imported;
            for c in clips {
                self.add(cue, c);
            }
        }
        // Placeholder clips no cue plays any more are dropped (and the indices renumbered).
        self.compact();
        Ok((cues, n))
    }

    fn compact(&mut self) {
        let mut used = alloc::vec![false; self.clips.len()];
        for c in &self.cues {
            for i in c {
                used[*i as usize] = true;
            }
        }
        let mut map = alloc::vec![u16::MAX; self.clips.len()];
        let mut kept = Vec::new();
        for (i, c) in core::mem::take(&mut self.clips).into_iter().enumerate() {
            if used[i] {
                map[i] = kept.len() as u16;
                kept.push(c);
            }
        }
        self.clips = kept;
        for c in &mut self.cues {
            for i in c.iter_mut() {
                *i = map[*i as usize];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use alloc::vec;

    fn leak(v: Vec<u8>) -> &'static [u8] {
        Box::leak(v.into_boxed_slice())
    }

    /// A pack in the importer's layout, written here without the importer (the game only reads).
    fn pack(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        use crate::pack::{DATA_ALIGN, ENTRY_SIZE, HEADER_SIZE, MAGIC, VERSION, name_hash};
        let mut strings = Vec::new();
        let mut offs = Vec::new();
        for (n, _) in entries {
            offs.push(strings.len() as u32);
            strings.extend_from_slice(n.as_bytes());
            strings.push(0);
        }
        let at = HEADER_SIZE + entries.len() * ENTRY_SIZE;
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        out.extend_from_slice(&(at as u32).to_be_bytes());
        out.resize(at, 0);
        out.extend_from_slice(&strings);
        for (i, (n, d)) in entries.iter().enumerate() {
            out.resize(out.len().next_multiple_of(DATA_ALIGN), 0);
            let off = out.len();
            out.extend_from_slice(d);
            let e = HEADER_SIZE + i * ENTRY_SIZE;
            out[e..e + 4].copy_from_slice(&name_hash(n).to_be_bytes());
            out[e + 4..e + 8].copy_from_slice(&offs[i].to_be_bytes());
            out[e + 8..e + 12].copy_from_slice(&(Kind::Sound as u32).to_be_bytes());
            out[e + 12..e + 16].copy_from_slice(&(off as u32).to_be_bytes());
            out[e + 16..e + 20].copy_from_slice(&(d.len() as u32).to_be_bytes());
        }
        out
    }

    #[test]
    fn a_pack_replaces_only_the_cues_it_has() {
        let tone: Vec<i16> = (0..700)
            .map(|i| (libm::sinf(i as f32 * 0.1) * 9000.0) as i16)
            .collect();
        let enc = adpcm::encode(&tone, [0, 0]);
        let clip = write_clip(
            32000,
            700,
            None,
            FORMAT_ADPCM,
            &enc.coefs,
            [0, 0],
            [0, 0],
            9000,
            &enc.data,
        );
        let looped = write_clip(
            32000,
            700,
            Some(14),
            FORMAT_ADPCM,
            &enc.coefs,
            [0, 0],
            [1, 2],
            9000,
            &enc.data,
        );
        let data = leak(pack(&[
            ("sound/pop/0", clip.clone()),
            ("sound/pop/1", clip),
            ("sound/grind_metal/0", looped),
        ]));
        let mut b = Bank::builtin();
        let before = b.of(Cue::Land).len();
        assert_eq!(b.load(data), Ok((2, 3)));
        assert_eq!(b.of(Cue::Pop).len(), 2);
        assert_eq!(b.source(Cue::Pop), Source::Imported);
        assert_eq!(b.source(Cue::Land), Source::Placeholder);
        assert_eq!(b.of(Cue::Land).len(), before);
        let g = b.clip(b.of(Cue::GrindMetal)[0]).unwrap();
        assert_eq!(
            (g.loop_start, g.loop_hist, g.peak),
            (Some(14), [1, 2], 9000)
        );
        assert!(matches!(g.samples, Samples::Adpcm { .. }));
        // Every cue still plays something, and every index is valid after the clean-up.
        for c in Cue::ALL {
            assert!(!b.of(c).is_empty(), "{c:?}");
            assert!(b.of(c).iter().all(|i| b.clip(*i).is_some()));
        }
        assert_eq!(b.imported(), 2);
    }

    #[test]
    fn bad_clips_are_rejected() {
        let bad = leak(pack(&[("sound/pop/0", vec![0u8; 10])]));
        let mut b = Bank::builtin();
        let copy = b.clone();
        assert!(b.load(bad).is_err());
        assert_eq!(b, copy);
        // A loop past the end, a short body.
        let c = write_clip(
            32000,
            10,
            Some(10),
            FORMAT_PCM,
            &[0; 16],
            [0; 2],
            [0; 2],
            0,
            &[0; 20],
        );
        assert!(Clip::parse(leak(c)).is_none());
        let c = write_clip(
            32000, 10, None, FORMAT_PCM, &[0; 16], [0; 2], [0; 2], 0, &[0; 19],
        );
        assert!(Clip::parse(leak(c)).is_none());
        let c = write_clip(
            32000, 10, None, FORMAT_PCM, &[0; 16], [0; 2], [0; 2], 0, &[0; 20],
        );
        assert_eq!(Clip::parse(leak(c)).unwrap().frames, 10);
    }
}
