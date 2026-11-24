//! EA Audio Core ("EAAC") sample streams as Skate 3 stores them, and the containers around them.
//!
//! Format knowledge from SK8-ENGINE/skate-3-rust-engine (GPL-3.0): `crates/skate-audio/src/formats/snr.rs`
//! and `tools/asset_pipeline/audio_formats.py` (docs/hails-additions/11-audio.md, "Formats"):
//!
//! - SNR header, big-endian: `u32 version(4) | codec(4) | channels-1(6) | sample rate(18)`, then
//!   `u32 type(2) | loop(1) | samples(29)`, then a `u32` loop start when looping. RAM streams (type 0) are
//!   followed by blocks: `u8 flag (0x00, or 0x80 for the last)`, `u24 block size` (with this header),
//!   `u32 samples in the block`, then the block's data.
//! - SPLC `.bnk` (patch banks): `u32` stream count at 0x18, then that many SNR streams back to back at
//!   unaligned offsets (found by scanning; SK8 checked the count on every bank of the disc).
//! - ABKC `.abk` banks: SNR streams embedded; scanning finds them in order (stream n = SK8's sample n).
//! - `.grain` (the rolling sweeps): `u32` offset of the SNR stream, `f32` duration (s), a seek table, then
//!   the stream.
//! - Codec 3 is EA-XMA (Xbox 360 XMA2): each block holds, per stereo pair of channels, a sub-block
//!   `u32 (size << 2) | flags` (size with this word) of whole 2048-byte XMA2 packets, the last one cut
//!   short (worked out on the owned disc's streams for this converter). Codec 2 is PCM16 big-endian
//!   (not used by Skate 3; flatground reads it with the channels interleaved, for its own tests).

pub const CODEC_PCM16BE: u8 = 2;
pub const CODEC_EAXMA: u8 = 3;

fn be32(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

/// One block of a stream: where its data is and how many samples it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// The data after the block's 8-byte header, as a range of the container.
    pub data: (usize, usize),
    pub samples: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    /// Where the header starts and the last block ends.
    pub offset: usize,
    pub end: usize,
    pub codec: u8,
    pub channels: u8,
    pub rate: u32,
    pub samples: u32,
    pub loop_start: Option<u32>,
    pub blocks: Vec<Block>,
}

impl Stream {
    pub fn seconds(&self) -> f32 {
        self.samples as f32 / self.rate as f32
    }
}

/// The RAM SNR stream at `at` of one of `codecs`, if its header is plausible and its block chain adds up.
pub fn stream_at(d: &[u8], at: usize, codecs: &[u8]) -> Option<Stream> {
    if at + 16 > d.len() {
        return None;
    }
    let w0 = be32(d, at);
    let (version, codec) = (w0 >> 28, ((w0 >> 24) & 0xF) as u8);
    let channels = ((w0 >> 18) & 0x3F) + 1;
    let rate = w0 & 0x3FFFF;
    if version > 1 || !codecs.contains(&codec) || !(8000..=48000).contains(&rate) || channels > 8 {
        return None;
    }
    let w1 = be32(d, at + 4);
    let (kind, looped, samples) = (w1 >> 30, (w1 >> 29) & 1 == 1, w1 & 0x1FFF_FFFF);
    if kind != 0 || samples == 0 {
        return None;
    }
    let loop_start = looped.then(|| be32(d, at + 8));
    let mut cur = at + if looped { 12 } else { 8 };
    let mut decoded = 0u32;
    let mut blocks = Vec::new();
    while decoded < samples {
        if cur + 8 > d.len() {
            return None;
        }
        let flag = d[cur];
        let size = (be32(d, cur) & 0xFF_FFFF) as usize;
        let n = be32(d, cur + 4);
        if (flag != 0 && flag != 0x80) || size <= 8 || cur + size > d.len() || n == 0 {
            return None;
        }
        blocks.push(Block {
            data: (cur + 8, cur + size),
            samples: n,
        });
        decoded = decoded.checked_add(n)?;
        cur += size;
        if flag == 0x80 {
            break;
        }
    }
    if decoded != samples {
        return None;
    }
    Some(Stream {
        offset: at,
        end: cur,
        codec,
        channels: channels as u8,
        rate,
        samples,
        loop_start,
        blocks,
    })
}

