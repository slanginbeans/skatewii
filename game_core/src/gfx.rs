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
    /// The screen's up (world up unless the camera rolls).
    pub up: Vec3,
    /// Vertical field of view in degrees.
    pub fov_y_deg: f32,
    pub near: f32,
    pub far: f32,
}

/// Virtual 2D screen used by [`Gfx::draw_2d`]: 640x480, origin top-left, whatever the TV's aspect.
pub const SCREEN_W: f32 = 640.0;
pub const SCREEN_H: f32 = 480.0;

/// Distance fog: the 3D scene fades to `rgba` from `start` to `end` metres from the eye (measured across
/// the screen as well as into it, so the edges fog like the middle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    pub start: f32,
    pub end: f32,
    pub rgba: u32,
}

pub trait Gfx {
    /// Starts a frame (without fog). `clear_rgba` is the sky colour.
    fn begin_frame(&mut self, cam: &Camera, clear_rgba: u32);
    /// Fog for this frame's 3D drawing from here on (None: none). The 2D overlay is never fogged.
    fn fog(&mut self, _fog: Option<Fog>) {}
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
    /// [`Gfx::draw_indexed`] for vertices that stay put until the park is unloaded ([`StaticVerts`]): the
    /// Wii reads them in place instead of copying them every frame.
    fn draw_indexed_static(
        &mut self,
        verts: &StaticVerts,
        indices: &[u16],
        texture: Option<&Texture>,
        model: &Mat34,
    ) {
        self.draw_indexed(verts, indices, texture, model)
    }
    /// [`Gfx::draw_indexed_static`] with the mesh's draw commands already built ([`GpuList`]): the Wii's
    /// GPU reads them itself instead of the CPU sending every index. Others draw the indices.
    fn draw_list(
        &mut self,
        verts: &StaticVerts,
        indices: &[u16],
        _list: &GpuList,
        texture: Option<&Texture>,
        model: &Mat34,
    ) {
        self.draw_indexed_static(verts, indices, texture, model)
    }
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

/// 32 bytes, 32-byte aligned: what a [`GpuList`] is made of.
#[repr(C, align(32))]
#[derive(Clone, Copy, Default)]
pub struct Line32(pub [u8; 32]);

/// A static mesh's draw commands in the Wii GPU's own format (a GX display list), built once when the mesh
/// loads: for each run of up to 65,535 indices, `0x91` (triangles, vertex format 1), the count (`u16`,
/// big-endian), then each index three times (position, colour, texture coordinate: 16-bit indices into the
/// mesh's vertex array), padded with `0x00` (no-op) to a multiple of 32 bytes, 32-byte aligned, as
/// `GX_CallDispList` needs. Format: libogc's `gx.h`/`gx.c` and YAGCD chapter 5.
#[derive(Clone, Default)]
pub struct GpuList {
    lines: Vec<Line32>,
    len: usize,
}

/// Opcode of a triangle list drawn with vertex format 1 (the indexed, textured one).
pub const GX_TRIANGLES_VTXFMT1: u8 = 0x90 | 1;

impl GpuList {
    /// The commands drawing `indices` (a whole number of triangles) as triangles.
    pub fn triangles(indices: &[u16]) -> GpuList {
        let mut bytes: Vec<u8> = Vec::with_capacity(indices.len() * 6 + 64);
        // Runs of whole triangles, each as many as a command's count takes.
        for run in indices.chunks(65535 - 65535 % 3) {
            bytes.push(GX_TRIANGLES_VTXFMT1);
            bytes.extend_from_slice(&(run.len() as u16).to_be_bytes());
            for &i in run {
                let b = i.to_be_bytes();
                bytes.extend_from_slice(&[b[0], b[1], b[0], b[1], b[0], b[1]]);
            }
        }
        let len = bytes.len().next_multiple_of(32);
        let mut lines = alloc::vec![Line32::default(); len / 32];
        for (line, chunk) in lines.iter_mut().zip(bytes.chunks(32)) {
            line.0[..chunk.len()].copy_from_slice(chunk);
        }
        GpuList { lines, len }
    }

