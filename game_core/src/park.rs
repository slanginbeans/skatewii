//! The skate park: a height field made of simple shapes. The board rides on top of the highest shape
//! under it; anything that rises more than a step above the board is a wall.
//!
//! The demo park is placeholder geometry defined in code. A real level pipeline (your own models,
//! converted to Wii packs on the PC) is ROADMAP phase 3.

use alloc::vec::Vec;

use crate::gfx::{MeshBuilder, Vertex, rgb};
use crate::math::{Vec3, v3};
use crate::rails::Rail;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    X,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Flat-topped box: ledge, manual pad, funbox deck.
    Block { min: Vec3, max: Vec3, rgba: u32 },
    /// Wedge over the rectangle `min..max` (x/z), rising from `h0` at the low end of `axis` to `h1`
    /// at the high end. Kickers, banks, funbox ramps.
    Ramp {
        min: Vec3,
        max: Vec3,
        axis: Axis,
        h0: f32,
        h1: f32,
        rgba: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub height: f32,
    pub normal: Vec3,
}

/// What the board rides on: the built-in demo park (a height field) or an imported park
/// (`collision::TriMesh`, triangles at any level).
pub trait Terrain {
    /// The surface at (x, z) that the board would stand on: the highest one no higher than `max`. A
    /// height field has one surface per point and ignores `max` (anything above `max` there is a wall).
    /// Nothing below `max`: height `f32::MIN`.
    fn ground(&self, x: f32, z: f32, max: f32) -> Surface;
    /// Is a move from `a` to `b` (board positions) blocked by a wall?
    fn blocked(&self, _a: Vec3, _b: Vec3) -> bool {
        false
    }
    /// Is there a wall on the straight line from `p` to `q` (the camera's line of sight)?
    fn occluded(&self, _p: Vec3, _q: Vec3) -> bool {
        false
    }
    /// Where the skater starts: position and yaw.
    fn spawn(&self) -> (Vec3, f32);
    /// Falling below this means out of the world.
    fn kill_height(&self) -> f32 {
        -5.0
    }
    /// Grind rails.
    fn rails(&self) -> &[Rail] {
        &[]
    }
    /// The first surface a ray from `from` along the unit `dir` meets within `max`: how far, where, and
    /// the surface normal (facing back along the ray). A height field is marched: its ground faces up and
    /// a rise between two steps is a wall facing the ray.
    fn raycast(&self, from: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        let top = |p: Vec3| self.ground(p.x, p.z, p.y + 0.5);
        let below = |p: Vec3| p.y < top(p).height - 1e-4;
        if below(from) {
            return None;
        }
        let step = 0.04;
        let n = (libm::ceilf(max / step) as usize).clamp(1, 400);
        let mut prev = from;
        for i in 1..=n {
            let t = (i as f32 * step).min(max);
            let p = from + dir * t;
            if below(p) {
                // Narrow down the crossing.
                let (mut lo, mut hi) = (t - step.min(t), t);
                for _ in 0..10 {
                    let mid = (lo + hi) * 0.5;
                    if below(from + dir * mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                let at = from + dir * hi;
                // Came in through the top, or through the side of something higher?
                let was = top(prev).height;
                let s = top(at);
                let normal = if was + 0.02 < s.height && prev.y < s.height - 0.02 {
                    let h = v3(-dir.x, 0.0, -dir.z);
                    h.normalize_or(Vec3::UP)
                } else {
                    s.normal
                };
                return Some(Hit {
                    t: hi,
                    point: at,
                    normal,
                    surface: 0,
                });
            }
            prev = p;
        }
        None
    }
}

/// Where a ray met a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub t: f32,
    pub point: Vec3,
    /// Unit surface normal, facing back along the ray.
    pub normal: Vec3,
    /// The collision surface id (0: none); its category is bits 7..11 ([`surface_category`]).
    pub surface: u16,
}

/// Skate 3's surface category of a collision surface id (bits 7..11, SK8 `packed_surface >> 7 & 31`):
/// 8 is stairs, 6 unrideable, 7 and 8 surfaces the board doesn't align to.
pub fn surface_category(surface: u16) -> u32 {
    (u32::from(surface) >> 7) & 31
}

/// The stairs category.
pub const STAIRS: u32 = 8;

impl Terrain for Park {
    fn ground(&self, x: f32, z: f32, _max: f32) -> Surface {
        self.surface(x, z)
    }
    fn spawn(&self) -> (Vec3, f32) {
        (self.spawn, self.spawn_yaw)
    }
    fn rails(&self) -> &[Rail] {
        &self.rails
    }
}

pub struct Park {
    pub shapes: Vec<Shape>,
    /// Rideable area is `-half_size..half_size` in x and z; outside it is a wall.
    pub half_size: f32,
    pub wall_height: f32,
    pub spawn: Vec3,
    pub spawn_yaw: f32,
    /// Grind rails (the ledge's top).
    pub rails: Vec<Rail>,
}

fn inside(x: f32, z: f32, min: Vec3, max: Vec3) -> bool {
    x >= min.x && x <= max.x && z >= min.z && z <= max.z
}

impl Shape {
    fn surface(&self, x: f32, z: f32) -> Option<Surface> {
        match *self {
            Shape::Block { min, max, .. } => inside(x, z, min, max).then_some(Surface {
                height: max.y,
                normal: Vec3::UP,
            }),
            Shape::Ramp {
                min,
                max,
                axis,
                h0,
                h1,
                ..
            } => {
                if !inside(x, z, min, max) {
                    return None;
                }
                let (t, len) = match axis {
                    Axis::X => ((x - min.x) / (max.x - min.x), max.x - min.x),
                    Axis::Z => ((z - min.z) / (max.z - min.z), max.z - min.z),
                };
                let slope = (h1 - h0) / len;
                let normal = match axis {
                    Axis::X => v3(-slope, 1.0, 0.0),
                    Axis::Z => v3(0.0, 1.0, -slope),
                }
                .normalize_or(Vec3::UP);
                Some(Surface {
                    height: h0 + (h1 - h0) * t,
                    normal,
                })
            }
        }
    }

