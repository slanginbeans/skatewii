//! Skate 3 world districts: the streamed asset files (`ATOC` tables + `SFIL` stream files inside
//! `worldDIST_*.big`) and the RenderWare ClusteredMesh collision inside their simulation assets.
//!
//! Ported from SK8-ENGINE/skate-3-rust-engine's vendored map tools (`skate3_streams.py`,
//! `retail_collision_mesh.py`), including SK8's correction that 16-bit compressed vertex offsets are
//! unsigned, as the game reads them. Collision is already in the game's world space: Y up, metres.

use crate::refpack;
use crate::rw4::be32;

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn be64(b: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(b[at..at + 8].try_into().unwrap())
}
fn bef(b: &[u8], at: usize) -> f32 {
    f32::from_bits(be32(b, at))
}

type Res<T> = Result<T, String>;

fn need(ok: bool, what: &str) -> Res<()> {
    if ok { Ok(()) } else { Err(what.to_string()) }
}

// ---- Streams ----------------------------------------------------------------------------------------

const ATOC_HEADER: usize = 24;
const ATOC_RECORD: usize = 64;
const SFIL_HEADER: usize = 128;
const SFIL_SECTION_TABLE: usize = 128;
const SFIL_SECTION_DATA: usize = 208;

/// One asset listed in a stream's `ATOC` table (`<district>_<stream>.xst`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetRecord {
    pub id: u64,
    pub total_size: u32,
    pub asset_type: u32,
}

pub fn read_atoc(data: &[u8]) -> Res<Vec<AssetRecord>> {
    need(
        data.len() >= ATOC_HEADER && &data[..4] == b"ATOC",
        "not an ATOC table",
    )?;
    need(be32(data, 4) == 3, "unsupported ATOC version")?;
    let count = be32(data, 16) as usize;
    need(
        data.len() == ATOC_HEADER + count * ATOC_RECORD,
        "ATOC size doesn't match its record count",
    )?;
    Ok((0..count)
        .map(|i| {
            let r = ATOC_HEADER + i * ATOC_RECORD;
            AssetRecord {
                id: be64(data, r),
                total_size: be32(data, r + 12),
                asset_type: be32(data, r + 36),
            }
        })
        .collect())
}

fn section(data: &[u8], at: usize, size: usize, stored: usize, compression: u32) -> Res<Vec<u8>> {
    if size == 0 && stored == 0 {
        return Ok(Vec::new());
    }
    let payload = data
        .get(at..at + stored)
        .ok_or("SFIL section runs past its file")?;
    let out = match compression {
        0 => payload.to_vec(),
        1 => refpack::decompress(payload, Some(size)).map_err(|e| e.0)?,
        c => return Err(format!("unsupported SFIL compression {c}")),
    };
    need(out.len() == size, "SFIL section decoded to the wrong size")?;
    Ok(out)
}

/// Every asset of one `SFIL` stream file, decoded: (record, bytes). Assets not in `records` are an
/// error (each stream file belongs to one table).
pub fn read_sfil(data: &[u8], records: &[AssetRecord]) -> Res<Vec<(AssetRecord, Vec<u8>)>> {
    need(
        data.len() >= SFIL_HEADER && &data[..4] == b"SFIL",
        "not an SFIL stream",
    )?;
    let first = be32(data, 16) as usize;
    need(
        (SFIL_HEADER..=data.len()).contains(&first),
        "SFIL has a bad first-asset offset",
    )?;
    let mut out = Vec::new();
    let mut at = first;
    while at < data.len() {
        // Zero padding to the end.
        if data[at..].iter().all(|&b| b == 0) {
            break;
        }
        need(at + SFIL_SECTION_DATA <= data.len(), "truncated SFIL asset")?;
        let id = be64(data, at);
        let stored = be32(data, at + 8) as usize;
        let header = be32(data, at + 12) as usize;
        let stride = be32(data, at + 16) as usize;
        let record = *records
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("SFIL asset {id:016x} isn't in its ATOC"))?;
        let bytes = if stored == record.total_size as usize && header >= SFIL_HEADER {
            data.get(at + header..at + header + stored)
                .ok_or("raw SFIL asset runs past its file")?
                .to_vec()
        } else {
            let v = |i: usize| be32(data, at + SFIL_SECTION_TABLE + 4 * i);
            let (cpu_size, cpu_stored, cpu_comp) = (v(0) as usize, v(1) as usize, v(2));
            let (gpu_size, gpu_stored, gpu_comp) = (v(8) as usize, v(9) as usize, v(10));
            need(
                stored == 0x50 + cpu_stored + gpu_stored,
                "SFIL asset sizes don't add up",
            )?;
            let cpu_at = at + SFIL_SECTION_DATA;
            let mut b = section(data, cpu_at, cpu_size, cpu_stored, cpu_comp)?;
            b.extend(section(
                data,
                cpu_at + cpu_stored,
                gpu_size,
                gpu_stored,
                gpu_comp,
            )?);
            b
        };
        need(
            bytes.len() == record.total_size as usize,
            "SFIL asset decoded to a size its ATOC doesn't declare",
        )?;
        need(
            stride >= header + stored && stride.is_multiple_of(first) && at + stride <= data.len(),
            "SFIL asset has a bad stride",
        )?;
        out.push((record, bytes));
        at += stride;
    }
    Ok(out)
}

