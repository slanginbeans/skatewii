//! Host preview: runs the real game with the scripted kicker run and renders chosen frames through a
//! small software rasteriser (same `Gfx` trait as the Wii) into PNGs.
//!
//!     cargo run -p fg_preview -- [out_dir] [--wide]
//!
//! Writes `frame_NNNN.png` for a few interesting moments plus the event log to stdout. It is a
//! stand-in for looking at the game until Dolphin runs are scripted; colours and framing match the Wii
//! build closely, but it is not a GX emulator.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use game_core::Game;
use game_core::autoplay::KickerRun;
use game_core::gfx::{Camera, Gfx, SCREEN_H, SCREEN_W, Vertex};
use game_core::math::{Mat34, Vec3, v3};
use game_core::skater::{Event, Mode};

struct Raster {
    w: usize,
    h: usize,
    rgb: Vec<[f32; 3]>,
    depth: Vec<f32>,
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
    focal: f32,
    near: f32,
}

#[derive(Clone, Copy)]
struct ViewVert {
    p: Vec3, // view space: x right, y up, z forward
    c: [f32; 4],
}

fn unpack(rgba: u32) -> [f32; 4] {
    [
        (rgba >> 24) as u8 as f32,
        (rgba >> 16) as u8 as f32,
        (rgba >> 8) as u8 as f32,
        rgba as u8 as f32 / 255.0,
    ]
}

impl Raster {
    fn new(w: usize, h: usize) -> Raster {
        Raster {
            w,
            h,
            rgb: vec![[0.0; 3]; w * h],
            depth: vec![f32::INFINITY; w * h],
            eye: Vec3::ZERO,
            right: v3(1.0, 0.0, 0.0),
            up: Vec3::UP,
            fwd: v3(0.0, 0.0, 1.0),
            focal: 1.0,
            near: 0.1,
        }
    }

    /// Fills a screen-space triangle (x, y in pixels; z = view depth for the depth test).
    fn fill(&mut self, v: [(f32, f32, f32, [f32; 4]); 3], depth_test: bool) {
        let (minx, maxx) = (
            v.iter().map(|p| p.0).fold(f32::MAX, f32::min),
            v.iter().map(|p| p.0).fold(f32::MIN, f32::max),
        );
        let (miny, maxy) = (
            v.iter().map(|p| p.1).fold(f32::MAX, f32::min),
            v.iter().map(|p| p.1).fold(f32::MIN, f32::max),
        );
        let x0 = minx.floor().max(0.0) as usize;
        let x1 = (maxx.ceil() as isize).min(self.w as isize - 1);
        let y0 = miny.floor().max(0.0) as usize;
        let y1 = (maxy.ceil() as isize).min(self.h as isize - 1);
        if x1 < 0 || y1 < 0 {
            return;
        }
        let edge = |a: (f32, f32), b: (f32, f32), p: (f32, f32)| {
            (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
        };
        let (a, b, c) = ((v[0].0, v[0].1), (v[1].0, v[1].1), (v[2].0, v[2].1));
        let area = edge(a, b, c);
        if area.abs() < 1e-8 {
            return;
        }
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let (w0, w1, w2) = (
                    edge(b, c, p) / area,
                    edge(c, a, p) / area,
                    edge(a, b, p) / area,
                );
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let i = y * self.w + x;
                if depth_test {
                    // Perspective-correct depth: interpolate 1/z.
                    let inv_z = w0 / v[0].2 + w1 / v[1].2 + w2 / v[2].2;
                    let z = 1.0 / inv_z;
                    if z >= self.depth[i] {
                        continue;
                    }
                    self.depth[i] = z;
                }
                let mut col = [0.0; 4];
                for (k, cv) in col.iter_mut().enumerate() {
                    *cv = w0 * v[0].3[k] + w1 * v[1].3[k] + w2 * v[2].3[k];
                }
                let alpha = col[3];
                for (dst, src) in self.rgb[i].iter_mut().zip(col) {
                    *dst = *dst * (1.0 - alpha) + src * alpha;
                }
            }
        }
    }

    fn project(&self, v: &ViewVert) -> (f32, f32, f32, [f32; 4]) {
        let sx = self.w as f32 / 2.0 + v.p.x / v.p.z * self.focal;
        let sy = self.h as f32 / 2.0 - v.p.y / v.p.z * self.focal;
        (sx, sy, v.p.z, v.c)
    }

    /// Clips a view-space triangle against the near plane (giving 0, 1 or 2 triangles) and fills it.
    fn tri(&mut self, t: [ViewVert; 3]) {
        let near = self.near;
        let mut poly: Vec<ViewVert> = Vec::with_capacity(4);
        for i in 0..3 {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            let (ina, inb) = (a.p.z >= near, b.p.z >= near);
            if ina {
                poly.push(a);
            }
            if ina != inb {
                let s = (near - a.p.z) / (b.p.z - a.p.z);
                let c: [f32; 4] = core::array::from_fn(|k| a.c[k] + (b.c[k] - a.c[k]) * s);
                poly.push(ViewVert {
                    p: a.p.lerp(b.p, s),
                    c,
                });
            }
        }
        for i in 1..poly.len().saturating_sub(1) {
            let v = [
                self.project(&poly[0]),
                self.project(&poly[i]),
                self.project(&poly[i + 1]),
            ];
            self.fill(v, true);
        }
    }

    fn save(&self, path: &Path) {
        let mut data = Vec::with_capacity(self.w * self.h * 3);
        for p in &self.rgb {
            data.extend(p.iter().map(|c| c.clamp(0.0, 255.0) as u8));
        }
        let file = BufWriter::new(File::create(path).expect("create png"));
        let mut enc = png::Encoder::new(file, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .and_then(|mut w| w.write_image_data(&data))
            .expect("write png");
    }
}

