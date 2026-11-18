//! Skinned character meshes in RW4 arenas (`.rx2`): bones with inverse bind matrices (`tRModelData`) and
//! meshes (vertex descriptor, vertex and index buffers, skin palette).
//! Layout as documented by SK8-ENGINE/skate-3-rust-engine (tools/vendor/skate3_anim/rx2_skeleton.py, GPL-3.0).

use crate::rw4::{Arena, Section, be32};

pub const T_MODEL_DATA: u32 = 0x00EB_0001;
pub const T_MESH_DESC: u32 = 0x00EB_0023;
pub const T_RAW_BUFFER: u32 = 0x0001_0031;
pub const T_VERTEX_DESC: u32 = 0x0002_00E9;
pub const T_VB_DESC: u32 = 0x0002_00EA;
pub const T_IB_DESC: u32 = 0x0002_00EB;

/// Character positions are int16 with a fixed 2^-14 scale and +0.8 m on Y, baked into the game's shader.
const POS_SCALE: f32 = 1.0 / 16384.0;
const POS_BIAS_Y: f32 = 0.8;

const FMT_SHORT4: u32 = 0x001A_215A;
const FMT_UBYTE4: u32 = 0x001A_2286;
const FMT_SHORT2N: u32 = 0x002C_2159;
const FMT_FLOAT2: u32 = 0x002C_23A5;
const FMT_PACKED_11_11_10: u32 = 0x002A_2190;

const USAGE_POSITION: u8 = 0;
const USAGE_BLENDWEIGHT: u8 = 1;
const USAGE_BLENDINDICES: u8 = 2;
const USAGE_TEXCOORD: u8 = 5;
const USAGE_TANGENT: u8 = 6;
const USAGE_BINORMAL: u8 = 7;

/// 4x4 matrix, row-major with row vectors (`v' = v * M`, translation in row 3), as the files store it.
pub type Mat4 = [[f32; 4]; 4];

#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    pub inverse_bind: Mat4,
}

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Indices into the model's `bones` (palette already resolved).
    pub skin_bones: Vec<[u16; 4]>,
    pub skin_weights: Vec<[f32; 4]>,
    pub indices: Vec<u16>,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub bones: Vec<Bone>,
    pub meshes: Vec<Mesh>,
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_bits(be32(b, at))
}
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn need(b: &[u8], at: usize, n: usize) -> Result<(), String> {
    if at.checked_add(n).is_some_and(|e| e <= b.len()) {
        Ok(())
    } else {
        Err(format!("read {at:#x}+{n} out of range"))
    }
}

/// File offset of a section's own data: raw GPU buffers are relative to the arena base.
fn file_offset(a: &Arena, s: &Section) -> usize {
    if s.type_id == T_RAW_BUFFER {
        a.data_base + s.info
    } else {
        s.info
    }
}

fn read_bones(b: &[u8], base: usize) -> Result<Vec<Bone>, String> {
    need(b, base, 0x40)?;
    let (p_ibp, p_names, p_pool) = (
        be32(b, base + 0x20) as usize,
        be32(b, base + 0x28) as usize,
        be32(b, base + 0x2C) as usize,
    );
    let n = u16_at(b, base + 0x34) as usize;
    if p_pool < p_names || (p_pool - p_names) / 4 != n {
        return Err("bone name table does not match the bone count".into());
    }
    need(b, base + p_ibp, 64 * n)?;
    (0..n)
        .map(|i| {
            let name_at = base + be32(b, base + p_names + 4 * i) as usize;
            let tail = b.get(name_at..).ok_or("bone name out of range")?;
            let end = tail
                .iter()
                .position(|&c| c == 0)
                .ok_or("unterminated bone name")?;
            let m = base + p_ibp + 64 * i;
            Ok(Bone {
                name: String::from_utf8_lossy(&tail[..end]).into_owned(),
                inverse_bind: std::array::from_fn(|r| {
                    std::array::from_fn(|c| f32_at(b, m + 16 * r + 4 * c))
                }),
            })
        })
        .collect()
}

struct Element {
    offset: usize,
    format: u32,
    usage: u8,
    usage_index: u8,
}

