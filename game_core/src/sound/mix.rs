//! The software mixer the platforms share: up to [`VOICES`] voices of a [`Bank`]'s clips, decoded from
//! DSP-ADPCM (or PCM) as they play, resampled by pitch (linear), panned (equal power) and summed into
//! 32 kHz stereo PCM16. Gains glide to their targets (a few milliseconds for changes, the asked fade for
//! starts and stops), so nothing clicks.
//!
//! Cost: per voice and output sample one ADPCM sample decoded (at pitch 1), an interpolation and two
//! multiply-adds, in f32 and i32 (the Wii's CPU has hardware floating point): `tests::cost` measures it.

use alloc::vec;
use alloc::vec::Vec;

use super::bank::{Bank, Clip, Samples};
use super::{Command, RATE, adpcm};

/// Voices mixed at once; a new sound when all are busy takes the quietest.
pub const VOICES: usize = 24;
/// A change of gain or pan glides over this many samples (10 ms).
const GLIDE: u32 = RATE / 100;

/// Fixed point position fraction (16 bits).
const FRAC: u32 = 1 << 16;

#[derive(Clone, Debug)]
struct Voice {
    id: u16,
    clip: u16,
    /// Samples left before a one-shot ends (counted down as `cur` advances).
    left: u32,
    looped: bool,
    /// The sample under the read position and the one after it, and the position between (16.16).
    cur: i32,
    next: i32,
    frac: u32,
    step: u32,
    /// The decoder: the next sample to decode and the two decoded before it.
    dec: u32,
    h1: i32,
    h2: i32,
    /// Gains now, per-sample change and samples the change lasts.
    gl: f32,
    gr: f32,
    dl: f32,
    dr: f32,
    ramp: u32,
    /// Targets (for a later Set) and the voice is fading out to stop.
    tl: f32,
    tr: f32,
    stopping: bool,
}

/// Left and right gains for `gain` at `pan` (-1..1), equal power.
fn pan_gains(gain: f32, pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4;
    (gain * libm::cosf(a), gain * libm::sinf(a))
}

fn step_for(clip: &Clip, pitch: f32) -> u32 {
    let r = clip.rate as f32 * pitch.clamp(0.05, 4.0) / RATE as f32;
    (r * FRAC as f32) as u32
}

impl Voice {
    /// The next sample of the clip (looping as it says), decoding on.
    #[inline]
    fn fetch(&mut self, clip: &Clip) -> i32 {
        if self.dec >= clip.frames {
            match clip.loop_start {
                Some(l) => {
                    self.dec = l;
                    self.h1 = clip.loop_hist[0] as i32;
                    self.h2 = clip.loop_hist[1] as i32;
                }
                None => return 0,
            }
        }
        let i = self.dec as usize;
        self.dec += 1;
        let s = match &clip.samples {
            Samples::Pcm(p) => p.get(i).copied().unwrap_or(0) as i32,
            Samples::Adpcm { coefs, data } => {
                let f = i / adpcm::FRAME_SAMPLES * adpcm::FRAME_BYTES;
                match data.get(f..f + adpcm::FRAME_BYTES) {
                    Some(frame) => adpcm::decode_sample(
                        frame[0],
                        adpcm::nibble(frame, i % adpcm::FRAME_SAMPLES),
                        coefs,
                        self.h1,
                        self.h2,
                    ) as i32,
                    None => 0,
                }
            }
        };
        self.h2 = self.h1;
        self.h1 = s;
        s
    }

    fn glide(&mut self, l: f32, r: f32, samples: u32) {
        self.tl = l;
        self.tr = r;
        let n = samples.max(1);
        self.dl = (l - self.gl) / n as f32;
        self.dr = (r - self.gr) / n as f32;
        self.ramp = n;
    }

    fn loudness(&self) -> f32 {
        self.gl
            .abs()
            .max(self.gr.abs())
            .max(self.tl.abs().max(self.tr.abs()))
    }
}

/// The mixer. Commands go in once a frame ([`Mixer::apply`]), PCM comes out as the platform needs it
/// ([`Mixer::mix`]).
#[derive(Clone, Debug)]
pub struct Mixer {
    voices: Vec<Voice>,
    acc: Vec<f32>,
    /// Voices started and dropped since the start (for the log).
    pub started: u32,
    pub stolen: u32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    pub fn new() -> Mixer {
        Mixer {
            voices: Vec::with_capacity(VOICES),
            acc: Vec::new(),
            started: 0,
            stolen: 0,
        }
    }

    /// Voices playing.
    pub fn playing(&self) -> usize {
        self.voices.len()
    }

