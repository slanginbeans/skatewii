//! Wii GX texture encodings (big-endian, tiled as the GPU reads them), plus downscaling.
//! GX tiles: CMPR = 8x8 tiles of four DXT1-like 4x4 sub-blocks (TL, TR, BL, BR); RGB5A3 and RGBA8 = 4x4 tiles.

use crate::texture::{Rgba, dxt1_block};

pub const GX_TF_RGB5A3: u8 = 0x5;
pub const GX_TF_RGBA8: u8 = 0x6;
pub const GX_TF_CMPR: u8 = 0xE;

pub fn name(fmt: u8) -> &'static str {
    match fmt {
        GX_TF_RGB5A3 => "RGB5A3",
        GX_TF_RGBA8 => "RGBA8",
        GX_TF_CMPR => "CMPR",
        _ => "?",
    }
}

/// Converts one PC DXT1 block to the GX CMPR sub-block layout: big-endian colours, and 2-bit indices
/// packed with texel 0 in the top bits of each row byte.
fn cmpr_block(pc: &[u8]) -> [u8; 8] {
    let mut o = [0u8; 8];
    o[0..2].copy_from_slice(&u16::from_le_bytes([pc[0], pc[1]]).to_be_bytes());
    o[2..4].copy_from_slice(&u16::from_le_bytes([pc[2], pc[3]]).to_be_bytes());
    for r in 0..4 {
        let b = pc[4 + r];
        o[4 + r] = ((b & 3) << 6) | ((b >> 2 & 3) << 4) | ((b >> 4 & 3) << 2) | (b >> 6 & 3);
    }
    o
}

/// Lays out PC DXT1 blocks (`bw` x `bh` blocks, row-major) as GX CMPR. The GX surface is padded to 8 texels;
/// padding blocks repeat the nearest edge block.
pub fn cmpr_from_dxt1(blocks: &[u8], bw: usize, bh: usize) -> Vec<u8> {
    let (tw, th) = (bw.div_ceil(2), bh.div_ceil(2));
    let mut out = Vec::with_capacity(tw * th * 32);
    for ty in 0..th {
        for tx in 0..tw {
            for (sx, sy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, y) = ((tx * 2 + sx).min(bw - 1), (ty * 2 + sy).min(bh - 1));
                let i = (y * bw + x) * 8;
                out.extend_from_slice(&cmpr_block(&blocks[i..i + 8]));
            }
        }
    }
    out
}

fn to565(p: [u8; 4]) -> u16 {
    ((p[0] as u16 >> 3) << 11) | ((p[1] as u16 >> 2) << 5) | (p[2] as u16 >> 3)
}

/// Encodes a 4x4 RGBA block as a PC DXT1 block (bounding-box endpoints, nearest palette entry; texels with
/// alpha < 128 use the punch-through transparent entry).
pub fn encode_dxt1_block(t: &[[u8; 4]; 16]) -> [u8; 8] {
    let transparent = t.iter().any(|p| p[3] < 128);
    // Endpoints: the two opaque texels furthest apart (16 texels, so the O(n^2) search is cheap).
    let opaque: Vec<[u8; 4]> = t.iter().copied().filter(|p| p[3] >= 128).collect();
    let dist = |a: &[u8; 4], b: &[u8; 4]| {
        (0..3)
            .map(|c| (a[c] as i32 - b[c] as i32).pow(2))
            .sum::<i32>()
    };
    let (mut hi, mut lo) = ([0u8; 4], [0u8; 4]);
    let mut best = -1;
    for a in &opaque {
        for b in &opaque {
            if dist(a, b) > best {
                best = dist(a, b);
                (hi, lo) = (*a, *b);
            }
        }
    }
    let (mut c0, mut c1) = (to565(hi), to565(lo));
    // c0 > c1 selects 4-colour mode; c0 <= c1 selects 3 colours + transparent.
    if transparent && c0 > c1 || !transparent && c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    if !transparent && c0 == c1 {
        // Single colour: 4-colour mode needs c0 > c1, but index 0 alone is exact either way.
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&c0.to_le_bytes());
        b[2..4].copy_from_slice(&c1.to_le_bytes());
        return b;
    }
    let mut b = [0u8; 8];
    b[0..2].copy_from_slice(&c0.to_le_bytes());
    b[2..4].copy_from_slice(&c1.to_le_bytes());
    let usable = if transparent { 3 } else { 4 };
    let pal: Vec<[u8; 4]> = (0..usable).map(|k| palette_entry(&b, k)).collect();
    for (i, p) in t.iter().enumerate() {
        let idx = if transparent && p[3] < 128 {
            3
        } else {
            (0..usable)
                .min_by_key(|&k| {
                    (0..3)
                        .map(|c| (p[c] as i32 - pal[k][c] as i32).pow(2))
                        .sum::<i32>()
                })
                .unwrap()
        };
        b[4 + i / 4] |= (idx as u8) << ((i % 4) * 2);
    }
    b
}

