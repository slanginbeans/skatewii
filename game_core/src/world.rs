//! An imported park (`park.fgpak` from `fg_import park`): its collision, plus what to draw.
//!
//! The park's own textured models (`park/mesh/<n>`, `park/tex/<n>`) are drawn when the pack has them.
//! Otherwise the collision triangles are drawn themselves, flat shaded and coloured by surface. Either way
//! only what is near the camera is sent each frame.
//!
//! `park/mesh/<n>` (big-endian): `u32` vertex count, `u32` index count, `u16` texture index (`0xffff`:
//! none), `u16` flags (bit 0: alpha-tested texture), then per vertex 3 `f32` position, `u32` RGBA (lighting
//! baked in), 2 `f32` UV, then the `u16` triangle-list indices. `park/tex/<n>`: a GX texture entry.

use alloc::format;
use alloc::vec::Vec;

use crate::collision::{self, TriMesh, WALKABLE_Y};
use crate::gfx::{Gfx, SUN, StaticVerts, TexVertex, Texture, Vertex, rgb, shade};
use crate::math::{Vec3, Xform, v3};
use crate::pack::{Kind, Pack};

/// Chunk edge for drawing (m).
const CHUNK: f32 = 16.0;
/// Chunks further than this from the camera (m, centre to eye, flat) are skipped; a district's draw distance
/// is a setting ([`MIN_DRAW_DISTANCE`]..[`MAX_DRAW_DISTANCE`]) and this is its default.
pub const DRAW_DISTANCE: f32 = 90.0;
pub const MIN_DRAW_DISTANCE: f32 = 40.0;
pub const MAX_DRAW_DISTANCE: f32 = 120.0;
/// A streamed district fades into the sky from here ...
pub const FOG_START: f32 = 50.0;
/// ... to here, short of the draw distance, so models never pop in or out in plain sight.
pub const FOG_END: f32 = DRAW_DISTANCE - 5.0;

/// The fog for a district drawn to `draw_distance`: [`FOG_START`]..[`FOG_END`] at the default, scaled with it.
pub fn fog_for(draw_distance: f32) -> (f32, f32) {
    let k = draw_distance / DRAW_DISTANCE;
    (FOG_START * k, FOG_END * k)
}

/// What the camera can see: within `reach` (flat, from the eye) and inside the view's four sides. Meshes
/// outside it aren't drawn.
#[derive(Clone, Copy, Debug)]
pub struct Sight {
    pub eye: Vec3,
    pub reach: f32,
    /// Inward normals of the left, right, bottom and top sides (planes through the eye); none: everything
    /// within reach is seen.
    sides: Option<[Vec3; 4]>,
}

impl Sight {
    /// Everything within `reach` of `eye`, whichever way it is.
    pub fn around(eye: Vec3, reach: f32) -> Sight {
        Sight {
            eye,
            reach,
            sides: None,
        }
    }

    /// What `cam` sees on a screen `aspect` wide (width / height), out to `reach`.
    pub fn of(cam: &crate::gfx::Camera, aspect: f32, reach: f32) -> Sight {
        let f = (cam.target - cam.eye).normalize_or(v3(0.0, 0.0, 1.0));
        let right = f.cross(cam.up).normalize_or(v3(1.0, 0.0, 0.0));
        let up = right.cross(f);
        let half_y = (cam.fov_y_deg * 0.5).to_radians();
        let half_x = libm::atanf(libm::tanf(half_y) * aspect);
        let side =
            |axis: Vec3, half: f32, s: f32| f * libm::sinf(half) + axis * (s * libm::cosf(half));
        Sight {
            eye: cam.eye,
            reach,
            sides: Some([
                side(right, half_x, 1.0),
                side(right, half_x, -1.0),
                side(up, half_y, 1.0),
                side(up, half_y, -1.0),
            ]),
        }
    }

    /// Whether the sphere at `c` (radius `r`) comes within reach, whichever way it is.
    pub fn within(&self, c: Vec3, r: f32) -> bool {
        let (dx, dz) = (c.x - self.eye.x, c.z - self.eye.z);
        let reach = self.reach + r;
        dx * dx + dz * dz <= reach * reach
    }

