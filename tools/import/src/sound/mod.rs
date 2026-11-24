//! `fg_import sound`: Skate 3's sounds from the player's disc as `sound.fgpak` (`game_core::sound::bank`):
//! the clips the game's sound layer plays (`game_core::sound::Cue`), 32 kHz mono DSP-ADPCM, small enough to
//! stay loaded whole.
//!
//! Which recordings play for what follows SK8-ENGINE/skate-3-rust-engine's play-tested choices
//! (docs/hails-additions/11-audio.md: "Cues", "Retail patch records", "How it was tuned"): sample n of a
//! bank is its n-th stream. The rolling grains are slow-to-fast sweeps, so each is cut into
//! [`ROLL_BANDS`] loops (SK8's speed bands): a window from the middle of each sixth, cross-faded into a
//! loop.
//!
//! EA-XMA is decoded by FFmpeg ([`xma`]); everything else (containers, mixing down, loops, the ADPCM
//! encoding, the pack) is done here.

pub mod snr;
pub mod xma;

use std::io::{Read, Seek};
use std::path::PathBuf;

use game_core::pack::Kind;
use game_core::sound::bank::{FORMAT_ADPCM, write_clip};
use game_core::sound::{Cue, RATE, ROLL_BANDS, adpcm};

use crate::xdvdfs::Disc;
use crate::{Res, pack, read_asset};

const BANKS: &str = "data/audio/audiofiles.big::data/audio/";

