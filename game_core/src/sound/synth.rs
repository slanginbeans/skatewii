//! The built-in placeholder sounds: flatground's own, synthesised at start from noise, filters and
//! decaying tones (nothing recorded). They stand in for every cue until the player imports Skate 3's
//! sounds from their disc, so the game always sounds like something: rolling is filtered noise that gets
//! brighter and louder with speed, the pop and landings are clicks over low thumps, grinds are noise with
//! metal partials or grit.

use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::TAU;

use super::bank::{Bank, Clip};
use super::{Cue, ROLL_BANDS};

/// The placeholders' sample rate (Hz): half the mix's, which halves their memory (the mixer resamples).
pub const SR: u32 = 16_000;

/// A small deterministic noise source (the same sounds every start).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.0 >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
    }
}

/// RBJ biquad (band-pass, constant peak gain) / low-pass.
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn band(hz: f32, q: f32) -> Biquad {
        let w = TAU * hz / SR as f32;
        let alpha = libm::sinf(w) / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b: [alpha / a0, 0.0, -alpha / a0],
            a: [-2.0 * libm::cosf(w) / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    fn low(hz: f32, q: f32) -> Biquad {
        let w = TAU * hz / SR as f32;
        let alpha = libm::sinf(w) / (2.0 * q);
        let c = libm::cosf(w);
        let a0 = 1.0 + alpha;
        Biquad {
            b: [(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0],
            a: [-2.0 * c / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

fn samples(seconds: f32) -> usize {
    (seconds * SR as f32) as usize
}

fn to_pcm(v: &[f32]) -> Vec<i16> {
    v.iter()
        .map(|x| (x * 32767.0).clamp(-32767.0, 32767.0) as i16)
        .collect()
}

/// Scales `v` so its peak is `peak` (0..1).
fn normalise(v: &mut [f32], peak: f32) {
    let m = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if m > 0.0 {
        for x in v {
            *x *= peak / m;
        }
    }
}

/// Makes a loop seamless: the last `fade` samples are blended into the first ones and dropped.
fn seamless(mut v: Vec<f32>, fade: usize) -> Vec<f32> {
    let n = v.len();
    let fade = fade.min(n / 2);
    for i in 0..fade {
        let w = i as f32 / fade as f32;
        v[i] = v[i] * w + v[n - fade + i] * (1.0 - w);
    }
    v.truncate(n - fade);
    v
}

/// A decaying tone (Hz, seconds to fall by 60 dB) from sample `at`.
fn ring(v: &mut [f32], at: usize, hz: f32, decay: f32, amp: f32) {
    let k = 6.9 / (decay * SR as f32);
    for (i, x) in v.iter_mut().enumerate().skip(at) {
        let t = (i - at) as f32;
        *x += amp * libm::expf(-k * t) * libm::sinf(TAU * hz * t / SR as f32);
    }
}

/// A burst of filtered noise from `at`: attack and decay in seconds.
#[allow(clippy::too_many_arguments)]
fn burst(
    v: &mut [f32],
    at: usize,
    noise: &mut Noise,
    filter: &mut Biquad,
    attack: f32,
    decay: f32,
    amp: f32,
) {
    let a = (attack * SR as f32).max(1.0);
    let k = 6.9 / (decay * SR as f32);
    for (i, x) in v.iter_mut().enumerate().skip(at) {
        let t = (i - at) as f32;
        let env = if t < a {
            t / a
        } else {
            libm::expf(-k * (t - a))
        };
        *x += amp * env * filter.run(noise.next());
    }
}

/// The rolling loop for speed band `band` (0 slow .. 5 fast): rumble and grit, brighter and louder with
/// speed.
fn roll_band(band: usize, noise: &mut Noise) -> Vec<i16> {
    let f = band as f32 / (ROLL_BANDS - 1) as f32;
    let n = samples(0.45);
    let mut low = Biquad::low(120.0 + 260.0 * f, 0.8);
    let mut mid = Biquad::band(500.0 + 1400.0 * f, 0.9);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            // The wheels' rumble, a little uneven, with the grit of the surface over it.
            let wob = 1.0 + 0.15 * libm::sinf(TAU * (7.0 + 9.0 * f) * i as f32 / SR as f32);
            low.run(noise.next()) * 1.6 * wob + mid.run(noise.next()) * (0.25 + 0.5 * f)
        })
        .collect();
    normalise(&mut v, 0.35 + 0.35 * f);
    to_pcm(&seamless(v, samples(0.05)))
}

/// A metal grind loop: hissing noise with a few inharmonic partials that wander.
fn grind_metal(noise: &mut Noise, base: f32) -> Vec<i16> {
    let n = samples(0.8);
    let mut hiss = Biquad::band(4200.0, 0.7);
    let partials = [1.0f32, 1.71, 2.43, 3.37];
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let mut s = hiss.run(noise.next()) * 0.6;
            for (k, p) in partials.iter().enumerate() {
                let drift = 1.0 + 0.004 * libm::sinf(TAU * (3.0 + k as f32) * t);
                s += libm::sinf(TAU * base * p * drift * t) * (0.25 / (k + 1) as f32);
            }
            s
        })
        .collect();
    normalise(&mut v, 0.5);
    to_pcm(&seamless(v, samples(0.08)))
}

/// A ledge grind loop: low gritty scraping with crackle.
fn grind_ledge(noise: &mut Noise) -> Vec<i16> {
    let n = samples(0.8);
    let mut body = Biquad::band(900.0, 0.6);
    let mut low = Biquad::low(300.0, 0.7);
    let mut v: Vec<f32> = (0..n)
        .map(|_| {
            let crackle = if noise.next() > 0.97 {
                noise.next() * 3.0
            } else {
                0.0
            };
            body.run(noise.next() + crackle) + low.run(noise.next()) * 0.8
        })
        .collect();
    normalise(&mut v, 0.5);
    to_pcm(&seamless(v, samples(0.08)))
}

/// A one-shot of `seconds`, built by `f`, peak `peak`.
fn shot(seconds: f32, peak: f32, f: impl FnOnce(&mut [f32])) -> Vec<i16> {
    let mut v = vec![0.0f32; samples(seconds)];
    f(&mut v);
    normalise(&mut v, peak);
    to_pcm(&v)
}

/// Fills `bank` with the placeholders: every cue gets clips.
pub fn fill(bank: &mut Bank) {
    let mut noise = Noise(0x5EED_F1A7);
    let pcm = |v: Vec<i16>, looped: bool| Clip::pcm(SR, v, looped);
    // Rolling: one set of bands for every surface.
    for band in 0..ROLL_BANDS {
        let c = pcm(roll_band(band, &mut noise), true);
        bank.add(Cue::RollAsphaltSmooth, c);
    }
    for cue in Cue::ALL.iter().copied().filter(|c| c.is_roll()) {
        if cue != Cue::RollAsphaltSmooth {
            bank.share(cue, Cue::RollAsphaltSmooth);
        }
    }
    // The pop: a sharp click on the tail with the deck's knock and a low thump.
    for (k, hz) in [880.0f32, 960.0, 1040.0].iter().enumerate() {
        let c = shot(0.18, 0.8, |v| {
            let mut click = Biquad::band(3000.0, 0.8);
            burst(v, 0, &mut noise, &mut click, 0.0005, 0.012, 1.0);
            ring(v, 0, *hz, 0.06, 0.6);
            ring(v, 0, 95.0 + 10.0 * k as f32, 0.09, 0.8);
        });
        bank.add(Cue::Pop, pcm(c, false));
    }
    // Landing: wheels slapping down and the deck's clack.
    for k in 0..3 {
        let c = shot(0.3, 0.8, |v| {
            let mut slap = Biquad::band(1800.0, 0.7);
            burst(v, 0, &mut noise, &mut slap, 0.001, 0.03, 0.7);
            ring(v, 0, 70.0 + 8.0 * k as f32, 0.18, 1.0);
            ring(v, samples(0.004), 700.0 + 60.0 * k as f32, 0.05, 0.4);
        });
        bank.add(Cue::Land, pcm(c, false));
    }
    for k in 0..2 {
        let c = shot(0.5, 0.9, |v| {
            let mut thud = Biquad::low(200.0, 0.7);
            burst(v, 0, &mut noise, &mut thud, 0.002, 0.15, 1.0);
            ring(v, 0, 48.0 + 6.0 * k as f32, 0.35, 1.0);
        });
        bank.add(Cue::LandHeavy, pcm(c, false));
    }
    // Catch and getting on: lighter knocks.
    for hz in [1150.0f32, 1300.0, 1450.0] {
        let c = shot(0.12, 0.6, |v| {
            let mut tap = Biquad::band(2500.0, 1.0);
            burst(v, 0, &mut noise, &mut tap, 0.0005, 0.01, 0.6);
            ring(v, 0, hz, 0.04, 0.8);
            ring(v, 0, 140.0, 0.05, 0.4);
        });
        bank.add(Cue::Catch, pcm(c, false));
    }
    for k in 0..2 {
        let c = shot(0.25, 0.7, |v| {
            ring(v, 0, 1000.0 + 100.0 * k as f32, 0.05, 0.5);
            ring(v, samples(0.06), 85.0, 0.12, 0.9);
            let mut tap = Biquad::band(2000.0, 1.0);
            burst(v, samples(0.06), &mut noise, &mut tap, 0.001, 0.02, 0.4);
        });
        bank.add(Cue::BoardOn, pcm(c, false));
    }
    // The board clattering down on its own: a few knocks.
    for k in 0..2 {
        let c = shot(0.45, 0.7, |v| {
            for (j, at) in [0.0f32, 0.07, 0.16].iter().enumerate() {
                let a = 1.0 / (j + 1) as f32;
                ring(
                    v,
                    samples(*at),
                    760.0 + 90.0 * (j + k) as f32,
                    0.06,
                    0.6 * a,
                );
                ring(v, samples(*at), 110.0, 0.08, 0.7 * a);
            }
        });
        bank.add(Cue::BoardDrop, pcm(c, false));
    }
    // Grinds and slides.
    let g = grind_metal(&mut noise, 1900.0);
    bank.add(Cue::GrindMetal, pcm(g, true));
    let g = grind_metal(&mut noise, 1500.0);
    bank.add(Cue::GrindMetal, pcm(g, true));
    let g = grind_ledge(&mut noise);
    bank.add(Cue::GrindLedge, pcm(g, true));
    for _ in 0..2 {
        let c = shot(0.25, 0.5, |v| {
            let mut s = Biquad::band(1200.0, 0.5);
            burst(v, 0, &mut noise, &mut s, 0.01, 0.2, 1.0);
        });
        bank.add(Cue::SlideScrape, pcm(c, false));
    }
    // Skids, the foot brake, steps.
    for hz in [2200.0f32, 2600.0, 3000.0] {
        let c = shot(0.2, 0.5, |v| {
            let mut s = Biquad::band(hz, 1.2);
            burst(v, 0, &mut noise, &mut s, 0.02, 0.15, 1.0);
        });
        bank.add(Cue::Skid, pcm(c, false));
    }
    for hz in [500.0f32, 650.0, 800.0] {
        let c = shot(0.2, 0.45, |v| {
            let mut s = Biquad::band(hz, 0.6);
            burst(v, 0, &mut noise, &mut s, 0.02, 0.16, 1.0);
        });
        bank.add(Cue::Brake, pcm(c, false));
    }
    for k in 0..4 {
        let c = shot(0.12, 0.45, |v| {
            ring(v, 0, 80.0 + 12.0 * k as f32, 0.05, 1.0);
            let mut s = Biquad::band(1500.0 + 200.0 * k as f32, 0.8);
            burst(v, samples(0.005), &mut noise, &mut s, 0.003, 0.04, 0.5);
        });
        bank.add(Cue::Step, pcm(c, false));
    }
    for k in 0..4 {
        let c = shot(0.1, 0.55, |v| {
            ring(v, 0, 90.0 + 10.0 * k as f32, 0.04, 1.0);
            let mut s = Biquad::band(1800.0 + 200.0 * k as f32, 0.8);
            burst(v, 0, &mut noise, &mut s, 0.002, 0.03, 0.7);
        });
        bank.add(Cue::RunStep, pcm(c, false));
    }
    // A bail: the body's thump and a slide.
    for k in 0..3 {
        let c = shot(0.45, 0.85, |v| {
            ring(v, 0, 60.0 + 10.0 * k as f32, 0.2, 1.0);
            let mut s = Biquad::band(700.0, 0.5);
            burst(v, samples(0.03), &mut noise, &mut s, 0.03, 0.3, 0.5);
        });
        bank.add(Cue::Bail, pcm(c, false));
    }
    // The board flipping: a short whoosh.
    for k in 0..2 {
        let c = shot(0.22, 0.4, |v| {
            let n = v.len();
            let mut f = Biquad::band(1600.0 + 300.0 * k as f32, 1.5);
            for (i, x) in v.iter_mut().enumerate() {
                let t = i as f32 / n as f32;
                *x = f.run(noise.next()) * libm::sinf(core::f32::consts::PI * t);
            }
        });
        bank.add(Cue::Flip, pcm(c, false));
    }
    // Wheels spinning in the air: a whirr fading out.
    let c = shot(1.0, 0.3, |v| {
        let mut f = Biquad::band(900.0, 2.0);
        let n = v.len();
        for (i, x) in v.iter_mut().enumerate() {
            let t = i as f32 / SR as f32;
            let am = 0.6 + 0.4 * libm::sinf(TAU * 23.0 * t);
            *x = f.run(noise.next()) * am * (1.0 - i as f32 / n as f32);
        }
    });
    bank.add(Cue::WheelSpin, pcm(c, false));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_has_a_placeholder_and_they_fit_in_memory() {
        let b = Bank::builtin();
        for c in Cue::ALL {
            assert!(!b.of(c).is_empty(), "{c:?}");
        }
        assert_eq!(b.of(Cue::RollWood).len(), ROLL_BANDS);
        // Loops loop, one-shots don't.
        assert!(b.clip(b.of(Cue::RollConcreteSmooth)[0]).unwrap().loop_start == Some(0));
        assert!(b.clip(b.of(Cue::GrindLedge)[0]).unwrap().loop_start == Some(0));
        assert!(b.clip(b.of(Cue::Pop)[0]).unwrap().loop_start.is_none());
        // Under half a megabyte of PCM.
        assert!(b.bytes() < 512 * 1024, "{} bytes", b.bytes());
        // Nothing clips, nothing is silent.
        for c in &b.clips {
            assert!(c.peak > 3000 && c.peak < 32767, "peak {}", c.peak);
        }
        // Faster rolling is louder.
        let peaks: Vec<u16> = b
            .of(Cue::RollAsphaltSmooth)
            .iter()
            .map(|i| b.clip(*i).unwrap().peak)
            .collect();
        assert!(peaks.windows(2).all(|w| w[1] > w[0]), "{peaks:?}");
        // The same every time.
        assert_eq!(Bank::builtin(), b);
    }
}
