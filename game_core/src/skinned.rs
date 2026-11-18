//! Skinned meshes as stored in `.fgpak` entries of kind `SkinnedMesh`, read in place. Big-endian.
//! Written on the PC by `tools/import`; animated by `anim::Clip` on the skeleton named alongside it.
//!
//! ```text
//! 0   u16 vertex count, u16 index count, u16 palette entries, u16 flags (FLAG_ALPHA_TEST)
//! 8   palette, 52 bytes each: u16 skeleton bone, u16 reserved,
//!     f32 bind[4][3]  (row vectors: rows 0..2 rotation, row 3 translation; takes a bind-pose vertex into
//!                     the space of that skeleton bone, so skin matrix = bind * bone world)
//! ..  vertices, VERTEX_SIZE bytes each:
//!     f32 position[3], i8 normal[3] (/127), u8 pad, i16 uv[2] (/4096), u8 palette index[4], u8 weight[4] (/255)
//! ..  u16 indices (triangle list)
//! ```

pub const PALETTE_SIZE: usize = 52;
pub const VERTEX_SIZE: usize = 28;
pub const HEADER_SIZE: usize = 8;
pub const FLAG_ALPHA_TEST: u16 = 1;

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub bones: [u8; 4],
    pub weights: [f32; 4],
}

pub struct SkinnedMesh<'a> {
    data: &'a [u8],
    vertices: usize,
    indices: usize,
    palette: usize,
}

impl<'a> SkinnedMesh<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, Truncated> {
        let h = data.get(..HEADER_SIZE).ok_or(Truncated)?;
        let (vertices, indices, palette) = (
            u16_at(h, 0) as usize,
            u16_at(h, 2) as usize,
            u16_at(h, 4) as usize,
        );
        if data.len() < HEADER_SIZE + palette * PALETTE_SIZE + vertices * VERTEX_SIZE + indices * 2
        {
            return Err(Truncated);
        }
        Ok(SkinnedMesh {
            data,
            vertices,
            indices,
            palette,
        })
    }
    pub fn vertex_count(&self) -> usize {
        self.vertices
    }
    pub fn index_count(&self) -> usize {
        self.indices
    }
    pub fn palette_len(&self) -> usize {
        self.palette
    }
    pub fn alpha_test(&self) -> bool {
        u16_at(self.data, 6) & FLAG_ALPHA_TEST != 0
    }
    /// (skeleton bone, bind matrix rows) of palette entry `i`.
    pub fn palette(&self, i: usize) -> (u16, [[f32; 3]; 4]) {
        let at = HEADER_SIZE + i * PALETTE_SIZE;
        (
            u16_at(self.data, at),
            core::array::from_fn(|r| {
                core::array::from_fn(|c| f32_at(self.data, at + 4 + 12 * r + 4 * c))
            }),
        )
    }
    pub fn vertex(&self, i: usize) -> Vertex {
        let d = self.data;
        let at = HEADER_SIZE + self.palette * PALETTE_SIZE + i * VERTEX_SIZE;
        Vertex {
            position: core::array::from_fn(|k| f32_at(d, at + 4 * k)),
            normal: core::array::from_fn(|k| d[at + 12 + k] as i8 as f32 / 127.0),
            uv: core::array::from_fn(|k| u16_at(d, at + 16 + 2 * k) as i16 as f32 / 4096.0),
            bones: core::array::from_fn(|k| d[at + 20 + k]),
            weights: core::array::from_fn(|k| d[at + 24 + k] as f32 / 255.0),
        }
    }
    pub fn index(&self, i: usize) -> u16 {
        u16_at(
            self.data,
            HEADER_SIZE + self.palette * PALETTE_SIZE + self.vertices * VERTEX_SIZE + 2 * i,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_data() {
        assert!(SkinnedMesh::parse(&[0, 1, 0, 3, 0, 0, 0, 0]).is_err());
        assert!(SkinnedMesh::parse(&[0; 4]).is_err());
        assert_eq!(SkinnedMesh::parse(&[0; 8]).unwrap().vertex_count(), 0);
    }
}
