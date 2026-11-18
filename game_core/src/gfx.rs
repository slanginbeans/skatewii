//! What the game needs from a renderer. The Wii implements this over GX (`wii/game`), the host preview
//! tool with a software rasteriser (`tools/preview`). Nothing in `game_core` touches a GPU directly.

use alloc::vec::Vec;

use crate::math::{Mat34, Vec3};

/// One vertex: position plus a pre-lit RGBA colour. Same layout as `FgVertex` in `fg_platform.h`
/// (16 bytes; GX reads the colour as a big-endian `0xRRGGBBAA` word).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub rgba: u32,
}

/// Vertex of an indexed, optionally textured mesh: position, pre-lit RGBA colour (multiplied with the
/// texture), texture coordinate. Same layout as `FgTexVertex` in `fg_platform.h` (24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TexVertex {
    pub pos: [f32; 3],
    pub rgba: u32,
    pub uv: [f32; 2],
}

/// A texture already in a GX format (from an imported `.fgpak`), read in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Texture<'a> {
    /// GX texture data; 32-byte aligned on the Wii (pack data is).
    pub data: &'a [u8],
    pub width: u16,
    pub height: u16,
    /// `GX_TF_*` (CMPR, RGB5A3, RGBA8, ...).
    pub format: u8,
    /// Discard texels with alpha below one half (hair cards).
    pub alpha_test: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    /// Vertical field of view in degrees.
    pub fov_y_deg: f32,
    pub near: f32,
    pub far: f32,
}

/// Virtual 2D screen used by [`Gfx::draw_2d`]: 640x480, origin top-left, whatever the TV's aspect.
pub const SCREEN_W: f32 = 640.0;
pub const SCREEN_H: f32 = 480.0;

pub trait Gfx {
    /// Starts a frame. `clear_rgba` is the sky colour.
    fn begin_frame(&mut self, cam: &Camera, clear_rgba: u32);
    /// Triangle list (3 vertices per triangle) in model space, placed by `model`.
    fn draw_tris(&mut self, verts: &[Vertex], model: &Mat34);
    /// Indexed triangle list (3 indices per triangle) in model space, placed by `model`, coloured by the
    /// vertex colour times `texture` when given. The data only has to live for the call.
    fn draw_indexed(
        &mut self,
        verts: &[TexVertex],
        indices: &[u16],
        texture: Option<&Texture>,
        model: &Mat34,
    );
    /// Triangle list in screen space ([`SCREEN_W`] x [`SCREEN_H`], z ignored), drawn over the 3D scene.
    fn draw_2d(&mut self, verts: &[Vertex]);
    fn end_frame(&mut self);
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xff
}

/// Multiplies the RGB channels by `k` (0..), keeping alpha.
pub fn shade(rgba: u32, k: f32) -> u32 {
    let ch = |s: u32| -> u32 {
        let v = ((rgba >> s) & 0xff) as f32 * k;
        (v.clamp(0.0, 255.0) as u32) << s
    };
    ch(24) | ch(16) | ch(8) | (rgba & 0xff)
}

/// Fixed sun used to pre-light geometry until the GX lighting path exists (ROADMAP phase 2).
pub const SUN: Vec3 = Vec3 {
    x: 0.42,
    y: 0.82,
    z: 0.39,
};

fn lit(rgba: u32, normal: Vec3) -> u32 {
    let n = normal.normalize_or(Vec3::UP);
    shade(rgba, 0.5 + 0.5 * n.dot(SUN).max(0.0))
}

/// Builds pre-lit triangle lists.
#[derive(Default)]
pub struct MeshBuilder {
    pub verts: Vec<Vertex>,
}

impl MeshBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, p: Vec3, rgba: u32) {
        self.verts.push(Vertex {
            pos: [p.x, p.y, p.z],
            rgba,
        });
    }

    /// Triangle lit by its own face normal.
    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, rgba: u32) {
        let n = (b - a).cross(c - a);
        let col = lit(rgba, n);
        self.push(a, col);
        self.push(b, col);
        self.push(c, col);
    }

    /// Quad a-b-c-d (counter-clockwise seen from the front).
    pub fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, rgba: u32) {
        self.tri(a, b, c, rgba);
        self.tri(a, c, d, rgba);
    }

    /// Axis-aligned box. `skip_bottom` leaves out the face nobody sees.
    pub fn cuboid(&mut self, min: Vec3, max: Vec3, rgba: u32) {
        let (a, b) = (min, max);
        let p = |x: f32, y: f32, z: f32| Vec3 { x, y, z };
        // top, sides (+x, -x, +z, -z); bottom left out
        self.quad(
            p(a.x, b.y, a.z),
            p(a.x, b.y, b.z),
            p(b.x, b.y, b.z),
            p(b.x, b.y, a.z),
            rgba,
        );
        self.quad(
            p(b.x, a.y, a.z),
            p(b.x, b.y, a.z),
            p(b.x, b.y, b.z),
            p(b.x, a.y, b.z),
            rgba,
        );
        self.quad(
            p(a.x, a.y, b.z),
            p(a.x, b.y, b.z),
            p(a.x, b.y, a.z),
            p(a.x, a.y, a.z),
            rgba,
        );
        self.quad(
            p(b.x, a.y, b.z),
            p(b.x, b.y, b.z),
            p(a.x, b.y, b.z),
            p(a.x, a.y, b.z),
            rgba,
        );
        self.quad(
            p(a.x, a.y, a.z),
            p(a.x, b.y, a.z),
            p(b.x, b.y, a.z),
            p(b.x, a.y, a.z),
            rgba,
        );
    }

    /// Screen-space rectangle (no lighting).
    pub fn rect_2d(&mut self, x: f32, y: f32, w: f32, h: f32, rgba: u32) {
        let p = |x: f32, y: f32| Vec3 { x, y, z: 0.0 };
        for q in [
            p(x, y),
            p(x + w, y),
            p(x + w, y + h),
            p(x, y),
            p(x + w, y + h),
            p(x, y + h),
        ] {
            self.push(q, rgba);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_layout_matches_c_header() {
        assert_eq!(core::mem::size_of::<Vertex>(), 16);
        assert_eq!(core::mem::offset_of!(Vertex, rgba), 12);
        assert_eq!(core::mem::size_of::<TexVertex>(), 24);
        assert_eq!(core::mem::offset_of!(TexVertex, rgba), 12);
        assert_eq!(core::mem::offset_of!(TexVertex, uv), 16);
    }

    #[test]
    fn shade_keeps_alpha_and_clamps() {
        assert_eq!(shade(0x80402010, 2.0), 0xff804010);
        assert_eq!(shade(0xffffff7f, 0.5), 0x7f7f7f7f);
    }
}