fn palette_entry(b: &[u8; 8], k: usize) -> [u8; 4] {
    let mut probe = *b;
    let fill = [0x00, 0x55, 0xAA, 0xFF][k];
    probe[4..8].copy_from_slice(&[fill; 4]);
    dxt1_block(&probe, true)[0]
}

fn block_at(img: &Rgba, bx: usize, by: usize) -> [[u8; 4]; 16] {
    let mut t = [[0u8; 4]; 16];
    for (i, p) in t.iter_mut().enumerate() {
        let (x, y) = (
            (bx * 4 + i % 4).min(img.width - 1),
            (by * 4 + i / 4).min(img.height - 1),
        );
        let s = (y * img.width + x) * 4;
        p.copy_from_slice(&img.pixels[s..s + 4]);
    }
    t
}

pub fn encode_cmpr(img: &Rgba) -> Vec<u8> {
    let (bw, bh) = (img.width.div_ceil(4), img.height.div_ceil(4));
    let mut blocks = Vec::with_capacity(bw * bh * 8);
    for by in 0..bh {
        for bx in 0..bw {
            blocks.extend_from_slice(&encode_dxt1_block(&block_at(img, bx, by)));
        }
    }
    cmpr_from_dxt1(&blocks, bw, bh)
}

pub fn encode_rgb5a3(img: &Rgba) -> Vec<u8> {
    let (tw, th) = (img.width.div_ceil(4), img.height.div_ceil(4));
    let mut out = Vec::with_capacity(tw * th * 32);
    for ty in 0..th {
        for tx in 0..tw {
            for p in block_at(img, tx, ty) {
                let v: u16 = if p[3] >= 0xE0 {
                    0x8000
                        | ((p[0] as u16 >> 3) << 10)
                        | ((p[1] as u16 >> 3) << 5)
                        | (p[2] as u16 >> 3)
                } else {
                    ((p[3] as u16 >> 5) << 12)
                        | ((p[0] as u16 >> 4) << 8)
                        | ((p[1] as u16 >> 4) << 4)
                        | (p[2] as u16 >> 4)
                };
                out.extend_from_slice(&v.to_be_bytes());
            }
        }
    }
    out
}

pub fn encode_rgba8(img: &Rgba) -> Vec<u8> {
    let (tw, th) = (img.width.div_ceil(4), img.height.div_ceil(4));
    let mut out = Vec::with_capacity(tw * th * 64);
    for ty in 0..th {
        for tx in 0..tw {
            let t = block_at(img, tx, ty);
            for p in &t {
                out.extend_from_slice(&[p[3], p[0]]);
            }
            for p in &t {
                out.extend_from_slice(&[p[1], p[2]]);
            }
        }
    }
    out
}

