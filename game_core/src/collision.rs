//! An imported park's collision (`park.fgpak` from `fg_import park`): triangles in world space (Y up,
//! metres), bucketed in a grid over X/Z for fast queries.
//!
//! Walkable triangles (facing up enough to ride) are ground; steep ones are walls. The board stands on the
//! highest walkable triangle under it that is no higher than a given height, so ledges, rails and
//! overhangs work at any level.
//!
//! Pack entries:
//! - `park/collision`, either of (all big-endian):
//!   - plain: `u32` triangle count, then per triangle 9 `f32` (three XYZ corners), `u16` surface id, `u16`
//!     flags. Copied into memory when parsed.
//!   - packed ([`encode_packed`], what the importer writes): read in place, nothing copied. `"FGC2"`, `u32`
//!     vertex count, `u32` triangle count, `f32` origin x, y, z, `f32` step (m), `f32` min x, y, z, `f32`
//!     max x, y, z, `u32` grid columns, `u32` grid rows, `u32` grid entries, `f32` grid square edge (1, 2
//!     or 4 m: finer where the triangles are small and many) (68 bytes); then per vertex 3
//!     `i16` (origin + value x step), padded to 4 bytes; per triangle 3 `u16` vertex indices, `u16`
//!     surface id, `u16` flags; per grid cell (rows x columns, then one more) the `u32` start of its
//!     entries; the entries, `u16` triangle indices. About 14 bytes a triangle, against 40 plain and 52
//!     parsed.
//! - `park/spawn`: 4 `f32`: spawn X, Y, Z and yaw.
//! - `park/rails` (optional): grind rails, see `rails.rs`.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use crate::math::{Vec3, v3};
use crate::pack::{Kind, Pack};
use crate::park::{Hit, Surface, Terrain};
use crate::rails::{self, Rail};

/// Size of a grid cell (m), unless a packed collision says otherwise.
pub const CELL: f32 = 4.0;
/// Grid squares a packed collision may use, finest first.
const PACKED_CELLS: [f32; 3] = [1.0, 2.0, 4.0];
/// Triangles whose normal is at least this upward are ground; flatter-to-the-side ones are walls.
pub const WALKABLE_Y: f32 = 0.35;
/// Height above the board at which walls are tested (a bit below the knees).
const WALL_PROBE: f32 = 0.25;
pub const TRIANGLE_BYTES: usize = 40;
/// Most samples along a ray or wall test (every half grid square): 1024 covers 512 m on a 1 m grid.
const MAX_STEPS: usize = 1024;
const PACKED_MAGIC: &[u8; 4] = b"FGC2";
const PACKED_HEADER: usize = 68;
const PACKED_VERTEX: usize = 6;
const PACKED_TRI: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tri {
    pub v: [Vec3; 3],
    /// Unit normal (right-handed winding).
    pub n: Vec3,
    pub surface: u16,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Missing,
    Bad,
}

pub struct TriMesh {
    tris: Store,
    pub min: Vec3,
    pub max: Vec3,
    cols: usize,
    rows: usize,
    /// The grid square's edge (m).
    cell: f32,
    grid: Grid,
    spawn: (Vec3, f32),
    /// Grind rails (`park/rails`, optional).
    pub rails: Vec<Rail>,
}

/// Bytes read in place (a pack's, until the park is unloaded) or a copy.
enum Bytes {
    Static(&'static [u8]),
    Owned(Vec<u8>),
}

impl core::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Static(b) => b,
            Bytes::Owned(v) => v,
        }
    }
}

/// The triangles: parsed, or packed (decoded when used).
enum Store {
    Owned(Vec<Tri>),
    Packed {
        data: Bytes,
        tris: usize,
        verts_at: usize,
        tris_at: usize,
        origin: Vec3,
        step: f32,
    },
}

/// The grid: `start[c]..start[c + 1]` indexes the entries (triangle indices) for cell `c`.
enum Grid {
    Owned {
        start: Vec<u32>,
        tris: Vec<u32>,
    },
    /// In the packed data: `u32` starts at `start`, `u16` entries at `tris`.
    Packed {
        start: usize,
        tris: usize,
    },
}

/// The triangles as a query reads them.
enum View<'a> {
    Owned(&'a [Tri]),
    Packed {
        verts: &'a [u8],
        tris: &'a [u8],
        origin: Vec3,
        step: f32,
    },
}

impl View<'_> {
    /// Triangle `i`'s corners.
    #[inline]
    fn corners(&self, i: usize) -> [Vec3; 3] {
        match self {
            View::Owned(t) => t[i].v,
            View::Packed {
                verts,
                tris,
                origin,
                step,
            } => {
                let t = &tris[i * PACKED_TRI..i * PACKED_TRI + 6];
                let corner = |k: usize| {
                    let at = u16::from_be_bytes([t[2 * k], t[2 * k + 1]]) as usize * PACKED_VERTEX;
                    let c = &verts[at..at + PACKED_VERTEX];
                    let q = |j: usize| i16::from_be_bytes([c[2 * j], c[2 * j + 1]]) as f32 * *step;
                    v3(origin.x + q(0), origin.y + q(1), origin.z + q(2))
                };
                [corner(0), corner(1), corner(2)]
            }
        }
    }

    /// Triangle `i`'s unit normal (`v`: its corners).
    #[inline]
    fn normal(&self, i: usize, v: &[Vec3; 3]) -> Vec3 {
        match self {
            View::Owned(t) => t[i].n,
            View::Packed { .. } => TriMesh::normal_of(v),
        }
    }
}