// ---- Collision ---------------------------------------------------------------------------------------

/// RW4 section type of a ClusteredMesh.
pub const TYPE_CLUSTERED_MESH: u32 = 0x0008_0006;
const MESH_HEADER: usize = 96;
const CLUSTER_HEADER: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub v: [[f32; 3]; 3],
    /// Surface id (material: concrete, wood, grass, ...); 0 when the unit has none.
    pub surface: u16,
    /// The unit's flag byte (0x80: has a surface id).
    pub flags: u8,
}

/// The sections of an Xbox 360 RW4 resource: (type, bytes).
pub fn rx2_sections(data: &[u8]) -> Res<Vec<(u32, &[u8])>> {
    need(
        data.len() >= 0x34 && &data[..7] == b"\x89RW4xb2",
        "not an Xbox 360 RW4 resource",
    )?;
    let count = be32(data, 0x20) as usize;
    let table = be32(data, 0x30) as usize;
    need(
        table + count * 24 <= data.len(),
        "RW4 section table runs past the file",
    )?;
    (0..count)
        .map(|i| {
            let r = table + i * 24;
            let (at, size, ty) = (
                be32(data, r) as usize,
                be32(data, r + 8) as usize,
                be32(data, r + 20),
            );
            let bytes = data
                .get(at..at + size)
                .ok_or("RW4 section runs past the file")?;
            Ok((ty, bytes))
        })
        .collect()
}

/// Little-endian id of 1 or 2 bytes (the one little-endian field in these big-endian blobs).
fn read_id(c: &[u8], at: &mut usize, width: u8) -> Res<u16> {
    let w = width as usize;
    need(w == 1 || w == 2, "unsupported collision id width")?;
    let b = c
        .get(*at..*at + w)
        .ok_or("collision id runs past its cluster")?;
    *at += w;
    Ok(if w == 1 {
        b[0] as u16
    } else {
        u16::from_le_bytes([b[0], b[1]])
    })
}