fn signed_11_11_10(u: u32) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (o, (shift, bits)) in out.iter_mut().zip([(0, 11), (11, 11), (22, 10)]) {
        let mut v = ((u >> shift) & ((1 << bits) - 1)) as i32;
        if v >= 1 << (bits - 1) {
            v -= 1 << bits;
        }
        *o = v as f32 / ((1 << (bits - 1)) - 1) as f32;
    }
    out
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (l > 1e-3).then(|| v.map(|c| c / l))
}

/// A section referenced by index from a mesh descriptor; falls back to the first of the wanted type.
fn section_ref<'s>(a: &'s Arena, index: u32, want: u32) -> Option<&'s Section> {
    a.sections
        .get(index as usize)
        .filter(|s| s.type_id == want)
        .or_else(|| a.of_type(want).next())
}

fn read_mesh(a: &Arena, base: usize, bone_count: usize) -> Result<Mesh, String> {
    let b = a.bytes;
    need(b, base, 0x50)?;
    let (count, off) = (be32(b, base + 0x48) as usize, be32(b, base + 0x4C) as usize);
    need(b, base + off, 2 * count)?;
    let palette: Vec<u16> = (0..count).map(|i| u16_at(b, base + off + 2 * i)).collect();
    let vdesc =
        section_ref(a, be32(b, base + 0x28), T_VERTEX_DESC).ok_or("no vertex descriptor")?;
    let ibd =
        section_ref(a, be32(b, base + 0x30), T_IB_DESC).ok_or("no index buffer descriptor")?;
    let vbd =
        section_ref(a, be32(b, base + 0x34), T_VB_DESC).ok_or("no vertex buffer descriptor")?;

    let vd = file_offset(a, vdesc);
    need(b, vd, 0x10)?;
    let n = u16_at(b, vd + 8) as usize;
    need(b, vd + 0x10, 16 * n + n)?;
    let elements: Vec<Element> = (0..n)
        .map(|i| {
            let e = vd + 0x10 + 16 * i;
            Element {
                offset: u16_at(b, e + 2) as usize,
                format: be32(b, e + 4),
                usage: b[e + 9],
                usage_index: b[e + 10],
            }
        })
        .collect();
    let stride = b[vd + 0x10 + 16 * n] as usize;
    if stride == 0 {
        return Err("vertex stride is zero".into());
    }
    need(b, file_offset(a, vbd), 0x24)?;
    need(b, file_offset(a, ibd), 0x24)?;
    let vb_size = be32(b, file_offset(a, vbd) + 0x20) as usize;
    let ib_size = be32(b, file_offset(a, ibd) + 0x1C) as usize;
    let index_count = be32(b, file_offset(a, ibd) + 0x20) as usize;

    // Raw buffers are matched to their descriptors by size; the vertex buffer comes first in the arena.
    let mut raws: Vec<&Section> = a.of_type(T_RAW_BUFFER).collect();
    raws.sort_by_key(|s| s.info);
    let vb = raws
        .iter()
        .find(|s| s.size == vb_size)
        .or(raws.first())
        .ok_or("no vertex buffer")?;
    let ib = raws
        .iter()
        .find(|s| s.size == ib_size && s.index != vb.index)
        .or(raws.get(1))
        .ok_or("no index buffer")?;
    let (vb_at, ib_at) = (file_offset(a, vb), file_offset(a, ib));
    let verts = vb_size / stride;
    need(b, vb_at, verts * stride)?;
    need(b, ib_at, 2 * index_count)?;

    let mut m = Mesh {
        indices: (0..index_count).map(|i| u16_at(b, ib_at + 2 * i)).collect(),
        ..Default::default()
    };
    if m.indices.iter().any(|&i| i as usize >= verts) {
        return Err("index past the vertex buffer".into());
    }
    let (mut tangents, mut binormals) = (Vec::new(), Vec::new());
    let mut skin_idx: Vec<[u8; 4]> = Vec::new();
    for e in &elements {
        let at = |v: usize| vb_at + v * stride + e.offset;
        match (e.usage, e.usage_index, e.format) {
            (USAGE_POSITION, 0, FMT_SHORT4) => {
                m.positions = (0..verts)
                    .map(|v| {
                        let p = at(v);
                        let s = |k: usize| u16_at(b, p + 2 * k) as i16 as f32 * POS_SCALE;
                        [s(0), s(1) + POS_BIAS_Y, s(2)]
                    })
                    .collect()
            }
            (USAGE_TEXCOORD, 0, FMT_SHORT2N) => {
                m.uvs = (0..verts)
                    .map(|v| {
                        std::array::from_fn(|k| u16_at(b, at(v) + 2 * k) as i16 as f32 / 32768.0)
                    })
                    .collect()
            }
            (USAGE_TEXCOORD, 0, FMT_FLOAT2) => {
                m.uvs = (0..verts)
                    .map(|v| std::array::from_fn(|k| f32_at(b, at(v) + 4 * k)))
                    .collect()
            }
            (USAGE_BLENDWEIGHT, 0, FMT_UBYTE4) => {
                m.skin_weights = (0..verts)
                    .map(|v| std::array::from_fn(|k| b[at(v) + k] as f32 / 255.0))
                    .collect()
            }
            (USAGE_BLENDINDICES, 0, FMT_UBYTE4) => {
                skin_idx = (0..verts)
                    .map(|v| std::array::from_fn(|k| b[at(v) + k]))
                    .collect()
            }
            (USAGE_TANGENT, 0, FMT_PACKED_11_11_10) => {
                tangents = (0..verts)
                    .map(|v| signed_11_11_10(be32(b, at(v))))
                    .collect()
            }
            (USAGE_BINORMAL, 0, FMT_PACKED_11_11_10) => {
                binormals = (0..verts)
                    .map(|v| signed_11_11_10(be32(b, at(v))))
                    .collect()
            }
            _ => {}
        }
    }
    if m.positions.is_empty() {
        return Err("mesh has no positions".into());
    }
    if m.uvs.is_empty() {
        m.uvs = vec![[0.0; 2]; verts];
    }
    // Resolve palette-local blend indices to model bones. Unskinned meshes bind fully to bone 0.
    m.skin_bones = if skin_idx.is_empty() {
        vec![[0; 4]; verts]
    } else {
        skin_idx
            .iter()
            .map(|q| {
                q.map(|i| {
                    palette
                        .get(i as usize)
                        .copied()
                        .unwrap_or(0)
                        .min(bone_count.saturating_sub(1) as u16)
                })
            })
            .collect()
    };
    if m.skin_weights.is_empty() {
        m.skin_weights = vec![[1.0, 0.0, 0.0, 0.0]; verts];
    }
    // No normal is stored: the shader rebuilds it as cross(tangent, binormal). Degenerate frames fall back to
    // the area-weighted face normal.
    let mut face = vec![[0f32; 3]; verts];
    for t in m.indices.chunks_exact(3) {
        let (p0, p1, p2) = (
            m.positions[t[0] as usize],
            m.positions[t[1] as usize],
            m.positions[t[2] as usize],
        );
        let n = cross(
            std::array::from_fn(|k| p1[k] - p0[k]),
            std::array::from_fn(|k| p2[k] - p0[k]),
        );
        for &i in t {
            for k in 0..3 {
                face[i as usize][k] += n[k];
            }
        }
    }
    m.normals = (0..verts)
        .map(|v| {
            let framed = (tangents.len() == verts && binormals.len() == verts)
                .then(|| normalize(cross(tangents[v], binormals[v])))
                .flatten();
            framed
                .or_else(|| normalize(face[v]))
                .unwrap_or([0.0, 1.0, 0.0])
        })
        .collect();
    Ok(m)
}

pub fn read_model(a: &Arena) -> Result<Model, String> {
    let md = a
        .of_type(T_MODEL_DATA)
        .next()
        .ok_or("no tRModelData section")?;
    let bones = read_bones(a.bytes, md.info)?;
    let meshes = a
        .of_type(T_MESH_DESC)
        .map(|s| read_mesh(a, s.info, bones.len()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Model { bones, meshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_tangent_components() {
        assert_eq!(signed_11_11_10(1023), [1.0, 0.0, 0.0]);
        let neg = signed_11_11_10(0x7FF); // -1 in 11 bits
        assert!((neg[0] + 1.0 / 1023.0).abs() < 1e-6);
        let z = signed_11_11_10(511 << 22);
        assert_eq!(z, [0.0, 0.0, 1.0]);
    }
}