    fn build(&self, m: &mut MeshBuilder) {
        match *self {
            Shape::Block { min, max, rgba } => m.cuboid(min, max, rgba),
            Shape::Ramp {
                min,
                max,
                axis,
                h0,
                h1,
                rgba,
            } => {
                let side = rgb(110, 92, 70);
                // Corner heights; "lo"/"hi" are the two ends along the ramp's axis.
                let (x0, x1, z0, z1) = (min.x, max.x, min.z, max.z);
                let (y0, y1) = (min.y, min.y);
                let p = v3;
                match axis {
                    Axis::Z => {
                        m.quad(
                            p(x0, h0, z0),
                            p(x0, h1, z1),
                            p(x1, h1, z1),
                            p(x1, h0, z0),
                            rgba,
                        );
                        m.quad(
                            p(x1, y0, z0),
                            p(x1, h0, z0),
                            p(x1, h1, z1),
                            p(x1, y1, z1),
                            side,
                        );
                        m.quad(
                            p(x0, y1, z1),
                            p(x0, h1, z1),
                            p(x0, h0, z0),
                            p(x0, y0, z0),
                            side,
                        );
                        m.quad(
                            p(x1, y1, z1),
                            p(x1, h1, z1),
                            p(x0, h1, z1),
                            p(x0, y1, z1),
                            side,
                        );
                        m.quad(
                            p(x0, y0, z0),
                            p(x0, h0, z0),
                            p(x1, h0, z0),
                            p(x1, y0, z0),
                            side,
                        );
                    }
                    Axis::X => {
                        m.quad(
                            p(x0, h0, z0),
                            p(x0, h0, z1),
                            p(x1, h1, z1),
                            p(x1, h1, z0),
                            rgba,
                        );
                        m.quad(
                            p(x0, y0, z1),
                            p(x0, h0, z1),
                            p(x1, h1, z1),
                            p(x1, y1, z1),
                            side,
                        );
                        m.quad(
                            p(x1, y1, z0),
                            p(x1, h1, z0),
                            p(x0, h0, z0),
                            p(x0, y0, z0),
                            side,
                        );
                        m.quad(
                            p(x1, y1, z0),
                            p(x1, y1, z1),
                            p(x1, h1, z1),
                            p(x1, h1, z0),
                            side,
                        );
                        m.quad(
                            p(x0, y0, z1),
                            p(x0, y0, z0),
                            p(x0, h0, z0),
                            p(x0, h0, z1),
                            side,
                        );
                    }
                }
            }
        }
    }
}

const WOOD: u32 = rgb(196, 160, 112);
const CONCRETE: u32 = rgb(176, 176, 170);
const PAINT: u32 = rgb(206, 72, 56);
const STEEL: u32 = rgb(120, 130, 140);

impl Park {
    /// A small original plaza: a kicker in the run-in lane, a funbox, a ledge, a manual pad and a bank.
    pub fn demo() -> Park {
        let s = alloc::vec![
            // Kicker in the spawn lane: lip at z = -2.5.
            Shape::Ramp {
                min: v3(-1.5, 0.0, -4.0),
                max: v3(1.5, 0.0, -2.5),
                axis: Axis::Z,
                h0: 0.0,
                h1: 0.55,
                rgba: WOOD
            },
            // Funbox: up ramp, deck, down ramp.
            Shape::Ramp {
                min: v3(6.0, 0.0, 0.0),
                max: v3(10.0, 0.0, 2.0),
                axis: Axis::Z,
                h0: 0.0,
                h1: 0.6,
                rgba: WOOD
            },
            Shape::Block {
                min: v3(6.0, 0.0, 2.0),
                max: v3(10.0, 0.6, 5.0),
                rgba: CONCRETE
            },
            Shape::Ramp {
                min: v3(6.0, 0.0, 5.0),
                max: v3(10.0, 0.0, 7.0),
                axis: Axis::Z,
                h0: 0.6,
                h1: 0.0,
                rgba: WOOD
            },
            // Long ledge with a painted top (grinds come later).
            Shape::Block {
                min: v3(-10.0, 0.0, -6.0),
                max: v3(-9.4, 0.4, 6.0),
                rgba: PAINT
            },
            // Manual pad: too tall to roll onto, ollie up.
            Shape::Block {
                min: v3(-5.0, 0.0, 6.0),
                max: v3(-2.5, 0.18, 9.5),
                rgba: CONCRETE
            },
            // Bank against the east wall.
            Shape::Ramp {
                min: v3(14.0, 0.0, -10.0),
                max: v3(18.0, 0.0, 10.0),
                axis: Axis::X,
                h0: 0.0,
                h1: 1.6,
                rgba: STEEL
            },
        ];
        Park {
            shapes: s,
            half_size: 20.0,
            wall_height: 1.2,
            spawn: v3(0.0, 0.0, -14.0),
            spawn_yaw: 0.0,
            // Along the middle of the long ledge's top.
            rails: alloc::vec![Rail::new(alloc::vec![
                v3(-9.7, 0.4, -6.0),
                v3(-9.7, 0.4, 6.0)
            ])],
        }
    }

