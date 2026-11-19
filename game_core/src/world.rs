//! An imported park (`park.fgpak` from `fg_import park`): its collision, plus a mesh to draw.
//!
//! Until the park's own textured models are converted, the collision triangles are drawn themselves,
//! flat shaded and coloured by surface, in chunks so only the ones near the camera are sent each frame.

use alloc::vec::Vec;

use crate::collision::{self, TriMesh, WALKABLE_Y};
use crate::gfx::{Gfx, SUN, Vertex, rgb, shade};
use crate::math::{Vec3, Xform, v3};
use crate::pack::Pack;

/// Chunk edge for drawing (m).
const CHUNK: f32 = 16.0;
/// Chunks further than this from the camera (m, centre to eye, flat) are skipped.
pub const DRAW_DISTANCE: f32 = 90.0;

pub struct Chunk {
    pub center: Vec3,
    pub verts: Vec<Vertex>,
}

pub struct ImportedPark {
    pub mesh: TriMesh,
    pub chunks: Vec<Chunk>,
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
    pub fn parse(bytes: &[u8]) -> Result<ImportedPark, collision::Error> {
        let pack = Pack::parse(bytes).map_err(|_| collision::Error::Bad)?;
        let mesh = TriMesh::from_pack(&pack)?;
        let chunks = Self::build_chunks(&mesh);
        Ok(ImportedPark { mesh, chunks })
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

    /// Draws the chunks within [`DRAW_DISTANCE`] of `eye`.
    pub fn draw(&self, gfx: &mut dyn Gfx, eye: Vec3) {
        let m = Xform::IDENTITY.to_mat34();
        for c in &self.chunks {
            let (dx, dz) = (c.center.x - eye.x, c.center.z - eye.z);
            if dx * dx + dz * dz <= DRAW_DISTANCE * DRAW_DISTANCE {
                gfx.draw_tris(&c.verts, &m);
            }
        }
    }

    pub fn vertex_count(&self) -> usize {
        self.chunks.iter().map(|c| c.verts.len()).sum()
    }
}