/// Halves the image (2x2 box filter) until both sides are at most `max`. GX textures are limited to 1024.
pub fn shrink_to(mut img: Rgba, max: usize) -> Rgba {
    while img.width > max || img.height > max {
        let (w, h) = ((img.width / 2).max(1), (img.height / 2).max(1));
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = (
                            (x * 2 + dx).min(img.width - 1),
                            (y * 2 + dy).min(img.height - 1),
                        );
                        sum += img.pixels[(sy * img.width + sx) * 4 + c] as u32;
                    }
                    px[(y * w + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        img = Rgba {
            width: w,
            height: h,
            pixels: px,
        };
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, p: [u8; 4]) -> Rgba {
        Rgba {
            width: w,
            height: h,
            pixels: p.repeat(w * h),
        }
    }

    #[test]
    fn cmpr_reorders_indices_and_endianness() {
        let pc = [0x00, 0xF8, 0x1F, 0x00, 0b11_10_01_00, 0, 0, 0];
        let gx = cmpr_block(&pc);
        assert_eq!(&gx[0..4], &[0xF8, 0x00, 0x00, 0x1F]);
        assert_eq!(gx[4], 0b00_01_10_11);
    }

    #[test]
    fn cmpr_tiles_are_8x8() {
        // 3x1 blocks -> 2x1 tiles of 4 sub-blocks, 32 bytes each.
        let blocks: Vec<u8> = (0..3u8).flat_map(|i| [i, 0, 0, 0, 0, 0, 0, 0]).collect();
        let gx = cmpr_from_dxt1(&blocks, 3, 1);
        assert_eq!(gx.len(), 64);
        let firsts: Vec<u8> = gx.chunks(8).map(|b| b[1]).collect();
        assert_eq!(firsts, [0, 1, 0, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn dxt1_encoder_round_trips_flat_and_two_tone() {
        let red = [255, 0, 0, 255];
        let b = encode_dxt1_block(&[red; 16]);
        assert!(dxt1_block(&b, true).iter().all(|p| *p == red));
        let mut t = [[0, 0, 255, 255]; 16];
        t[..8].fill(red);
        t[15] = [0, 0, 0, 0];
        let d = dxt1_block(&encode_dxt1_block(&t), true);
        assert_eq!(d[0], red);
        assert_eq!(d[8], [0, 0, 255, 255]);
        assert_eq!(d[15][3], 0);
    }

    #[test]
    fn rgb5a3_and_rgba8_sizes_and_values() {
        let img = solid(8, 4, [255, 0, 0, 255]);
        let v = encode_rgb5a3(&img);
        assert_eq!(v.len(), 8 * 4 * 2);
        assert_eq!(&v[0..2], &[0xFC, 0x00]);
        let v = encode_rgba8(&solid(4, 4, [1, 2, 3, 4]));
        assert_eq!(v.len(), 64);
        assert_eq!(&v[0..2], &[4, 1]);
        assert_eq!(&v[32..34], &[2, 3]);
    }

    /// Reads GX CMPR the way the Wii GPU lays it out, independently of the encoder.
    fn decode_cmpr(data: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut px = vec![0u8; w * h * 4];
        let tiles_w = w.div_ceil(8);
        for (si, sub) in data.chunks_exact(8).enumerate() {
            let (tile, q) = (si / 4, si % 4);
            let (bx, by) = (
                (tile % tiles_w) * 8 + (q % 2) * 4,
                (tile / tiles_w) * 8 + (q / 2) * 4,
            );
            // Back to PC DXT1: little-endian colours, texel 0 in the low bits.
            let mut pc = [0u8; 8];
            pc[0..2].copy_from_slice(&u16::from_be_bytes([sub[0], sub[1]]).to_le_bytes());
            pc[2..4].copy_from_slice(&u16::from_be_bytes([sub[2], sub[3]]).to_le_bytes());
            for r in 0..4 {
                let b = sub[4 + r];
                pc[4 + r] = (b >> 6) | ((b >> 4 & 3) << 2) | ((b >> 2 & 3) << 4) | ((b & 3) << 6);
            }
            for (i, t) in dxt1_block(&pc, true).iter().enumerate() {
                let (x, y) = (bx + i % 4, by + i / 4);
                if x < w && y < h {
                    px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(t);
                }
            }
        }
        px
    }

    #[test]
    fn cmpr_round_trips_a_pattern() {
        // 12x8: not a multiple of the 8x8 tile, so padding is exercised too.
        let (w, h) = (12, 8);
        let mut img = solid(w, h, [0, 0, 0, 255]);
        for y in 0..h {
            for x in 0..w {
                let c = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]][(x / 4 + y / 4) % 3];
                img.pixels[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&c);
            }
        }
        assert_eq!(decode_cmpr(&encode_cmpr(&img), w, h), img.pixels);
    }

    #[test]
    fn shrinks_by_halves() {
        let s = shrink_to(solid(1024, 256, [10, 20, 30, 255]), 256);
        assert_eq!((s.width, s.height), (256, 64));
        assert_eq!(&s.pixels[0..4], &[10, 20, 30, 255]);
    }
}
