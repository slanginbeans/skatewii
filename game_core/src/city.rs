//! A streamed district: Skate 3's city districts (and Skate School) cut into square cells, each its own
//! pack (`fg_import city`), listed by an index.
//!
//! On the SD card: `data/city/<District>/index.fgpak` and one `c_<x>_<z>.fgpak` per cell, named by the
//! cell's centre in metres. A cell pack is an ordinary park pack (`world::ImportedPark`: `park/collision`,
//! `park/spawn`, `park/rails`, `park/mesh/<n>`, `park/tex/<n>`), already in world space, holding what lies
//! in the cell's square (collision triangles and meshes by their centre, rails by their middle).
//!
//! `city/index` in the index pack (big-endian): `"FGCI"`, `u32` version (2), `f32` cell edge, `f32` the
//! district's lowest collision height, `u32` start count, `u32` cell count, then per start `f32` x, y, z,
//! yaw, then per cell `i32` centre x, `i32` centre z, `u32` pack bytes, `u32` collision triangles, `u32`
//! vertices, `u32` texture bytes.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::math::{Vec3, v3};

pub const VERSION: u32 = 2;
const MAGIC: &[u8; 4] = b"FGCI";
const HEADER: usize = 24;
const START_BYTES: usize = 16;
const CELL_BYTES: usize = 24;

/// One cell of a district.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellInfo {
    /// The cell's centre (m).
    pub x: i32,
    pub z: i32,
    /// The cell pack's size.
    pub bytes: u32,
    pub tris: u32,
    pub vertices: u32,
    pub texture_bytes: u32,
}

impl CellInfo {
    /// The cell pack's file name, inside the district's folder.
    pub fn file(&self) -> String {
        format!("c_{}_{}.fgpak", self.x, self.z)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    /// A cell's edge (m).
    pub cell: f32,
    /// The district's lowest collision height (m): below it the rider has fallen out of the world.
    pub floor: f32,
    /// Where a rider can start in the district, and the yaw, best first (at least one).
    pub starts: Vec<(Vec3, f32)>,
    /// Sorted by (x, z).
    pub cells: Vec<CellInfo>,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Index {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            HEADER + self.starts.len() * START_BYTES + self.cells.len() * CELL_BYTES,
        );
        out.extend_from_slice(MAGIC);
        for w in [
            VERSION,
            self.cell.to_bits(),
            self.floor.to_bits(),
            self.starts.len() as u32,
            self.cells.len() as u32,
        ] {
            out.extend_from_slice(&w.to_be_bytes());
        }
        for (p, yaw) in &self.starts {
            for f in [p.x, p.y, p.z, *yaw] {
                out.extend_from_slice(&f.to_bits().to_be_bytes());
            }
        }
        for c in &self.cells {
            for w in [
                c.x as u32,
                c.z as u32,
                c.bytes,
                c.tris,
                c.vertices,
                c.texture_bytes,
            ] {
                out.extend_from_slice(&w.to_be_bytes());
            }
        }
        out
    }

    /// None for anything that isn't a version-2 index with a start, whose starts and cells fit its length.
    pub fn parse(b: &[u8]) -> Option<Index> {
        if b.len() < HEADER || &b[..4] != MAGIC || be32(b, 4) != VERSION {
            return None;
        }
        let f = |at: usize| f32::from_bits(be32(b, at));
        let (cell, floor) = (f(8), f(12));
        let (s, n) = (be32(b, 16) as usize, be32(b, 20) as usize);
        let first = HEADER.checked_add(s.checked_mul(START_BYTES)?)?;
        if cell.is_nan()
            || cell <= 0.0
            || floor.is_nan()
            || s == 0
            || b.len() != first.checked_add(n.checked_mul(CELL_BYTES)?)?
        {
            return None;
        }
        let starts = (0..s)
            .map(|i| {
                let at = HEADER + i * START_BYTES;
                (v3(f(at), f(at + 4), f(at + 8)), f(at + 12))
            })
            .collect();
        let mut cells: Vec<CellInfo> = (0..n)
            .map(|i| {
                let at = first + i * CELL_BYTES;
                CellInfo {
                    x: be32(b, at) as i32,
                    z: be32(b, at + 4) as i32,
                    bytes: be32(b, at + 8),
                    tris: be32(b, at + 12),
                    vertices: be32(b, at + 16),
                    texture_bytes: be32(b, at + 20),
                }
            })
            .collect();
        cells.sort_by_key(|c| (c.x, c.z));
        Some(Index {
            cell,
            floor,
            starts,
            cells,
        })
    }