    /// Whether any of the sphere at `c` (radius `r`) can be seen.
    pub fn sees(&self, c: Vec3, r: f32) -> bool {
        if !self.within(c, r) {
            return false;
        }
        let d = c - self.eye;
        // A little slack, so nothing on an edge flickers.
        let r = r + 1.0;
        self.sides
            .is_none_or(|sides| sides.iter().all(|n| n.dot(d) >= -r))
    }
}
/// The old mesh header (no magic): vertices not aligned, copied when parsed.
const MESH_HEADER_V1: usize = 12;
/// `"FGM2"`, `u32` vertex count, `u32` index count, `u16` texture, `u16` flags, `f32` centre x, y, z,
/// `f32` radius: 32 bytes, so the vertices after it are 32-byte aligned in a pack and the Wii reads them
/// (and the indices) in place.
const MESH_HEADER: usize = 32;
const MESH_MAGIC: &[u8; 4] = b"FGM2";
const MESH_VERTEX: usize = 24;
pub const MESH_ALPHA_TEST: u16 = 1;

pub struct Chunk {
    pub center: Vec3,
    pub verts: Vec<Vertex>,
}

/// One of the park's own meshes, ready to draw.
pub struct ParkMesh {
    /// Read in place by the Wii's GPU.
    pub verts: StaticVerts,
    pub indices: Indices,
    pub texture: Option<Texture<'static>>,
    /// Bounding sphere for culling.
    pub center: Vec3,
    pub radius: f32,
}

/// A mesh's indices: a copy, or (on a big-endian machine) a pack's bytes in place.
pub enum Indices {
    Owned(Vec<u16>),
    InPlace(&'static [u16]),
}

impl core::ops::Deref for Indices {
    type Target = [u16];
    fn deref(&self) -> &[u16] {
        match self {
            Indices::Owned(v) => v,
            Indices::InPlace(s) => s,
        }
    }
}

impl Indices {
    /// `bytes` (big-endian `u16`s) read where they are on a big-endian machine, else copied.
    fn new(bytes: &'static [u8]) -> Indices {
        if cfg!(target_endian = "big")
            && (bytes.as_ptr() as usize).is_multiple_of(2)
            && bytes.len().is_multiple_of(2)
        {
            // SAFETY: big-endian, aligned, whole u16s, living as long as the program.
            Indices::InPlace(unsafe {
                core::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2)
            })
        } else {
            Indices::Owned(bytes.chunks_exact(2).map(|b| be16(b, 0)).collect())
        }
    }

    /// Heap bytes the copy takes (0 in place).
    pub fn heap_bytes(&self) -> usize {
        match self {
            Indices::Owned(v) => v.len() * 2,
            Indices::InPlace(_) => 0,
        }
    }
}

impl ImportedPark {
    /// Heap bytes the parsed park takes beyond what it reads in place from its pack.
    pub fn heap_bytes(&self) -> usize {
        self.mesh.heap_bytes()
            + self
                .models
                .iter()
                .map(|m| {
                    m.verts.heap_bytes() + m.indices.heap_bytes() + core::mem::size_of::<ParkMesh>()
                })
                .sum::<usize>()
            + self
                .chunks
                .iter()
                .map(|c| c.verts.len() * core::mem::size_of::<Vertex>())
                .sum::<usize>()
            + self.mesh.rails.len() * core::mem::size_of::<crate::rails::Rail>()
    }
}

pub struct ImportedPark {
    pub mesh: TriMesh,
    /// The collision drawn as the park, when there are no models.
    pub chunks: Vec<Chunk>,
    pub models: Vec<ParkMesh>,
}