fn decode_cluster(
    c: &[u8],
    granularity: f32,
    group_width: u8,
    surface_width: u8,
    out: &mut Vec<Triangle>,
) -> Res<()> {
    need(c.len() >= CLUSTER_HEADER, "cluster smaller than its header")?;
    let units = be16(c, 0) as usize;
    let unit_bytes = be16(c, 2) as usize;
    let vertex_blocks = be16(c, 4) as usize;
    let vertex_count = c[10] as usize;
    let compression = c[12];
    let unit_at = (vertex_blocks + 1) * 16;
    let unit_end = unit_at + unit_bytes;
    need(
        CLUSTER_HEADER <= unit_at && unit_end <= c.len(),
        "cluster has bad vertex/unit spans",
    )?;
    let mut verts = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let v = match compression {
            // 16-bit offsets from a 32-bit base, unsigned (as the game reads them).
            1 => {
                let at = CLUSTER_HEADER + 12 + i * 6;
                need(at + 6 <= unit_at, "16-bit vertex runs into the units")?;
                let base = |k: usize| be32(c, CLUSTER_HEADER + 4 * k) as i32 as i64;
                std::array::from_fn(|k| {
                    ((base(k) + be16(c, at + 2 * k) as i64) as f64 * granularity as f64) as f32
                })
            }
            2 => {
                let at = CLUSTER_HEADER + i * 12;
                need(at + 12 <= unit_at, "32-bit vertex runs into the units")?;
                std::array::from_fn(|k| {
                    (be32(c, at + 4 * k) as i32 as f64 * granularity as f64) as f32
                })
            }
            0 => {
                let at = CLUSTER_HEADER + i * 16;
                need(at + 16 <= unit_at, "vertex runs into the units")?;
                std::array::from_fn(|k| bef(c, at + 4 * k))
            }
            x => return Err(format!("unsupported cluster compression {x}")),
        };
        let v: [f32; 3] = v;
        need(
            v.iter().all(|x| x.is_finite()),
            "non-finite collision vertex",
        )?;
        verts.push(v);
    }
    let mut at = unit_at;
    for _ in 0..units {
        need(at < unit_end, "unit stream ends early")?;
        let flags = c[at];
        at += 1;
        let tris = match flags & 0x0f {
            1 => 1,
            2 => 2,
            3 => return Err("triangle-list collision units aren't supported yet".into()),
            t => return Err(format!("unsupported collision unit type {t}")),
        };
        let n = tris + 2;
        let idx = c
            .get(at..at + n)
            .ok_or("unit indices run past the stream")?
            .to_vec();
        at += n;
        need(
            idx.iter().all(|&i| (i as usize) < vertex_count),
            "unit refers to a missing vertex",
        )?;
        if flags & 0x20 != 0 {
            at += n; // edge codes (not needed for contact)
        }
        if flags & 0x40 != 0 {
            read_id(c, &mut at, group_width)?;
        }
        let surface = if flags & 0x80 != 0 {
            read_id(c, &mut at, surface_width)?
        } else {
            0
        };
        need(at <= unit_end, "unit runs past the stream")?;
        let v = |i: u8| verts[i as usize];
        // RenderWare's split of a quad: 0-1-2, then 3-2-1.
        let mut push = |a: u8, b: u8, cc: u8| {
            out.push(Triangle {
                v: [v(a), v(b), v(cc)],
                surface,
                flags,
            })
        };
        push(idx[0], idx[1], idx[2]);
        if tris == 2 {
            push(idx[3], idx[2], idx[1]);
        }
    }
    need(at == unit_end, "unit stream has trailing bytes")?;
    Ok(())
}

