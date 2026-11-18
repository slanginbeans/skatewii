//! Xbox 360 (Xenos) textures from RW4 texture sections: untiling and decoding of the base mip level.
//! Formats and tiling as documented by SK8-ENGINE/skate-3-rust-engine (rx2_parser.py, xenos.py, GPL-3.0).
//!
//! The section's 40-byte info block carries the GPU fetch constant: format byte @+35 (low 6 bits are the
//! Xenos format), size word @+36 (width-1 in bits 0..13, height-1 in bits 13..26). Compressed blocks are
//! stored with every 16-bit word byte-swapped relative to PC DXT, and surfaces are tiled.

use crate::rw4::{Arena, Section, be32};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    A8,
    B5G6R5,
    A8R8G8B8,
    Dxt1,
    Dxt3,
    Dxt5,
    Ati2,
    Dxt1Normal,
}

impl Format {
    fn from_xenos(code: u8) -> Option<Self> {
        Some(match code & 0x3F {
            2 => Format::A8,
            4 => Format::B5G6R5,
            6 => Format::A8R8G8B8,
            18 => Format::Dxt1,
            19 => Format::Dxt3,
            20 => Format::Dxt5,
            49 => Format::Ati2,
            60 => Format::Dxt1Normal,
            _ => return None,
        })
    }
    /// (block edge in texels, bytes per block)
    fn layout(self) -> (usize, usize) {
        match self {
            Format::A8 => (1, 1),
            Format::B5G6R5 => (1, 2),
            Format::A8R8G8B8 => (1, 4),
            Format::Dxt1 | Format::Dxt1Normal => (4, 8),
            Format::Dxt3 | Format::Dxt5 | Format::Ati2 => (4, 16),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub width: usize,
    pub height: usize,
    pub format: Format,
    pub tiled: bool,
    /// Fetch-constant `packed_mips` (dword 5 bit 11): small levels share a packed mip-tail tile.
    pub packed: bool,
}

pub fn read_info(block: &[u8]) -> Result<Info, String> {
    if block.len() < 40 {
        return Err("texture info block truncated".into());
    }
    let format = Format::from_xenos(block[35])
        .ok_or_else(|| format!("unsupported Xenos format {:#04x}", block[35]))?;
    let size = be32(block, 36);
    let (width, height) = (
        (size & 0x1FFF) as usize + 1,
        ((size >> 13) & 0x1FFF) as usize + 1,
    );
    // Some DXT5 textures written by later converters are stored linear (rx2_parser.py `dxt5_variant`).
    let tiled = !(format == Format::Dxt5 && matches!(block[28], 1 | 2 | 84));
    let packed = block.len() >= 52 && be32(block, 48) & (1 << 11) != 0;
    Ok(Info {
        width,
        height,
        format,
        tiled,
        packed,
    })
}

/// Xbox 360 tiled address (in blocks) of block (x, y) on a surface `width` blocks wide.
fn tiled_offset(x: usize, y: usize, width: usize, log_bpb: u32) -> usize {
    let aligned = (width + 31) & !31;
    let macro_ = ((x >> 5) + (y >> 5) * (aligned >> 5)) << (log_bpb + 7);
    let micro = ((x & 7) + ((y & 0xE) << 2)) << log_bpb;
    let offset = macro_ + ((micro & !0xF) << 1) + (micro & 0xF) + ((y & 1) << 4);
    let addr = ((offset & !0x1FF) << 3)
        + ((y & 16) << 7)
        + ((offset & 0x1C0) << 2)
        + (((((y & 8) >> 2) + (x >> 3)) & 3) << 6)
        + (offset & 0x3F);
    addr >> log_bpb
}

/// Where level 0 sits inside a packed mip tail, in texels. Xenos packs a texture into one shared 32x32
/// tile once either side is 16 texels or less: level 0 then starts at x=16 (square/tall) or y=16 (wide).
/// (Level-0 case of Xenia's GetPackedMipOffset, as used by SK8's retail_texture_decode.py.)
fn packed_base_offset(width: usize, height: usize) -> (usize, usize) {
    let log2 = |v: usize| usize::BITS - (v - 1).leading_zeros();
    let (lw, lh) = (log2(width), log2(height));
    match () {
        _ if lw.min(lh) > 4 => (0, 0),
        _ if lw > lh => (0, 16),
        _ => (16, 0),
    }
}

/// Linear blocks of the base level, in the console's byte order (no swapping yet).
fn linear_blocks(data: &[u8], info: &Info) -> Result<Vec<u8>, String> {
    let (edge, bpb) = info.format.layout();
    let (bw, bh) = (info.width.div_ceil(edge), info.height.div_ceil(edge));
    let mut out = vec![0u8; bw * bh * bpb];
    let tiled_w = if edge == 4 {
        ((info.width + 127) & !127) / 4
    } else {
        (info.width + 31) & !31
    };
    let (px, py) = if info.packed {
        packed_base_offset(info.width, info.height)
    } else {
        (0, 0)
    };
    let (px, py) = (px / edge, py / edge);
    for y in 0..bh {
        for x in 0..bw {
            let src = if info.tiled {
                tiled_offset(x + px, y + py, tiled_w, bpb.trailing_zeros())
            } else {
                y * bw + x
            } * bpb;
            let block = data
                .get(src..src + bpb)
                .ok_or_else(|| format!("texture payload too short ({} bytes)", data.len()))?;
            let dst = (y * bw + x) * bpb;
            out[dst..dst + bpb].copy_from_slice(block);
        }
    }
    Ok(out)
}

fn swap16(b: &mut [u8]) {
    for p in b.chunks_exact_mut(2) {
        p.swap(0, 1);
    }
}

/// Standard (PC, little-endian) DXT1 blocks for a DXT1 texture, for lossless transcoding to GX CMPR.
pub fn dxt1_blocks(data: &[u8], info: &Info) -> Result<Vec<u8>, String> {
    if info.format != Format::Dxt1 {
        return Err("not DXT1".into());
    }
    let mut b = linear_blocks(data, info)?;
    swap16(&mut b);
    Ok(b)
}

fn rgb565(v: u16) -> [u8; 3] {
    let (r, g, b) = ((v >> 11) & 31, (v >> 5) & 63, v & 31);
    [
        (r * 255 / 31) as u8,
        (g * 255 / 63) as u8,
        (b * 255 / 31) as u8,
    ]
}

/// Decodes one PC-order DXT1 colour block into 16 RGBA texels.
pub fn dxt1_block(b: &[u8], punch_through: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u16, b: u16, wa: u16, wb: u16, d: u16| ((a * wa + b * wb + d / 2) / d) as u8;
    let mut pal = [
        [p0[0], p0[1], p0[2], 255],
        [p1[0], p1[1], p1[2], 255],
        [0; 4],
        [0; 4],
    ];
    if c0 > c1 || !punch_through {
        for i in 0..3 {
            let (a, b) = (p0[i] as u16, p1[i] as u16);
            pal[2][i] = mix(a, b, 2, 1, 3);
            pal[3][i] = mix(a, b, 1, 2, 3);
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for i in 0..3 {
            pal[2][i] = ((p0[i] as u16 + p1[i] as u16) / 2) as u8;
        }
        pal[2][3] = 255;
    }
    let mut out = [[0u8; 4]; 16];
    for y in 0..4 {
        for x in 0..4 {
            out[y * 4 + x] = pal[((b[4 + y] >> (x * 2)) & 3) as usize];
        }
    }
    out
}

/// DXT5-style 8-byte interpolated alpha (also the channels of ATI2).
fn alpha_block(b: &[u8]) -> [u8; 16] {
    let (a0, a1) = (b[0] as u32, b[1] as u32);
    let mut pal = [a0, a1, 0, 0, 0, 0, 0, 0];
    if a0 > a1 {
        for i in 1..7 {
            pal[i + 1] = ((7 - i as u32) * a0 + i as u32 * a1) / 7;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = ((5 - i as u32) * a0 + i as u32 * a1) / 5;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = pal[((bits >> (3 * i)) & 7) as usize] as u8;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Rgba {
    pub fn has_alpha(&self) -> bool {
        self.pixels.chunks_exact(4).any(|p| p[3] != 255)
    }
}

pub fn decode(data: &[u8], info: &Info) -> Result<Rgba, String> {
    let mut lin = linear_blocks(data, info)?;
    let (edge, bpb) = info.format.layout();
    let (w, h) = (info.width, info.height);
    let mut px = vec![0u8; w * h * 4];
    if edge == 1 {
        for i in 0..w * h {
            let s = &lin[i * bpb..i * bpb + bpb];
            let p = match info.format {
                Format::A8 => [255, 255, 255, s[0]],
                Format::A8R8G8B8 => [s[1], s[2], s[3], s[0]],
                _ => {
                    let c = rgb565(u16::from_be_bytes([s[0], s[1]]));
                    [c[0], c[1], c[2], 255]
                }
            };
            px[i * 4..i * 4 + 4].copy_from_slice(&p);
        }
        return Ok(Rgba {
            width: w,
            height: h,
            pixels: px,
        });
    }
    swap16(&mut lin);
    let bw = w.div_ceil(4);
    for (bi, b) in lin.chunks_exact(bpb).enumerate() {
        let texels: [[u8; 4]; 16] = match info.format {
            Format::Dxt1 => dxt1_block(b, true),
            Format::Dxt1Normal => {
                let mut t = dxt1_block(b, false);
                for p in &mut t {
                    let (r, g) = (
                        p[0] as f32 / 255.0 * 2.0 - 1.0,
                        p[1] as f32 / 255.0 * 2.0 - 1.0,
                    );
                    p[2] =
                        (((1.0 - r * r - g * g).max(0.0).sqrt() * 0.5 + 0.5) * 255.0).round() as u8;
                    p[3] = 255;
                }
                t
            }
            Format::Dxt3 => {
                let mut t = dxt1_block(&b[8..], false);
                for (i, p) in t.iter_mut().enumerate() {
                    let a = (b[i / 2] >> ((i & 1) * 4)) & 0xF;
                    p[3] = a * 17;
                }
                t
            }
            Format::Dxt5 => {
                let mut t = dxt1_block(&b[8..], false);
                let a = alpha_block(&b[..8]);
                for (p, a) in t.iter_mut().zip(a) {
                    p[3] = a;
                }
                t
            }
            Format::Ati2 => {
                let (x, y) = (alpha_block(&b[..8]), alpha_block(&b[8..]));
                let mut t = [[0u8; 4]; 16];
                for i in 0..16 {
                    let (r, g) = (
                        x[i] as f32 / 255.0 * 2.0 - 1.0,
                        y[i] as f32 / 255.0 * 2.0 - 1.0,
                    );
                    let z = (1.0 - r * r - g * g).max(0.0).sqrt();
                    t[i] = [x[i], y[i], ((z * 0.5 + 0.5) * 255.0).round() as u8, 255];
                }
                t
            }
            _ => unreachable!(),
        };
        let (bx, by) = (bi % bw, bi / bw);
        for (i, t) in texels.iter().enumerate() {
            let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
            if x < w && y < h {
                px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(t);
            }
        }
    }
    Ok(Rgba {
        width: w,
        height: h,
        pixels: px,
    })
}

pub struct Texture<'a> {
    pub section: usize,
    pub info: Info,
    pub data: &'a [u8],
}

/// Every texture section of an arena, with its info and payload; unsupported ones come back as errors.
pub fn textures<'a>(arena: &Arena<'a>) -> Vec<Result<Texture<'a>, String>> {
    arena
        .of_type(crate::rw4::T_TEXTURE)
        .map(|s: &Section| {
            let block = arena
                .bytes
                .get(s.info..(s.info + 52).min(arena.bytes.len()))
                .ok_or("info block out of range")?;
            let info = read_info(block)?;
            let (at, n) = s.data.ok_or("texture has no payload")?;
            Ok(Texture {
                section: s.index,
                info,
                data: &arena.bytes[at..at + n],
            })
        })
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn info_block(format: u8, w: usize, h: usize) -> Vec<u8> {
        let mut b = vec![0u8; 40];
        b[35] = format;
        let size = ((h as u32 - 1) << 13) | (w as u32 - 1);
        b[36..40].copy_from_slice(&size.to_be_bytes());
        b
    }

    /// Tiles linear console-order blocks the way the Xenos GPU stores them (inverse of `linear_blocks`).
    pub fn tile(linear: &[u8], info: &Info) -> Vec<u8> {
        let (edge, bpb) = info.format.layout();
        let (bw, bh) = (info.width.div_ceil(edge), info.height.div_ceil(edge));
        let tiled_w = if edge == 4 {
            ((info.width + 127) & !127) / 4
        } else {
            (info.width + 31) & !31
        };
        let (px, py) = if info.packed {
            packed_base_offset(info.width, info.height)
        } else {
            (0, 0)
        };
        let (px, py) = (px / edge, py / edge);
        let rows = (bh + py + 31) & !31;
        let mut out = vec![0u8; tiled_w * rows * bpb];
        for y in 0..bh {
            for x in 0..bw {
                let d = tiled_offset(x + px, y + py, tiled_w, bpb.trailing_zeros()) * bpb;
                let s = (y * bw + x) * bpb;
                out[d..d + bpb].copy_from_slice(&linear[s..s + bpb]);
            }
        }
        out
    }

    #[test]
    fn tiling_is_a_permutation() {
        let w = 64;
        let mut seen = std::collections::HashSet::new();
        for y in 0..32 {
            for x in 0..w {
                assert!(seen.insert(tiled_offset(x, y, w, 2)));
            }
        }
    }

    #[test]
    fn reads_info() {
        let i = read_info(&info_block(0x52, 256, 128)).unwrap();
        assert_eq!(
            (i.width, i.height, i.format, i.tiled),
            (256, 128, Format::Dxt1, true)
        );
        assert!(read_info(&info_block(0x3F, 4, 4)).is_err());
    }

    #[test]
    fn decodes_tiled_dxt1_with_and_without_packed_tail() {
        for packed in [false, true] {
            let info = Info {
                width: 8,
                height: 4,
                format: Format::Dxt1,
                tiled: true,
                packed,
            };
            // Two PC-order blocks: solid red (c0 = c1 = 0xF800, idx 0) and solid blue (idx 1 of red/blue).
            let mut pc = Vec::new();
            pc.extend_from_slice(&[0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0]);
            pc.extend_from_slice(&[0x00, 0xF8, 0x1F, 0x00, 0x55, 0x55, 0x55, 0x55]);
            let mut console = pc.clone();
            swap16(&mut console);
            let tiled = tile(&console, &info);
            // In a packed tail, a wide level 0 starts 16 texels down, so the tile origin stays empty.
            assert_eq!(tiled[..8].iter().all(|&b| b == 0), packed);
            let img = decode(&tiled, &info).unwrap();
            assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]);
            assert_eq!(&img.pixels[4 * 4..4 * 4 + 4], &[0, 0, 255, 255]);
            assert_eq!(dxt1_blocks(&tiled, &info).unwrap(), pc);
        }
    }

    #[test]
    fn packed_tail_offsets() {
        assert_eq!(packed_base_offset(64, 64), (0, 0));
        assert_eq!(packed_base_offset(16, 16), (16, 0));
        assert_eq!(packed_base_offset(8, 32), (16, 0));
        assert_eq!(packed_base_offset(64, 8), (0, 16));
    }

    #[test]
    fn decodes_argb_and_dxt5_alpha() {
        let info = Info {
            width: 2,
            height: 1,
            format: Format::A8R8G8B8,
            tiled: true,
            packed: false,
        };
        let lin = [0x80, 1, 2, 3, 0xFF, 4, 5, 6];
        let img = decode(&tile(&lin, &info), &info).unwrap();
        assert_eq!(img.pixels, [1, 2, 3, 0x80, 4, 5, 6, 0xFF]);

        let info = Info {
            width: 4,
            height: 4,
            format: Format::Dxt5,
            tiled: false,
            packed: false,
        };
        let mut pc = vec![255, 0, 0, 0, 0, 0, 0, 0]; // alpha all index 0 -> 255
        pc[2] = 1; // texel 0 -> a1 = 0
        pc.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]); // white
        let mut console = pc.clone();
        swap16(&mut console);
        let img = decode(&console, &info).unwrap();
        assert_eq!(&img.pixels[0..8], &[255, 255, 255, 0, 255, 255, 255, 255]);
    }
}