/// What a cue's clips are cut from.
#[derive(Clone, Copy, Debug)]
enum Take {
    /// These streams (inclusive ranges) as one-shots.
    Shots(&'static [(usize, usize)]),
    /// These streams as loops (cross-faded end into start).
    Loops(&'static [(usize, usize)]),
    /// A grain's sweep cut into the rolling bands.
    Bands,
    /// The first `.0` seconds of stream 0, faded out over the last `.1`.
    Head(f32, f32),
}

struct Pick {
    cue: Cue,
    /// `<file.big>::<entry>` on the disc (BANKS + a bank name for the sample banks).
    asset: &'static str,
    take: Take,
}

const fn bank(cue: Cue, file: &'static str, take: Take) -> Pick {
    Pick {
        cue,
        asset: file,
        take,
    }
}

/// The sounds converted (SK8's picks: `Skate_Collisions` pop 1074-1078, landing 1088-1092, heavy
/// 1097-1100, board down 0-2, getting on 422-426, the deck knocks of a catch 530-535 and 548-552; `GRINDS`
/// 54-56 on metal (the most tonal), 9 and 11 on ledges (the least); `WHEEL_SKID_BANK` 0-7, `FOOT_DRAG`
/// 36-39 and 44-47, `fstep_skateshoe1_sm` 74-84 walking and pushing and 6-17 running, `Bodyslide` 0-14 in
/// three tiers of five, `Sk8_Air_Flip_Tricks` 1-3, `board_scrapes` 0-9, and the wheel spin of a jump).
const PICKS: &[Pick] = &[
    bank(
        Cue::Pop,
        "Skate_Collisions.bnk",
        Take::Shots(&[(1074, 1078)]),
    ),
    bank(
        Cue::Land,
        "Skate_Collisions.bnk",
        Take::Shots(&[(1088, 1092)]),
    ),
    bank(
        Cue::LandHeavy,
        "Skate_Collisions.bnk",
        Take::Shots(&[(1097, 1100)]),
    ),
    bank(
        Cue::Catch,
        "Skate_Collisions.bnk",
        Take::Shots(&[(530, 535), (548, 552)]),
    ),
    bank(
        Cue::BoardOn,
        "Skate_Collisions.bnk",
        Take::Shots(&[(422, 426)]),
    ),
    bank(
        Cue::BoardDrop,
        "Skate_Collisions.bnk",
        Take::Shots(&[(0, 2)]),
    ),
    bank(Cue::GrindMetal, "GRINDS.abk", Take::Loops(&[(54, 56)])),
    bank(
        Cue::GrindLedge,
        "GRINDS.abk",
        Take::Loops(&[(9, 9), (11, 11)]),
    ),
    bank(
        Cue::SlideScrape,
        "board_scrapes.abk",
        Take::Shots(&[(0, 9)]),
    ),
    bank(Cue::Skid, "WHEEL_SKID_BANK.abk", Take::Shots(&[(0, 7)])),
    bank(
        Cue::Brake,
        "FOOT_DRAG.abk",
        Take::Shots(&[(36, 39), (44, 47)]),
    ),
    bank(
        Cue::Step,
        "fstep_skateshoe1_sm.abk",
        Take::Shots(&[(74, 84)]),
    ),
    bank(
        Cue::RunStep,
        "fstep_skateshoe1_sm.abk",
        Take::Shots(&[(6, 17)]),
    ),
    bank(Cue::Bail, "Bodyslide.abk", Take::Shots(&[(0, 14)])),
    bank(Cue::Flip, "Sk8_Air_Flip_Tricks.abk", Take::Shots(&[(1, 3)])),
    Pick {
        cue: Cue::WheelSpin,
        asset: "data/audio/wheels.big::Whls_spins_Jump_1.snr",
        take: Take::Head(3.0, 0.4),
    },
    grain(Cue::RollAsphaltSmooth, "asphalt_smooth_hard"),
    grain(Cue::RollAsphaltRough, "asphalt_rough_hard"),
    grain(Cue::RollConcreteSmooth, "concrete_smooth_hard"),
    grain(Cue::RollConcreteRough, "concrete_rough_hard"),
    grain(Cue::RollConcreteAggregate, "concrete_aggregate_hard"),
    grain(Cue::RollWood, "wood_ramp_hard"),
    grain(Cue::RollMetal, "metal_smooth_hard"),
];

const fn grain(cue: Cue, _name: &'static str) -> Pick {
    Pick {
        cue,
        asset: match cue {
            Cue::RollAsphaltSmooth => "data/audio/grains.big::asphalt_smooth_hard.grain",
            Cue::RollAsphaltRough => "data/audio/grains.big::asphalt_rough_hard.grain",
            Cue::RollConcreteSmooth => "data/audio/grains.big::concrete_smooth_hard.grain",
            Cue::RollConcreteRough => "data/audio/grains.big::concrete_rough_hard.grain",
            Cue::RollConcreteAggregate => "data/audio/grains.big::concrete_aggregate_hard.grain",
            Cue::RollWood => "data/audio/grains.big::wood_ramp_hard.grain",
            _ => "data/audio/grains.big::metal_smooth_hard.grain",
        },
        take: Take::Bands,
    }
}

impl Pick {
    fn path(&self) -> String {
        if self.asset.contains("::") {
            self.asset.to_string()
        } else {
            format!("{BANKS}{}", self.asset)
        }
    }
}

/// Each rolling band's loop (s) and every loop's cross-fade (s).
const BAND_SECONDS: f32 = 1.5;
const LOOP_FADE: f32 = 0.08;

// ---- PCM ----

/// Resamples `x` from `from` Hz to `to` Hz (windowed sinc, low-passed below the lower Nyquist).
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let step = from as f64 / to as f64;
    let cut = (to as f64 / from as f64).min(1.0) * 0.92;
    let half = (12.0 / cut).ceil() as i64;
    let n = ((x.len() as f64) / step).floor() as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 * step;
            let c = t.floor() as i64;
            let mut acc = 0.0f64;
            for k in (c - half + 1)..=(c + half) {
                if k < 0 || k as usize >= x.len() {
                    continue;
                }
                let d = t - k as f64;
                let sinc = if d.abs() < 1e-9 {
                    cut
                } else {
                    (std::f64::consts::PI * d * cut).sin() / (std::f64::consts::PI * d)
                };
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * d / half as f64).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * d / half as f64).cos();
                acc += x[k as usize] as f64 * sinc * w.max(0.0);
            }
            // The sinc's sum over whole samples is 1 (unity gain at DC) at any cut-off.
            acc as f32
        })
        .collect()
}