/// Every triangle of one serialized ClusteredMesh section.
pub fn decode_clustered_mesh(d: &[u8]) -> Res<Vec<Triangle>> {
    need(d.len() >= MESH_HEADER, "collision mesh too small")?;
    let expected = be32(d, 40) as usize;
    let cluster_table = be32(d, 52) as usize;
    let granularity = bef(d, 56);
    let group_width = d[62];
    let surface_width = d[63];
    let clusters = be32(d, 64) as usize;
    let mesh_bytes = be32(d, 80) as usize;
    need(
        granularity.is_finite() && granularity > 0.0,
        "bad collision granularity",
    )?;
    need(
        (MESH_HEADER..=d.len()).contains(&mesh_bytes),
        "bad collision mesh size",
    )?;
    need(
        cluster_table >= MESH_HEADER && cluster_table + clusters * 4 <= mesh_bytes,
        "bad cluster table",
    )?;
    let offsets: Vec<usize> = (0..clusters)
        .map(|i| be32(d, cluster_table + 4 * i) as usize)
        .collect();
    need(
        offsets.windows(2).all(|w| w[0] < w[1]),
        "cluster offsets out of order",
    )?;
    let mut out = Vec::with_capacity(expected);
    for (i, &at) in offsets.iter().enumerate() {
        let end = offsets.get(i + 1).copied().unwrap_or(mesh_bytes);
        need(at + CLUSTER_HEADER <= end, "cluster outside the mesh")?;
        let size = be16(d, at + 8) as usize;
        need(at + size <= end, "cluster overlaps the next one")?;
        decode_cluster(
            &d[at..at + size],
            granularity,
            group_width,
            surface_width,
            &mut out,
        )
        .map_err(|e| format!("cluster {i}: {e}"))?;
    }
    need(
        out.len() == expected,
        "triangle count doesn't match the header",
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be32v(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_be_bytes());
    }

    /// One cluster with 16-bit vertices (one offset above 0x7fff, which must stay positive) and a quad
    /// unit with a surface id.
    fn synthetic_mesh() -> Vec<u8> {
        let mut cl = vec![0u8; 16];
        // header: units 1, unit bytes, vertex blocks, -, cluster bytes, vertex count 4, -, compression 1
        let base = [1000i32, 0, -2000];
        let verts: [[u16; 3]; 4] = [[0, 0, 0], [40000, 0, 0], [0, 0, 1000], [40000, 0, 1000]];
        let mut body = Vec::new();
        for b in base {
            body.extend_from_slice(&b.to_be_bytes());
        }
        for v in verts {
            for x in v {
                body.extend_from_slice(&x.to_be_bytes());
            }
        }
        // Pad vertices to whole 16-byte blocks after the header.
        let blocks = body.len().div_ceil(16);
        body.resize(blocks * 16, 0);
        // Quad unit: flags (type 2 | has surface), 4 indices, surface id 7 (1 byte, little-endian).
        let units = [0x82u8, 0, 1, 2, 3, 7];
        let unit_bytes = units.len();
        cl.extend_from_slice(&body);
        cl.extend_from_slice(&units);
        let size = cl.len();
        cl[0..2].copy_from_slice(&1u16.to_be_bytes());
        cl[2..4].copy_from_slice(&(unit_bytes as u16).to_be_bytes());
        cl[4..6].copy_from_slice(&(blocks as u16).to_be_bytes());
        cl[8..10].copy_from_slice(&(size as u16).to_be_bytes());
        cl[10] = 4;
        cl[12] = 1;

        let mut m = vec![0u8; MESH_HEADER];
        m[40..44].copy_from_slice(&2u32.to_be_bytes());
        let table = MESH_HEADER;
        m[52..56].copy_from_slice(&(table as u32).to_be_bytes());
        m[56..60].copy_from_slice(&0.001f32.to_be_bytes());
        m[62] = 1;
        m[63] = 1;
        m[64..68].copy_from_slice(&1u32.to_be_bytes());
        let cluster_at = table + 4;
        be32v(&mut m, cluster_at as u32);
        m.extend_from_slice(&cl);
        let total = m.len() as u32;
        m[80..84].copy_from_slice(&total.to_be_bytes());
        m
    }

    #[test]
    fn decodes_a_clustered_mesh() {
        let tris = decode_clustered_mesh(&synthetic_mesh()).unwrap();
        assert_eq!(tris.len(), 2);
        // 1000 + 40000 (unsigned) = 41000 * 0.001 = 41 m; signed would have wrapped to -24.5 m.
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4);
        assert!(close(tris[0].v[1], [41.0, 0.0, -2.0]), "{:?}", tris[0].v[1]);
        assert!(close(tris[0].v[0], [1.0, 0.0, -2.0]));
        // Quad split 0-1-2 then 3-2-1.
        assert!(close(tris[1].v[0], [41.0, 0.0, -1.0]));
        assert!(tris.iter().all(|t| t.surface == 7));
    }

    #[test]
    fn reads_atoc_and_raw_sfil() {
        let mut atoc = b"ATOC".to_vec();
        be32v(&mut atoc, 3);
        atoc.resize(16, 0);
        be32v(&mut atoc, 1);
        atoc.resize(24, 0);
        let mut rec = vec![0u8; 64];
        rec[0..8].copy_from_slice(&0x1122u64.to_be_bytes());
        rec[12..16].copy_from_slice(&5u32.to_be_bytes());
        rec[36..40].copy_from_slice(&1u32.to_be_bytes());
        atoc.extend_from_slice(&rec);
        let records = read_atoc(&atoc).unwrap();
        assert_eq!(
            records,
            vec![AssetRecord {
                id: 0x1122,
                total_size: 5,
                asset_type: 1
            }]
        );

        // SFIL: header, then one raw asset (128-byte header + 5 bytes, stride 256), then padding.
        let mut s = b"SFIL".to_vec();
        s.resize(16, 0);
        be32v(&mut s, 128);
        s.resize(128, 0);
        let mut a = vec![0u8; 128];
        a[0..8].copy_from_slice(&0x1122u64.to_be_bytes());
        a[8..12].copy_from_slice(&5u32.to_be_bytes());
        a[12..16].copy_from_slice(&128u32.to_be_bytes());
        a[16..20].copy_from_slice(&256u32.to_be_bytes());
        a.extend_from_slice(b"hello");
        a.resize(256, 0);
        s.extend_from_slice(&a);
        s.resize(s.len() + 128, 0);
        let assets = read_sfil(&s, &records).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].1, b"hello");
    }
}