    /// The commands (a multiple of 32 bytes, 32-byte aligned).
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `Line32` is 32 bytes without padding, laid end to end; `len` is the lines' bytes.
        unsafe { core::slice::from_raw_parts(self.lines.as_ptr() as *const u8, self.len) }
    }

    pub fn heap_bytes(&self) -> usize {
        self.len
    }
}

/// Four vertices in 96 bytes, 32-byte aligned (no padding: 96 is three cache lines).
#[repr(C, align(32))]
#[derive(Clone, Copy, Default)]
struct Quad([TexVertex; 4]);

const _: () = assert!(core::mem::size_of::<Quad>() == 4 * core::mem::size_of::<TexVertex>());

/// Vertices the Wii's GPU can read where they are: 32-byte aligned (as GX vertex arrays need), and never
/// written after they are built (the platform flushes them from the CPU cache once, after a park loads).
/// Reads as a `[TexVertex]`: a copy, or (on a big-endian machine, the Wii) a pack's bytes in place.
#[derive(Clone, Default)]
pub struct StaticVerts {
    blocks: Vec<Quad>,
    len: usize,
    in_place: Option<&'static [TexVertex]>,
}

impl StaticVerts {
    pub fn new(verts: &[TexVertex]) -> StaticVerts {
        let mut blocks = alloc::vec![Quad::default(); verts.len().div_ceil(4)];
        for (b, chunk) in blocks.iter_mut().zip(verts.chunks(4)) {
            b.0[..chunk.len()].copy_from_slice(chunk);
        }
        StaticVerts {
            blocks,
            len: verts.len(),
            in_place: None,
        }
    }

    /// `bytes` (big-endian `TexVertex`es, as a pack holds them) read where they are, when this machine
    /// is big-endian and they are 32-byte aligned; None otherwise (decode and copy them instead).
    pub fn in_place(bytes: &'static [u8]) -> Option<StaticVerts> {
        let size = core::mem::size_of::<TexVertex>();
        if cfg!(target_endian = "little")
            || !(bytes.as_ptr() as usize).is_multiple_of(32)
            || !bytes.len().is_multiple_of(size)
        {
            return None;
        }
        // SAFETY: big-endian, so the bytes are `TexVertex`es as they are (`repr(C)`: three f32, a u32, two
        // f32, no padding, any bit pattern valid), aligned, and they live as long as the program.
        let v = unsafe {
            core::slice::from_raw_parts(bytes.as_ptr() as *const TexVertex, bytes.len() / size)
        };
        Some(StaticVerts {
            blocks: Vec::new(),
            len: v.len(),
            in_place: Some(v),
        })
    }

    /// True when the vertices are read in place (no copy).
    pub fn is_in_place(&self) -> bool {
        self.in_place.is_some()
    }

    /// The memory the vertices take (whole blocks), for flushing it from the CPU cache.
    pub fn bytes(&self) -> (*const u8, usize) {
        match self.in_place {
            Some(v) => (v.as_ptr() as *const u8, core::mem::size_of_val(v)),
            None => (
                self.blocks.as_ptr() as *const u8,
                self.blocks.len() * core::mem::size_of::<Quad>(),
            ),
        }
    }

    /// Heap bytes the copy takes (0 in place).
    pub fn heap_bytes(&self) -> usize {
        self.blocks.len() * core::mem::size_of::<Quad>()
    }
}

impl core::ops::Deref for StaticVerts {
    type Target = [TexVertex];
    fn deref(&self) -> &[TexVertex] {
        if let Some(v) = self.in_place {
            return v;
        }
        // SAFETY: the blocks are `repr(C)` arrays of `TexVertex` without padding, laid end to end, and
        // hold at least `len` vertices.
        unsafe { core::slice::from_raw_parts(self.blocks.as_ptr() as *const TexVertex, self.len) }
    }
}

#[cfg(test)]
mod static_tests {
    use super::*;

