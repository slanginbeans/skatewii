//! Grind rails: polylines in world space (ledge edges, handrails, coping). An imported park's come from
//! its `park/rails` pack entry (`fg_import park` samples the game's spline rails); the demo park has one
//! along its ledge.
//!
//! Each segment has a class (thin rail, fat rail, ledge, curb, or blocked) and, for ledges, the side that
//! is high, found by probing the park beside it as Skate 3 does (SK8-ENGINE/skate-3-rust-engine,
//! GPL-3.0: `air/trajectory/grind_surface.rs`, `grind_surface/classify.rs`): [`classify`]. The class
//! picks the grind's friction, its lean-off and whether a tip on it is a blunt.
//!
//! `park/rails` (version 2): `u32` [`MAGIC`], `u32` rail count, then per rail a `u32` point count, that
//! many XYZ `f32` points, and per segment (one fewer) a `u8` class ([`RailClass`]) and an `i8` high side
//! (+1: towards `UP x direction`, -1: the other side); all big-endian. The first version (no magic: the
//! rail count first, points only) still loads, every segment a thin rail.

use alloc::vec::Vec;

use crate::math::{Vec3, sqrtf, v3};
use crate::park::Terrain;

/// Marks a version-2 `park/rails` entry ("FGR2"; a park never has this many rails).
pub const MAGIC: u32 = 0x4647_5232;

/// What a rail segment is, from what is beside it (SK8 `GeometryType`, plus the curb flag).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RailClass {
    /// Nothing beside it within a few centimetres: a handrail.
    #[default]
    Thin,
    /// Something beside it at its height, but no top carrying on: a thick rail or a narrow edge.
    Fat,
    /// The edge of a top that carries on to one side (a ledge, a box, coping): that side is high.
    Ledge,
    /// A ledge whose low side is only a curb's height down.
    Curb,
    /// Something on both sides or just above it: not grindable.
    Blocked,
}