    /// The surface under (x, z): the highest shape, the ground, or a wall outside the park.
    pub fn surface(&self, x: f32, z: f32) -> Surface {
        if x.abs() > self.half_size || z.abs() > self.half_size {
            return Surface {
                height: self.wall_height,
                normal: Vec3::UP,
            };
        }
        let ground = Surface {
            height: 0.0,
            normal: Vec3::UP,
        };
        self.shapes
            .iter()
            .filter_map(|s| s.surface(x, z))
            .fold(
                ground,
                |best, s| if s.height > best.height { s } else { best },
            )
    }

    /// Pre-lit triangle list for the whole park (built once at load).
    pub fn build_mesh(&self) -> Vec<Vertex> {
        let mut m = MeshBuilder::new();
        // Ground: 2 m concrete tiles in a subtle checker.
        let tile = 2.0;
        let n = (self.half_size / tile) as i32;
        for i in -n..n {
            for j in -n..n {
                let (x, z) = (i as f32 * tile, j as f32 * tile);
                let c = if (i + j) & 1 == 0 {
                    rgb(150, 150, 146)
                } else {
                    rgb(140, 141, 138)
                };
                m.quad(
                    v3(x, 0.0, z),
                    v3(x, 0.0, z + tile),
                    v3(x + tile, 0.0, z + tile),
                    v3(x + tile, 0.0, z),
                    c,
                );
            }
        }
        // Perimeter walls.
        let (h, w, t) = (self.half_size, self.wall_height, 0.4);
        let wall = rgb(92, 104, 118);
        m.cuboid(v3(-h - t, 0.0, -h - t), v3(h + t, w, -h), wall);
        m.cuboid(v3(-h - t, 0.0, h), v3(h + t, w, h + t), wall);
        m.cuboid(v3(-h - t, 0.0, -h), v3(-h, w, h), wall);
        m.cuboid(v3(h, 0.0, -h), v3(h + t, w, h), wall);
        for s in &self.shapes {
            s.build(&mut m);
        }
        m.verts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights() {
        let p = Park::demo();
        assert_eq!(p.surface(0.0, -10.0).height, 0.0);
        let mid = p.surface(0.0, -3.25);
        assert!((mid.height - 0.275).abs() < 1e-4);
        assert!(mid.normal.z < 0.0, "kicker faces the run-in (-z)");
        assert_eq!(p.surface(8.0, 3.0).height, 0.6);
        assert_eq!(p.surface(25.0, 0.0).height, p.wall_height);
        let bank = p.surface(16.0, 0.0);
        assert!((bank.height - 0.8).abs() < 1e-4 && bank.normal.x < 0.0);
    }

    #[test]
    fn mesh_is_whole_triangles() {
        let v = Park::demo().build_mesh();
        assert!(!v.is_empty());
        assert_eq!(v.len() % 3, 0);
        // GX_Begin takes a u16 count; the Wii renderer splits on whole triangles.
        assert!(v.len() < 200_000);
    }
}
