//! Host preview: runs the real game with the scripted kicker run and renders chosen frames through a
//! small software rasteriser (same `Gfx` trait as the Wii) into PNGs.
//!
//!     cargo run -p fg_preview -- [out_dir] [--wide] [--skater imported/skater.fgpak] [--every N] [--tour]
//!
//! Writes `frame_NNNN.png` for a few interesting moments plus the event log to stdout. It is a
//! stand-in for looking at the game until Dolphin runs are scripted; colours and framing match the Wii
//! build closely, but it is not a GX emulator.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use game_core::Game;
use game_core::autoplay::{KickerRun, TrickTour};
use game_core::input::PadRaw;
mod gxtex;

use game_core::gfx::{Camera, Gfx, SCREEN_H, SCREEN_W, TexVertex, Texture, Vertex};
use game_core::math::{Mat34, Vec3, v3, yaw_dir};
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
    /// Decoded GX textures by data address (pack data lives for the whole run).
    textures: HashMap<usize, Rc<Tex>>,
}

/// Screen x, y, view depth, colour, texture coordinate.
type Screen = (f32, f32, f32, [f32; 4], [f32; 2]);

struct Tex {
    rgba: gxtex::Rgba,
    alpha_test: bool,
}

#[derive(Clone, Copy)]
struct ViewVert {
    p: Vec3, // view space: x right, y up, z forward
    c: [f32; 4],
    uv: [f32; 2],
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
            textures: HashMap::new(),
        }
    }

    /// Fills a screen-space triangle (x, y in pixels; z = view depth for the depth test).
    fn fill(&mut self, v: [Screen; 3], depth_test: bool, tex: Option<&Tex>) {
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
                let mut z = 0.0;
                if depth_test {
                    // Perspective-correct depth: interpolate 1/z.
                    let inv_z = w0 / v[0].2 + w1 / v[1].2 + w2 / v[2].2;
                    z = 1.0 / inv_z;
                    if z >= self.depth[i] {
                        continue;
                    }
                }
                let mut col = [0.0; 4];
                for (k, cv) in col.iter_mut().enumerate() {
                    *cv = w0 * v[0].3[k] + w1 * v[1].3[k] + w2 * v[2].3[k];
                }
                if let Some(t) = tex {
                    // Perspective-correct texture coordinates (uv / z interpolated, times z).
                    let uv: [f32; 2] = std::array::from_fn(|k| {
                        z * (w0 * v[0].4[k] / v[0].2
                            + w1 * v[1].4[k] / v[1].2
                            + w2 * v[2].4[k] / v[2].2)
                    });
                    let texel = t.rgba.sample(uv[0], uv[1]);
                    if t.alpha_test && texel[3] < 128 {
                        continue;
                    }
                    for k in 0..3 {
                        col[k] *= texel[k] as f32 / 255.0;
                    }
                }
                if depth_test {
                    self.depth[i] = z;
                }
                let alpha = col[3];
                for (dst, src) in self.rgb[i].iter_mut().zip(col) {
                    *dst = *dst * (1.0 - alpha) + src * alpha;
                }
            }
        }
    }

    fn project(&self, v: &ViewVert) -> Screen {
        let sx = self.w as f32 / 2.0 + v.p.x / v.p.z * self.focal;
        let sy = self.h as f32 / 2.0 - v.p.y / v.p.z * self.focal;
        (sx, sy, v.p.z, v.c, v.uv)
    }

    /// Clips a view-space triangle against the near plane (giving 0, 1 or 2 triangles) and fills it.
    fn tri(&mut self, t: [ViewVert; 3], tex: Option<&Tex>) {
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
                    uv: core::array::from_fn(|k| a.uv[k] + (b.uv[k] - a.uv[k]) * s),
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
            self.fill(v, true, tex);
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
                    uv: [0.0; 2],
                }
            });
            self.tri([vv[0], vv[1], vv[2]], None);
        }
    }

    fn draw_2d(&mut self, verts: &[Vertex]) {
        let (sx, sy) = (self.w as f32 / SCREEN_W, self.h as f32 / SCREEN_H);
        for t in verts.chunks_exact(3) {
            let v = [0, 1, 2].map(|i| {
                (
                    t[i].pos[0] * sx,
                    t[i].pos[1] * sy,
                    0.0,
                    unpack(t[i].rgba),
                    [0.0; 2],
                )
            });
            self.fill(v, false, None);
        }
    }

    fn draw_indexed(
        &mut self,
        verts: &[TexVertex],
        indices: &[u16],
        texture: Option<&Texture>,
        m: &Mat34,
    ) {
        let tex = texture.and_then(|t| {
            let key = t.data.as_ptr() as usize;
            if let Some(rc) = self.textures.get(&key) {
                return Some(rc.clone());
            }
            let rgba = gxtex::decode(t.data, t.width as usize, t.height as usize, t.format)?;
            let rc = Rc::new(Tex {
                rgba,
                alpha_test: t.alpha_test,
            });
            self.textures.insert(key, rc.clone());
            Some(rc)
        });
        let view: Vec<ViewVert> = verts
            .iter()
            .map(|v| {
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
                    uv: v.uv,
                }
            })
            .collect();
        for t in indices.chunks_exact(3) {
            let tri = [0, 1, 2].map(|k| view[t[k] as usize]);
            self.tri(tri, tex.as_deref());
        }
    }

    fn end_frame(&mut self) {}
}