impl RailClass {
    pub fn from_u8(v: u8) -> RailClass {
        match v {
            1 => RailClass::Fat,
            2 => RailClass::Ledge,
            3 => RailClass::Curb,
            4 => RailClass::Blocked,
            _ => RailClass::Thin,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// A top carries on to one side (a ledge or a curb): it has a high side.
    pub fn is_ledge(self) -> bool {
        matches!(self, RailClass::Ledge | RailClass::Curb)
    }

    /// Skate 3's three friction columns: thin, fat, ledge (curbs count as ledges).
    pub fn column(self) -> usize {
        match self {
            RailClass::Thin => 0,
            RailClass::Fat => 1,
            _ => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RailClass::Thin => "thin rail",
            RailClass::Fat => "fat rail",
            RailClass::Ledge => "ledge",
            RailClass::Curb => "curb",
            RailClass::Blocked => "blocked",
        }
    }
}

/// A segment's class and which side is high (+1 towards `UP x direction`, -1 the other way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentClass {
    pub class: RailClass,
    pub high: i8,
}

impl Default for SegmentClass {
    fn default() -> Self {
        SegmentClass {
            class: RailClass::Thin,
            high: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rail {
    pub points: Vec<Vec3>,
    /// Distance along the rail at each point.
    cum: Vec<f32>,
    /// Per segment (between points `i` and `i + 1`): its class and high side.
    pub classes: Vec<SegmentClass>,
    /// The rail's bounding box.
    pub min: Vec3,
    pub max: Vec3,
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

/// The rail's upward normal across a direction (SK8 `upright_normal`): perpendicular to it, facing up.
pub fn upright(dir: Vec3) -> Vec3 {
    let n = Vec3::UP.cross(dir).cross(dir);
    let n = if n.y < 0.0 { -n } else { n };
    n.normalize_or(v3(1.0, 0.0, 0.0))
}

/// The horizontal side of a direction the high side is measured against: `UP x direction`.
pub fn side_of(dir: Vec3) -> Vec3 {
    Vec3::UP.cross(dir).normalize_or(v3(1.0, 0.0, 0.0))
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
        let classes = alloc::vec![SegmentClass::default(); points.len().saturating_sub(1)];
        let (mut min, mut max) = (Vec3::ZERO, Vec3::ZERO);
        for (i, p) in points.iter().enumerate() {
            if i == 0 {
                (min, max) = (*p, *p);
            }
            min = v3(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
            max = v3(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
        }
        Rail {
            points,
            cum,
            classes,
            min,
            max,
        }
    }

    /// Whether `p` is within `margin` of the rail's bounding box.
    pub fn near(&self, p: Vec3, margin: f32) -> bool {
        p.x >= self.min.x - margin
            && p.x <= self.max.x + margin
            && p.y >= self.min.y - margin
            && p.y <= self.max.y + margin
            && p.z >= self.min.z - margin
            && p.z <= self.max.z + margin
    }

    /// The same rail with its segments classified against the park ([`classify`]).
    pub fn classified(mut self, park: &dyn Terrain) -> Rail {
        self.classes = classify(&self.points, park, FAR_SIDE);
        self
    }

    pub fn length(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Which segment holds distance `s` (clamped).
    pub fn segment(&self, s: f32) -> usize {
        let n = self.points.len();
        if n < 2 {
            return 0;
        }
        match self.cum.iter().position(|&c| c > s) {
            Some(0) => 0,
            Some(k) => k - 1,
            None => n - 2,
        }
    }

    /// Distance along the rail where segment `i` starts.
    pub fn segment_start(&self, i: usize) -> f32 {
        self.cum.get(i).copied().unwrap_or(0.0)
    }

    /// The class at distance `s` and the high side as a horizontal unit vector.
    pub fn class_at(&self, s: f32) -> (RailClass, Vec3) {
        let i = self.segment(s);
        let c = self.classes.get(i).copied().unwrap_or_default();
        let (_, d) = self.at(s);
        (c.class, side_of(d) * f32::from(c.high))
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
        let i = self.segment(s);
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

    /// The point of the rail nearest to `p` in 3-D, and its distance.
    pub fn nearest_3d(&self, p: Vec3) -> Option<(f32, Vec3, f32)> {
        let mut best: Option<(f32, Vec3, f32)> = None;
        for i in 0..self.points.len().saturating_sub(1) {
            let (a, b) = (self.points[i], self.points[i + 1]);
            let d = b - a;
            let len2 = d.dot(d);
            let k = if len2 > 1e-9 {
                ((p - a).dot(d) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let q = a + d * k;
            let dist = (p - q).length();
            if best.is_none_or(|b| dist < b.2) {
                best = Some((self.cum[i] + k * (self.cum[i + 1] - self.cum[i]), q, dist));
            }
        }
        best
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A rail as points and its segments' classes (missing classes are thin rails).
pub type RailData = (Vec<[f32; 3]>, Vec<SegmentClass>);

/// Encodes rails as the `park/rails` entry (version 2, for the importer and tests).
pub fn encode(rails: &[RailData]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(&(rails.len() as u32).to_be_bytes());
    for (r, classes) in rails {
        out.extend_from_slice(&(r.len() as u32).to_be_bytes());
        for p in r {
            for c in p {
                out.extend_from_slice(&c.to_be_bytes());
            }
        }
        for i in 0..r.len().saturating_sub(1) {
            let c = classes.get(i).copied().unwrap_or_default();
            out.push(c.class.to_u8());
            out.push(c.high as u8);
        }
    }
    out
}

/// Parses a `park/rails` entry (either version). `None` if it is malformed.
pub fn parse(data: &[u8]) -> Option<Vec<Rail>> {
    let v2 = be32(data, 0)? == MAGIC;
    let mut at = if v2 { 4 } else { 0 };
    let n = be32(data, at)? as usize;
    at += 4;
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
        let mut classes = Vec::new();
        if v2 {
            for _ in 0..count.saturating_sub(1) {
                let b = data.get(at..at + 2)?;
                classes.push(SegmentClass {
                    class: RailClass::from_u8(b[0]),
                    high: if (b[1] as i8) < 0 { -1 } else { 1 },
                });
                at += 2;
            }
        }
        if pts.len() >= 2 {
            let mut r = Rail::new(pts);
            if v2 {
                r.classes = classes;
            }
            rails.push(r);
        }
    }
    Some(rails)
}

/// How far beside the rail the near probes look and how far up and down they reach (SK8
/// grind_surface.rs: 0.09 and 0.04 m); the far probes reach out by the deck-centre-to-truck distance and
/// 1.06 times that up and down; the blocking probe crosses 0.09 m over the rail. A far probe meeting a
/// top within 0.65 of its length (from the top) makes a ledge.
const NEAR_SIDE: f32 = 0.09;
const NEAR_REACH: f32 = 0.04;
/// The far probes' reach out without the disc's deck-centre-to-truck distance (flatground's).
pub const FAR_SIDE: f32 = 0.3;
const LEDGE_FRACTION: f32 = 0.65;

/// Classifies each segment of a rail by probing the park beside its middle (SK8
/// `grind_surface::prepare` and `classify::resolve`): something on both near sides (or just above)
/// blocks it; something on one near side is a fat rail, or a ledge when the far probe on that side meets
/// a top high enough; nothing beside it is a thin rail. A ledge whose top is level with the rail and
/// whose other side drops 0.20 to 0.24 m onto a flat top is a curb. The high side is the side with
/// something beside it.
///
/// `far` is how far out the far probes reach: the board's deck-centre-to-truck distance
/// (`physics_grinds/default/DeckCenterToTruck` from the player's disc at import, else [`FAR_SIDE`]).
pub fn classify(points: &[Vec3], park: &dyn Terrain, far: f32) -> Vec<SegmentClass> {
    points
        .windows(2)
        .map(|w| classify_segment(w[0], w[1], park, far))
        .collect()
}

type ProbeHit = Option<(f32, Vec3, Vec3)>;

fn classify_segment(a: Vec3, b: Vec3, park: &dyn Terrain, far: f32) -> SegmentClass {
    let dir = (b - a).normalize_or(v3(0.0, 0.0, 1.0));
    let up = upright(dir);
    let centre = a.lerp(b, 0.5);
    let side = up.cross(dir).normalize_or(side_of(dir));
    // A probe from `top` to `bottom`: where along it (0..1) it meets the park, the point and its normal.
    let probe = |top: Vec3, bottom: Vec3| -> ProbeHit {
        let d = bottom - top;
        let len = d.length().max(1e-6);
        park.raycast(top, d * (1.0 / len), len)
            .map(|h| (h.t / len, h.point, h.normal))
    };
    let line = |at: Vec3, half: Vec3| probe(at + half, at - half);
    let h0 = line(centre + side * NEAR_SIDE, up * NEAR_REACH);
    let h1 = line(centre - side * NEAR_SIDE, up * NEAR_REACH);
    let h2 = line(centre + side * far, up * (far * 1.06));
    let h3 = line(centre - side * far, up * (far * 1.06));
    let raised = centre + up * NEAR_SIDE;
    let h5 = probe(raised + side * NEAR_SIDE, raised - side * NEAR_SIDE);
    let blocked = (h0.is_some() && h1.is_some()) || h5.is_some();
    let ledge = |h: ProbeHit| h.is_some_and(|h| h.0 < LEDGE_FRACTION);
    let class = if blocked {
        RailClass::Blocked
    } else if h0.is_some() {
        if ledge(h2) {
            RailClass::Ledge
        } else {
            RailClass::Fat
        }
    } else if h1.is_some() {
        if ledge(h3) {
            RailClass::Ledge
        } else {
            RailClass::Fat
        }
    } else {
        RailClass::Thin
    };
    // The high side: towards `side` unless only the other side has something beside it.
    let towards = if !blocked && h0.is_none() && h1.is_some() {
        -side
    } else {
        side
    };
    let high = if towards.dot(side_of(dir)) >= 0.0 {
        1
    } else {
        -1
    };
    // A curb: both far probes meet flat tops, the higher level with the rail, the lower a curb's height
    // down (SK8 `is_curb`).
    let curb = class == RailClass::Ledge
        && match (h2, h3) {
            (Some(p), Some(q)) if up.dot(p.2) > 0.99 && up.dot(q.2) > 0.99 => {
                let (hp, hq) = (up.dot(p.1 - centre), up.dot(q.1 - centre));
                let (hi, lo) = if hp >= hq { (hp, hq) } else { (hq, hp) };
                hi > -0.035 && hi < 0.035 && lo > -0.24 && lo < -0.2
            }
            _ => false,
        };
    SegmentClass {
        class: if curb { RailClass::Curb } else { class },
        high,
    }
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
        assert_eq!(r.segment(5.0), 1);
        let (s, q, dist) = r.nearest_3d(v3(0.0, 1.5, 2.0)).unwrap();
        assert!((s - 2.0).abs() < 1e-5 && (q.y - 0.5).abs() < 1e-5 && (dist - 1.0).abs() < 1e-5);
    }

    #[test]
    fn round_trips_through_the_pack_entry() {
        let ledge = SegmentClass {
            class: RailClass::Ledge,
            high: -1,
        };
        let data = encode(&[
            (
                alloc::vec![[0.0, 1.0, 2.0], [3.0, 4.0, 5.0]],
                alloc::vec![ledge],
            ),
            (alloc::vec![[9.0, 9.0, 9.0]], Vec::new()), // a single point is not a rail
        ]);
        let rails = parse(&data).unwrap();
        assert_eq!(rails.len(), 1);
        assert_eq!(rails[0].points[1], v3(3.0, 4.0, 5.0));
        assert_eq!(rails[0].classes, alloc::vec![ledge]);
        assert!(parse(&data[..data.len() - 2]).is_none(), "truncated");
    }

    #[test]
    fn version_one_entries_still_load_as_thin_rails() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(&2u32.to_be_bytes());
        for c in [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0] {
            data.extend_from_slice(&c.to_be_bytes());
        }
        let rails = parse(&data).unwrap();
        assert_eq!(rails.len(), 1);
        assert_eq!(rails[0].classes, alloc::vec![SegmentClass::default()]);
        assert_eq!(rails[0].class_at(0.5).0, RailClass::Thin);
    }

    /// Synthetic geometry: a free-standing rail, a ledge (a top carrying on to +x, 0.5 m over the floor),
    /// a curb (0.22 m), a narrow strip just beside a rail (fat) and a rail sunk in a flat top (blocked).
    pub(crate) fn yard() -> crate::collision::TriMesh {
        let mut t = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            t.push(([a, b, c], 1u16, 0u16));
            t.push(([a, c, d], 1, 0));
        };
        quad(
            [-20., 0., -20.],
            [-20., 0., 20.],
            [20., 0., 20.],
            [20., 0., -20.],
        );
        quad([0., 0.5, -5.], [0., 0.5, 5.], [2., 0.5, 5.], [2., 0.5, -5.]);
        quad([0., 0., -5.], [0., 0.5, -5.], [0., 0.5, 5.], [0., 0., 5.]);
        quad(
            [6., 0.22, -5.],
            [6., 0.22, 5.],
            [8., 0.22, 5.],
            [8., 0.22, -5.],
        );
        quad(
            [12.03, 0.68, -5.],
            [12.03, 0.68, 5.],
            [12.13, 0.68, 5.],
            [12.13, 0.68, -5.],
        );
        quad(
            [14., 0.4, -5.],
            [14., 0.4, 5.],
            [18., 0.4, 5.],
            [18., 0.4, -5.],
        );
        let data = crate::collision::encode(&t);
        crate::collision::TriMesh::parse(&data, (v3(0.0, 0.0, 0.0), 0.0)).unwrap()
    }

    #[test]
    fn rails_are_classed_by_what_is_beside_them() {
        let park = yard();
        let class = |x: f32, y: f32| {
            let r = Rail::new(alloc::vec![v3(x, y, -2.0), v3(x, y, 2.0)]).classified(&park);
            r.class_at(1.0)
        };
        assert_eq!(class(-10.0, 0.7).0, RailClass::Thin);
        let (c, high) = class(0.0, 0.5);
        assert_eq!(c, RailClass::Ledge);
        assert!(high.x > 0.9, "the top is on +x: {high:?}");
        // The same ledge walked the other way: still high towards +x.
        let r = Rail::new(alloc::vec![v3(0.0, 0.5, 2.0), v3(0.0, 0.5, -2.0)]).classified(&park);
        assert!(r.class_at(1.0).1.x > 0.9);
        assert_eq!(class(6.0, 0.22).0, RailClass::Curb);
        assert_eq!(class(12.0, 0.7).0, RailClass::Fat);
        assert_eq!(class(16.0, 0.4).0, RailClass::Blocked);
    }
}