impl Gfx for Raster {
    fn begin_frame(&mut self, cam: &Camera, clear_rgba: u32) {
        let c = unpack(clear_rgba);
        self.rgb.fill([c[0], c[1], c[2]]);
        self.depth.fill(f32::INFINITY);
        self.eye = cam.eye;
        self.fwd = (cam.target - cam.eye).normalize_or(v3(0.0, 0.0, 1.0));
        self.right = self.fwd.cross(Vec3::UP).normalize_or(v3(1.0, 0.0, 0.0));
        self.up = self.right.cross(self.fwd);
        self.focal = (self.h as f32 / 2.0) / (cam.fov_y_deg.to_radians() / 2.0).tan();
        self.near = cam.near;
    }

    fn draw_tris(&mut self, verts: &[Vertex], m: &Mat34) {
        for t in verts.chunks_exact(3) {
            let vv = [t[0], t[1], t[2]].map(|v| {
                let [x, y, z] = v.pos;
                let w = v3(
                    m[0][0] * x + m[0][1] * y + m[0][2] * z + m[0][3],
                    m[1][0] * x + m[1][1] * y + m[1][2] * z + m[1][3],
                    m[2][0] * x + m[2][1] * y + m[2][2] * z + m[2][3],
                );
                let d = w - self.eye;
                ViewVert {
                    p: v3(d.dot(self.right), d.dot(self.up), d.dot(self.fwd)),
                    c: unpack(v.rgba),
                }
            });
            self.tri([vv[0], vv[1], vv[2]]);
        }
    }

    fn draw_2d(&mut self, verts: &[Vertex]) {
        let (sx, sy) = (self.w as f32 / SCREEN_W, self.h as f32 / SCREEN_H);
        for t in verts.chunks_exact(3) {
            let v = [0, 1, 2].map(|i| (t[i].pos[0] * sx, t[i].pos[1] * sy, 0.0, unpack(t[i].rgba)));
            self.fill(v, false);
        }
    }

    fn end_frame(&mut self) {}
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wide = args.iter().any(|a| a == "--wide");
    let out = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out/preview"));
    fs::create_dir_all(&out).expect("create output dir");

    // The Wii renders 640x480 and the TV stretches it in 16:9; preview square pixels instead.
    let mut raster = if wide {
        Raster::new(854, 480)
    } else {
        Raster::new(640, 480)
    };
    let mut game = Game::new();
    let mut script = KickerRun::new();
    let dt = 1.0 / 60.0;

    let mut shots: Vec<(u32, &str)> = vec![(45, "rolling")];
    let mut took_air = false;
    let mut took_land = false;
    for frame in 0..(60 * 7) {
        let pad = script.next(&game);
        game.update(&pad, dt);
        for e in &game.events {
            println!("{:>5.2}s  {e:?}", frame as f32 * dt);
            if matches!(e, Event::Landed { .. }) && !took_land {
                shots.push((frame + 10, "landed"));
                took_land = true;
            }
        }
        if let (Mode::Air { .. }, Some(t)) = (game.skater.mode, &game.skater.trick)
            && !took_air
            && t.progress() > 0.3
        {
            shots.push((frame, "mid-flip"));
            took_air = true;
        }
        if let Some((_, label)) = shots.iter().find(|(f, _)| *f == frame) {
            game.render(&mut raster);
            let path = out.join(format!("frame_{frame:04}.png"));
            raster.save(&path);
            println!("        wrote {} ({label})", path.display());
        }
    }
    println!("score: {}", game.score);
}
