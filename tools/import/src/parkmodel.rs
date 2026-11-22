//! A park's own models and textures, from the district's `Pres` stream, as `park/mesh/<n>` and
//! `park/tex/<n>` entries (format in `game_core::world`).
//!
//! Models are RW4 arenas with several meshes, already in world space. Each mesh descriptor (`0x00EB0023`)
//! points at its vertex descriptor, vertex buffer and index buffer descriptors; each buffer descriptor
//! follows its raw buffer. The texture a mesh uses: its descriptor's material handle (+0x24) -> the
//! external-reference table (`0x00EB000B`: GUID -> handle) -> the material whose `Name` carries that GUID
//! (`0x00EB0005`) -> its `diffuse` parameter (`...0x<16 hex>`) -> the texture asset named
//! `0x<16 hex>.Texture`. Chain as worked out by SK8's vendored map tools (prepare_hawaiian_dream.py,
//! utt's mdl_parser).

use std::collections::HashMap;

use crate::mesh::{T_IB_DESC, T_MESH_DESC, T_RAW_BUFFER, T_VB_DESC, T_VERTEX_DESC};
use crate::rw4::{Arena, be32};
use crate::world::AssetRecord;
use crate::{gx, pack, texture};
use game_core::pack::Kind;

const T_MATERIALS: u32 = 0x00eb_0005;
const T_EXTERNAL_REFS: u32 = 0x00eb_000b;
const T_MATERIAL_IMPORT: u32 = 0x00eb_0066;
pub(crate) const ASSET_MODEL: u32 = 4;
pub(crate) const ASSET_TEXTURE: u32 = 0x1000;

const FMT_FLOAT3: u32 = 0x002a_23b9;
const FMT_HALF2: u32 = 0x002c_235f;
const FMT_HALF4: u32 = 0x001a_2360;
const FMT_FLOAT2: u32 = 0x002c_23a5;
const FMT_SHORT2N: u32 = 0x002c_2159;
const USAGE_POSITION: u8 = 0;
const USAGE_TEXCOORD: u8 = 5;

type Res<T> = Result<T, String>;

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let man = (h & 0x3ff) as f32;
    sign * match exp {
        0 => man * 2f32.powi(-24),
        31 => f32::INFINITY,
        e => (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    }
}

/// A zero-terminated string at `at` (empty if out of range).
fn cstr(b: &[u8], at: usize) -> &str {
    let Some(s) = b.get(at..) else {
        return "";
    };
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    std::str::from_utf8(&s[..end]).unwrap_or("")
}

/// The `0x<16 hex>` id in a texture name, lower case.
fn hex_id(s: &str) -> Option<String> {
    let i = s.rfind("0x")?;
    let h = s.get(i + 2..i + 18)?;
    h.chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| format!("0x{}", h.to_ascii_lowercase()))
}

/// A texture asset's id: the last `0x<16 hex>.Texture` name in it.
pub(crate) fn texture_name(data: &[u8]) -> Option<String> {
    let pat = b".Texture";
    let mut found = None;
    for i in 18..=data.len().saturating_sub(pat.len()) {
        if &data[i..i + pat.len()] == pat
            && let Ok(s) = std::str::from_utf8(&data[i - 18..i])
            && s.starts_with("0x")
        {
            found = hex_id(s);
        }
    }
    found
}

/// Material handle -> diffuse texture id, for one model arena.
fn material_textures(a: &Arena) -> HashMap<u32, String> {
    let b = a.bytes;
    // Materials in order: (Name GUID, diffuse texture id).
    let mut mats: Vec<(u64, Option<String>)> = Vec::new();
    for s in a.sections.iter().filter(|s| s.type_id == T_MATERIALS) {
        let base = s.info;
        if base + 32 > b.len() {
            continue;
        }
        let (count, header, params) = (
            be32(b, base + 4) as usize,
            be32(b, base + 12) as usize,
            be32(b, base + 16) as usize,
        );
        if count == 0 || params < header || (params - header) / count != 32 {
            continue;
        }
        for i in 0..count {
            let p = base + header + 32 * i;
            if p + 32 > b.len() {
                break;
            }
            let kind = cstr(b, base + be32(b, p) as usize);
            match kind {
                "Name" => {
                    let guid = ((be32(b, p + 16) as u64) << 32) | be32(b, p + 20) as u64;
                    mats.push((guid, None));
                }
                "diffuse" => {
                    if let Some(m) = mats.last_mut()
                        && m.1.is_none()
                    {
                        m.1 = hex_id(cstr(b, base + be32(b, p + 24) as usize));
                    }
                }
                _ => {}
            }
        }
    }
    // External references: (GUID, handle), in order.
    let mut imports: Vec<(u64, u32)> = Vec::new();
    for s in a.sections.iter().filter(|s| s.type_id == T_EXTERNAL_REFS) {
        let base = s.info;
        if base + 8 > b.len() {
            continue;
        }
        let (n, off) = (be32(b, base) as usize, be32(b, base + 4) as usize);
        for i in 0..n {
            let e = base + off + 24 * i;
            if e + 24 > b.len() {
                break;
            }
            if be32(b, e + 16) == T_MATERIAL_IMPORT {
                let guid = ((be32(b, e + 8) as u64) << 32) | be32(b, e + 12) as u64;
                imports.push((guid, be32(b, e + 20)));
            }
        }
    }
    // Each material takes the next import with its GUID.
    let mut out = HashMap::new();
    let mut cursor = 0;
    for (guid, tex) in mats {
        while cursor < imports.len() && imports[cursor].0 != guid {
            cursor += 1;
        }
        let Some(&(_, handle)) = imports.get(cursor) else {
            break;
        };
        cursor += 1;
        if let Some(t) = tex {
            out.insert(handle, t);
        }
    }
    out
}

pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    pub texture: Option<String>,
}

/// Every mesh of one model arena.
pub fn read_meshes(a: &Arena) -> Res<Vec<Mesh>> {
    let b = a.bytes;
    let mats = material_textures(a);
    let section = |i: u32, want: u32| {
        a.sections
            .get(i as usize)
            .filter(|s| s.type_id == want)
            .ok_or_else(|| format!("section {i} is not {want:#x}"))
    };
    // A buffer descriptor's raw data: the raw buffer just before it.
    let raw = |desc_index: usize| {
        a.sections
            .get(desc_index.wrapping_sub(1))
            .filter(|s| s.type_id == T_RAW_BUFFER)
            .map(|s| a.data_base + s.info)
            .ok_or("buffer descriptor without a raw buffer before it")
    };
    let mut out = Vec::new();
    for s in a.sections.iter().filter(|s| s.type_id == T_MESH_DESC) {
        let base = s.info;
        let vd = section(be32(b, base + 0x28), T_VERTEX_DESC)?;
        let ibd = section(be32(b, base + 0x30), T_IB_DESC)?;
        let vbd = section(be32(b, base + 0x34), T_VB_DESC)?;
        let n = be16(b, vd.info + 8) as usize;
        let stride = b[vd.info + 0x10 + 16 * n] as usize;
        if stride == 0 {
            return Err("vertex stride is zero".into());
        }
        let vb_size = be32(b, vbd.info + 0x20) as usize;
        let index_count = be32(b, ibd.info + 0x20) as usize;
        let (vb, ib) = (raw(vbd.index)?, raw(ibd.index)?);
        let verts = vb_size / stride;
        if vb + verts * stride > b.len() || ib + 2 * index_count > b.len() {
            return Err("mesh buffers run past the arena".into());
        }
        let mut m = Mesh {
            positions: Vec::new(),
            uvs: vec![[0.0; 2]; verts],
            indices: (0..index_count).map(|i| be16(b, ib + 2 * i)).collect(),
            texture: mats.get(&be32(b, base + 0x24)).cloned(),
        };
        for i in 0..n {
            let e = vd.info + 0x10 + 16 * i;
            let (off, fmt, usage, idx) =
                (be16(b, e + 2) as usize, be32(b, e + 4), b[e + 9], b[e + 10]);
            let at = |v: usize| vb + v * stride + off;
            match (usage, idx, fmt) {
                (USAGE_POSITION, 0, FMT_FLOAT3) => {
                    m.positions = (0..verts)
                        .map(|v| std::array::from_fn(|k| f32::from_bits(be32(b, at(v) + 4 * k))))
                        .collect()
                }
                (USAGE_TEXCOORD, 0, FMT_HALF2 | FMT_HALF4) => {
                    m.uvs = (0..verts)
                        .map(|v| std::array::from_fn(|k| half(be16(b, at(v) + 2 * k))))
                        .collect()
                }
                (USAGE_TEXCOORD, 0, FMT_FLOAT2) => {
                    m.uvs = (0..verts)
                        .map(|v| std::array::from_fn(|k| f32::from_bits(be32(b, at(v) + 4 * k))))
                        .collect()
                }
                (USAGE_TEXCOORD, 0, FMT_SHORT2N) => {
                    m.uvs = (0..verts)
                        .map(|v| {
                            std::array::from_fn(|k| be16(b, at(v) + 2 * k) as i16 as f32 / 32768.0)
                        })
                        .collect()
                }
                _ => {}
            }
        }
        if m.positions.is_empty() {
            return Err("mesh without float positions".into());
        }
        // Whole triangles only, all in range.
        m.indices.truncate(m.indices.len() / 3 * 3);
        if m.indices.iter().any(|&i| i as usize >= verts) {
            return Err("index past the vertex buffer".into());
        }
        out.push(m);
    }
    Ok(out)
}

