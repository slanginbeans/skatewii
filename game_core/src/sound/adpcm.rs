//! GameCube / Wii DSP-ADPCM: 4 bits a sample, 14 samples in an 8-byte frame, 2-tap prediction from one of
//! eight coefficient pairs per sound. The Wii's audio DSP plays it natively; flatground's mixer decodes it
//! on the CPU (libogc's sound libraries mix PCM), which keeps sounds at a quarter of their PCM16 size.
//!
//! A frame is a header byte (bits 4..6: the coefficient pair, bits 0..3: the scale's shift) and 7 bytes of
//! nibbles, high nibble first. A sample is
//! `clamp(((nibble << shift) << 11) + 1024 + c0 * h1 + c1 * h2) >> 11)` with `h1`, `h2` the last two
//! decoded samples and the coefficients in 5.11 fixed point.
//!
//! The encoder is flatground's own: per-frame least-squares predictors clustered into the eight pairs, then
//! every frame tried with each pair and the shifts around the one its residual needs, keeping the one
//! that decodes closest (the decoder's own arithmetic, so what is measured is what plays).

use alloc::vec;
use alloc::vec::Vec;

/// Samples in a frame, and its size in bytes.
pub const FRAME_SAMPLES: usize = 14;
pub const FRAME_BYTES: usize = 8;

/// Bytes for `samples` samples (whole frames).
pub const fn bytes_for(samples: usize) -> usize {
    samples.div_ceil(FRAME_SAMPLES) * FRAME_BYTES
}

/// One sample from its frame's header byte, its nibble (0..15) and the two samples before it.
#[inline]
pub fn decode_sample(header: u8, nibble: u8, coefs: &[i16; 16], h1: i32, h2: i32) -> i16 {
    let pair = ((header >> 4) & 7) as usize;
    let shift = (header & 0xF) as u32;
    let n = ((nibble as i32) << 28) >> 28;
    let c0 = coefs[pair * 2] as i32;
    let c1 = coefs[pair * 2 + 1] as i32;
    let v = (((n << shift) << 11) + 1024 + c0 * h1 + c1 * h2) >> 11;
    v.clamp(-32768, 32767) as i16
}

/// The nibble of sample `k` (0..14) of the frame at `frame` (its 8 bytes).
#[inline]
pub fn nibble(frame: &[u8], k: usize) -> u8 {
    let b = frame[1 + k / 2];
    if k & 1 == 0 { b >> 4 } else { b & 0xF }
}

/// Decodes `samples` samples of `data` starting from history `hist` ([h1, h2]).
pub fn decode(data: &[u8], coefs: &[i16; 16], samples: usize, hist: [i16; 2]) -> Vec<i16> {
    let mut out = Vec::with_capacity(samples);
    let (mut h1, mut h2) = (hist[0] as i32, hist[1] as i32);
    for i in 0..samples {
        let f = i / FRAME_SAMPLES * FRAME_BYTES;
        let Some(frame) = data.get(f..f + FRAME_BYTES) else {
            break;
        };
        let s = decode_sample(frame[0], nibble(frame, i % FRAME_SAMPLES), coefs, h1, h2) as i32;
        h2 = h1;
        h1 = s;
        out.push(s as i16);
    }
    out
}

/// A sound in DSP-ADPCM.
#[derive(Clone, Debug, PartialEq)]
pub struct Encoded {
    pub coefs: [i16; 16],
    pub data: Vec<u8>,
}

/// Coefficients are 5.11 fixed point.
const ONE: f32 = 2048.0;

/// The least-squares 2-tap predictor of `x[n]` from `x[n-1]`, `x[n-2]` over a frame, with the frame's
/// energy (`prev`: the two samples before it, [x[-1], x[-2]]).
fn frame_predictor(frame: &[i16], prev: [i16; 2]) -> ([f32; 2], f32) {
    let (mut r11, mut r12, mut r22, mut r01, mut r02, mut e) = (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut p1, mut p2) = (prev[0] as f32, prev[1] as f32);
    for &s in frame {
        let x = s as f32;
        r11 += p1 * p1;
        r12 += p1 * p2;
        r22 += p2 * p2;
        r01 += x * p1;
        r02 += x * p2;
        e += x * x;
        p2 = p1;
        p1 = x;
    }
    // A little ridge keeps silent or degenerate frames well-defined.
    let ridge = 1.0 + (r11 + r22) * 1e-4;
    let (a11, a22) = (r11 + ridge, r22 + ridge);
    let det = a11 * a22 - r12 * r12;
    if det.abs() < 1e-6 {
        return ([0.0, 0.0], e);
    }
    let c0 = (r01 * a22 - r02 * r12) / det;
    let c1 = (r02 * a11 - r01 * r12) / det;
    ([c0, c1], e)
}

/// Keeps a predictor stable and inside the fixed-point range.
fn stable(c: [f32; 2]) -> [f32; 2] {
    let c1 = c[1].clamp(-0.98, 0.98);
    let lim = (1.0 - c1).min(1.98);
    [c[0].clamp(-lim, lim), c1]
}