/// Wraps the rasteriser with a camera placed beside the skater (for looking at the character).
struct CloseUp<'a> {
    inner: &'a mut Raster,
    target: Vec3,
    side: Vec3,
}

impl Gfx for CloseUp<'_> {
    fn begin_frame(&mut self, _cam: &Camera, clear_rgba: u32) {
        let target = self.target + v3(0.0, 0.9, 0.0);
        let cam = Camera {
            eye: target + self.side * 3.2 + v3(0.0, 0.4, 0.0),
            target,
            fov_y_deg: 45.0,
            near: 0.1,
            far: 60.0,
        };
        self.inner.begin_frame(&cam, clear_rgba);
    }
    fn draw_tris(&mut self, verts: &[Vertex], model: &Mat34) {
        self.inner.draw_tris(verts, model)
    }
    fn draw_indexed(
        &mut self,
        verts: &[TexVertex],
        indices: &[u16],
        texture: Option<&Texture>,
        model: &Mat34,
    ) {
        self.inner.draw_indexed(verts, indices, texture, model)
    }
    fn draw_2d(&mut self, _verts: &[Vertex]) {}
    fn end_frame(&mut self) {}
}

/// Wraps the rasteriser with a fixed camera (overviews), drawing the world without the HUD.
struct Fixed<'a> {
    inner: &'a mut Raster,
    cam: Camera,
}