/// Interleaved `channels` mixed down to mono (the average).
pub fn mono(x: &[i16], channels: usize) -> Vec<f32> {
    let c = channels.max(1);
    x.chunks_exact(c)
        .map(|f| f.iter().map(|s| *s as f32).sum::<f32>() / c as f32)
        .collect()
}

/// Drops a one-shot's leading silence (up to 30 ms, keeping 1 ms before the first sound) and fades its
/// last 2 ms out (no click where it stops).
pub fn tidy_shot(mut x: Vec<f32>) -> Vec<f32> {
    let lead = x
        .iter()
        .take((RATE as usize * 3) / 100)
        .position(|s| s.abs() > 64.0)
        .unwrap_or(0)
        .saturating_sub(RATE as usize / 1000);
    x.drain(..lead);
    let fade = (RATE as usize / 500).min(x.len());
    let n = x.len();
    for i in 0..fade {
        x[n - fade + i] *= 1.0 - (i + 1) as f32 / fade as f32;
    }
    x
}

/// Makes `x` a seamless loop: its last `fade` seconds blended into its start (equal weights summing to 1,
/// so it can't get louder), then dropped.
pub fn make_loop(mut x: Vec<f32>, fade: f32) -> Vec<f32> {
    let n = x.len();
    let f = ((fade * RATE as f32) as usize).min(n / 3);
    for i in 0..f {
        let w = i as f32 / f as f32;
        x[i] = x[i] * w + x[n - f + i] * (1.0 - w);
    }
    x.truncate(n - f);
    x
}

/// The rolling bands of a grain's sweep (already at the game's rate): band k is a loop cut from the middle
/// of the k-th of [`ROLL_BANDS`] equal parts.
pub fn bands(sweep: &[f32]) -> Vec<Vec<f32>> {
    let part = sweep.len() / ROLL_BANDS;
    let want = ((BAND_SECONDS + LOOP_FADE) * RATE as f32) as usize;
    (0..ROLL_BANDS)
        .map(|k| {
            let len = want.min(part);
            let start = k * part + (part - len) / 2;
            make_loop(sweep[start..start + len].to_vec(), LOOP_FADE)
        })
        .collect()
}

/// A clip entry: `pcm` (at [`RATE`]) encoded, looping from its start when `looped`.
pub fn encode_clip(pcm: &[f32], looped: bool) -> Vec<u8> {
    let s: Vec<i16> = pcm
        .iter()
        .map(|v| v.round().clamp(-32768.0, 32767.0) as i16)
        .collect();
    let n = s.len();
    let hist = if looped && n >= 2 {
        [s[n - 1], s[n - 2]]
    } else {
        [0, 0]
    };
    let e = adpcm::encode(&s, hist);
    let dec = adpcm::decode(&e.data, &e.coefs, n, hist);
    let loop_hist = if looped && n >= 2 {
        [dec[n - 1], dec[n - 2]]
    } else {
        [0, 0]
    };
    let peak = dec
        .iter()
        .map(|v| v.unsigned_abs())
        .max()
        .unwrap_or(0)
        .min(32767);
    write_clip(
        RATE,
        n as u32,
        looped.then_some(0),
        FORMAT_ADPCM,
        &e.coefs,
        hist,
        loop_hist,
        peak,
        &e.data,
    )
}

// ---- Decoding ----

/// How streams are decoded: EA-XMA through FFmpeg; PCM16 here.
pub struct Decoder {
    pub ffmpeg: Option<PathBuf>,
}

impl Decoder {
    /// Stream `s` of container `d` as mono at the game's rate.
    pub fn decode(&self, d: &[u8], s: &snr::Stream) -> Result<Vec<f32>, String> {
        match s.codec {
            snr::CODEC_PCM16BE => {
                let m = mono(&snr::pcm16(d, s), s.channels as usize);
                Ok(resample(&m, s.rate, RATE))
            }
            snr::CODEC_EAXMA => {
                let ff = self
                    .ffmpeg
                    .as_deref()
                    .ok_or("EA-XMA needs FFmpeg (--ffmpeg or FG_FFMPEG)")?;
                let packets = snr::xma_packets(d, s)?;
                if packets.len() != 1 {
                    return Err(format!("{} channels (only mono and stereo)", s.channels));
                }
                let wav = xma::riff(&packets[0], s.channels as u16, s.rate, s.samples);
                let pcm = xma::decode(ff, &wav, RATE)?;
                if pcm.is_empty() {
                    return Err("FFmpeg decoded nothing".into());
                }
                Ok(pcm.into_iter().map(|v| v as f32).collect())
            }
            c => Err(format!("codec {c} not supported")),
        }
    }
}