/// Vertex RGBA with lighting baked in: the sun on the averaged face normals (ambient 0.6).
fn lit(m: &Mesh) -> Vec<u32> {
    let mut n = vec![[0f32; 3]; m.positions.len()];
    for t in m.indices.chunks_exact(3) {
        let p = |i: u16| m.positions[i as usize];
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let f = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for &i in t {
            for k in 0..3 {
                n[i as usize][k] += f[k];
            }
        }
    }
    let sun = [0.42f32, 0.82, 0.38];
    let len = (sun[0] * sun[0] + sun[1] * sun[1] + sun[2] * sun[2]).sqrt();
    n.iter()
        .map(|v| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
            let d = (v[0] * sun[0] + v[1] * sun[1] + v[2] * sun[2]) / (l * len);
            let k = (0.6 + 0.4 * d.max(0.0)).min(1.0);
            let c = (255.0 * k) as u32;
            (c << 24) | (c << 16) | (c << 8) | 0xff
        })
        .collect()
}

/// Statistics for the converter's report.
#[derive(Default, Debug)]
pub struct Report {
    pub meshes: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub textures: usize,
    pub untextured: usize,
    pub skipped: Vec<String>,
}

/// Pack items for every textured mesh and the textures they use (at most `max` pixels on a side).
pub fn convert(assets: &[(AssetRecord, Vec<u8>)], max: usize) -> (Vec<pack::Item>, Report) {
    let mut report = Report::default();
    // Texture assets by name.
    let mut by_name: HashMap<String, &[u8]> = HashMap::new();
    for (r, d) in assets.iter().filter(|(r, _)| r.asset_type == ASSET_TEXTURE) {
        match texture_name(d) {
            Some(n) => {
                by_name.entry(n).or_insert(d);
            }
            None => report
                .skipped
                .push(format!("texture {:016x}: unnamed", r.id)),
        }
    }
    let mut items = Vec::new();
    let mut tex_index: HashMap<String, Option<(u16, bool)>> = HashMap::new();
    let mut mesh_n = 0usize;
    for (r, d) in assets.iter().filter(|(r, _)| r.asset_type == ASSET_MODEL) {
        let meshes = match Arena::parse(d)
            .map_err(|e| e.to_string())
            .and_then(|a| read_meshes(&a))
        {
            Ok(m) => m,
            Err(e) => {
                report.skipped.push(format!("model {:016x}: {e}", r.id));
                continue;
            }
        };
        for m in meshes {
            // Convert each texture once, the first time a mesh uses it.
            let tex = m.texture.as_ref().and_then(|name| {
                *tex_index.entry(name.clone()).or_insert_with(|| {
                    let data = by_name.get(name)?;
                    let a = crate::rw4::Arena::parse(data).ok()?;
                    let t = texture::textures(&a).into_iter().next()?.ok()?;
                    let img = texture::decode(t.data, &t.info).ok()?;
                    let mut small = gx::shrink_to(img, max);
                    // Cut-outs (fences, foliage) keep their alpha and are alpha tested.
                    let holes = small.pixels.chunks_exact(4).filter(|p| p[3] < 128).count();
                    let alpha = holes * 100 > small.pixels.len() / 4;
                    let (fmt, data) = if alpha {
                        (gx::GX_TF_RGB5A3, gx::encode_rgb5a3(&small))
                    } else {
                        small.pixels.chunks_exact_mut(4).for_each(|p| p[3] = 255);
                        (gx::GX_TF_CMPR, gx::encode_cmpr(&small))
                    };
                    let i = report.textures as u16;
                    report.textures += 1;
                    items.push(pack::Item {
                        name: format!("park/tex/{i}"),
                        kind: Kind::Texture,
                        data,
                        width: small.width as u16,
                        height: small.height as u16,
                        gx_format: fmt,
                    });
                    Some((i, alpha))
                })
            });
            if tex.is_none() {
                report.untextured += 1;
            }
            let rgba = lit(&m);
            let verts: Vec<([f32; 3], u32, [f32; 2])> = (0..m.positions.len())
                .map(|i| (m.positions[i], rgba[i], m.uvs[i]))
                .collect();
            report.meshes += 1;
            report.vertices += verts.len();
            report.triangles += m.indices.len() / 3;
            let flags = if tex.is_some_and(|t| t.1) {
                game_core::world::MESH_ALPHA_TEST
            } else {
                0
            };
            items.push(pack::Item {
                name: format!("park/mesh/{mesh_n}"),
                kind: Kind::Data,
                data: game_core::world::encode_mesh(&verts, &m.indices, tex.map(|t| t.0), flags),
                width: 0,
                height: 0,
                gx_format: 0,
            });
            mesh_n += 1;
        }
    }
    (items, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0xc000), -2.0);
        assert_eq!(half(0x3800), 0.5);
        assert_eq!(half(0), 0.0);
    }

    #[test]
    fn texture_names() {
        let mut d = b"junk\0data/0x00ABCDEF01234567.Texture\0".to_vec();
        d.extend_from_slice(b"more 0x1111222233334444.Texture");
        assert_eq!(texture_name(&d).as_deref(), Some("0x1111222233334444"));
        assert_eq!(
            hex_id("concrete_0xDEADBEEF00112233").as_deref(),
            Some("0xdeadbeef00112233")
        );
        assert_eq!(hex_id("no id here"), None);
    }
}