    /// Is `id` playing?
    pub fn has(&self, id: u16) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    pub fn apply(&mut self, bank: &Bank, commands: &[Command]) {
        for c in commands {
            match *c {
                Command::Play {
                    voice,
                    clip,
                    gain,
                    pan,
                    pitch,
                    fade,
                } => {
                    let Some(cl) = bank.clip(clip) else { continue };
                    self.voices.retain(|v| v.id != voice);
                    if self.voices.len() >= VOICES {
                        // The quietest goes.
                        if let Some((i, _)) = self.voices.iter().enumerate().min_by(|a, b| {
                            a.1.loudness()
                                .partial_cmp(&b.1.loudness())
                                .unwrap_or(core::cmp::Ordering::Equal)
                        }) {
                            self.voices.swap_remove(i);
                            self.stolen += 1;
                        }
                    }
                    let (l, r) = pan_gains(gain, pan);
                    let mut v = Voice {
                        id: voice,
                        clip,
                        left: cl.frames,
                        looped: cl.loop_start.is_some(),
                        cur: 0,
                        next: 0,
                        frac: 0,
                        step: step_for(cl, pitch),
                        dec: 0,
                        h1: cl.hist[0] as i32,
                        h2: cl.hist[1] as i32,
                        gl: 0.0,
                        gr: 0.0,
                        dl: 0.0,
                        dr: 0.0,
                        ramp: 0,
                        tl: l,
                        tr: r,
                        stopping: false,
                    };
                    v.cur = v.fetch(cl);
                    v.next = v.fetch(cl);
                    if fade > 0.0 {
                        v.glide(l, r, (fade * RATE as f32) as u32);
                    } else {
                        v.gl = l;
                        v.gr = r;
                    }
                    self.voices.push(v);
                    self.started += 1;
                }
                Command::Set {
                    voice,
                    gain,
                    pan,
                    pitch,
                } => {
                    if let Some(v) = self
                        .voices
                        .iter_mut()
                        .find(|v| v.id == voice && !v.stopping)
                    {
                        let (l, r) = pan_gains(gain, pan);
                        v.glide(l, r, GLIDE);
                        if let Some(cl) = bank.clip(v.clip) {
                            v.step = step_for(cl, pitch);
                        }
                    }
                }
                Command::Stop { voice, fade } => {
                    if let Some(v) = self.voices.iter_mut().find(|v| v.id == voice) {
                        v.stopping = true;
                        v.glide(0.0, 0.0, ((fade * RATE as f32) as u32).max(64));
                    }
                }
            }
        }
    }

    /// Mixes `out.len() / 2` stereo frames (interleaved left, right) at master volume `master`.
    pub fn mix(&mut self, bank: &Bank, out: &mut [i16], master: f32) {
        let frames = out.len() / 2;
        self.acc.clear();
        self.acc.resize(frames * 2, 0.0);
        let acc = &mut self.acc;
        self.voices.retain_mut(|v| {
            let Some(clip) = bank.clip(v.clip) else {
                return false;
            };
            for f in 0..frames {
                let t = v.frac as f32 * (1.0 / FRAC as f32);
                let s = v.cur as f32 + (v.next - v.cur) as f32 * t;
                acc[f * 2] += s * v.gl;
                acc[f * 2 + 1] += s * v.gr;
                if v.ramp > 0 {
                    v.gl += v.dl;
                    v.gr += v.dr;
                    v.ramp -= 1;
                    if v.ramp == 0 {
                        v.gl = v.tl;
                        v.gr = v.tr;
                        if v.stopping {
                            return false;
                        }
                    }
                }
                v.frac += v.step;
                while v.frac >= FRAC {
                    v.frac -= FRAC;
                    v.cur = v.next;
                    v.next = v.fetch(clip);
                    if !v.looped {
                        v.left = v.left.saturating_sub(1);
                        if v.left == 0 {
                            return false;
                        }
                    }
                }
            }
            true
        });
        let m = master.clamp(0.0, 1.0);
        for (o, a) in out.iter_mut().zip(acc.iter()) {
            *o = (a * m).clamp(-32768.0, 32767.0) as i16;
        }
        if out.len() % 2 == 1 {
            out[out.len() - 1] = 0;
        }
    }

    /// Everything stops at once (a new location, say).
    pub fn clear(&mut self) {
        self.voices.clear();
    }
}

/// Mixes `seconds` of the mixer into a new buffer (tests, the preview).
pub fn render(mixer: &mut Mixer, bank: &Bank, seconds: f32, master: f32) -> Vec<i16> {
    let mut out = vec![0i16; (seconds * RATE as f32) as usize * 2];
    mixer.mix(bank, &mut out, master);
    out
}

#[cfg(test)]
mod tests {
    use super::super::Cue;
    use super::*;

    fn tone_bank(looped: bool) -> Bank {
        let mut b = Bank::empty();
        let pcm: Vec<i16> = (0..3200)
            .map(|i| {
                (libm::sinf(i as f32 * core::f32::consts::TAU * 400.0 / 32000.0) * 10000.0) as i16
            })
            .collect();
        let enc = adpcm::encode(&pcm, [pcm[3199], pcm[3198]]);
        b.add(
            Cue::Pop,
            Clip {
                rate: RATE,
                frames: 3200,
                loop_start: looped.then_some(0),
                hist: [pcm[3199], pcm[3198]],
                loop_hist: [pcm[3199], pcm[3198]],
                peak: 10000,
                samples: Samples::Adpcm {
                    coefs: enc.coefs,
                    data: alloc::boxed::Box::leak(enc.data.into_boxed_slice()),
                },
            },
        );
        b
    }