/// The streams a pick takes, in order.
fn wanted(take: Take, count: usize) -> Vec<usize> {
    match take {
        Take::Shots(r) | Take::Loops(r) => r
            .iter()
            .flat_map(|(a, b)| *a..=*b)
            .filter(|i| *i < count)
            .collect(),
        Take::Bands | Take::Head(..) => vec![0],
    }
}

/// One pick's clips from its container's bytes.
fn convert_pick(
    dec: &Decoder,
    take: Take,
    data: &[u8],
    codecs: &[u8],
) -> Result<Vec<Vec<u8>>, String> {
    let streams = snr::streams(data, codecs)?;
    let idx = wanted(take, streams.len());
    if idx.is_empty() {
        return Err(format!(
            "none of its streams are there ({} streams)",
            streams.len()
        ));
    }
    // Decoded in parallel: FFmpeg runs once per stream.
    let decoded: Vec<Result<Vec<f32>, String>> = std::thread::scope(|sc| {
        let hs: Vec<_> = idx
            .iter()
            .map(|&i| {
                let s = &streams[i];
                sc.spawn(move || dec.decode(data, s))
            })
            .collect();
        hs.into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("decoder thread failed".into()))
            })
            .collect()
    });
    let mut out = Vec::new();
    for pcm in decoded {
        let pcm = pcm?;
        match take {
            Take::Shots(_) => out.push(encode_clip(&tidy_shot(pcm), false)),
            Take::Loops(_) => out.push(encode_clip(&make_loop(pcm, LOOP_FADE), true)),
            Take::Bands => {
                for b in bands(&pcm) {
                    out.push(encode_clip(&b, true));
                }
            }
            Take::Head(secs, fade) => {
                let mut x = pcm;
                x.truncate((secs * RATE as f32) as usize);
                let f = ((fade * RATE as f32) as usize).min(x.len());
                let n = x.len();
                for i in 0..f {
                    x[n - f + i] *= 1.0 - (i + 1) as f32 / f as f32;
                }
                out.push(encode_clip(&x, false));
            }
        }
    }
    Ok(out)
}

/// The pack's items for every pick, reading each container once. A pick that fails is reported and left
/// out (its cue keeps the game's placeholder).
fn convert_all<F>(dec: &Decoder, picks: &[Pick], codecs: &[u8], mut read: F) -> Vec<pack::Item>
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    let mut cache: Vec<(String, Vec<u8>)> = Vec::new();
    let mut items = Vec::new();
    for p in picks {
        let path = p.path();
        if !cache.iter().any(|c| c.0 == path) {
            match read(&path) {
                Ok(d) => cache.push((path.clone(), d)),
                Err(e) => {
                    eprintln!("sound: {}: {path}: {e}", p.cue.name());
                    continue;
                }
            }
        }
        let data = &cache.iter().find(|c| c.0 == path).expect("cached").1;
        match convert_pick(dec, p.take, data, codecs) {
            Ok(clips) => {
                for (n, c) in clips.into_iter().enumerate() {
                    items.push(pack::Item {
                        name: format!("sound/{}/{n}", p.cue.name()),
                        kind: Kind::Sound,
                        data: c,
                        width: 0,
                        height: 0,
                        gx_format: 0,
                    });
                }
            }
            Err(e) => eprintln!("sound: {}: {path}: {e} (placeholder kept)", p.cue.name()),
        }
    }
    items
}

