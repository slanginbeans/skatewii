//! Decodes the GX texture formats imported packs use (CMPR, RGB5A3, RGBA8) back to RGBA for the
//! software rasteriser. Mirrors the encoders in `tools/import/src/gx.rs`.

pub const GX_TF_RGB5A3: u8 = 5;
pub const GX_TF_RGBA8: u8 = 6;
pub const GX_TF_CMPR: u8 = 14;

pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 4]>,
}

impl Rgba {
    /// Nearest texel, wrapping (GX_REPEAT).
    pub fn sample(&self, u: f32, v: f32) -> [u8; 4] {
        let x = (u.rem_euclid(1.0) * self.width as f32) as usize % self.width;
        let y = (v.rem_euclid(1.0) * self.height as f32) as usize % self.height;
        self.pixels[y * self.width + x]
    }
}

fn be16(d: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([d[at], d[at + 1]])
}

fn rgb565(c: u16) -> [u8; 4] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [
        (r << 3 | r >> 2) as u8,
        (g << 2 | g >> 4) as u8,
        (b << 3 | b >> 2) as u8,
        255,
    ]
}

pub fn decode(data: &[u8], width: usize, height: usize, format: u8) -> Option<Rgba> {
    let mut px = vec![[0u8; 4]; width * height];
    let mut put = |x: usize, y: usize, c: [u8; 4]| {
        if x < width && y < height {
            px[y * width + x] = c;
        }
    };
    let (tw, th) = match format {
        GX_TF_CMPR => (8, 8),
        GX_TF_RGB5A3 | GX_TF_RGBA8 => (4, 4),
        _ => return None,
    };
    let (tiles_x, tiles_y) = (width.div_ceil(tw), height.div_ceil(th));
    let mut at = 0;
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let (x0, y0) = (tx * tw, ty * th);
            match format {
                GX_TF_CMPR => {
                    for sub in 0..4 {
                        let b = data.get(at..at + 8)?;
                        at += 8;
                        let (c0, c1) = (be16(b, 0), be16(b, 2));
                        let (p0, p1) = (rgb565(c0), rgb565(c1));
                        let mix = |a: u8, b: u8, wa: u16, wb: u16, d: u16| {
                            ((a as u16 * wa + b as u16 * wb) / d) as u8
                        };
                        let pal: [[u8; 4]; 4] = if c0 > c1 {
                            [
                                p0,
                                p1,
                                core::array::from_fn(|k| mix(p0[k], p1[k], 2, 1, 3)),
                                core::array::from_fn(|k| mix(p0[k], p1[k], 1, 2, 3)),
                            ]
                        } else {
                            [
                                p0,
                                p1,
                                core::array::from_fn(|k| mix(p0[k], p1[k], 1, 1, 2)),
                                [0, 0, 0, 0],
                            ]
                        };
                        let (sx, sy) = (x0 + (sub % 2) * 4, y0 + (sub / 2) * 4);
                        for row in 0..4 {
                            let bits = b[4 + row];
                            for col in 0..4 {
                                let i = (bits >> (6 - 2 * col)) & 3;
                                put(sx + col, sy + row, pal[i as usize]);
                            }
                        }
                    }
                }
                GX_TF_RGB5A3 => {
                    for i in 0..16 {
                        let c = be16(data.get(at..at + 2)?, 0);
                        at += 2;
                        let rgba = if c & 0x8000 != 0 {
                            let f = |s: u16| {
                                let v = (c >> s) & 31;
                                (v << 3 | v >> 2) as u8
                            };
                            [f(10), f(5), f(0), 255]
                        } else {
                            let f = |s: u16| ((c >> s) & 15) as u8 * 17;
                            let a = ((c >> 12) & 7) as u8;
                            [f(8), f(4), f(0), a << 5 | a << 2 | a >> 1]
                        };
                        put(x0 + i % 4, y0 + i / 4, rgba);
                    }
                }
                _ => {
                    let b = data.get(at..at + 64)?;
                    at += 64;
                    for i in 0..16 {
                        let (ar, gb) = (&b[2 * i..2 * i + 2], &b[32 + 2 * i..34 + 2 * i]);
                        put(x0 + i % 4, y0 + i / 4, [ar[1], gb[0], gb[1], ar[0]]);
                    }
                }
            }
        }
    }
    Some(Rgba {
        width,
        height,
        pixels: px,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rgb5a3_opaque_and_translucent() {
        // One 4x4 tile: first texel opaque red, second half-ish transparent white.
        let mut d = vec![0u8; 32];
        d[0..2].copy_from_slice(&0xFC00u16.to_be_bytes());
        d[2..4].copy_from_slice(&0x4FFFu16.to_be_bytes());
        let t = decode(&d, 4, 4, GX_TF_RGB5A3).unwrap();
        assert_eq!(t.pixels[0], [255, 0, 0, 255]);
        assert_eq!(t.pixels[1][..3], [255, 255, 255]);
        assert!(t.pixels[1][3] > 100 && t.pixels[1][3] < 160);
    }

    #[test]
    fn decodes_a_cmpr_block() {
        // 8x8 tile, all four sub-blocks: colour0 white, colour1 black, every index 0 -> white.
        let mut d = vec![];
        for _ in 0..4 {
            d.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0, 0, 0, 0]);
        }
        let t = decode(&d, 8, 8, GX_TF_CMPR).unwrap();
        assert!(t.pixels.iter().all(|p| *p == [255, 255, 255, 255]));
    }
}