/// Encodes one park mesh entry (for the importer and tests).
pub fn encode_mesh(
    verts: &[([f32; 3], u32, [f32; 2])],
    indices: &[u16],
    texture: Option<u16>,
    flags: u16,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(MESH_HEADER + verts.len() * MESH_VERTEX + indices.len() * 2);
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (p, _, _) in verts {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let (center, radius) = if verts.is_empty() {
        ([0.0; 3], 0.0)
    } else {
        let c: [f32; 3] = core::array::from_fn(|k| (lo[k] + hi[k]) * 0.5);
        let r = libm::sqrtf((0..3).map(|k| (hi[k] - c[k]) * (hi[k] - c[k])).sum());
        (c, r)
    };
    out.extend_from_slice(MESH_MAGIC);
    out.extend_from_slice(&(verts.len() as u32).to_be_bytes());
    out.extend_from_slice(&(indices.len() as u32).to_be_bytes());
    out.extend_from_slice(&texture.unwrap_or(0xffff).to_be_bytes());
    out.extend_from_slice(&flags.to_be_bytes());
    for f in [center[0], center[1], center[2], radius] {
        out.extend_from_slice(&f.to_be_bytes());
    }
    for (p, rgba, uv) in verts {
        for c in p {
            out.extend_from_slice(&c.to_be_bytes());
        }
        out.extend_from_slice(&rgba.to_be_bytes());
        for c in uv {
            out.extend_from_slice(&c.to_be_bytes());
        }
    }
    for i in indices {
        out.extend_from_slice(&i.to_be_bytes());
    }
    out
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

/// Parses one `park/mesh/<n>` entry; `texture` looks a texture index up in the pack.
fn parse_mesh(
    d: &'static [u8],
    texture: &dyn Fn(u16) -> Option<Texture<'static>>,
) -> Result<ParkMesh, collision::Error> {
    if d.starts_with(MESH_MAGIC) {
        return parse_mesh_v2(d, texture);
    }
    parse_mesh_v1(d, texture)
}

/// A `"FGM2"` mesh: the vertices and indices read in place where the machine allows.
fn parse_mesh_v2(
    d: &'static [u8],
    texture: &dyn Fn(u16) -> Option<Texture<'static>>,
) -> Result<ParkMesh, collision::Error> {
    if d.len() < MESH_HEADER {
        return Err(collision::Error::Bad);
    }
    let (nv, ni) = (be32(d, 4) as usize, be32(d, 8) as usize);
    let (tex, flags) = (be16(d, 12), be16(d, 14));
    let f = |at: usize| f32::from_bits(be32(d, at));
    let (center, radius) = (v3(f(16), f(20), f(24)), f(28));
    let end = nv
        .checked_mul(MESH_VERTEX)
        .and_then(|v| v.checked_add(MESH_HEADER + ni.checked_mul(2)?))
        .ok_or(collision::Error::Bad)?;
    if d.len() < end
        || nv > 0x10000
        || !(center.x.is_finite() && center.y.is_finite() && center.z.is_finite())
        || !(radius.is_finite() && radius >= 0.0)
    {
        return Err(collision::Error::Bad);
    }
    let vb = &d[MESH_HEADER..MESH_HEADER + nv * MESH_VERTEX];
    let ib = &d[MESH_HEADER + nv * MESH_VERTEX..end];
    if ib.chunks_exact(2).any(|b| be16(b, 0) as usize >= nv) {
        return Err(collision::Error::Bad);
    }
    let verts = StaticVerts::in_place(vb).unwrap_or_else(|| {
        let f = |at: usize| f32::from_bits(be32(vb, at));
        let v: Vec<TexVertex> = (0..nv)
            .map(|i| {
                let at = i * MESH_VERTEX;
                TexVertex {
                    pos: [f(at), f(at + 4), f(at + 8)],
                    rgba: be32(vb, at + 12),
                    uv: [f(at + 16), f(at + 20)],
                }
            })
            .collect();
        StaticVerts::new(&v)
    });
    let texture = (tex != 0xffff)
        .then(|| texture(tex))
        .flatten()
        .map(|mut t| {
            t.alpha_test = flags & MESH_ALPHA_TEST != 0;
            t
        });
    Ok(ParkMesh {
        verts,
        indices: Indices::new(ib),
        texture,
        center,
        radius,
    })
}

/// An old mesh (no magic): copied.
fn parse_mesh_v1(
    d: &[u8],
    texture: &dyn Fn(u16) -> Option<Texture<'static>>,
) -> Result<ParkMesh, collision::Error> {
    const MESH_HEADER: usize = MESH_HEADER_V1;
    if d.len() < MESH_HEADER {
        return Err(collision::Error::Bad);
    }
    let (nv, ni) = (be32(d, 0) as usize, be32(d, 4) as usize);
    let (tex, flags) = (be16(d, 8), be16(d, 10));
    if d.len() < MESH_HEADER + nv * MESH_VERTEX + ni * 2 || nv > 0x10000 {
        return Err(collision::Error::Bad);
    }
    let f = |at: usize| f32::from_bits(be32(d, at));
    let verts: Vec<TexVertex> = (0..nv)
        .map(|i| {
            let at = MESH_HEADER + i * MESH_VERTEX;
            TexVertex {
                pos: [f(at), f(at + 4), f(at + 8)],
                rgba: be32(d, at + 12),
                uv: [f(at + 16), f(at + 20)],
            }
        })
        .collect();
    let at = MESH_HEADER + nv * MESH_VERTEX;
    let indices: Vec<u16> = (0..ni).map(|i| be16(d, at + 2 * i)).collect();
    if indices.iter().any(|&i| i as usize >= nv) {
        return Err(collision::Error::Bad);
    }
    let (mut lo, mut hi) = (
        v3(f32::MAX, f32::MAX, f32::MAX),
        v3(f32::MIN, f32::MIN, f32::MIN),
    );
    for v in &verts {
        let p = v3(v.pos[0], v.pos[1], v.pos[2]);
        lo = v3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = v3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    let center = (lo + hi) * 0.5;
    let texture = (tex != 0xffff)
        .then(|| texture(tex))
        .flatten()
        .map(|mut t| {
            t.alpha_test = flags & MESH_ALPHA_TEST != 0;
            t
        });
    Ok(ParkMesh {
        radius: if verts.is_empty() {
            0.0
        } else {
            (hi - center).length()
        },
        verts: StaticVerts::new(&verts),
        indices: Indices::Owned(indices),
        texture,
        center,
    })
}

/// A colour per surface id: concrete greys, with a hint of colour so materials read apart.
fn surface_colour(surface: u16, wall: bool) -> u32 {
    let h = (surface as u32).wrapping_mul(2_654_435_761);
    let base = 150 + (h >> 24) % 60;
    let tint = |s: u32| base - 10 + ((h >> s) & 0x1f);
    let c = rgb(tint(8) as u8, tint(13) as u8, tint(18) as u8);
    if wall { shade(c, 0.8) } else { c }
}

impl ImportedPark {
    /// The bytes stay borrowed for the rest of the game: the Wii reads the park's textures in place.
    pub fn parse(bytes: &'static [u8]) -> Result<ImportedPark, collision::Error> {
        let pack = Pack::parse(bytes).map_err(|_| collision::Error::Bad)?;
        let mesh = TriMesh::from_pack(&pack)?;
        let texture = |i: u16| {
            pack.find(&format!("park/tex/{i}"))
                .filter(|e| e.kind == Kind::Texture as u32)
                .map(|e| Texture {
                    data: e.data,
                    width: e.width,
                    height: e.height,
                    format: e.gx_format,
                    alpha_test: false,
                })
        };
        let mut models = Vec::new();
        for e in pack.iter() {
            if e.name.starts_with("park/mesh/") {
                models.push(parse_mesh(e.data, &texture)?);
            }
        }
        // The collision is only drawn when there are no models to draw.
        let chunks = if models.is_empty() {
            Self::build_chunks(&mesh)
        } else {
            Vec::new()
        };
        Ok(ImportedPark {
            mesh,
            chunks,
            models,
        })
    }

    fn build_chunks(mesh: &TriMesh) -> Vec<Chunk> {
        let cols = (((mesh.max.x - mesh.min.x) / CHUNK) as usize + 1).max(1);
        let rows = (((mesh.max.z - mesh.min.z) / CHUNK) as usize + 1).max(1);
        let mut chunks: Vec<Chunk> = (0..cols * rows)
            .map(|i| Chunk {
                center: v3(
                    mesh.min.x + ((i % cols) as f32 + 0.5) * CHUNK,
                    0.0,
                    mesh.min.z + ((i / cols) as f32 + 0.5) * CHUNK,
                ),
                verts: Vec::new(),
            })
            .collect();
        for t in mesh.tris() {
            let mid = (t.v[0] + t.v[1] + t.v[2]) * (1.0 / 3.0);
            let c = (((mid.x - mesh.min.x) / CHUNK) as usize).min(cols - 1);
            let r = (((mid.z - mesh.min.z) / CHUNK) as usize).min(rows - 1);
            // Two-sided: light whichever side faces the sun.
            let n = if t.n.y < 0.0 { -t.n } else { t.n };
            let wall = n.y < WALKABLE_Y;
            let k = 0.55 + 0.45 * n.dot(SUN).abs();
            let rgba = shade(surface_colour(t.surface, wall), k);
            let ch = &mut chunks[r * cols + c];
            for p in t.v {
                ch.verts.push(Vertex {
                    pos: [p.x, p.y, p.z],
                    rgba,
                });
            }
        }
        chunks.retain(|c| !c.verts.is_empty());
        // Heights for distance checks: the middle of each chunk's geometry.
        for c in &mut chunks {
            let (lo, hi) = c.verts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
                (lo.min(v.pos[1]), hi.max(v.pos[1]))
            });
            c.center.y = (lo + hi) / 2.0;
        }
        chunks
    }

    /// Draws what is within [`DRAW_DISTANCE`] of `eye`: the park's models, else its collision. Returns how
    /// many meshes and vertices it drew.
    pub fn draw(&self, gfx: &mut dyn Gfx, sight: &Sight) -> (u32, u32) {
        let m = Xform::IDENTITY.to_mat34();
        let (mut draws, mut verts) = (0, 0);
        for p in &self.models {
            if sight.sees(p.center, p.radius) {
                gfx.draw_indexed_static(&p.verts, &p.indices, p.texture.as_ref(), &m);
                draws += 1;
                verts += p.verts.len() as u32;
            }
        }
        for c in &self.chunks {
            // Collision drawn for want of models (a fallback): its chunks' heights aren't kept, so only
            // the distance is checked.
            if sight.within(c.center, 0.0) {
                gfx.draw_tris(&c.verts, &m);
                draws += 1;
                verts += c.verts.len() as u32;
            }
        }
        (draws, verts)
    }

    pub fn vertex_count(&self) -> usize {
        self.chunks.iter().map(|c| c.verts.len()).sum::<usize>()
            + self.models.iter().map(|m| m.verts.len()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::name_hash;
    use alloc::vec;

    /// One pack entry: name, kind, data, width, height, GX format.
    type Entry<'a> = (&'a str, Kind, Vec<u8>, u16, u16, u8);

    /// A pack in memory.
    fn pack(items: &[Entry]) -> &'static [u8] {
        let mut strings = vec![];
        let mut name_at = vec![];
        for (n, ..) in items {
            name_at.push(strings.len() as u32);
            strings.extend_from_slice(n.as_bytes());
            strings.push(0);
        }
        let strings_at = 16 + 32 * items.len();
        let mut data_at = (strings_at + strings.len()).next_multiple_of(32);
        let mut out = b"FGPA".to_vec();
        for x in [1u32, items.len() as u32, strings_at as u32] {
            out.extend_from_slice(&x.to_be_bytes());
        }
        let mut offsets = vec![];
        for (i, (n, kind, d, w, h, f)) in items.iter().enumerate() {
            for x in [
                name_hash(n),
                name_at[i],
                *kind as u32,
                data_at as u32,
                d.len() as u32,
            ] {
                out.extend_from_slice(&x.to_be_bytes());
            }
            out.extend_from_slice(&w.to_be_bytes());
            out.extend_from_slice(&h.to_be_bytes());
            out.extend_from_slice(&[*f, 0, 0, 0, 0, 0, 0, 0]);
            offsets.push(data_at);
            data_at = (data_at + d.len()).next_multiple_of(32);
        }
        out.extend_from_slice(&strings);
        for (i, (_, _, d, ..)) in items.iter().enumerate() {
            out.resize(offsets[i], 0);
            out.extend_from_slice(d);
        }
        // 32-byte aligned, as the game loads packs (the Wii's `memalign`), so meshes can be read in place;
        // a plain Vec's alignment would make that depend on the allocator.
        let layout = core::alloc::Layout::from_size_align(out.len().max(1), 32).unwrap();
        // SAFETY: the layout has a non-zero size; the block is filled before it is read and never freed.
        unsafe {
            let p = alloc::alloc::alloc(layout);
            assert!(!p.is_null());
            core::ptr::copy_nonoverlapping(out.as_ptr(), p, out.len());
            core::slice::from_raw_parts(p, out.len())
        }
    }

    fn floor() -> Vec<u8> {
        collision::encode(&[([[-5., 0., -5.], [-5., 0., 5.], [5., 0., 5.]], 1, 0)])
    }

    #[test]
    fn only_what_the_camera_sees_is_drawn() {
        // Eye at the origin looking along +z, 60 degrees tall on a 4:3 screen (about 75 degrees wide).
        let cam = crate::gfx::Camera {
            eye: v3(0.0, 2.0, 0.0),
            target: v3(0.0, 2.0, 10.0),
            up: v3(0.0, 1.0, 0.0),
            fov_y_deg: 60.0,
            near: 0.1,
            far: 500.0,
        };
        let s = Sight::of(&cam, 4.0 / 3.0, 90.0);
        assert!(s.sees(v3(0.0, 2.0, 30.0), 1.0), "ahead");
        assert!(!s.sees(v3(0.0, 2.0, -30.0), 1.0), "behind");
        assert!(!s.sees(v3(40.0, 2.0, 10.0), 1.0), "off to one side");
        assert!(!s.sees(v3(-40.0, 2.0, 10.0), 1.0), "off to the other");
        assert!(!s.sees(v3(0.0, 60.0, 20.0), 1.0), "far above the view");
        assert!(
            !s.sees(v3(0.0, 2.0, 120.0), 1.0),
            "beyond the draw distance"
        );
        // Just outside an edge but big enough to reach into the view; right beside the eye.
        assert!(s.sees(v3(30.0, 2.0, 30.0), 10.0));
        assert!(s.sees(v3(0.0, 2.0, -2.0), 3.0));
        // A wider screen sees further to the sides.
        let wide = Sight::of(&cam, 16.0 / 9.0, 90.0);
        let side = v3(30.0, 2.0, 30.0);
        assert!(!s.sees(side, 1.0) && wide.sees(side, 1.0));
        // Around: everything within reach.
        assert!(Sight::around(cam.eye, 90.0).sees(v3(0.0, 2.0, -30.0), 1.0));

        // A park draws only the meshes in sight.
        struct Count(u32);
        impl Gfx for Count {
            fn begin_frame(&mut self, _: &crate::gfx::Camera, _: u32) {}
            fn draw_tris(&mut self, _: &[Vertex], _: &crate::math::Mat34) {}
            fn draw_2d(&mut self, _: &[Vertex]) {}
            fn draw_indexed(
                &mut self,
                _: &[TexVertex],
                _: &[u16],
                _: Option<&Texture>,
                _: &crate::math::Mat34,
            ) {
            }
            fn draw_indexed_static(
                &mut self,
                _: &StaticVerts,
                _: &[u16],
                _: Option<&Texture>,
                _: &crate::math::Mat34,
            ) {
                self.0 += 1;
            }
            fn end_frame(&mut self) {}
        }
        let quad = |x: f32, z: f32| {
            let q = [
                ([x, 0.0, z], 0xffff_ffff, [0.0, 0.0]),
                ([x + 1.0, 0.0, z], 0xffff_ffff, [1.0, 0.0]),
                ([x + 1.0, 0.0, z + 1.0], 0xffff_ffff, [1.0, 1.0]),
            ];
            encode_mesh(&q, &[0, 1, 2], None, 0)
        };
        let p = pack(&[
            ("park/collision", Kind::Collision, floor(), 0, 0, 0),
            ("park/spawn", Kind::Data, vec![0; 16], 0, 0, 0),
            ("park/mesh/0", Kind::Data, quad(0.0, 20.0), 0, 0, 0),
            ("park/mesh/1", Kind::Data, quad(0.0, -20.0), 0, 0, 0),
            ("park/mesh/2", Kind::Data, quad(60.0, 5.0), 0, 0, 0),
        ]);
        let park = ImportedPark::parse(p).unwrap();
        let mut g = Count(0);
        assert_eq!(park.draw(&mut g, &s), (1, 3));
        assert_eq!(g.0, 1);
        assert_eq!(park.draw(&mut Count(0), &Sight::around(cam.eye, 90.0)).0, 3);
    }

    #[test]
    fn the_fog_follows_the_draw_distance() {
        assert_eq!(fog_for(DRAW_DISTANCE), (FOG_START, FOG_END));
        for d in [MIN_DRAW_DISTANCE, 60.0, MAX_DRAW_DISTANCE] {
            let (start, end) = fog_for(d);
            assert!(0.0 < start && start < end && end < d, "{d}: {start} {end}");
        }
    }

    #[test]
    fn loads_textured_models_and_skips_the_collision_mesh() {
        let quad = [
            ([0.0, 0.0, 0.0], 0xffff_ffff, [0.0, 0.0]),
            ([1.0, 0.0, 0.0], 0xffff_ffff, [1.0, 0.0]),
            ([1.0, 0.0, 1.0], 0xffff_ffff, [1.0, 1.0]),
            ([0.0, 0.0, 1.0], 0xffff_ffff, [0.0, 1.0]),
        ];
        let mesh = encode_mesh(&quad, &[0, 1, 2, 0, 2, 3], Some(0), MESH_ALPHA_TEST);
        let p = pack(&[
            ("park/collision", Kind::Collision, floor(), 0, 0, 0),
            ("park/spawn", Kind::Data, vec![0; 16], 0, 0, 0),
            ("park/mesh/0", Kind::Data, mesh, 0, 0, 0),
            ("park/tex/0", Kind::Texture, vec![0xab; 32], 8, 4, 14),
        ]);
        let park = ImportedPark::parse(p).unwrap();
        assert!(
            park.chunks.is_empty(),
            "models replace the collision drawing"
        );
        assert_eq!(park.models.len(), 1);
        let m = &park.models[0];
        assert_eq!(m.verts.len(), 4);
        assert_eq!(m.verts[2].uv, [1.0, 1.0]);
        assert_eq!(&m.indices[..], [0, 1, 2, 0, 2, 3]);
        let t = m.texture.as_ref().unwrap();
        assert_eq!(
            (t.width, t.height, t.format, t.alpha_test),
            (8, 4, 14, true)
        );
        assert!((m.center - v3(0.5, 0.0, 0.5)).length() < 1e-6);
        assert!((m.radius - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(park.vertex_count(), 4);
        // In place on a big-endian machine (the Wii): nothing copied but the collision's grid.
        if cfg!(target_endian = "big") {
            assert!(m.verts.is_in_place());
            assert_eq!(m.verts.heap_bytes() + m.indices.heap_bytes(), 0);
        }

        // A pack from before the 32-byte mesh header still loads.
        let mut old = Vec::new();
        for w in [4u32, 6] {
            old.extend_from_slice(&w.to_be_bytes());
        }
        old.extend_from_slice(&0u16.to_be_bytes());
        old.extend_from_slice(&MESH_ALPHA_TEST.to_be_bytes());
        old.extend_from_slice(&mesh_bytes_after_header(&quad));
        let p = pack(&[
            ("park/collision", Kind::Collision, floor(), 0, 0, 0),
            ("park/spawn", Kind::Data, vec![0; 16], 0, 0, 0),
            ("park/mesh/0", Kind::Data, old, 0, 0, 0),
            ("park/tex/0", Kind::Texture, vec![0xab; 32], 8, 4, 14),
        ]);
        let park = ImportedPark::parse(p).unwrap();
        let m = &park.models[0];
        assert_eq!(m.verts[2].uv, [1.0, 1.0]);
        assert_eq!(&m.indices[..], [0, 1, 2, 0, 2, 3]);
        assert!(m.texture.as_ref().unwrap().alpha_test);
        assert!((m.center - v3(0.5, 0.0, 0.5)).length() < 1e-6);
    }

    /// The vertices and indices of the quad above, as both mesh forms store them.
    fn mesh_bytes_after_header(quad: &[([f32; 3], u32, [f32; 2])]) -> Vec<u8> {
        let new = encode_mesh(quad, &[0, 1, 2, 0, 2, 3], Some(0), 0);
        new[MESH_HEADER..].to_vec()
    }

    #[test]
    fn without_models_the_collision_is_drawn() {
        let p = pack(&[
            ("park/collision", Kind::Collision, floor(), 0, 0, 0),
            ("park/spawn", Kind::Data, vec![0; 16], 0, 0, 0),
        ]);
        let park = ImportedPark::parse(p).unwrap();
        assert!(park.models.is_empty());
        assert_eq!(park.vertex_count(), 3);
        // A mesh pointing past its vertices is rejected.
        let bad = encode_mesh(&[([0.0; 3], 0, [0.0; 2])], &[0, 0, 5], None, 0);
        let p = pack(&[
            ("park/collision", Kind::Collision, floor(), 0, 0, 0),
            ("park/spawn", Kind::Data, vec![0; 16], 0, 0, 0),
            ("park/mesh/0", Kind::Data, bad, 0, 0, 0),
        ]);
        assert!(ImportedPark::parse(p).is_err());
    }
}