    fn play(voice: u16, gain: f32, pan: f32, pitch: f32) -> Command {
        Command::Play {
            voice,
            clip: 0,
            gain,
            pan,
            pitch,
            fade: 0.0,
        }
    }

    fn peak(v: &[i16], ch: usize) -> i32 {
        v.iter()
            .skip(ch)
            .step_by(2)
            .map(|s| (*s as i32).abs())
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn a_one_shot_plays_once_at_its_level_and_pan() {
        let b = tone_bank(false);
        let mut m = Mixer::new();
        m.apply(&b, &[play(1, 1.0, 0.0, 1.0)]);
        let out = render(&mut m, &b, 0.05, 1.0);
        // Centre: each side at cos(45 deg).
        let p = peak(&out, 0);
        assert!((6500..7500).contains(&p), "{p}");
        assert!((peak(&out, 1) - p).abs() < 200);
        // 3200 samples = 0.1 s: still playing, then gone.
        assert!(m.has(1));
        let out = render(&mut m, &b, 0.1, 1.0);
        assert!(!m.has(1));
        assert_eq!(peak(&out[out.len() - 200..], 0), 0);
        // Hard right, half gain, half master.
        m.apply(&b, &[play(2, 0.5, 1.0, 1.0)]);
        let out = render(&mut m, &b, 0.05, 0.5);
        assert!(peak(&out, 0) < 50);
        let p = peak(&out, 1);
        assert!((2300..2700).contains(&p), "{p}");
    }

    #[test]
    fn loops_keep_going_and_stop_with_a_fade() {
        let b = tone_bank(true);
        let mut m = Mixer::new();
        m.apply(&b, &[play(7, 1.0, 0.0, 1.0)]);
        let out = render(&mut m, &b, 1.0, 1.0);
        assert!(m.has(7));
        assert!(peak(&out[out.len() - 2000..], 0) > 6000);
        m.apply(
            &b,
            &[Command::Stop {
                voice: 7,
                fade: 0.05,
            }],
        );
        let _ = render(&mut m, &b, 0.06, 1.0);
        assert!(!m.has(7));
    }

    #[test]
    fn pitch_changes_how_fast_it_plays() {
        let b = tone_bank(false);
        let mut m = Mixer::new();
        m.apply(&b, &[play(1, 1.0, 0.0, 2.0)]);
        let _ = render(&mut m, &b, 0.049, 1.0);
        assert!(m.has(1));
        let _ = render(&mut m, &b, 0.002, 1.0);
        assert!(!m.has(1), "twice as fast: over in 0.05 s");
        // A set glides the gain (no click): the first samples after it are near the old level.
        m.apply(&b, &[play(3, 1.0, 0.0, 1.0)]);
        let _ = render(&mut m, &b, 0.01, 1.0);
        m.apply(
            &b,
            &[Command::Set {
                voice: 3,
                gain: 0.0,
                pan: 0.0,
                pitch: 1.0,
            }],
        );
        let out = render(&mut m, &b, 0.02, 1.0);
        assert!(peak(&out[..20], 0) > 500 && peak(&out[out.len() - 100..], 0) < 30);
    }

    #[test]
    fn too_many_voices_drop_the_quietest() {
        let b = tone_bank(true);
        let mut m = Mixer::new();
        for i in 0..VOICES as u16 {
            m.apply(&b, &[play(i + 1, 0.5 + i as f32 * 0.01, 0.0, 1.0)]);
        }
        assert_eq!(m.playing(), VOICES);
        m.apply(&b, &[play(100, 1.0, 0.0, 1.0)]);
        assert_eq!(m.playing(), VOICES);
        assert!(!m.has(1) && m.has(100) && m.stolen == 1);
        // Clamped, not wrapped, when it all adds up.
        let out = render(&mut m, &b, 0.05, 1.0);
        assert!(out.iter().any(|s| *s == 32767 || *s == -32768));
    }

    /// The mixer's cost: 16 looping ADPCM voices at varied pitch for a second of output. On the host this
    /// prints the time (release builds: `cargo test --release -p game_core cost -- --nocapture`); the Wii
    /// logs its own (the once-a-second frame line's `audio`).
    #[test]
    fn cost() {
        let b = tone_bank(true);
        let mut m = Mixer::new();
        for i in 0..16u16 {
            m.apply(&b, &[play(i + 1, 0.05, 0.0, 0.8 + i as f32 * 0.03)]);
        }
        let start = std::time::Instant::now();
        let out = render(&mut m, &b, 1.0, 1.0);
        let us = start.elapsed().as_micros();
        assert_eq!(out.len(), RATE as usize * 2);
        assert_eq!(m.playing(), 16);
        std::println!("mixer: 16 voices, 1 s of 32 kHz stereo in {us} us");
    }
}