impl Gfx for Fixed<'_> {
    fn begin_frame(&mut self, _cam: &Camera, clear_rgba: u32) {
        self.inner.begin_frame(&self.cam, clear_rgba);
    }
    fn draw_tris(&mut self, verts: &[Vertex], model: &Mat34) {
        self.inner.draw_tris(verts, model)
    }
    fn draw_indexed(
        &mut self,
        verts: &[TexVertex],
        indices: &[u16],
        texture: Option<&Texture>,
        model: &Mat34,
    ) {
        self.inner.draw_indexed(verts, indices, texture, model)
    }
    fn draw_2d(&mut self, _verts: &[Vertex]) {}
    fn end_frame(&mut self) {}
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wide = args.iter().any(|a| a == "--wide");
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let skater = value("--skater");
    let every: Option<u32> = value("--every").map(|v| v.parse().expect("--every <frames>"));
    let out = args
        .iter()
        .enumerate()
        .find(|(i, a)| {
            !a.starts_with("--")
                && (*i == 0 || !matches!(args[i - 1].as_str(), "--skater" | "--every"))
        })
        .map(|(_, a)| PathBuf::from(a))
        .unwrap_or_else(|| PathBuf::from("out/preview"));
    fs::create_dir_all(&out).expect("create output dir");

    // The Wii renders 640x480 and the TV stretches it in 16:9; preview square pixels instead.
    let mut raster = if wide {
        Raster::new(854, 480)
    } else {
        Raster::new(640, 480)
    };
    let mut game = Game::new();
    if let Some(path) = skater {
        // Leaked like the Wii does: the pack stays loaded for the whole game.
        let bytes: &'static [u8] = Vec::leak(fs::read(path).expect("read skater pack"));
        game.load_skater(bytes).expect("skater pack");
        println!("using imported skater {path}");
    }
    // --park: ride an imported park (fg_import park) instead of the demo park.
    let park = value("--park");
    if let Some(path) = park {
        let bytes = fs::read(path).expect("read park pack");
        game.load_park(&bytes).expect("park pack");
        let p = game.imported_park().expect("loaded");
        println!(
            "using imported park {path}: {} triangles, {} chunks, {} vertices drawn at most",
            p.mesh.tris.len(),
            p.chunks.len(),
            p.vertex_count()
        );
        // Overview from high above the middle, looking down.
        let (min, max) = (p.mesh.min, p.mesh.max);
        let mid = (min + max) * 0.5;
        let span = (max.x - min.x).max(max.z - min.z);
        let cam = Camera {
            eye: v3(mid.x, max.y + span * 0.9, mid.z - span * 0.45),
            target: v3(mid.x, min.y, mid.z),
            fov_y_deg: 60.0,
            near: 1.0,
            far: span * 4.0,
        };
        // Without the roof: only triangles below 10 m above the start.
        let cut = game.terrain().spawn().0.y + 10.0;
        let low: Vec<Vertex> = p
            .chunks
            .iter()
            .flat_map(|c| c.verts.chunks_exact(3))
            .filter(|t| t.iter().all(|v| v.pos[1] < cut))
            .flatten()
            .copied()
            .collect();
        let mut fixed = Fixed {
            inner: &mut raster,
            cam,
        };
        fixed.begin_frame(&cam, 0x202430ff);
        fixed.draw_tris(&low, &game_core::math::Xform::IDENTITY.to_mat34());
        let path = out.join("park_overview.png");
        raster.save(&path);
        println!("        wrote {}", path.display());
    }
    // --grind-test: drop the skater onto the park's longest fairly level rail, moving along it.
    if args.iter().any(|a| a == "--grind-test") {
        let rails = game.terrain().rails();
        let level = |r: &game_core::rails::Rail| {
            let (a, b) = (r.points[0], *r.points.last().unwrap());
            (b.y - a.y).abs() < 0.15 * r.length()
        };
        if let Some(r) = rails
            .iter()
            .filter(|r| level(r))
            .max_by(|a, b| a.length().total_cmp(&b.length()))
            .cloned()
        {
            let r = &r;
            let (p, d) = r.at(0.5);
            let s = &mut game.skater;
            s.pos = p + d * -0.6 + v3(0.0, 0.25, 0.0);
            s.heading = d.x.atan2(d.z);
            s.vel = d * 5.0 + v3(0.0, -0.5, 0.0);
            s.mode = Mode::Air { time: 0.5 };
            println!(
                "grind test: rail of {:.1} m starting at {:.1?}",
                r.length(),
                [p.x, p.y, p.z]
            );
        }
    }
    // --tour: the whole move set (carves, manual, grab, bail) after the kicker run.
    let tour = args.iter().any(|a| a == "--tour");
    let mut kicker = KickerRun::new();
    let mut trick_tour = TrickTour::new();
    let mut done_at: Option<u32> = None;
    let dt = 1.0 / 60.0;

    let mut shots: Vec<(u32, &str)> = vec![(45, "rolling")];
    let mut took_air = false;
    let mut took_land = false;
    let grind_test = args.iter().any(|a| a == "--grind-test");
    let frames = if grind_test {
        60 * 6
    } else if park.is_some() {
        60 * 30
    } else if tour {
        60 * 20
    } else {
        60 * 7
    };
    for frame in 0..frames {
        let pad = if park.is_some() {
            // Push up to a cruise and weave gently: enough to see the park and its collision.
            let s = &game.skater;
            PadRaw {
                flags: game_core::input::flag::CONNECTED,
                buttons: if !grind_test && s.speed < 4.5 && frame % 30 == 0 {
                    game_core::input::btn::A
                } else {
                    0
                },
                stick_x: if grind_test {
                    0
                } else {
                    (40.0 * (frame as f32 * 0.02).sin()) as i8
                },
                ..Default::default()
            }
        } else if tour {
            trick_tour.next(&game)
        } else {
            kicker.next(&game)
        };
        if trick_tour.done() {
            // Let the bail play out, then stop.
            let at = *done_at.get_or_insert(frame);
            if frame > at + 150 {
                break;
            }
        }
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
        let label = shots
            .iter()
            .find(|(f, _)| *f == frame)
            .map(|(_, l)| *l)
            .or_else(|| every.filter(|n| frame % n == 0).map(|_| "every"));
        if let Some(label) = label {
            let clip = game
                .rider()
                .map(|r| format!(", clip {} @ {:.2}s", r.current().role, r.current().time))
                .unwrap_or_default();
            if label != "every" || park.is_some() {
                game.render(&mut raster);
                let path = out.join(format!("frame_{frame:04}.png"));
                raster.save(&path);
                println!("        wrote {} ({label}{clip})", path.display());
            }
            if game.rider().is_some() {
                let side = yaw_dir(game.skater.heading + std::f32::consts::FRAC_PI_2);
                game.render(&mut CloseUp {
                    inner: &mut raster,
                    target: game.skater.pos,
                    side,
                });
                let path = out.join(format!("close_{frame:04}.png"));
                raster.save(&path);
                println!("        wrote {} ({label}{clip})", path.display());
            }
        }
    }
    println!("score: {}", game.score);
}