/// A grid cell's triangle indices.
enum CellIter<'a> {
    Owned(core::slice::Iter<'a, u32>),
    Packed(core::slice::ChunksExact<'a, u8>),
}

impl Iterator for CellIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        match self {
            CellIter::Owned(i) => i.next().copied(),
            CellIter::Packed(c) => c.next().map(|b| u16::from_be_bytes([b[0], b[1]]) as u32),
        }
    }
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn bef(b: &[u8], at: usize) -> f32 {
    f32::from_bits(be32(b, at))
}

/// Encodes triangles as the `park/collision` entry (for the importer and tests).
pub fn encode(tris: &[([[f32; 3]; 3], u16, u16)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + tris.len() * TRIANGLE_BYTES);
    out.extend_from_slice(&(tris.len() as u32).to_be_bytes());
    for (v, surface, flags) in tris {
        for p in v {
            for c in p {
                out.extend_from_slice(&c.to_be_bytes());
            }
        }
        out.extend_from_slice(&surface.to_be_bytes());
        out.extend_from_slice(&flags.to_be_bytes());
    }
    out
}

/// Encodes triangles as a packed `park/collision` entry, read in place by the game: corners quantized to
/// `step` (1/256 m, or the finest power of two of that which reaches every corner from the middle), shared
/// corners stored once, slivers that quantizing flattens left out, and the grid built in. None when there
/// are too many triangles or corners for `u16` indices (more than 65535), or a corner isn't finite.
pub fn encode_packed(tris: &[([[f32; 3]; 3], u16, u16)]) -> Option<Vec<u8>> {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (v, _, _) in tris {
        for p in v {
            for k in 0..3 {
                if !p[k].is_finite() {
                    return None;
                }
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    if tris.is_empty() {
        (lo, hi) = ([0.0; 3], [0.0; 3]);
    }
    let origin: [f32; 3] = core::array::from_fn(|k| libm::roundf((lo[k] + hi[k]) * 0.5));
    let reach = (0..3)
        .map(|k| (hi[k] - origin[k]).max(origin[k] - lo[k]))
        .fold(0.0, f32::max);
    let mut step = 1.0 / 256.0;
    while reach / step > 32000.0 {
        step *= 2.0;
    }
    let q = |p: [f32; 3]| -> [i16; 3] {
        core::array::from_fn(|k| libm::roundf((p[k] - origin[k]) / step) as i16)
    };
    let back = |c: [i16; 3]| {
        v3(
            origin[0] + c[0] as f32 * step,
            origin[1] + c[1] as f32 * step,
            origin[2] + c[2] as f32 * step,
        )
    };
    let mut index: BTreeMap<[i16; 3], u16> = BTreeMap::new();
    let mut verts: Vec<[i16; 3]> = Vec::new();
    let mut out_tris: Vec<([u16; 3], u16, u16)> = Vec::new();
    let mut parsed: Vec<Tri> = Vec::new();
    for (v, surface, flags) in tris {
        let c = [q(v[0]), q(v[1]), q(v[2])];
        let pts = [back(c[0]), back(c[1]), back(c[2])];
        let n = (pts[1] - pts[0]).cross(pts[2] - pts[0]);
        if n.length() < 1e-8 {
            continue;
        }
        let mut ids = [0u16; 3];
        for k in 0..3 {
            ids[k] = match index.get(&c[k]) {
                Some(&i) => i,
                None => {
                    if verts.len() >= 0xffff {
                        return None;
                    }
                    let i = verts.len() as u16;
                    index.insert(c[k], i);
                    verts.push(c[k]);
                    i
                }
            };
        }
        if out_tris.len() >= 0xffff {
            return None;
        }
        out_tris.push((ids, *surface, *flags));
        parsed.push(Tri {
            v: pts,
            n: n.normalize_or(Vec3::UP),
            surface: *surface,
            flags: *flags,
        });
    }
    // The finest grid whose squares' starts and entries take no more than the triangles and corners
    // bytes (at least 4 KB): a dense cell's queries then test tens of triangles, not hundreds.
    let geometry = verts.len() * PACKED_VERTEX + out_tris.len() * PACKED_TRI;
    let grid_bytes = |m: &TriMesh| match &m.grid {
        Grid::Owned { start, tris } => start.len() * 4 + tris.len() * 2,
        Grid::Packed { .. } => usize::MAX,
    };
    let mut m = TriMesh::build_with(parsed, (Vec3::ZERO, 0.0), PACKED_CELLS[0]);
    for &cell in &PACKED_CELLS[1..] {
        if grid_bytes(&m) <= geometry.max(4096) {
            break;
        }
        let Store::Owned(t) = core::mem::replace(&mut m.tris, Store::Owned(Vec::new())) else {
            return None;
        };
        m = TriMesh::build_with(t, (Vec3::ZERO, 0.0), cell);
    }
    let Grid::Owned {
        start,
        tris: entries,
    } = &m.grid
    else {
        return None;
    };
    let mut out = Vec::new();
    out.extend_from_slice(PACKED_MAGIC);
    for w in [verts.len() as u32, out_tris.len() as u32] {
        out.extend_from_slice(&w.to_be_bytes());
    }
    for f in [
        origin[0], origin[1], origin[2], step, m.min.x, m.min.y, m.min.z, m.max.x, m.max.y, m.max.z,
    ] {
        out.extend_from_slice(&f.to_be_bytes());
    }
    for w in [m.cols as u32, m.rows as u32, entries.len() as u32] {
        out.extend_from_slice(&w.to_be_bytes());
    }
    out.extend_from_slice(&m.cell.to_be_bytes());
    for c in &verts {
        for v in c {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    for (ids, surface, flags) in &out_tris {
        for w in [ids[0], ids[1], ids[2], *surface, *flags] {
            out.extend_from_slice(&w.to_be_bytes());
        }
    }
    for w in start {
        out.extend_from_slice(&w.to_be_bytes());
    }
    for &w in entries {
        out.extend_from_slice(&(w as u16).to_be_bytes());
    }
    Some(out)
}

impl TriMesh {
    /// From a park pack's entries (a packed collision is read in place).
    pub fn from_pack(pack: &Pack<'static>) -> Result<TriMesh, Error> {
        let c = pack
            .find("park/collision")
            .filter(|e| e.kind == Kind::Collision as u32)
            .ok_or(Error::Missing)?;
        let s = pack.find("park/spawn").ok_or(Error::Missing)?;
        if s.data.len() < 16 {
            return Err(Error::Bad);
        }
        let spawn = (
            v3(bef(s.data, 0), bef(s.data, 4), bef(s.data, 8)),
            bef(s.data, 12),
        );
        let mut m = TriMesh::parse_static(c.data, spawn)?;
        if let Some(r) = pack.find("park/rails") {
            m.rails = rails::parse(r.data).ok_or(Error::Bad)?;
        }
        Ok(m)
    }

    /// A `park/collision` entry, either form; packed data read in place.
    pub fn parse_static(data: &'static [u8], spawn: (Vec3, f32)) -> Result<TriMesh, Error> {
        if data.starts_with(PACKED_MAGIC) {
            TriMesh::packed(Bytes::Static(data), spawn)
        } else {
            TriMesh::parse(data, spawn)
        }
    }

    fn packed(data: Bytes, spawn: (Vec3, f32)) -> Result<TriMesh, Error> {
        if data.len() < PACKED_HEADER {
            return Err(Error::Bad);
        }
        let u = |at: usize| be32(&data, at) as usize;
        let f = |at: usize| bef(&data, at);
        let (nv, nt) = (u(4), u(8));
        let origin = v3(f(12), f(16), f(20));
        let step = f(24);
        let (min, max) = (v3(f(28), f(32), f(36)), v3(f(40), f(44), f(48)));
        let (cols, rows, entries) = (u(52), u(56), u(60));
        let cell = f(64);
        let cells = cols.checked_mul(rows).ok_or(Error::Bad)?;
        if nv > 0xffff
            || nt > 0xffff
            || cells == 0
            || cells > 1 << 24
            || !PACKED_CELLS.contains(&cell)
        {
            return Err(Error::Bad);
        }
        let verts_at = PACKED_HEADER;
        let tris_at = (verts_at + nv * PACKED_VERTEX).next_multiple_of(4);
        let start = tris_at + nt * PACKED_TRI;
        let entries_at = start + (cells + 1) * 4;
        let finite = [origin, min, max]
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite());
        if !finite
            || !(step.is_finite() && step > 0.0)
            || data.len() != entries_at + entries * 2
            || u(start) != 0
            || u(start + cells * 4) != entries
        {
            return Err(Error::Bad);
        }
        // Every index in range, so queries can't read past the data.
        let bad_corner = (0..nt)
            .any(|i| (0..3).any(|k| be16(&data, tris_at + i * PACKED_TRI + 2 * k) as usize >= nv));
        let bad_entry = (0..entries).any(|i| be16(&data, entries_at + 2 * i) as usize >= nt);
        let bad_start = (0..cells).any(|c| u(start + c * 4) > u(start + c * 4 + 4));
        if bad_corner || bad_entry || bad_start {
            return Err(Error::Bad);
        }
        Ok(TriMesh {
            tris: Store::Packed {
                data,
                tris: nt,
                verts_at,
                tris_at,
                origin,
                step,
            },
            min,
            max,
            cols,
            rows,
            cell,
            grid: Grid::Packed {
                start,
                tris: entries_at,
            },
            spawn,
            rails: Vec::new(),
        })
    }

    /// The number of triangles.
    pub fn len(&self) -> usize {
        match &self.tris {
            Store::Owned(t) => t.len(),
            Store::Packed { tris, .. } => *tris,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Triangle `i` (`i < len()`).
    pub fn tri(&self, i: usize) -> Tri {
        match &self.tris {
            Store::Owned(t) => t[i],
            Store::Packed { data, tris_at, .. } => {
                let v = self.corners(i);
                let at = tris_at + i * PACKED_TRI;
                Tri {
                    v,
                    n: TriMesh::normal_of(&v),
                    surface: be16(data, at + 6),
                    flags: be16(data, at + 8),
                }
            }
        }
    }

    /// Triangle `i`'s corners: what the queries test first, without working out the normal.
    fn corners(&self, i: usize) -> [Vec3; 3] {
        self.view().corners(i)
    }

    /// The triangles, unpacked once for a query's loop.
    fn view(&self) -> View<'_> {
        match &self.tris {
            Store::Owned(t) => View::Owned(t),
            Store::Packed {
                data,
                verts_at,
                tris_at,
                origin,
                step,
                ..
            } => View::Packed {
                verts: &data[*verts_at..*tris_at],
                tris: &data[*tris_at..],
                origin: *origin,
                step: *step,
            },
        }
    }

    fn normal_of(v: &[Vec3; 3]) -> Vec3 {
        (v[1] - v[0]).cross(v[2] - v[0]).normalize_or(Vec3::UP)
    }

    /// Every triangle.
    pub fn tris(&self) -> impl Iterator<Item = Tri> + '_ {
        (0..self.len()).map(|i| self.tri(i))
    }

    /// Heap bytes the collision takes beyond data read in place.
    pub fn heap_bytes(&self) -> usize {
        let t = match &self.tris {
            Store::Owned(t) => t.len() * core::mem::size_of::<Tri>(),
            Store::Packed {
                data: Bytes::Owned(v),
                ..
            } => v.len(),
            Store::Packed { .. } => 0,
        };
        let g = match &self.grid {
            Grid::Owned { start, tris } => (start.len() + tris.len()) * 4,
            Grid::Packed { .. } => 0,
        };
        t + g
    }

    /// A `park/collision` entry, either form, copied.
    pub fn parse(data: &[u8], spawn: (Vec3, f32)) -> Result<TriMesh, Error> {
        if data.starts_with(PACKED_MAGIC) {
            return TriMesh::packed(Bytes::Owned(data.to_vec()), spawn);
        }
        if data.len() < 4 {
            return Err(Error::Bad);
        }
        let n = be32(data, 0) as usize;
        if data.len() < 4 + n * TRIANGLE_BYTES {
            return Err(Error::Bad);
        }
        let mut tris = Vec::with_capacity(n);
        for i in 0..n {
            let at = 4 + i * TRIANGLE_BYTES;
            let p = |k: usize| {
                let o = at + 12 * k;
                v3(bef(data, o), bef(data, o + 4), bef(data, o + 8))
            };
            let v = [p(0), p(1), p(2)];
            if v.iter()
                .any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()))
            {
                return Err(Error::Bad);
            }
            let n = (v[1] - v[0]).cross(v[2] - v[0]);
            // Degenerate slivers can't be stood on or hit.
            if n.length() < 1e-8 {
                continue;
            }
            tris.push(Tri {
                v,
                n: n.normalize_or(Vec3::UP),
                surface: u16::from_be_bytes([data[at + 36], data[at + 37]]),
                flags: u16::from_be_bytes([data[at + 38], data[at + 39]]),
            });
        }
        Ok(TriMesh::build(tris, spawn))
    }

    fn build(tris: Vec<Tri>, spawn: (Vec3, f32)) -> TriMesh {
        TriMesh::build_with(tris, spawn, CELL)
    }

    /// With grid squares `cell` m across.
    fn build_with(tris: Vec<Tri>, spawn: (Vec3, f32), cell: f32) -> TriMesh {
        let mut min = v3(f32::MAX, f32::MAX, f32::MAX);
        let mut max = v3(f32::MIN, f32::MIN, f32::MIN);
        for t in &tris {
            for p in &t.v {
                min = v3(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
                max = v3(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
            }
        }
        if tris.is_empty() {
            min = Vec3::ZERO;
            max = Vec3::ZERO;
        }
        let cols = (((max.x - min.x) / cell) as usize + 1).max(1);
        let rows = (((max.z - min.z) / cell) as usize + 1).max(1);
        let mut m = TriMesh {
            tris: Store::Owned(Vec::new()),
            min,
            max,
            cols,
            rows,
            cell,
            grid: Grid::Owned {
                start: Vec::new(),
                tris: Vec::new(),
            },
            spawn,
            rails: Vec::new(),
        };
        // Two passes: count per cell, then fill.
        let mut counts = vec![0u32; cols * rows];
        for t in &tris {
            m.each_cell(t, |c| counts[c] += 1);
        }
        let mut start = Vec::with_capacity(counts.len() + 1);
        let mut total = 0u32;
        for c in &counts {
            start.push(total);
            total += c;
        }
        start.push(total);
        let mut fill = start.clone();
        let mut cell_tris = vec![0u32; total as usize];
        for (i, t) in tris.iter().enumerate() {
            m.each_cell(t, |c| {
                cell_tris[fill[c] as usize] = i as u32;
                fill[c] += 1;
            });
        }
        m.grid = Grid::Owned {
            start,
            tris: cell_tris,
        };
        m.tris = Store::Owned(tris);
        m
    }

    fn cell_of(&self, x: f32, z: f32) -> (isize, isize) {
        (
            ((x - self.min.x) / self.cell) as isize - (x < self.min.x) as isize,
            ((z - self.min.z) / self.cell) as isize - (z < self.min.z) as isize,
        )
    }

    fn each_cell(&self, t: &Tri, mut f: impl FnMut(usize)) {
        let (lx, lz) = (
            t.v.iter().map(|p| p.x).fold(f32::MAX, f32::min),
            t.v.iter().map(|p| p.z).fold(f32::MAX, f32::min),
        );
        let (hx, hz) = (
            t.v.iter().map(|p| p.x).fold(f32::MIN, f32::max),
            t.v.iter().map(|p| p.z).fold(f32::MIN, f32::max),
        );
        let (c0, r0) = self.cell_of(lx, lz);
        let (c1, r1) = self.cell_of(hx, hz);
        for r in r0.max(0)..=r1.min(self.rows as isize - 1) {
            for c in c0.max(0)..=c1.min(self.cols as isize - 1) {
                f(r as usize * self.cols + c as usize);
            }
        }
    }

    /// Triangles in the cell holding (x, z).
    fn cell(&self, x: f32, z: f32) -> CellIter<'_> {
        let (c, r) = self.cell_of(x, z);
        if c < 0 || r < 0 || c >= self.cols as isize || r >= self.rows as isize {
            return CellIter::Owned([].iter());
        }
        let i = r as usize * self.cols + c as usize;
        match (&self.grid, &self.tris) {
            (Grid::Owned { start, tris }, _) => {
                CellIter::Owned(tris[start[i] as usize..start[i + 1] as usize].iter())
            }
            (Grid::Packed { start, tris: at }, Store::Packed { data, .. }) => {
                let a = be32(data, start + i * 4) as usize;
                let b = be32(data, start + i * 4 + 4) as usize;
                CellIter::Packed(data[at + 2 * a..at + 2 * b].chunks_exact(2))
            }
            (Grid::Packed { .. }, Store::Owned(_)) => CellIter::Owned([].iter()),
        }
    }

    /// Height of `t`'s plane at (x, z) if (x, z) is inside it seen from above.
    fn height_at(v: &[Vec3; 3], x: f32, z: f32) -> Option<f32> {
        let [a, b, c] = *v;
        let d = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
        if d.abs() < 1e-12 {
            return None;
        }
        let w0 = ((b.z - c.z) * (x - c.x) + (c.x - b.x) * (z - c.z)) / d;
        let w1 = ((c.z - a.z) * (x - c.x) + (a.x - c.x) * (z - c.z)) / d;
        let w2 = 1.0 - w0 - w1;
        const E: f32 = -1e-4;
        (w0 >= E && w1 >= E && w2 >= E).then_some(w0 * a.y + w1 * b.y + w2 * c.y)
    }

    /// Does the segment p..q cross a wall (a steep triangle)? Checks every cell along the segment.
    pub fn wall_between(&self, p: Vec3, q: Vec3) -> bool {
        let view = self.view();
        let d = q - p;
        let steps = ((d.x.abs().max(d.z.abs()) / (self.cell * 0.5)) as usize + 1).min(MAX_STEPS);
        let mut last = None;
        for s in 0..=steps {
            let at = p + d * (s as f32 / steps as f32);
            let cell = self.cell_of(at.x, at.z);
            if last == Some(cell) {
                continue;
            }
            last = Some(cell);
            let hit = self.cell(at.x, at.z).any(|i| {
                let v = view.corners(i as usize);
                TriMesh::segment_hits(&v, p, q) && view.normal(i as usize, &v).y.abs() < WALKABLE_Y
            });
            if hit {
                return true;
            }
        }
        false
    }

    /// The triangles that come within `r` of `c` (corners and unit normal), appended to `out`: the
    /// rigid board's narrow phase tests its spheres against them (SK8's sphere-triangle queries).
    pub fn triangles_near(&self, c: Vec3, r: f32, out: &mut Vec<([Vec3; 3], Vec3)>) {
        let view = self.view();
        let (c0, r0) = self.cell_of(c.x - r, c.z - r);
        let (c1, r1) = self.cell_of(c.x + r, c.z + r);
        let first = out.len();
        let mut seen: Vec<u32> = Vec::new();
        for row in r0.max(0)..=r1.min(self.rows as isize - 1) {
            for col in c0.max(0)..=c1.min(self.cols as isize - 1) {
                let x = self.min.x + (col as f32 + 0.5) * self.cell;
                let z = self.min.z + (row as f32 + 0.5) * self.cell;
                for i in self.cell(x, z) {
                    if seen.contains(&i) {
                        continue;
                    }
                    seen.push(i);
                    let v = view.corners(i as usize);
                    let lo = v[0].y.min(v[1].y).min(v[2].y);
                    let hi = v[0].y.max(v[1].y).max(v[2].y);
                    let lx = v[0].x.min(v[1].x).min(v[2].x);
                    let hx = v[0].x.max(v[1].x).max(v[2].x);
                    let lz = v[0].z.min(v[1].z).min(v[2].z);
                    let hz = v[0].z.max(v[1].z).max(v[2].z);
                    if c.y + r < lo
                        || c.y - r > hi
                        || c.x + r < lx
                        || c.x - r > hx
                        || c.z + r < lz
                        || c.z - r > hz
                    {
                        continue;
                    }
                    let n = view.normal(i as usize, &v);
                    if (c - v[0]).dot(n).abs() <= r {
                        out.push((v, n));
                    }
                }
            }
        }
        let _ = first;
    }

    /// The nearest triangle (either side) a ray from `from` along the unit `dir` meets within `max`.
    pub fn ray(&self, from: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        let view = self.view();
        let to = from + dir * max;
        let steps =
            (((to.x - from.x).abs().max((to.z - from.z).abs()) / (self.cell * 0.5)) as usize + 1)
                .min(MAX_STEPS);
        let mut best: Option<(f32, usize)> = None;
        let mut last = None;
        for s in 0..=steps {
            let at = from + (to - from) * (s as f32 / steps as f32);
            let cell = self.cell_of(at.x, at.z);
            if last == Some(cell) {
                continue;
            }
            last = Some(cell);
            for i in self.cell(at.x, at.z) {
                let v = view.corners(i as usize);
                if let Some(k) = TriMesh::segment_t(&v, from, to)
                    && best.is_none_or(|(b, _)| k < b)
                {
                    best = Some((k, i as usize));
                }
            }
        }
        let (k, i) = best?;
        let t = self.tri(i);
        let n = t.n;
        Some(Hit {
            t: k * max,
            point: from + dir * (k * max),
            normal: if n.dot(dir) > 0.0 { -n } else { n },
            surface: t.surface,
        })
    }

    /// Where along p..q (0..1) the segment crosses triangle t, if it does. (Möller-Trumbore, both sides.)
    fn segment_t(t: &[Vec3; 3], p: Vec3, q: Vec3) -> Option<f32> {
        let d = q - p;
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let h = d.cross(e2);
        let a = e1.dot(h);
        if a.abs() < 1e-10 {
            return None;
        }
        let f = 1.0 / a;
        let s = p - t[0];
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let qv = s.cross(e1);
        let v = f * d.dot(qv);
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let k = f * e2.dot(qv);
        (0.0..=1.0).contains(&k).then_some(k)
    }

    /// Does the segment p..q cross triangle t? (Möller-Trumbore, both sides.)
    fn segment_hits(t: &[Vec3; 3], p: Vec3, q: Vec3) -> bool {
        let d = q - p;
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let h = d.cross(e2);
        let a = e1.dot(h);
        if a.abs() < 1e-10 {
            return false;
        }
        let f = 1.0 / a;
        let s = p - t[0];
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let qv = s.cross(e1);
        let v = f * d.dot(qv);
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        let k = f * e2.dot(qv);
        (0.0..=1.0).contains(&k)
    }
}

impl Terrain for TriMesh {
    fn ground(&self, x: f32, z: f32, max: f32) -> Surface {
        let mut best = Surface {
            height: f32::MIN,
            normal: Vec3::UP,
        };
        let view = self.view();
        for i in self.cell(x, z) {
            let v = view.corners(i as usize);
            let Some(h) = TriMesh::height_at(&v, x, z) else {
                continue;
            };
            if h > max || h <= best.height {
                continue;
            }
            // Ground faces up; triangles wound the other way are flipped (collision is two-sided).
            let n = view.normal(i as usize, &v);
            let n = if n.y < 0.0 { -n } else { n };
            if n.y >= WALKABLE_Y {
                best = Surface {
                    height: h,
                    normal: n,
                };
            }
        }
        best
    }

    fn blocked(&self, a: Vec3, b: Vec3) -> bool {
        let (p, q) = (a + Vec3::UP * WALL_PROBE, b + Vec3::UP * WALL_PROBE);
        self.wall_between(p, q)
    }

    fn occluded(&self, p: Vec3, q: Vec3) -> bool {
        self.wall_between(p, q)
    }

    fn raycast(&self, from: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        self.ray(from, dir, max)
    }

    fn triangles_near(&self, c: Vec3, r: f32, out: &mut Vec<([Vec3; 3], Vec3)>) -> bool {
        TriMesh::triangles_near(self, c, r, out);
        true
    }

    fn spawn(&self) -> (Vec3, f32) {
        self.spawn
    }

    fn rails(&self) -> &[Rail] {
        &self.rails
    }

    fn kill_height(&self) -> f32 {
        self.min.y - 5.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;

    /// A 20 x 20 m floor, a 0.5 m high box (top + one wall facing -Z) and an upward ramp.
    fn yard() -> TriMesh {
        let q = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            [([a, b, c], 1u16, 0u16), ([a, c, d], 1, 0)]
        };
        let mut t = Vec::new();
        // Floor, wound to face up (counter-clockwise seen from above with Y up: x then -z).
        t.extend(q(
            [-10., 0., -10.],
            [-10., 0., 10.],
            [10., 0., 10.],
            [10., 0., -10.],
        ));
        // Box top at y = 0.5 over x 2..4, z 2..4.
        t.extend(q(
            [2., 0.5, 2.],
            [2., 0.5, 4.],
            [4., 0.5, 4.],
            [4., 0.5, 2.],
        ));
        // Its wall facing -Z at z = 2.
        t.extend(q([2., 0., 2.], [2., 0.5, 2.], [4., 0.5, 2.], [4., 0., 2.]));
        let data = encode(&t);
        TriMesh::parse(&data, (v3(0.0, 0.0, 0.0), 0.0)).unwrap()
    }

    #[test]
    fn stands_on_the_highest_walkable_surface_below_a_height() {
        let m = yard();
        assert_eq!(m.len(), 6);
        let g = m.ground(3.0, 3.0, 10.0);
        assert!((g.height - 0.5).abs() < 1e-5 && g.normal.y > 0.99);
        // Below the box top (rolling under a ledge, or approaching it): the floor.
        assert!(m.ground(3.0, 3.0, 0.2).height.abs() < 1e-5);
        assert!(m.ground(-5.0, 7.0, 1.0).height.abs() < 1e-5);
        // Off the edge of the world: nothing.
        assert_eq!(m.ground(30.0, 0.0, 1.0).height, f32::MIN);
        assert!(m.kill_height() < -4.0);
    }

    #[test]
    fn steep_faces_are_walls() {
        let m = yard();
        // Rolling at floor level towards the box's -Z face.
        assert!(m.blocked(v3(3.0, 0.0, 1.9), v3(3.0, 0.0, 2.1)));
        assert!(!m.blocked(v3(3.0, 0.0, 1.0), v3(3.0, 0.0, 1.5)));
        // Above the box: clear.
        assert!(!m.blocked(v3(3.0, 0.6, 1.9), v3(3.0, 0.6, 2.1)));
        // Beside it.
        assert!(!m.blocked(v3(5.0, 0.0, 1.9), v3(5.0, 0.0, 2.1)));
        // A long sight line through the wall, from cells that hold no wall at either end.
        assert!(m.occluded(v3(3.0, 0.3, -8.0), v3(3.0, 0.3, 9.0)));
        assert!(
            !m.occluded(v3(3.0, 0.8, -8.0), v3(3.0, 0.8, 9.0)),
            "over the top"
        );
    }

    fn yard_tris() -> Vec<([[f32; 3]; 3], u16, u16)> {
        let m = yard();
        m.tris()
            .map(|t| {
                (
                    t.v.map(|p| [p.x + 100.3, p.y + 2.0, p.z - 40.7]),
                    t.surface + 3,
                    t.flags,
                )
            })
            .collect()
    }

    #[test]
    fn packed_collision_answers_as_the_plain_one() {
        let tris = yard_tris();
        let plain = TriMesh::parse(&encode(&tris), (Vec3::ZERO, 0.0)).unwrap();
        let bytes = encode_packed(&tris).unwrap();
        let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let packed = TriMesh::parse_static(data, (Vec3::ZERO, 0.0)).unwrap();
        assert_eq!(packed.len(), plain.len());
        assert_eq!(packed.heap_bytes(), 0, "read in place");
        assert!(plain.heap_bytes() > 0);
        for (a, b) in packed.tris().zip(plain.tris()) {
            for k in 0..3 {
                assert!((a.v[k] - b.v[k]).length() < 0.003, "{a:?} {b:?}");
            }
            assert!(a.n.dot(b.n) > 0.9999 && a.surface == b.surface);
        }
        assert!((packed.min - plain.min).length() < 0.003);
        let o = v3(100.3, 2.0, -40.7);
        for (x, z) in [
            (3.0, 3.0),
            (-5.0, 7.0),
            (30.0, 0.0),
            (9.9, -9.9),
            (2.05, 2.05),
        ] {
            for max in [10.0, 2.2, 1.0] {
                let (g, h) = (
                    packed.ground(o.x + x, o.z + z, o.y + max),
                    plain.ground(o.x + x, o.z + z, o.y + max),
                );
                assert!(
                    (g.height - h.height).abs() < 0.003 || g.height == h.height,
                    "({x}, {z}) under {max}: {} {}",
                    g.height,
                    h.height
                );
            }
        }
        let p = |x, y, z| o + v3(x, y, z);
        assert!(packed.blocked(p(3.0, 0.0, 1.9), p(3.0, 0.0, 2.1)));
        assert!(!packed.blocked(p(3.0, 0.6, 1.9), p(3.0, 0.6, 2.1)));
        let (r, s) = (
            packed
                .ray(p(3.0, 5.0, 3.0), v3(0.0, -1.0, 0.0), 10.0)
                .unwrap(),
            plain
                .ray(p(3.0, 5.0, 3.0), v3(0.0, -1.0, 0.0), 10.0)
                .unwrap(),
        );
        assert!((r.t - s.t).abs() < 0.003 && r.surface == s.surface);
        // The copying parse reads it too.
        assert_eq!(
            TriMesh::parse(data, (Vec3::ZERO, 0.0)).unwrap().len(),
            plain.len()
        );
    }

    #[test]
    fn dense_collision_gets_a_finer_grid() {
        // A 200 m strip of floor in 0.25 m triangles (dense, as some of Industrial is), and a wall at its
        // far end.
        let mut tris = Vec::new();
        for i in 0..800 {
            for j in 0..8 {
                let (x, z) = (i as f32 * 0.25, j as f32 * 0.25);
                tris.push((
                    [[x, 0.0, z], [x, 0.0, z + 0.25], [x + 0.25, 0.0, z + 0.25]],
                    1,
                    0,
                ));
                tris.push((
                    [[x, 0.0, z], [x + 0.25, 0.0, z + 0.25], [x + 0.25, 0.0, z]],
                    1,
                    0,
                ));
            }
        }
        tris.push((
            [[199.0, 0.0, -1.0], [199.0, 3.0, -1.0], [199.0, 3.0, 3.0]],
            2,
            0,
        ));
        tris.push((
            [[199.0, 0.0, -1.0], [199.0, 3.0, 3.0], [199.0, 0.0, 3.0]],
            2,
            0,
        ));
        let data: &'static [u8] = Box::leak(encode_packed(&tris).unwrap().into_boxed_slice());
        let packed = TriMesh::parse_static(data, (Vec3::ZERO, 0.0)).unwrap();
        let plain = TriMesh::parse(&encode(&tris), (Vec3::ZERO, 0.0)).unwrap();
        assert!(packed.cell < CELL, "a finer grid: {} m", packed.cell);
        for k in 0..50 {
            let (x, z) = (k as f32 * 3.9 + 0.1, (k % 7) as f32 * 0.27 + 0.05);
            assert_eq!(
                packed.ground(x, z, 1.0),
                plain.ground(x, z, 1.0),
                "at {x} {z}"
            );
        }
        // A long ray down the strip reaches the wall through every grid square on the way.
        let from = v3(1.0, 1.0, 1.0);
        let (a, b) = (
            packed.ray(from, v3(1.0, 0.0, 0.0), 250.0).unwrap(),
            plain.ray(from, v3(1.0, 0.0, 0.0), 250.0).unwrap(),
        );
        assert!((a.t - 198.0).abs() < 0.01 && (b.t - 198.0).abs() < 0.01);
        assert_eq!(a.surface, 2);
        assert!(packed.blocked(v3(198.5, 0.0, 1.0), v3(199.5, 0.0, 1.0)));
        assert!(packed.occluded(v3(2.0, 1.0, 1.0), v3(220.0, 1.0, 1.0)));
    }

    #[test]
    fn packed_collision_rejects_bad_data() {
        let good = encode_packed(&yard_tris()).unwrap();
        let parse = |b: &[u8]| TriMesh::parse(b, (Vec3::ZERO, 0.0)).err();
        assert_eq!(parse(&good), None);
        assert_eq!(parse(&good[..good.len() - 2]), Some(Error::Bad));
        assert_eq!(parse(&good[..40]), Some(Error::Bad));
        // A triangle's corner out of range.
        let mut b = good.clone();
        let nv = be32(&b, 4) as usize;
        let at = (PACKED_HEADER + nv * PACKED_VERTEX).next_multiple_of(4);
        b[at..at + 2].copy_from_slice(&(nv as u16).to_be_bytes());
        assert_eq!(parse(&b), Some(Error::Bad));
        // A grid entry out of range.
        let mut b = good.clone();
        let n = b.len();
        b[n - 2..].copy_from_slice(&0xffffu16.to_be_bytes());
        assert_eq!(parse(&b), Some(Error::Bad));
        // Too many corners for u16 indices: not packed.
        let many: Vec<_> = (0..70_000)
            .map(|i| {
                let x = i as f32 * 0.01;
                ([[x, 0.0, 0.0], [x, 0.0, 1.0], [x + 0.005, 1.0, 0.0]], 0, 0)
            })
            .collect();
        assert!(encode_packed(&many).is_none());
    }

    #[test]
    fn rejects_short_data() {
        assert_eq!(
            TriMesh::parse(&[0, 0, 0, 5], (Vec3::ZERO, 0.0)).err(),
            Some(Error::Bad)
        );
    }
}
