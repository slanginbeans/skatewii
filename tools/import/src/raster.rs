//! A tiny z-buffered software rasteriser for checking converted characters on the PC (PNG previews only).

use crate::texture::Rgba;

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub color: Vec<u8>,
    depth: Vec<f32>,
}

pub struct Surface<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub uvs: &'a [[f32; 2]],
    pub indices: &'a [u16],
    pub texture: Option<&'a Rgba>,
    pub tint: [f32; 3],
    /// Discard texels with alpha < 128 (only for textures whose alpha is coverage).
    pub alpha_test: bool,
}

/// Orthographic view: `right`, `up` and `forward` axes (world space), centre and metres per canvas height.
pub struct View {
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub center: [f32; 3],
    pub span: f32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Canvas {
    pub fn new(width: usize, height: usize, background: [u8; 3]) -> Self {
        let mut color = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            color.extend_from_slice(&[background[0], background[1], background[2], 255]);
        }
        Canvas {
            width,
            height,
            color,
            depth: vec![f32::INFINITY; width * height],
        }
    }

    pub fn draw(&mut self, s: &Surface, view: &View, x0: usize, w: usize) {
        let scale = self.height as f32 / view.span;
        let project = |p: [f32; 3]| {
            let d = [
                p[0] - view.center[0],
                p[1] - view.center[1],
                p[2] - view.center[2],
            ];
            [
                x0 as f32 + w as f32 / 2.0 + dot(d, view.right) * scale,
                self.height as f32 / 2.0 - dot(d, view.up) * scale,
                dot(d, view.forward),
            ]
        };
        let light = {
            let l = [0.4f32, 0.8, -0.45];
            let n = dot(l, l).sqrt();
            l.map(|c| c / n)
        };
        for t in s.indices.chunks_exact(3) {
            let v = [t[0] as usize, t[1] as usize, t[2] as usize];
            let p = v.map(|i| project(s.positions[i]));
            let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            if area.abs() < 1e-9 {
                continue;
            }
            let (minx, maxx) = (
                p.iter().map(|q| q[0]).fold(f32::MAX, f32::min),
                p.iter().map(|q| q[0]).fold(f32::MIN, f32::max),
            );
            let (miny, maxy) = (
                p.iter().map(|q| q[1]).fold(f32::MAX, f32::min),
                p.iter().map(|q| q[1]).fold(f32::MIN, f32::max),
            );
            let (x_lo, x_hi) = (
                minx.max(x0 as f32).floor() as usize,
                maxx.min((x0 + w) as f32 - 1.0).ceil() as usize,
            );
            let (y_lo, y_hi) = (
                miny.max(0.0).floor() as usize,
                maxy.min(self.height as f32 - 1.0).ceil() as usize,
            );
            for y in y_lo..=y_hi.min(self.height - 1) {
                for x in x_lo..=x_hi.min(x0 + w - 1) {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let b0 =
                        ((p[1][0] - px) * (p[2][1] - py) - (p[2][0] - px) * (p[1][1] - py)) / area;
                    let b1 =
                        ((p[2][0] - px) * (p[0][1] - py) - (p[0][0] - px) * (p[2][1] - py)) / area;
                    let b2 = 1.0 - b0 - b1;
                    if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                        continue;
                    }
                    let z = b0 * p[0][2] + b1 * p[1][2] + b2 * p[2][2];
                    let i = y * self.width + x;
                    if z >= self.depth[i] {
                        continue;
                    }
                    let lerp3 = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| -> [f32; 3] {
                        std::array::from_fn(|k| b0 * a[k] + b1 * b[k] + b2 * c[k])
                    };
                    let mut n = lerp3(s.normals[v[0]], s.normals[v[1]], s.normals[v[2]]);
                    let len = dot(n, n).sqrt().max(1e-6);
                    n = n.map(|c| c / len);
                    let shade = 0.35 + 0.65 * dot(n, light).abs();
                    let base = match s.texture {
                        Some(tex) => {
                            let uv: [f32; 2] = std::array::from_fn(|k| {
                                b0 * s.uvs[v[0]][k] + b1 * s.uvs[v[1]][k] + b2 * s.uvs[v[2]][k]
                            });
                            let tx =
                                ((uv[0].rem_euclid(1.0)) * tex.width as f32) as usize % tex.width;
                            let ty =
                                ((uv[1].rem_euclid(1.0)) * tex.height as f32) as usize % tex.height;
                            let o = (ty * tex.width + tx) * 4;
                            if s.alpha_test && tex.pixels[o + 3] < 128 {
                                continue;
                            }
                            [
                                tex.pixels[o] as f32,
                                tex.pixels[o + 1] as f32,
                                tex.pixels[o + 2] as f32,
                            ]
                        }
                        None => [200.0, 200.0, 200.0],
                    };
                    self.depth[i] = z;
                    for (k, (b, t)) in base.iter().zip(s.tint).enumerate() {
                        self.color[i * 4 + k] = (b * t * shade).clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }

    pub fn into_rgba(self) -> Rgba {
        Rgba {
            width: self.width,
            height: self.height,
            pixels: self.color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_triangle_with_depth() {
        let mut c = Canvas::new(8, 8, [0, 0, 0]);
        let view = View {
            right: [1., 0., 0.],
            up: [0., 1., 0.],
            forward: [0., 0., 1.],
            center: [0.; 3],
            span: 2.0,
        };
        let pos = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
        let n = [[0.0, 0.0, -1.0]; 3];
        let uv = [[0.0; 2]; 3];
        c.draw(
            &Surface {
                positions: &pos,
                normals: &n,
                uvs: &uv,
                indices: &[0, 1, 2],
                texture: None,
                tint: [1.0; 3],
                alpha_test: false,
            },
            &view,
            0,
            8,
        );
        let centre = (4 * 8 + 4) * 4;
        assert!(c.color[centre] > 0);
        assert_eq!(c.color[0], 0, "corner stays background");
    }
}