    #[test]
    fn a_gpu_list_is_the_triangle_command_padded_to_32_bytes() {
        let l = GpuList::triangles(&[0, 1, 2, 0x0102, 2, 3]);
        let b = l.bytes();
        assert_eq!(b.as_ptr() as usize % 32, 0);
        assert_eq!(b.len(), 64, "3 + 6 * 6 = 39 bytes, padded");
        assert_eq!(&b[..3], &[0x91, 0, 6]);
        // Each index three times: position, colour, texture coordinate.
        assert_eq!(&b[3..9], &[0, 0, 0, 0, 0, 0]);
        assert_eq!(&b[9..15], &[0, 1, 0, 1, 0, 1]);
        assert_eq!(&b[21..27], &[1, 2, 1, 2, 1, 2]);
        assert!(b[39..].iter().all(|&x| x == 0), "no-ops after");
        // Nothing to draw: no commands. More than one command's worth is split into whole triangles.
        assert_eq!(GpuList::triangles(&[]).bytes().len(), 0);
        let many: Vec<u16> = (0..70_000u32).map(|i| (i % 3) as u16).collect();
        let l = GpuList::triangles(&many[..69_999]);
        let b = l.bytes();
        let first = u16::from_be_bytes([b[1], b[2]]) as usize;
        assert_eq!((b[0], first), (0x91, 65_535));
        let at = 3 + first * 6;
        let second = u16::from_be_bytes([b[at + 1], b[at + 2]]) as usize;
        assert_eq!((b[at], first + second), (0x91, 69_999));
        assert_eq!(l.heap_bytes() % 32, 0);
    }

    #[test]
    fn static_vertices_read_back_and_are_aligned() {
        let v: Vec<TexVertex> = (0..7)
            .map(|i| TexVertex {
                pos: [i as f32, 0.0, 0.0],
                rgba: i,
                uv: [0.0, i as f32],
            })
            .collect();
        let s = StaticVerts::new(&v);
        assert_eq!(&s[..], &v[..]);
        let (p, n) = s.bytes();
        assert_eq!(p as usize % 32, 0);
        assert_eq!(n, 2 * 96);
        assert!(StaticVerts::new(&[]).is_empty());
    }

    #[test]
    fn vertices_in_place_on_a_big_endian_machine() {
        let v: Vec<TexVertex> = (0..5)
            .map(|i| TexVertex {
                pos: [i as f32, 1.5, -2.0],
                rgba: 0x1122_3300 + i,
                uv: [0.25, i as f32],
            })
            .collect();
        let be: Vec<u8> = v
            .iter()
            .flat_map(|t| {
                let mut b = Vec::new();
                for f in t.pos {
                    b.extend_from_slice(&f.to_be_bytes());
                }
                b.extend_from_slice(&t.rgba.to_be_bytes());
                for f in t.uv {
                    b.extend_from_slice(&f.to_be_bytes());
                }
                b
            })
            .collect();
        // 32-byte aligned, as in a pack.
        let mut blocks = alloc::vec![Quad::default(); 2];
        let raw: &'static mut [u8] =
            unsafe { core::slice::from_raw_parts_mut(blocks.as_mut_ptr() as *mut u8, 2 * 96) };
        core::mem::forget(blocks);
        raw[..be.len()].copy_from_slice(&be);
        let raw: &'static [u8] = raw;
        let s = StaticVerts::in_place(&raw[..be.len()]);
        assert_eq!(
            s.is_some(),
            cfg!(target_endian = "big"),
            "in place only on the Wii"
        );
        if let Some(s) = s {
            assert!(s.is_in_place() && s.heap_bytes() == 0);
            assert_eq!(&s[..], &v[..]);
            assert_eq!(s.bytes(), (raw.as_ptr(), be.len()));
        }
        assert!(
            StaticVerts::in_place(&raw[4..4 + 24]).is_none(),
            "not aligned"
        );
        assert!(
            StaticVerts::in_place(&raw[..25]).is_none(),
            "not whole vertices"
        );
    }
}
