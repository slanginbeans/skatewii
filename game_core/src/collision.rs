//! An imported park's collision (`park.fgpak` from `fg_import park`): triangles in world space (Y up,
//! metres), bucketed in a grid over X/Z for fast queries.
//!
//! Walkable triangles (facing up enough to ride) are ground; steep ones are walls. The board stands on the
//! highest walkable triangle under it that is no higher than a given height, so ledges, rails and
//! overhangs work at any level.
//!
//! Pack entries:
//! - `park/collision`: `u32` triangle count, then per triangle 9 `f32` (three XYZ corners), `u16` surface
//!   id, `u16` flags; all big-endian.
//! - `park/spawn`: 4 `f32`: spawn X, Y, Z and yaw.
//! - `park/rails` (optional): grind rails, see `rails.rs`.

use alloc::vec;
use alloc::vec::Vec;

use crate::math::{Vec3, v3};
use crate::pack::{Kind, Pack};
use crate::park::{Hit, Surface, Terrain};
use crate::rails::{self, Rail};

/// Size of a grid cell (m).
pub const CELL: f32 = 4.0;
/// Triangles whose normal is at least this upward are ground; flatter-to-the-side ones are walls.
pub const WALKABLE_Y: f32 = 0.35;
/// Height above the board at which walls are tested (a bit below the knees).
const WALL_PROBE: f32 = 0.25;
pub const TRIANGLE_BYTES: usize = 40;

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
    pub tris: Vec<Tri>,
    pub min: Vec3,
    pub max: Vec3,
    cols: usize,
    rows: usize,
    /// `cell_start[c]..cell_start[c + 1]` indexes `cell_tris` for cell `c`.
    cell_start: Vec<u32>,
    cell_tris: Vec<u32>,
    spawn: (Vec3, f32),
    /// Grind rails (`park/rails`, optional).
    pub rails: Vec<Rail>,
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

impl TriMesh {
    /// From a park pack's entries.
    pub fn from_pack(pack: &Pack) -> Result<TriMesh, Error> {
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
        let mut m = TriMesh::parse(c.data, spawn)?;
        if let Some(r) = pack.find("park/rails") {
            m.rails = rails::parse(r.data).ok_or(Error::Bad)?;
        }
        Ok(m)
    }

    pub fn parse(data: &[u8], spawn: (Vec3, f32)) -> Result<TriMesh, Error> {
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
        let cols = (((max.x - min.x) / CELL) as usize + 1).max(1);
        let rows = (((max.z - min.z) / CELL) as usize + 1).max(1);
        let mut m = TriMesh {
            tris,
            min,
            max,
            cols,
            rows,
            cell_start: Vec::new(),
            cell_tris: Vec::new(),
            spawn,
            rails: Vec::new(),
        };
        // Two passes: count per cell, then fill.
        let mut counts = vec![0u32; cols * rows];
        for t in &m.tris {
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
        for (i, t) in m.tris.iter().enumerate() {
            m.each_cell(t, |c| {
                cell_tris[fill[c] as usize] = i as u32;
                fill[c] += 1;
            });
        }
        m.cell_start = start;
        m.cell_tris = cell_tris;
        m
    }

    fn cell_of(&self, x: f32, z: f32) -> (isize, isize) {
        (
            ((x - self.min.x) / CELL) as isize - (x < self.min.x) as isize,
            ((z - self.min.z) / CELL) as isize - (z < self.min.z) as isize,
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
    fn cell(&self, x: f32, z: f32) -> &[u32] {
        let (c, r) = self.cell_of(x, z);
        if c < 0 || r < 0 || c >= self.cols as isize || r >= self.rows as isize {
            return &[];
        }
        let i = r as usize * self.cols + c as usize;
        &self.cell_tris[self.cell_start[i] as usize..self.cell_start[i + 1] as usize]
    }

    /// Height of `t`'s plane at (x, z) if (x, z) is inside it seen from above.
    fn height_at(t: &Tri, x: f32, z: f32) -> Option<f32> {
        let [a, b, c] = t.v;
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
        let d = q - p;
        let steps = ((d.x.abs().max(d.z.abs()) / (CELL * 0.5)) as usize + 1).min(64);
        let mut last = None;
        for s in 0..=steps {
            let at = p + d * (s as f32 / steps as f32);
            let cell = self.cell_of(at.x, at.z);
            if last == Some(cell) {
                continue;
            }
            last = Some(cell);
            let hit = self.cell(at.x, at.z).iter().any(|i| {
                let t = &self.tris[*i as usize];
                t.n.y.abs() < WALKABLE_Y && TriMesh::segment_hits(t, p, q)
            });
            if hit {
                return true;
            }
        }
        false
    }

    /// The nearest triangle (either side) a ray from `from` along the unit `dir` meets within `max`.
    pub fn ray(&self, from: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        let to = from + dir * max;
        let steps = (((to.x - from.x).abs().max((to.z - from.z).abs()) / (CELL * 0.5)) as usize
            + 1)
        .min(64);
        let mut best: Option<(f32, usize)> = None;
        let mut last = None;
        for s in 0..=steps {
            let at = from + (to - from) * (s as f32 / steps as f32);
            let cell = self.cell_of(at.x, at.z);
            if last == Some(cell) {
                continue;
            }
            last = Some(cell);
            for &i in self.cell(at.x, at.z) {
                let t = &self.tris[i as usize];
                if let Some(k) = TriMesh::segment_t(t, from, to)
                    && best.is_none_or(|(b, _)| k < b)
                {
                    best = Some((k, i as usize));
                }
            }
        }
        let (k, i) = best?;
        let n = self.tris[i].n;
        Some(Hit {
            t: k * max,
            point: from + dir * (k * max),
            normal: if n.dot(dir) > 0.0 { -n } else { n },
            surface: self.tris[i].surface,
        })
    }

    /// Where along p..q (0..1) the segment crosses triangle t, if it does. (Möller-Trumbore, both sides.)
    fn segment_t(t: &Tri, p: Vec3, q: Vec3) -> Option<f32> {
        let d = q - p;
        let (e1, e2) = (t.v[1] - t.v[0], t.v[2] - t.v[0]);
        let h = d.cross(e2);
        let a = e1.dot(h);
        if a.abs() < 1e-10 {
            return None;
        }
        let f = 1.0 / a;
        let s = p - t.v[0];
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
    fn segment_hits(t: &Tri, p: Vec3, q: Vec3) -> bool {
        let d = q - p;
        let (e1, e2) = (t.v[1] - t.v[0], t.v[2] - t.v[0]);
        let h = d.cross(e2);
        let a = e1.dot(h);
        if a.abs() < 1e-10 {
            return false;
        }
        let f = 1.0 / a;
        let s = p - t.v[0];
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
        for &i in self.cell(x, z) {
            let t = &self.tris[i as usize];
            // Ground faces up; triangles wound the other way are flipped (collision is two-sided).
            let n = if t.n.y < 0.0 { -t.n } else { t.n };
            if n.y < WALKABLE_Y {
                continue;
            }
            if let Some(h) = TriMesh::height_at(t, x, z)
                && h <= max
                && h > best.height
            {
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
        assert_eq!(m.tris.len(), 6);
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

    #[test]
    fn rejects_short_data() {
        assert_eq!(
            TriMesh::parse(&[0, 0, 0, 5], (Vec3::ZERO, 0.0)).err(),
            Some(Error::Bad)
        );
    }
}
