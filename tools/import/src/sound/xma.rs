//! Decoding EA-XMA (Xbox 360 XMA2): the stream's packets are put in a standard XMA2 RIFF/WAVE file
//! (`WAVEFORMATEX` tag 0x166 with the 34-byte XMA2 extension) and decoded by FFmpeg, which the player
//! installs (its `xma2` decoder; https://ffmpeg.org, LGPL). FFmpeg also mixes to mono and resamples to the
//! game's 32 kHz. The converter ships no decoder of its own and no FFmpeg; without one the build goes on
//! without sound and the game plays its placeholders (docs/importing.md).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// XMA2 packets are 2048 bytes.
pub const PACKET: usize = 2048;

/// An XMA2 RIFF file of one XMA stream (`channels` 1 or 2) holding `packets` (padded to whole packets).
pub fn riff(packets: &[u8], channels: u16, rate: u32, samples: u32) -> Vec<u8> {
    let mut data = packets.to_vec();
    data.resize(data.len().next_multiple_of(PACKET), 0);
    let blocks = (data.len() / PACKET) as u16;
    let mut fmt = Vec::new();
    // WAVEFORMATEX: tag, channels, rate, bytes/s, block align, bits, extension size.
    fmt.extend_from_slice(&0x166u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    fmt.extend_from_slice(&(PACKET as u16).to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    fmt.extend_from_slice(&34u16.to_le_bytes());
    // XMA2WAVEFORMATEX: streams, channel mask, samples encoded, bytes per block, play begin / length,
    // loop begin / length / count, encoder version, block count.
    fmt.extend_from_slice(&1u16.to_le_bytes());
    let mask: u32 = if channels == 1 { 0x4 } else { 0x3 };
    fmt.extend_from_slice(&mask.to_le_bytes());
    fmt.extend_from_slice(&samples.to_le_bytes());
    fmt.extend_from_slice(&(PACKET as u32).to_le_bytes());
    fmt.extend_from_slice(&0u32.to_le_bytes());
    fmt.extend_from_slice(&samples.to_le_bytes());
    fmt.extend_from_slice(&0u32.to_le_bytes());
    fmt.extend_from_slice(&0u32.to_le_bytes());
    fmt.push(0);
    fmt.push(4);
    fmt.extend_from_slice(&blocks.to_le_bytes());
    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(&data);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// The FFmpeg to decode with: `--ffmpeg`, then `FG_FFMPEG`, then `ffmpeg` on the PATH. None when it can't
/// be run or has no XMA2 decoder.
pub fn find_ffmpeg(given: Option<&Path>) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match given {
        Some(p) => vec![p.to_path_buf()],
        None => std::env::var_os("FG_FFMPEG")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .into_iter()
            .chain([PathBuf::from("ffmpeg")])
            .collect(),
    };
    candidates.into_iter().find(|p| {
        Command::new(p)
            .args(["-hide_banner", "-decoders"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| {
                o.status.success()
                    && String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .any(|l| l.split_whitespace().nth(1) == Some("xma2"))
            })
    })
}

/// Runs FFmpeg on a WAVE file in memory: mono 16-bit PCM at `rate` Hz.
pub fn decode(ffmpeg: &Path, wav: &[u8], rate: u32) -> Result<Vec<i16>, String> {
    let mut child = Command::new(ffmpeg)
        .args([
            "-v", "error", "-f", "wav", "-i", "pipe:0", "-ac", "1", "-ar",
        ])
        .arg(rate.to_string())
        .args(["-f", "s16le", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", ffmpeg.display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let mut pcm = Vec::new();
    let mut err = String::new();
    std::thread::scope(|s| {
        s.spawn(move || {
            let _ = stdin.write_all(wav);
        });
        s.spawn(|| {
            let _ = stderr.read_to_string(&mut err);
        });
        let _ = stdout.read_to_end(&mut pcm);
    });
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("ffmpeg failed: {}", err.trim()));
    }
    Ok(pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_riff_header_describes_the_packets() {
        let w = riff(&[7u8; 3000], 1, 48000, 4096);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize,
            w.len() - 8
        );
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[16..20].try_into().unwrap()), 52);
        let fmt = &w[20..72];
        assert_eq!(u16::from_le_bytes([fmt[0], fmt[1]]), 0x166);
        assert_eq!(u16::from_le_bytes([fmt[2], fmt[3]]), 1);
        assert_eq!(u32::from_le_bytes(fmt[4..8].try_into().unwrap()), 48000);
        assert_eq!(u16::from_le_bytes([fmt[16], fmt[17]]), 34);
        // Samples encoded, and two blocks of packets (3000 bytes padded to 4096).
        assert_eq!(u32::from_le_bytes(fmt[24..28].try_into().unwrap()), 4096);
        assert_eq!(u16::from_le_bytes([fmt[50], fmt[51]]), 2);
        assert_eq!(&w[72..76], b"data");
        assert_eq!(u32::from_le_bytes(w[76..80].try_into().unwrap()), 4096);
        assert_eq!(w.len(), 80 + 4096);
        assert!(w[80..3080].iter().all(|b| *b == 7) && w[3080..].iter().all(|b| *b == 0));
    }
}