/// What a sound pack holds, for the report: cues, clips, seconds, sample bytes.
pub fn summary(items: &[pack::Item]) -> (usize, usize, f32, usize) {
    let mut cues: Vec<&str> = items
        .iter()
        .filter_map(|i| i.name.split('/').nth(1))
        .collect();
    cues.dedup();
    let secs: f32 = items
        .iter()
        .map(|i| u32::from_be_bytes(i.data[4..8].try_into().unwrap()) as f32 / RATE as f32)
        .sum();
    let bytes = items.iter().map(|i| i.data.len()).sum();
    (cues.len(), items.len(), secs, bytes)
}

pub const USAGE: &str = "usage:
  fg_import sound <disc.iso> [--out <dir>] [--ffmpeg <path>]   Skate 3's sounds as sound.fgpak
  fg_import sound <disc.iso> --list <asset>                     list the sound streams of an asset";

pub fn run<R: Read + Seek>(disc: &mut Disc<R>, args: &[String]) -> Res<()> {
    let mut out = PathBuf::from("imported");
    let mut ffmpeg: Option<PathBuf> = None;
    let mut list: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(USAGE);
        match a.as_str() {
            "--out" => out = PathBuf::from(val()?),
            "--ffmpeg" => ffmpeg = Some(PathBuf::from(val()?)),
            "--list" => list = Some(val()?),
            _ => return Err(format!("unknown option {a}\n{USAGE}").into()),
        }
    }
    let codecs = [snr::CODEC_EAXMA, snr::CODEC_PCM16BE];
    if let Some(asset) = list {
        let d = read_asset(disc, &asset)?;
        for (i, s) in snr::streams(&d, &codecs)?.iter().enumerate() {
            println!(
                "{i:>5} at {:#010x} codec {} {} ch {} Hz {:.2} s{}",
                s.offset,
                s.codec,
                s.channels,
                s.rate,
                s.seconds(),
                s.loop_start
                    .map_or(String::new(), |l| format!(" loop from {l}"))
            );
        }
        return Ok(());
    }
    let ffmpeg = xma::find_ffmpeg(ffmpeg.as_deref()).ok_or(
        "no FFmpeg with an XMA2 decoder (install FFmpeg, or pass --ffmpeg / set FG_FFMPEG)",
    )?;
    let dec = Decoder {
        ffmpeg: Some(ffmpeg.clone()),
    };
    let t0 = std::time::Instant::now();
    let items = convert_all(&dec, PICKS, &codecs, |p| {
        read_asset(disc, p).map_err(|e| e.to_string())
    });
    if items.is_empty() {
        return Err("no sounds converted".into());
    }
    let (cues, clips, secs, bytes) = summary(&items);
    std::fs::create_dir_all(&out)?;
    let path = out.join("sound.fgpak");
    let packed = pack::write(&items);
    std::fs::write(&path, &packed)?;
    println!(
        "sound: {cues} of {} cues, {clips} clips, {secs:.1} s of 32 kHz mono DSP-ADPCM ({} KiB of samples), {} KiB pack, decoded with {} in {:.1} s -> {}",
        Cue::COUNT,
        bytes / 1024,
        packed.len() / 1024,
        ffmpeg.display(),
        t0.elapsed().as_secs_f32(),
        path.display()
    );
    Ok(())
}