    /// The centre of the cell holding (x, z) (whether the district has it or not).
    pub fn centre_of(&self, x: f32, z: f32) -> (i32, i32) {
        let c = |v: f32| ((libm::floorf(v / self.cell) + 0.5) * self.cell) as i32;
        (c(x), c(z))
    }

    /// The district's cell with this centre.
    pub fn find(&self, x: i32, z: i32) -> Option<&CellInfo> {
        self.cells
            .binary_search_by_key(&(x, z), |c| (c.x, c.z))
            .ok()
            .map(|i| &self.cells[i])
    }

    /// The district's cells whose square comes within `r` of (x, z), nearest first.
    pub fn near(&self, x: f32, z: f32, r: f32) -> Vec<(&CellInfo, f32)> {
        let h = self.cell * 0.5;
        let reach = (libm::ceilf((r + h) / self.cell)) as i32;
        let (cx, cz) = self.centre_of(x, z);
        let step = self.cell as i32;
        let mut out = Vec::new();
        for i in -reach..=reach {
            for j in -reach..=reach {
                let Some(c) = self.find(cx + i * step, cz + j * step) else {
                    continue;
                };
                let dx = (libm::fabsf(x - c.x as f32) - h).max(0.0);
                let dz = (libm::fabsf(z - c.z as f32) - h).max(0.0);
                let d = libm::sqrtf(dx * dx + dz * dz);
                if d <= r {
                    out.push((c, d));
                }
            }
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn cell(x: i32, z: i32) -> CellInfo {
        CellInfo {
            x,
            z,
            bytes: 1000 + x.unsigned_abs(),
            tris: 3,
            vertices: 4,
            texture_bytes: 5,
        }
    }

    fn index() -> Index {
        Index {
            cell: 100.0,
            floor: -12.5,
            starts: vec![(v3(10.0, 2.0, -30.0), 1.5), (v3(-40.0, 0.5, 60.0), -0.25)],
            cells: vec![
                cell(-50, -50),
                cell(-50, 50),
                cell(50, -50),
                cell(50, 50),
                cell(150, 50),
            ],
        }
    }

    #[test]
    fn round_trip_and_rejects() {
        let i = index();
        let b = i.encode();
        assert_eq!(Index::parse(&b), Some(i));
        assert_eq!(Index::parse(&b[..b.len() - 1]), None);
        let mut v = b.clone();
        v[7] = 9;
        assert_eq!(Index::parse(&v), None, "another version");
        assert_eq!(Index::parse(b"FGCX"), None);
        let none = Index {
            starts: vec![],
            ..index()
        };
        assert_eq!(Index::parse(&none.encode()), None, "no start");
        assert_eq!(cell(-150, 250).file(), "c_-150_250.fgpak");
    }

    #[test]
    fn cells_by_position() {
        let i = index();
        assert_eq!(i.centre_of(0.0, 0.0), (50, 50));
        assert_eq!(i.centre_of(-0.1, 99.9), (-50, 50));
        assert_eq!(i.centre_of(-100.0, -100.1), (-50, -150));
        assert_eq!(i.find(50, 50).map(|c| c.bytes), Some(1050));
        assert!(i.find(250, 50).is_none());
        // Standing in (50, 50) near its +x edge: that cell, then the one across the edge.
        let near: Vec<_> = i
            .near(95.0, 50.0, 10.0)
            .iter()
            .map(|(c, _)| (c.x, c.z))
            .collect();
        assert_eq!(near, vec![(50, 50), (150, 50)]);
        // Far enough to reach every cell, nearest first.
        let all = i.near(0.0, 0.0, 200.0);
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].1, 0.0);
        assert!(all.windows(2).all(|w| w[0].1 <= w[1].1));
    }
}