/// Every consistent stream in `d`, in order.
pub fn scan(d: &[u8], codecs: &[u8]) -> Vec<Stream> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 16 <= d.len() {
        match stream_at(d, at, codecs) {
            Some(s) => {
                at = s.end;
                out.push(s);
            }
            None => at += 1,
        }
    }
    out
}

/// The streams of a container: an SPLC bank (its declared count checked), a grain file, or anything else
/// scanned (ABKC banks, standalone `.snr`).
pub fn streams(d: &[u8], codecs: &[u8]) -> Result<Vec<Stream>, String> {
    if d.starts_with(b"SPLC") {
        if d.len() < 0x1C {
            return Err("SPLC bank too short".into());
        }
        let want = be32(d, 0x18) as usize;
        let got = scan(d, codecs);
        if got.len() != want {
            return Err(format!(
                "SPLC bank declares {want} streams, found {}",
                got.len()
            ));
        }
        return Ok(got);
    }
    if d.len() >= 8 {
        // A grain: its stream where the first word says, the duration matching.
        let at = be32(d, 0) as usize;
        let secs = f32::from_bits(be32(d, 4));
        if at > 8
            && at < d.len()
            && let Some(s) = stream_at(d, at, codecs)
            && (s.seconds() - secs).abs() < 0.01
        {
            return Ok(vec![s]);
        }
    }
    let got = scan(d, codecs);
    if got.is_empty() {
        return Err("no sound streams".into());
    }
    Ok(got)
}

/// The XMA2 packets of each stereo pair (stream) of an EA-XMA stream, concatenated.
pub fn xma_packets(d: &[u8], s: &Stream) -> Result<Vec<Vec<u8>>, String> {
    let pairs = (s.channels as usize).div_ceil(2);
    let mut out = vec![Vec::new(); pairs];
    for b in &s.blocks {
        let mut at = b.data.0;
        for (k, packets) in out.iter_mut().enumerate() {
            if at + 4 > b.data.1 {
                return Err(format!("block at {:#x}: no sub-block {k}", b.data.0));
            }
            let size = (be32(d, at) >> 2) as usize;
            if size < 4 || at + size > b.data.1 {
                return Err(format!(
                    "block at {:#x}: bad sub-block size {size}",
                    b.data.0
                ));
            }
            packets.extend_from_slice(&d[at + 4..at + size]);
            at += size;
        }
    }
    Ok(out)
}

/// A PCM16BE stream's samples (interleaved channels).
pub fn pcm16(d: &[u8], s: &Stream) -> Vec<i16> {
    let mut out = Vec::with_capacity(s.samples as usize * s.channels as usize);
    for b in &s.blocks {
        let want = b.samples as usize * s.channels as usize;
        let end = (b.data.0 + want * 2).min(b.data.1);
        out.extend(
            d[b.data.0..end]
                .chunks_exact(2)
                .map(|c| i16::from_be_bytes([c[0], c[1]])),
        );
    }
    out
}