/// The eight coefficient pairs for `pcm`: (0, 0) for transients, the rest clustered (k-means) from the
/// frames' own predictors, weighted by how loud the frame is.
fn design_coefs(pcm: &[i16], start: [i16; 2]) -> [i16; 16] {
    let mut preds: Vec<([f32; 2], f32)> = Vec::new();
    let mut prev = start;
    for frame in pcm.chunks(FRAME_SAMPLES) {
        let (c, e) = frame_predictor(frame, prev);
        if e > 0.0 {
            preds.push((stable(c), libm::sqrtf(e)));
        }
        let n = frame.len();
        prev = [frame[n - 1], if n > 1 { frame[n - 2] } else { prev[0] }];
    }
    // Starting points spread over the predictors typical of audio (from flat to strongly low-pass).
    let mut centres: [[f32; 2]; 8] = [
        [0.0, 0.0],
        [0.9, 0.0],
        [1.5, -0.6],
        [1.8, -0.85],
        [1.2, -0.3],
        [0.5, 0.2],
        [1.95, -0.97],
        [0.3, -0.1],
    ];
    for _ in 0..10 {
        let mut sum = [[0.0f32; 3]; 8];
        for (c, w) in &preds {
            let k = nearest(&centres, *c);
            sum[k][0] += c[0] * w;
            sum[k][1] += c[1] * w;
            sum[k][2] += w;
        }
        for k in 1..8 {
            if sum[k][2] > 0.0 {
                centres[k] = stable([sum[k][0] / sum[k][2], sum[k][1] / sum[k][2]]);
            }
        }
    }
    let mut coefs = [0i16; 16];
    for (k, c) in centres.iter().enumerate() {
        coefs[k * 2] = libm::roundf(c[0] * ONE).clamp(-32768.0, 32767.0) as i16;
        coefs[k * 2 + 1] = libm::roundf(c[1] * ONE).clamp(-32768.0, 32767.0) as i16;
    }
    coefs
}

fn nearest(centres: &[[f32; 2]; 8], c: [f32; 2]) -> usize {
    let mut best = (0, f32::MAX);
    for (k, m) in centres.iter().enumerate() {
        let d = (m[0] - c[0]) * (m[0] - c[0]) + (m[1] - c[1]) * (m[1] - c[1]);
        if d < best.1 {
            best = (k, d);
        }
    }
    best.0
}

/// Encodes one frame with pair `pair` and shift `shift` from history (h1, h2): its bytes, squared error and
/// the history after it.
fn try_frame(
    frame: &[i16],
    coefs: &[i16; 16],
    pair: usize,
    shift: u32,
    h: (i32, i32),
) -> ([u8; FRAME_BYTES], i64, (i32, i32)) {
    let mut out = [0u8; FRAME_BYTES];
    let header = ((pair as u8) << 4) | shift as u8;
    out[0] = header;
    let (c0, c1) = (coefs[pair * 2] as i32, coefs[pair * 2 + 1] as i32);
    let (mut h1, mut h2) = h;
    let mut err = 0i64;
    for k in 0..FRAME_SAMPLES {
        let x = frame.get(k).copied().unwrap_or(0) as i32;
        let pred = (1024 + c0 * h1 + c1 * h2) >> 11;
        let step = 1i32 << shift;
        let r = x - pred;
        // Round to the nearest step.
        let n = if r >= 0 {
            (r + step / 2) / step
        } else {
            -((-r + step / 2) / step)
        };
        let mut n = n.clamp(-8, 7);
        // The clamp in the decoder can make the rounded nibble worse: try its neighbour too.
        let mut best = (i64::MAX, 0i32, 0i32);
        for cand in [n, (n - 1).max(-8), (n + 1).min(7)] {
            let v = decode_sample(header, (cand & 0xF) as u8, coefs, h1, h2) as i32;
            let e = ((x - v) as i64) * ((x - v) as i64);
            if e < best.0 {
                best = (e, cand, v);
            }
        }
        n = best.1;
        if k < frame.len() {
            err += best.0;
        }
        out[1 + k / 2] |= if k & 1 == 0 {
            ((n & 0xF) as u8) << 4
        } else {
            (n & 0xF) as u8
        };
        h2 = h1;
        h1 = best.2;
    }
    (out, err, (h1, h2))
}