/// Can the build convert sounds (an FFmpeg with XMA2)? Says why not when it can't.
pub fn decoder_available() -> bool {
    xma::find_ffmpeg(None).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_core::sound::bank::{Bank, Source};
    use game_core::sound::{Command, Mixer, mix};

    /// A tone at `hz`, `secs` long at `rate`, peak `amp`.
    fn tone(hz: f32, secs: f32, rate: u32, amp: f32) -> Vec<i16> {
        (0..(secs * rate as f32) as usize)
            .map(|i| ((i as f32 * hz * std::f32::consts::TAU / rate as f32).sin() * amp) as i16)
            .collect()
    }

    fn pcm_stream(samples: &[i16], rate: u32, channels: u8) -> Vec<u8> {
        // Blocks of 1000 frames, as the stream format allows.
        let per = 1000 * channels as usize;
        let blocks: Vec<(Vec<u8>, u32)> = samples
            .chunks(per)
            .map(|c| {
                (
                    c.iter().flat_map(|s| s.to_be_bytes()).collect(),
                    (c.len() / channels as usize) as u32,
                )
            })
            .collect();
        snr::write(snr::CODEC_PCM16BE, channels, rate, None, &blocks)
    }

    /// The strongest frequency of `x` among `candidates` (Hz).
    fn loudest(x: &[f32], rate: u32, candidates: &[f32]) -> f32 {
        let power = |hz: f32| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, v) in x.iter().enumerate() {
                let a = i as f64 * hz as f64 * std::f64::consts::TAU / rate as f64;
                re += *v as f64 * a.cos();
                im += *v as f64 * a.sin();
            }
            re * re + im * im
        };
        *candidates
            .iter()
            .max_by(|a, b| power(**a).partial_cmp(&power(**b)).unwrap())
            .unwrap()
    }

    #[test]
    fn resampling_keeps_the_tone_and_the_level() {
        let x: Vec<f32> = tone(1000.0, 0.5, 48000, 10000.0)
            .into_iter()
            .map(|v| v as f32)
            .collect();
        let y = resample(&x, 48000, 32000);
        assert_eq!(y.len(), 16000);
        assert_eq!(
            loudest(&y[1000..9000], 32000, &[500.0, 1000.0, 1500.0, 2000.0]),
            1000.0
        );
        let peak = y[1000..15000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 10000.0).abs() < 400.0, "{peak}");
        // A tone above the new Nyquist is filtered out, not folded down.
        let hi: Vec<f32> = tone(20000.0, 0.2, 48000, 10000.0)
            .into_iter()
            .map(|v| v as f32)
            .collect();
        let y = resample(&hi, 48000, 32000);
        let peak = y[500..6000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 800.0, "aliased: {peak}");
    }

    #[test]
    fn loops_and_bands() {
        let x: Vec<f32> = (0..32000).map(|i| i as f32).collect();
        let l = make_loop(x.clone(), 0.08);
        assert_eq!(l.len(), 32000 - 2560);
        // The start becomes the end's continuation: its first sample is the cut-off tail's first.
        assert_eq!(l[0], 32000.0 - 2560.0);
        // Bands: six loops, from each sixth of the sweep in order.
        let sweep: Vec<f32> = (0..32000 * 18).map(|i| (i / 32000) as f32).collect();
        let b = bands(&sweep);
        assert_eq!(b.len(), ROLL_BANDS);
        for (k, band) in b.iter().enumerate() {
            assert_eq!(band.len(), (1.5 * 32000.0) as usize);
            let mid = band[band.len() / 2];
            assert!(mid >= (k * 3) as f32 && mid < ((k + 1) * 3) as f32);
        }
        // A one-shot loses its leading silence and fades out.
        let mut s = vec![0.0f32; 320];
        s.extend(vec![1000.0f32; 1000]);
        let t = tidy_shot(s);
        assert_eq!(t.len(), 1000 + 32);
        assert_eq!(*t.last().unwrap(), 0.0);
    }

    /// The whole path on synthetic data: a 48 kHz tone written as an EA stream in an SPLC bank and a
    /// looping one in a grain-less ABK-style bank, found, decoded, resampled, encoded as ADPCM, packed,
    /// loaded by the game and mixed.
    #[test]
    fn a_synthetic_bank_becomes_sounds_the_game_plays() {
        let a = pcm_stream(&tone(600.0, 0.3, 48000, 12000.0), 48000, 1);
        // Stereo, the same tone both sides: mixed down to mono.
        let st: Vec<i16> = tone(900.0, 0.6, 44100, 9000.0)
            .into_iter()
            .flat_map(|v| [v, v])
            .collect();
        let b = pcm_stream(&st, 44100, 2);
        let mut splc = b"SPLC".to_vec();
        splc.resize(0x18, 0);
        splc.extend_from_slice(&2u32.to_be_bytes());
        splc.extend_from_slice(&[0x5A; 100]);
        splc.extend_from_slice(&a);
        splc.extend_from_slice(&b);
        let picks = [
            Pick {
                cue: Cue::Pop,
                asset: "test.bnk",
                take: Take::Shots(&[(0, 0), (5, 9)]),
            },
            Pick {
                cue: Cue::GrindMetal,
                asset: "test.bnk",
                take: Take::Loops(&[(1, 1)]),
            },
            Pick {
                cue: Cue::Land,
                asset: "missing.bnk",
                take: Take::Shots(&[(0, 0)]),
            },
        ];
        let dec = Decoder { ffmpeg: None };
        let mut reads = 0;
        let items = convert_all(&dec, &picks, &[snr::CODEC_PCM16BE], |p| {
            reads += 1;
            if p.ends_with("test.bnk") {
                Ok(splc.clone())
            } else {
                Err("no such file".into())
            }
        });
        assert_eq!(reads, 2, "each container read once");
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["sound/pop/0", "sound/grind_metal/0"]);
        let (cues, clips, secs, _) = summary(&items);
        assert_eq!((cues, clips), (2, 2));
        assert!((secs - (0.3 + 0.6 - 0.08)).abs() < 0.03, "{secs}");
        // Into the game.
        let packed: &'static [u8] = Box::leak(pack::write(&items).into_boxed_slice());
        let mut bank = Bank::builtin();
        assert_eq!(bank.load(packed), Ok((2, 2)));
        assert_eq!(bank.source(Cue::Pop), Source::Imported);
        assert_eq!(bank.source(Cue::Land), Source::Placeholder);
        let pop = bank.clip(bank.of(Cue::Pop)[0]).unwrap().clone();
        assert_eq!((pop.rate, pop.loop_start), (RATE, None));
        assert!((pop.peak as i32 - 12000).abs() < 1500, "{}", pop.peak);
        let grind = bank.clip(bank.of(Cue::GrindMetal)[0]).unwrap().clone();
        assert_eq!(grind.loop_start, Some(0));
        // Mixed: the pop is its tone at its level.
        let mut m = Mixer::new();
        m.apply(
            &bank,
            &[Command::Play {
                voice: 1,
                clip: bank.of(Cue::Pop)[0],
                gain: 1.0,
                pan: -1.0,
                pitch: 1.0,
                fade: 0.0,
            }],
        );
        let out = mix::render(&mut m, &bank, 0.25, 1.0);
        let left: Vec<f32> = out.iter().step_by(2).map(|v| *v as f32).collect();
        assert_eq!(
            loudest(&left[800..6000], RATE, &[300.0, 600.0, 900.0, 1200.0]),
            600.0
        );
        let peak = left.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 12000.0).abs() < 1500.0, "{peak}");
        // The loop keeps playing well past its length, at its tone.
        m.apply(
            &bank,
            &[Command::Play {
                voice: 2,
                clip: bank.of(Cue::GrindMetal)[0],
                gain: 1.0,
                pan: 1.0,
                pitch: 1.0,
                fade: 0.0,
            }],
        );
        let out = mix::render(&mut m, &bank, 2.0, 1.0);
        let right: Vec<f32> = out.iter().skip(1).step_by(2).map(|v| *v as f32).collect();
        assert!(m.has(2));
        let tail = &right[right.len() - 8000..];
        assert_eq!(loudest(tail, RATE, &[600.0, 900.0, 1200.0]), 900.0);
        assert!(tail.iter().fold(0.0f32, |m, v| m.max(v.abs())) > 7000.0);
    }

    #[test]
    fn every_pick_names_a_cue_once_and_all_cues_are_covered() {
        let mut seen: Vec<Cue> = PICKS.iter().map(|p| p.cue).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), PICKS.len());
        assert_eq!(seen.len(), Cue::COUNT);
        assert!(PICKS.iter().all(|p| p.path().contains("::")));
        assert_eq!(
            wanted(Take::Shots(&[(2, 4), (8, 9)]), 9),
            [2, 3, 4, 8],
            "streams past the end are skipped"
        );
    }
}
