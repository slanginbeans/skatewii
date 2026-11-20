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
use crate::gfx::{Gfx, SUN, TexVertex, Texture, Vertex, rgb, shade};
use crate::math::{Vec3, Xform, v3};
use crate::pack::{Kind, Pack};

/// Chunk edge for drawing (m).
const CHUNK: f32 = 16.0;
/// Chunks further than this from the camera (m, centre to eye, flat) are skipped.
pub const DRAW_DISTANCE: f32 = 90.0;
const MESH_HEADER: usize = 12;
const MESH_VERTEX: usize = 24;
pub const MESH_ALPHA_TEST: u16 = 1;

pub struct Chunk {
    pub center: Vec3,
    pub verts: Vec<Vertex>,
}

/// One of the park's own meshes, ready to draw.
pub struct ParkMesh {
    pub verts: Vec<TexVertex>,
    pub indices: Vec<u16>,
    pub texture: Option<Texture<'static>>,
    /// Bounding sphere for culling.
    pub center: Vec3,
    pub radius: f32,
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
    out.extend_from_slice(&(verts.len() as u32).to_be_bytes());
    out.extend_from_slice(&(indices.len() as u32).to_be_bytes());
    out.extend_from_slice(&texture.unwrap_or(0xffff).to_be_bytes());
    out.extend_from_slice(&flags.to_be_bytes());
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
    d: &[u8],
    texture: &dyn Fn(u16) -> Option<Texture<'static>>,
) -> Result<ParkMesh, collision::Error> {
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
        verts,
        indices,
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
        for t in &mesh.tris {
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
    pub fn draw(&self, gfx: &mut dyn Gfx, eye: Vec3) -> (u32, u32) {
        let m = Xform::IDENTITY.to_mat34();
        let (mut draws, mut verts) = (0, 0);
        for p in &self.models {
            let (dx, dz) = (p.center.x - eye.x, p.center.z - eye.z);
            let reach = DRAW_DISTANCE + p.radius;
            if dx * dx + dz * dz <= reach * reach {
                gfx.draw_indexed(&p.verts, &p.indices, p.texture.as_ref(), &m);
                draws += 1;
                verts += p.verts.len() as u32;
            }
        }
        for c in &self.chunks {
            let (dx, dz) = (c.center.x - eye.x, c.center.z - eye.z);
            if dx * dx + dz * dz <= DRAW_DISTANCE * DRAW_DISTANCE {
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
        alloc::boxed::Box::leak(out.into_boxed_slice())
    }

    fn floor() -> Vec<u8> {
        collision::encode(&[([[-5., 0., -5.], [-5., 0., 5.], [5., 0., 5.]], 1, 0)])
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
        assert_eq!(m.indices, [0, 1, 2, 0, 2, 3]);
        let t = m.texture.as_ref().unwrap();
        assert_eq!(
            (t.width, t.height, t.format, t.alpha_test),
            (8, 4, 14, true)
        );
        assert!((m.center - v3(0.5, 0.0, 0.5)).length() < 1e-6);
        assert_eq!(park.vertex_count(), 4);
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
