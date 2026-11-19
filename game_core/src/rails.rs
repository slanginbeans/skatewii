//! Grind rails: polylines in world space (ledge edges, handrails, coping). An imported park's come from
//! its `park/rails` pack entry (`fg_import park` samples the game's spline rails); the demo park has one
//! along its ledge.
//!
//! `park/rails`: `u32` rail count, then per rail a `u32` point count and that many XYZ `f32` points; all
//! big-endian.

use alloc::vec::Vec;

use crate::math::{Vec3, sqrtf, v3};

#[derive(Clone, Debug, PartialEq)]
pub struct Rail {
    pub points: Vec<Vec3>,
    /// Distance along the rail at each point.
    cum: Vec<f32>,
}

/// The nearest point of a rail to some position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nearest {
    /// Distance along the rail.
    pub s: f32,
    pub point: Vec3,
    /// Horizontal distance from the position to the rail.
    pub dist_h: f32,
}

impl Rail {
    pub fn new(points: Vec<Vec3>) -> Rail {
        let mut cum = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                total += (*p - points[i - 1]).length();
            }
            cum.push(total);
        }
        Rail { points, cum }
    }

    pub fn length(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Position and unit direction (towards increasing `s`) at distance `s` along the rail (clamped).
    pub fn at(&self, s: f32) -> (Vec3, Vec3) {
        let n = self.points.len();
        if n < 2 {
            return (
                self.points.first().copied().unwrap_or(Vec3::ZERO),
                v3(0.0, 0.0, 1.0),
            );
        }
        let s = s.clamp(0.0, self.length());
        // Segment holding s.
        let i = match self.cum.iter().position(|&c| c > s) {
            Some(0) => 0,
            Some(k) => k - 1,
            None => n - 2,
        };
        let (a, b) = (self.points[i], self.points[i + 1]);
        let len = self.cum[i + 1] - self.cum[i];
        let k = if len > 0.0 {
            (s - self.cum[i]) / len
        } else {
            0.0
        };
        (a.lerp(b, k), (b - a).normalize_or(v3(0.0, 0.0, 1.0)))
    }

    /// The point of the rail nearest to `p` seen from above (the rail's height doesn't count).
    pub fn nearest(&self, p: Vec3) -> Option<Nearest> {
        let mut best: Option<Nearest> = None;
        for i in 0..self.points.len().saturating_sub(1) {
            let (a, b) = (self.points[i], self.points[i + 1]);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let len2 = dx * dx + dz * dz;
            let k = if len2 > 1e-9 {
                (((p.x - a.x) * dx + (p.z - a.z) * dz) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let q = a.lerp(b, k);
            let d = sqrtf((p.x - q.x) * (p.x - q.x) + (p.z - q.z) * (p.z - q.z));
            if best.is_none_or(|b| d < b.dist_h) {
                best = Some(Nearest {
                    s: self.cum[i] + k * (self.cum[i + 1] - self.cum[i]),
                    point: q,
                    dist_h: d,
                });
            }
        }
        best
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Encodes rails as the `park/rails` entry (for the importer and tests).
pub fn encode(rails: &[Vec<[f32; 3]>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(rails.len() as u32).to_be_bytes());
    for r in rails {
        out.extend_from_slice(&(r.len() as u32).to_be_bytes());
        for p in r {
            for c in p {
                out.extend_from_slice(&c.to_be_bytes());
            }
        }
    }
    out
}

/// Parses a `park/rails` entry. `None` if it is malformed.
pub fn parse(data: &[u8]) -> Option<Vec<Rail>> {
    let n = be32(data, 0)? as usize;
    let mut at = 4;
    let mut rails = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let count = be32(data, at)? as usize;
        at += 4;
        let mut pts = Vec::with_capacity(count.min(65536));
        for _ in 0..count {
            let f = |o: usize| be32(data, at + o).map(f32::from_bits);
            let p = v3(f(0)?, f(4)?, f(8)?);
            if !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
                return None;
            }
            pts.push(p);
            at += 12;
        }
        if pts.len() >= 2 {
            rails.push(Rail::new(pts));
        }
    }
    Some(rails)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_and_finds_points_on_a_rail() {
        // An L: 4 m along +Z, then 3 m along +X, at 0.5 m.
        let r = Rail::new(alloc::vec![
            v3(0., 0.5, 0.),
            v3(0., 0.5, 4.),
            v3(3., 0.5, 4.)
        ]);
        assert!((r.length() - 7.0).abs() < 1e-5);
        let (p, d) = r.at(5.0);
        assert!((p - v3(1.0, 0.5, 4.0)).length() < 1e-5 && (d - v3(1.0, 0.0, 0.0)).length() < 1e-5);
        assert_eq!(r.at(99.0).0, v3(3.0, 0.5, 4.0), "clamped to the end");
        let n = r.nearest(v3(0.3, 1.0, 2.0)).unwrap();
        assert!((n.s - 2.0).abs() < 1e-5 && (n.dist_h - 0.3).abs() < 1e-5);
        assert_eq!(n.point, v3(0.0, 0.5, 2.0));
    }

    #[test]
    fn round_trips_through_the_pack_entry() {
        let data = encode(&[
            alloc::vec![[0.0, 1.0, 2.0], [3.0, 4.0, 5.0]],
            alloc::vec![[9.0, 9.0, 9.0]], // a single point is not a rail
        ]);
        let rails = parse(&data).unwrap();
        assert_eq!(rails.len(), 1);
        assert_eq!(rails[0].points[1], v3(3.0, 4.0, 5.0));
        assert!(parse(&data[..data.len() - 2]).is_none(), "truncated");
    }
}