/// Encodes `pcm`, the decoder starting from history `hist` ([h1, h2]; a loop from the start uses the
/// sound's last two samples, so the encoding wraps round cleanly).
pub fn encode(pcm: &[i16], hist: [i16; 2]) -> Encoded {
    let coefs = design_coefs(pcm, hist);
    let mut data = vec![0u8; bytes_for(pcm.len())];
    let mut h = (hist[0] as i32, hist[1] as i32);
    for (f, frame) in pcm.chunks(FRAME_SAMPLES).enumerate() {
        let mut best: Option<([u8; FRAME_BYTES], i64, (i32, i32))> = None;
        for pair in 0..8 {
            let (c0, c1) = (coefs[pair * 2] as i32, coefs[pair * 2 + 1] as i32);
            // The shift the open-loop residual needs, then the ones either side of it.
            let (mut h1, mut h2) = h;
            let mut peak = 0i32;
            for &s in frame {
                let pred = (1024 + c0 * h1 + c1 * h2) >> 11;
                peak = peak.max((s as i32 - pred).abs());
                h2 = h1;
                h1 = s as i32;
            }
            let mut need = 0u32;
            while need < 12 && (7i32 << need) < peak {
                need += 1;
            }
            let lo = need.saturating_sub(1);
            let hi = (need + 1).min(12);
            for shift in lo..=hi {
                let t = try_frame(frame, &coefs, pair, shift.min(12), h);
                if best.as_ref().is_none_or(|b| t.1 < b.1) {
                    best = Some(t);
                }
            }
        }
        let (bytes, _, after) = best.unwrap_or(([0; FRAME_BYTES], 0, h));
        data[f * FRAME_BYTES..f * FRAME_BYTES + FRAME_BYTES].copy_from_slice(&bytes);
        h = after;
    }
    Encoded { coefs, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signal to noise ratio of `b` against `a` (dB).
    fn snr(a: &[i16], b: &[i16]) -> f32 {
        let (mut s, mut n) = (0.0f64, 0.0f64);
        for (x, y) in a.iter().zip(b) {
            s += (*x as f64) * (*x as f64);
            n += ((*x - *y) as f64) * ((*x - *y) as f64);
        }
        (10.0 * libm::log10(s / n.max(1.0))) as f32
    }

    fn tone(n: usize, hz: f32, amp: f32) -> Vec<i16> {
        (0..n)
            .map(|i| (libm::sinf(i as f32 * hz * core::f32::consts::TAU / 32000.0) * amp) as i16)
            .collect()
    }

    #[test]
    fn a_tone_round_trips_cleanly() {
        let pcm = tone(32000, 440.0, 20000.0);
        let e = encode(&pcm, [0, 0]);
        assert_eq!(e.data.len(), bytes_for(pcm.len()));
        assert_eq!(e.data.len(), 32000usize.div_ceil(14) * 8);
        let back = decode(&e.data, &e.coefs, pcm.len(), [0, 0]);
        assert_eq!(back.len(), pcm.len());
        let r = snr(&pcm, &back);
        assert!(r > 30.0, "tone SNR {r:.1} dB");
    }

    #[test]
    fn noise_chords_and_clicks_round_trip() {
        // A chord with a click and a burst of noise in it: transients and broadband sound.
        let mut seed = 1u32;
        let mut pcm: Vec<i16> = (0..16000)
            .map(|i| {
                let t = i as f32 / 32000.0;
                let c = libm::sinf(t * 220.0 * core::f32::consts::TAU) * 8000.0
                    + libm::sinf(t * 330.0 * core::f32::consts::TAU) * 6000.0
                    + libm::sinf(t * 1700.0 * core::f32::consts::TAU) * 3000.0;
                c as i16
            })
            .collect();
        pcm[4000] = 32000;
        pcm[4001] = -32000;
        for s in &mut pcm[9000..11000] {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *s = s.saturating_add(((seed >> 16) as i16) / 3);
        }
        let e = encode(&pcm, [0, 0]);
        let back = decode(&e.data, &e.coefs, pcm.len(), [0, 0]);
        let r = snr(&pcm, &back);
        assert!(r > 18.0, "mixed SNR {r:.1} dB");
        // Silence stays silent.
        let quiet = encode(&[0i16; 100], [0, 0]);
        assert!(
            decode(&quiet.data, &quiet.coefs, 100, [0, 0])
                .iter()
                .all(|s| *s == 0)
        );
    }

    #[test]
    fn the_decoder_matches_the_format() {
        // Pair 1 = (1.0, 0.0): each sample is the last plus nibble << shift.
        let mut coefs = [0i16; 16];
        coefs[2] = 2048;
        let frame = [0x12u8, 0x1F, 0x70, 0, 0, 0, 0, 0];
        let out = decode(&frame, &coefs, 4, [100, 0]);
        // nibbles 1, -1, 7, 0; shift 2.
        assert_eq!(out, [104, 100, 128, 128]);
        // Clamped at the range ends.
        assert_eq!(decode_sample(0x1C, 7, &coefs, 32000, 0), 32767);
    }

    #[test]
    fn a_loop_encoded_from_its_own_end_wraps_without_a_jump() {
        let pcm = tone(14 * 200, 400.0, 15000.0);
        let hist = [pcm[pcm.len() - 1], pcm[pcm.len() - 2]];
        let e = encode(&pcm, hist);
        let once = decode(&e.data, &e.coefs, pcm.len(), hist);
        // Decoding on from the end with the decoded end as history matches decoding from the start.
        let n = once.len();
        let again = decode(&e.data, &e.coefs, 50, [once[n - 1], once[n - 2]]);
        let first = &once[..50];
        let worst = first
            .iter()
            .zip(&again)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap();
        assert!(worst < 600, "loop seam off by {worst}");
    }
}