/// Writes an SNR stream (tests build synthetic containers with it): header, then `blocks` of
/// (data, samples).
#[cfg(test)]
pub fn write(
    codec: u8,
    channels: u8,
    rate: u32,
    loop_start: Option<u32>,
    blocks: &[(Vec<u8>, u32)],
) -> Vec<u8> {
    let samples: u32 = blocks.iter().map(|b| b.1).sum();
    let mut out = Vec::new();
    let w0 = ((codec as u32) << 24) | (((channels as u32) - 1) << 18) | rate;
    out.extend_from_slice(&w0.to_be_bytes());
    let w1 = ((loop_start.is_some() as u32) << 29) | samples;
    out.extend_from_slice(&w1.to_be_bytes());
    if let Some(l) = loop_start {
        out.extend_from_slice(&l.to_be_bytes());
    }
    for (i, (data, n)) in blocks.iter().enumerate() {
        let last = i + 1 == blocks.len();
        let size = (data.len() + 8) as u32;
        out.extend_from_slice(&(((last as u32) << 31) | size).to_be_bytes());
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm_block(samples: &[i16]) -> (Vec<u8>, u32) {
        (
            samples.iter().flat_map(|s| s.to_be_bytes()).collect(),
            samples.len() as u32,
        )
    }

    #[test]
    fn finds_streams_in_banks_and_grains() {
        let a = write(CODEC_PCM16BE, 1, 32000, None, &[pcm_block(&[1, 2, 3])]);
        let b = write(
            CODEC_PCM16BE,
            1,
            48000,
            Some(2),
            &[pcm_block(&[4, 5]), pcm_block(&[6, 7, 8])],
        );
        // An SPLC bank: the count at 0x18, junk before the streams, streams back to back.
        let mut bank = b"SPLC".to_vec();
        bank.resize(0x18, 0);
        bank.extend_from_slice(&2u32.to_be_bytes());
        bank.extend_from_slice(&[0xAB; 37]);
        bank.extend_from_slice(&a);
        bank.extend_from_slice(&b);
        bank.extend_from_slice(&[0; 5]);
        let s = streams(&bank, &[CODEC_PCM16BE]).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].rate, s[0].samples, s[0].loop_start), (32000, 3, None));
        assert_eq!(
            (s[1].rate, s[1].samples, s[1].loop_start),
            (48000, 5, Some(2))
        );
        assert_eq!(s[1].blocks.len(), 2);
        assert_eq!(pcm16(&bank, &s[1]), [4, 5, 6, 7, 8]);
        // A wrong count is refused; the XMA scanner sees nothing in PCM.
        bank[0x1B] = 3;
        assert!(streams(&bank, &[CODEC_PCM16BE]).is_err());
        assert!(scan(&bank, &[CODEC_EAXMA]).is_empty());
        // A grain: offset, duration, a seek table, the stream.
        let mut g = Vec::new();
        g.extend_from_slice(&20u32.to_be_bytes());
        g.extend_from_slice(&(3.0f32 / 32000.0).to_bits().to_be_bytes());
        g.extend_from_slice(&[0x11; 12]);
        g.extend_from_slice(&a);
        let s = streams(&g, &[CODEC_PCM16BE]).unwrap();
        assert_eq!((s.len(), s[0].offset), (1, 20));
        // A broken chain (block sizes not adding up to the samples) is not a stream.
        let mut bad = a.clone();
        bad[7] = 9;
        assert!(stream_at(&bad, 0, &[CODEC_PCM16BE]).is_none());
    }

    #[test]
    fn xma_packets_come_out_of_the_sub_blocks() {
        // Two blocks of one mono stream: each a sub-block word (size << 2 | 2) and packet bytes.
        let sub = |bytes: &[u8]| {
            let mut v = ((((bytes.len() + 4) as u32) << 2) | 2)
                .to_be_bytes()
                .to_vec();
            v.extend_from_slice(bytes);
            v
        };
        let s = write(
            CODEC_EAXMA,
            1,
            48000,
            None,
            &[(sub(&[1; 2048]), 512 * 4), (sub(&[2; 600]), 512)],
        );
        let st = stream_at(&s, 0, &[CODEC_EAXMA]).unwrap();
        let p = xma_packets(&s, &st).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].len(), 2648);
        assert!(p[0][..2048].iter().all(|b| *b == 1) && p[0][2048..].iter().all(|b| *b == 2));
        // Stereo pairs: three channels are two XMA streams, each its own sub-block.
        let mut body = sub(&[3; 100]);
        body.extend(sub(&[4; 50]));
        let s = write(CODEC_EAXMA, 3, 48000, None, &[(body, 512)]);
        let st = stream_at(&s, 0, &[CODEC_EAXMA]).unwrap();
        let p = xma_packets(&s, &st).unwrap();
        assert_eq!((p[0].len(), p[1].len()), (100, 50));
    }
}
