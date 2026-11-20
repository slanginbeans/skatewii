//! Flick-stick gestures: the C-stick's path is matched against stick shapes, the way Skate 3 reads its
//! right stick.
//!
//! A shape is a short list of stick positions with a tolerance radius. A gesture starts when the stick
//! comes within the radius of the first point, advances each time a sample is within the radius of the
//! next one, and completes on the last; samples that fit neither count as misses, and too many drop it.
//! When several complete on the same sample the tightest, quickest one wins. How fast the stick moved
//! through the shape is the flick's strength (a faster flick pops higher).
//!
//! Shapes come from the player's disc (`rules/gestures`, converted by `fg_import rules` from Skate 3's
//! `skater.pat`); without it a built-in set of flatground's own shapes is used. Positions are in
//! gesture space: x right, **y towards the player** (stick pulled back is +y).
//!
//! The recognition rules follow SK8-ENGINE/skate-3-rust-engine's reading of the game (GPL-3.0).
//!
//! `rules/gestures` entry: `u32` count, then per shape a `u8` name length and name, `f32` tolerance,
//! `u8` point count and that many x, y `f32` pairs. All big-endian.

use alloc::string::String;
use alloc::vec::Vec;

use crate::math::{PI, cosf, sinf};

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub name: String,
    pub points: Vec<[f32; 2]>,
    /// Tolerance radius, squared.
    pub tol2: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Node {
    active: bool,
    complete: bool,
    next: usize,
    elapsed: u16,
    misses: u8,
    dist: f32,
}

impl Node {
    fn tick(&mut self, shape: &Shape, p: [f32; 2], max_misses: u8) {
        if !self.active {
            let d = dist2(shape.points[0], p);
            if d <= shape.tol2 {
                *self = Node {
                    active: true,
                    next: 1,
                    elapsed: 1,
                    dist: d,
                    ..Node::default()
                };
            }
            return;
        }
        if self.complete {
            return;
        }
        let d = dist2(shape.points[self.next], p);
        if d <= shape.tol2 {
            self.dist += d;
            self.elapsed = (self.elapsed + 1) & 0x3ff;
            if self.next + 1 == shape.points.len() {
                self.complete = true;
            } else {
                self.next += 1;
                self.misses = 0;
            }
        } else {
            // Waiting on the first point doesn't age a gesture that has only just started.
            if self.next != 1 || dist2(shape.points[0], p) > shape.tol2 {
                self.elapsed = (self.elapsed + 1) & 0x3ff;
                self.misses = self.misses.saturating_add(1);
            }
            if self.misses > max_misses {
                *self = Node::default();
            }
        }
    }

    fn score(&self, points: usize) -> f32 {
        let n = points as f32;
        let mean = (self.dist.max(0.15) / n).min(0.15);
        (n * n * n * n) / (mean * self.elapsed.max(1) as f32)
    }
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    let (x, y) = (a[0] - b[0], a[1] - b[1]);
    x * x + y * y
}

/// A recognised gesture.
#[derive(Clone, Debug, PartialEq)]
pub struct Recognised {
    /// Index into the recognizer's shapes.
    pub shape: usize,
    /// 0..1: how fast the stick went through the shape.
    pub strength: f32,
}

#[derive(Clone, Debug)]
pub struct Recognizer {
    shapes: Vec<Shape>,
    nodes: Vec<Node>,
    started: bool,
    refractory: bool,
    held: Option<usize>,
}

/// Samples per shape point at or below which a flick is full strength, and at or above which it has
/// none.
const FAST: f32 = 1.75;
const SLOW: f32 = 4.4;

impl Recognizer {
    pub fn new(shapes: Vec<Shape>) -> Recognizer {
        let shapes: Vec<Shape> = shapes
            .into_iter()
            .filter(|s| (2..=15).contains(&s.points.len()) && s.tol2.is_finite() && s.tol2 >= 0.0)
            .collect();
        Recognizer {
            nodes: alloc::vec![Node::default(); shapes.len()],
            shapes,
            started: false,
            refractory: false,
            held: None,
        }
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// The last recognised shape, while the stick stays on its final point.
    pub fn held(&mut self, p: [f32; 2]) -> Option<usize> {
        if let Some(i) = self.held {
            let s = &self.shapes[i];
            if dist2(*s.points.last().unwrap_or(&[0.0, 0.0]), p) > s.tol2 {
                self.held = None;
            }
        }
        self.held
    }

    /// Feeds one sample (gesture space); returns a shape when one completes.
    pub fn sample(&mut self, p: [f32; 2], max_misses: u8) -> Option<Recognised> {
        if self.refractory {
            self.nodes.fill(Node::default());
            self.refractory = false;
            return None;
        }
        if !self.started {
            self.started = true;
            return None;
        }
        for (n, s) in self.nodes.iter_mut().zip(&self.shapes) {
            n.tick(s, p, max_misses);
        }
        let mut best: Option<usize> = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if n.complete
                && best.is_none_or(|b| {
                    n.score(self.shapes[i].points.len())
                        > self.nodes[b].score(self.shapes[b].points.len())
                })
            {
                best = Some(i);
            }
        }
        let i = best?;
        let ratio = self.nodes[i].elapsed as f32 / self.shapes[i].points.len() as f32;
        let strength = ((SLOW - ratio) / (SLOW - FAST)).clamp(0.0, 1.0);
        self.refractory = true;
        self.held = Some(i);
        Some(Recognised { shape: i, strength })
    }

    /// Forgets any gesture in progress (the next sample starts fresh).
    pub fn reset(&mut self) {
        self.nodes.fill(Node::default());
        self.held = None;
    }
}

/// Encodes shapes as a `rules/gestures` entry (tolerance given as a radius).
pub fn encode(shapes: &[(String, f32, Vec<[f32; 2]>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(shapes.len() as u32).to_be_bytes());
    for (name, tol, pts) in shapes {
        out.push(name.len().min(255) as u8);
        out.extend_from_slice(&name.as_bytes()[..name.len().min(255)]);
        out.extend_from_slice(&tol.to_be_bytes());
        out.push(pts.len().min(255) as u8);
        for p in pts.iter().take(255) {
            out.extend_from_slice(&p[0].to_be_bytes());
            out.extend_from_slice(&p[1].to_be_bytes());
        }
    }
    out
}

/// Parses a `rules/gestures` entry. `None` if it is malformed.
pub fn parse(data: &[u8]) -> Option<Vec<Shape>> {
    let f = |at: usize| Some(f32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let n = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?) as usize;
    let mut at = 4;
    let mut out = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        let len = *data.get(at)? as usize;
        let name = core::str::from_utf8(data.get(at + 1..at + 1 + len)?).ok()?;
        at += 1 + len;
        let tol = f(at)?;
        let count = *data.get(at + 4)? as usize;
        at += 5;
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            let p = [f(at)?, f(at + 4)?];
            if !(p[0].is_finite() && p[1].is_finite()) {
                return None;
            }
            points.push(p);
            at += 8;
        }
        out.push(Shape {
            name: name.into(),
            points,
            tol2: tol * tol,
        });
    }
    Some(out)
}

/// A point on the stick's rim, `deg` degrees clockwise from straight forward (0 = forward, 90 = right,
/// 180 = back), in gesture space.
fn rim(deg: f32, r: f32) -> [f32; 2] {
    let a = deg * PI / 180.0;
    [sinf(a) * r, -cosf(a) * r]
}

/// flatground's own shapes (used without a rules pack): flicks from the back of the stick for tricks
/// off the tail, from the front for nollies. Kickflips end to the right of forward and heelflips to the
/// left (for a goofy rider; the stance mirrors them), shove-its roll around the rim, combinations roll
/// and then flick across.
pub fn built_in() -> Vec<Shape> {
    let shape = |name: &str, tol: f32, pts: &[(f32, f32)]| Shape {
        name: name.into(),
        points: pts.iter().map(|&(d, r)| rim(d, r)).collect(),
        tol2: tol * tol,
    };
    let mut v = Vec::new();
    for (prefix, base) in [("", 180.0f32), ("N_", 0.0)] {
        let at = |d: f32| base + d;
        let nm = |n: &str| {
            let mut s = String::from(prefix);
            s.push_str(n);
            s
        };
        let ollie = if prefix.is_empty() { "Ollie" } else { "Nollie" };
        v.push(shape(ollie, 0.5, &[(at(0.0), 0.95), (at(180.0), 0.95)]));
        v.push(shape(
            &nm("Kickflip"),
            0.42,
            &[(at(0.0), 0.8), (at(-130.0), 0.95)],
        ));
        v.push(shape(
            &nm("Heelflip"),
            0.42,
            &[(at(0.0), 0.8), (at(130.0), 0.95)],
        ));
        // Shove-its: half way round the rim; 360s start further round.
        v.push(shape(
            &nm("PopShuvit"),
            0.36,
            &[(at(-20.0), 0.95), (at(-70.0), 0.95), (at(-110.0), 0.95)],
        ));
        v.push(shape(
            &nm("FSPopShuvit"),
            0.36,
            &[(at(20.0), 0.95), (at(70.0), 0.95), (at(110.0), 0.95)],
        ));
        v.push(shape(
            &nm("360PopShuvit"),
            0.36,
            &[(at(60.0), 0.95), (at(-15.0), 0.95), (at(-95.0), 0.95)],
        ));
        v.push(shape(
            &nm("FS360PopShuvit"),
            0.36,
            &[(at(-60.0), 0.95), (at(15.0), 0.95), (at(95.0), 0.95)],
        ));
        // Roll a quarter round, then flick across to the far side.
        v.push(shape(
            &nm("VarialKickflip"),
            0.42,
            &[(at(45.0), 0.95), (at(-10.0), 0.95), (at(-120.0), 0.95)],
        ));
        v.push(shape(
            &nm("VarialHeelflip"),
            0.42,
            &[(at(-45.0), 0.95), (at(10.0), 0.95), (at(150.0), 0.9)],
        ));
        v.push(shape(
            &nm("Hardflip"),
            0.42,
            &[(at(-45.0), 0.95), (at(10.0), 0.95), (at(-140.0), 0.95)],
        ));
        v.push(shape(
            &nm("InwardHeelflip"),
            0.42,
            &[(at(45.0), 0.95), (at(-10.0), 0.95), (at(135.0), 0.95)],
        ));
        // Three-quarters round, then across.
        v.push(shape(
            &nm("360Flip"),
            0.42,
            &[
                (at(80.0), 0.95),
                (at(35.0), 0.95),
                (at(-15.0), 0.95),
                (at(-115.0), 0.95),
            ],
        ));
        v.push(shape(
            &nm("Laserflip"),
            0.42,
            &[
                (at(-80.0), 0.95),
                (at(-35.0), 0.95),
                (at(15.0), 0.95),
                (at(120.0), 0.95),
            ],
        ));
        v.push(shape(
            &nm("360Hardflip"),
            0.42,
            &[
                (at(-80.0), 0.95),
                (at(-35.0), 0.95),
                (at(10.0), 0.95),
                (at(-140.0), 0.95),
            ],
        ));
        v.push(shape(
            &nm("360InwardHeelflip"),
            0.42,
            &[
                (at(80.0), 0.95),
                (at(35.0), 0.95),
                (at(-10.0), 0.95),
                (at(140.0), 0.95),
            ],
        ));
    }
    v
}

/// What a flick does: the board's motion and how it is named and scored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trick {
    Ollie,
    Kickflip,
    Heelflip,
    PopShuvit,
    FsPopShuvit,
    Bs360Shuvit,
    Fs360Shuvit,
    VarialKickflip,
    VarialHeelflip,
    Hardflip,
    InwardHeelflip,
    TreFlip,
    Laserflip,
    Hardflip360,
    InwardHeelflip360,
}

pub const TRICKS: [Trick; 15] = [
    Trick::Ollie,
    Trick::Kickflip,
    Trick::Heelflip,
    Trick::PopShuvit,
    Trick::FsPopShuvit,
    Trick::Bs360Shuvit,
    Trick::Fs360Shuvit,
    Trick::VarialKickflip,
    Trick::VarialHeelflip,
    Trick::Hardflip,
    Trick::InwardHeelflip,
    Trick::TreFlip,
    Trick::Laserflip,
    Trick::Hardflip360,
    Trick::InwardHeelflip360,
];

impl Trick {
    /// The gesture name it is recognised as (without the nollie prefix).
    pub fn gesture(self) -> &'static str {
        match self {
            Trick::Ollie => "Ollie",
            Trick::Kickflip => "Kickflip",
            Trick::Heelflip => "Heelflip",
            Trick::PopShuvit => "PopShuvit",
            Trick::FsPopShuvit => "FSPopShuvit",
            Trick::Bs360Shuvit => "360PopShuvit",
            Trick::Fs360Shuvit => "FS360PopShuvit",
            Trick::VarialKickflip => "VarialKickflip",
            Trick::VarialHeelflip => "VarialHeelflip",
            Trick::Hardflip => "Hardflip",
            Trick::InwardHeelflip => "InwardHeelflip",
            Trick::TreFlip => "360Flip",
            Trick::Laserflip => "Laserflip",
            Trick::Hardflip360 => "360Hardflip",
            Trick::InwardHeelflip360 => "360InwardHeelflip",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Trick::Ollie => "OLLIE",
            Trick::Kickflip => "KICKFLIP",
            Trick::Heelflip => "HEELFLIP",
            Trick::PopShuvit => "POP SHOVE-IT",
            Trick::FsPopShuvit => "FS POP SHOVE-IT",
            Trick::Bs360Shuvit => "360 SHOVE-IT",
            Trick::Fs360Shuvit => "FS 360 SHOVE-IT",
            Trick::VarialKickflip => "VARIAL KICKFLIP",
            Trick::VarialHeelflip => "VARIAL HEELFLIP",
            Trick::Hardflip => "HARDFLIP",
            Trick::InwardHeelflip => "INWARD HEELFLIP",
            Trick::TreFlip => "360 FLIP",
            Trick::Laserflip => "LASER FLIP",
            Trick::Hardflip360 => "360 HARDFLIP",
            Trick::InwardHeelflip360 => "360 INWARD HEELFLIP",
        }
    }

    /// Board rotation: turns around its long axis (kickflip direction +1) and half turns under the feet
    /// (backside +1).
    pub fn motion(self) -> (f32, f32) {
        match self {
            Trick::Ollie => (0.0, 0.0),
            Trick::Kickflip => (1.0, 0.0),
            Trick::Heelflip => (-1.0, 0.0),
            Trick::PopShuvit => (0.0, 1.0),
            Trick::FsPopShuvit => (0.0, -1.0),
            Trick::Bs360Shuvit => (0.0, 2.0),
            Trick::Fs360Shuvit => (0.0, -2.0),
            Trick::VarialKickflip => (1.0, 1.0),
            Trick::VarialHeelflip => (-1.0, -1.0),
            Trick::Hardflip => (1.0, -1.0),
            Trick::InwardHeelflip => (-1.0, 1.0),
            Trick::TreFlip => (1.0, 2.0),
            Trick::Laserflip => (-1.0, -2.0),
            Trick::Hardflip360 => (1.0, -2.0),
            Trick::InwardHeelflip360 => (-1.0, 2.0),
        }
    }

    pub fn points(self) -> u32 {
        let (flip, shove) = self.motion();
        100 + 200 * flip.abs() as u32 + 150 * shove.abs() as u32
    }

    /// The mirror-image trick (what the same stick motion does for the other stance).
    pub fn mirrored(self) -> Trick {
        match self {
            Trick::Ollie => Trick::Ollie,
            Trick::Kickflip => Trick::Heelflip,
            Trick::Heelflip => Trick::Kickflip,
            Trick::PopShuvit => Trick::FsPopShuvit,
            Trick::FsPopShuvit => Trick::PopShuvit,
            Trick::Bs360Shuvit => Trick::Fs360Shuvit,
            Trick::Fs360Shuvit => Trick::Bs360Shuvit,
            Trick::VarialKickflip => Trick::VarialHeelflip,
            Trick::VarialHeelflip => Trick::VarialKickflip,
            Trick::Hardflip => Trick::InwardHeelflip,
            Trick::InwardHeelflip => Trick::Hardflip,
            Trick::TreFlip => Trick::Laserflip,
            Trick::Laserflip => Trick::TreFlip,
            Trick::Hardflip360 => Trick::InwardHeelflip360,
            Trick::InwardHeelflip360 => Trick::Hardflip360,
        }
    }
}

/// A flick trick: what the board does, and whether it pops off the nose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flick {
    pub trick: Trick,
    pub nollie: bool,
}

impl Flick {
    pub const OLLIE: Flick = Flick {
        trick: Trick::Ollie,
        nollie: false,
    };
    pub const NOLLIE: Flick = Flick {
        trick: Trick::Ollie,
        nollie: true,
    };

    pub const fn of(trick: Trick) -> Flick {
        Flick {
            trick,
            nollie: false,
        }
    }

    /// From a gesture name (`Kickflip`, `N_360Flip`, `Nollie`...). `None` for gestures that aren't
    /// ground flick tricks.
    pub fn from_gesture(name: &str) -> Option<Flick> {
        if name == "Nollie" {
            return Some(Flick::NOLLIE);
        }
        let (nollie, base) = match name.strip_prefix("N_") {
            Some(b) => (true, b),
            None => (false, name),
        };
        let trick = TRICKS.iter().copied().find(|t| t.gesture() == base)?;
        if nollie && trick == Trick::Ollie {
            return None;
        }
        Some(Flick { trick, nollie })
    }

    pub fn points(self) -> u32 {
        self.trick.points() + if self.nollie { 20 } else { 0 }
    }
}

/// Which family of flicks the board allows right now (Skate 3's gesture groups).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Four wheels down: every flick as recognised.
    Square,
    /// On the front wheels (nose manual): tail flicks become their nollie version.
    Nose,
    /// On the back wheels (manual): nollie flicks become their tail version.
    Tail,
}

/// The trick a recognised gesture gives: mirrored for the stance (`mirrored`: the rider's front foot is
/// the other one from the shapes' goofy rider), then fitted to the group. Multi-axis tricks off the
/// wrong end of the board collapse to a plain ollie or nollie, as in Skate 3.
pub fn select(flick: Flick, group: Group, mirrored: bool) -> Flick {
    let trick = if mirrored {
        flick.trick.mirrored()
    } else {
        flick.trick
    };
    let shove_only = matches!(
        trick,
        Trick::PopShuvit | Trick::FsPopShuvit | Trick::Bs360Shuvit | Trick::Fs360Shuvit
    );
    let half = |t: Trick| match t {
        Trick::Bs360Shuvit => Trick::PopShuvit,
        Trick::Fs360Shuvit => Trick::FsPopShuvit,
        t => t,
    };
    match (group, flick.nollie) {
        (Group::Square, n) => Flick { trick, nollie: n },
        (Group::Nose, true) | (Group::Tail, false) => Flick {
            trick,
            nollie: flick.nollie,
        },
        // Off the other end: shove-its keep their direction (as half turns), the rest are plain pops.
        (Group::Nose, false) | (Group::Tail, true) => Flick {
            trick: if shove_only {
                half(trick)
            } else {
                Trick::Ollie
            },
            nollie: group == Group::Nose,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn run(r: &mut Recognizer, samples: &[[f32; 2]]) -> Vec<(String, f32)> {
        let mut out = Vec::new();
        for p in samples {
            if let Some(g) = r.sample(*p, 8) {
                out.push((r.shapes()[g.shape].name.clone(), g.strength));
            }
        }
        out
    }

    /// Stick at `deg` clockwise from forward, as a gesture-space sample.
    fn at(deg: f32) -> [f32; 2] {
        rim(deg, 1.0)
    }

    fn hold(p: [f32; 2], n: usize) -> Vec<[f32; 2]> {
        vec![p; n]
    }

    #[test]
    fn built_in_flicks() {
        for (end, want) in [
            (0.0, "Ollie"),
            (-125.0 + 180.0, "Kickflip"),
            (125.0 + 180.0, "Heelflip"),
        ] {
            let mut r = Recognizer::new(built_in());
            let mut s = hold(at(180.0), 10);
            s.push([0.0, 0.0]);
            // The kickflip end point is "back, then 130 degrees round", i.e. forward and to the side.
            s.push(at(end));
            let got = run(&mut r, &s);
            assert_eq!(
                got.first().map(|g| g.0.as_str()),
                Some(want),
                "end {end}: {got:?}"
            );
        }
    }

    #[test]
    fn quick_flicks_are_strong_and_slow_ones_weak_or_dropped() {
        let mut r = Recognizer::new(built_in());
        let mut s = hold(at(180.0), 5);
        s.push(at(0.0));
        let got = run(&mut r, &s);
        assert_eq!(got[0].0, "Ollie");
        assert_eq!(got[0].1, 1.0);

        let mut r = Recognizer::new(built_in());
        let mut s = hold(at(180.0), 5);
        s.extend(hold([0.0, 0.0], 6));
        s.push(at(0.0));
        let got = run(&mut r, &s);
        assert_eq!(got[0].0, "Ollie");
        assert!(got[0].1 < 0.7, "{got:?}");

        // Lingering in the middle longer than the misses allow drops it.
        let mut r = Recognizer::new(built_in());
        let mut s = hold(at(180.0), 5);
        s.extend(hold([0.0, 0.0], 20));
        s.push(at(0.0));
        assert!(run(&mut r, &s).is_empty());
    }

    #[test]
    fn rim_rolls_are_shove_its_and_combos() {
        let mut r = Recognizer::new(built_in());
        let mut s = hold(at(160.0), 4);
        s.extend([130.0, 90.0, 70.0].map(at));
        assert_eq!(run(&mut r, &s)[0].0, "PopShuvit");
        // Three quarters round from the back, then across: 360 flip.
        let mut r = Recognizer::new(built_in());
        let mut s = hold(at(260.0), 4);
        s.extend([235.0, 215.0, 190.0, 165.0].map(at));
        // ...and across the middle to the far side.
        s.extend([[0.6, 0.3], at(65.0)]);
        assert_eq!(run(&mut r, &s)[0].0, "360Flip");
    }

    #[test]
    fn nollies_from_the_front() {
        let mut r = Recognizer::new(built_in());
        let mut s = hold(at(0.0), 6);
        s.push([0.0, 0.0]);
        s.push(at(180.0));
        assert_eq!(run(&mut r, &s)[0].0, "Nollie");
    }

    #[test]
    fn holding_the_end_is_reported_until_the_stick_leaves() {
        let mut r = Recognizer::new(built_in());
        let mut s = hold(at(180.0), 5);
        s.push(at(50.0));
        let got = run(&mut r, &s);
        assert_eq!(got[0].0, "Kickflip");
        assert!(r.held(at(50.0)).is_some());
        assert!(r.held(at(180.0)).is_none());
    }

    #[test]
    fn entry_round_trip() {
        let data = encode(&[("Ollie".into(), 0.5, vec![[0.0, 1.0], [0.0, -1.0]])]);
        let s = parse(&data).unwrap();
        assert_eq!(s[0].name, "Ollie");
        assert_eq!(s[0].points, vec![[0.0, 1.0], [0.0, -1.0]]);
        assert!((s[0].tol2 - 0.25).abs() < 1e-6);
        assert!(parse(&data[..data.len() - 1]).is_none());
    }

    #[test]
    fn mapping_mirrors_and_fits_the_board() {
        let kf = Flick::from_gesture("Kickflip").unwrap();
        assert_eq!(select(kf, Group::Square, false).trick, Trick::Kickflip);
        assert_eq!(select(kf, Group::Square, true).trick, Trick::Heelflip);
        let tre = Flick::from_gesture("N_360Flip").unwrap();
        assert!(tre.nollie && tre.trick == Trick::TreFlip);
        assert_eq!(select(tre, Group::Square, true).trick, Trick::Laserflip);
        // From a manual (back wheels) a nollie flip is just an ollie; on the nose a shove-it stays one.
        assert_eq!(select(tre, Group::Tail, false), Flick::OLLIE);
        let shove = Flick::from_gesture("360PopShuvit").unwrap();
        assert_eq!(
            select(shove, Group::Nose, false),
            Flick {
                trick: Trick::PopShuvit,
                nollie: true
            }
        );
        assert_eq!(Flick::from_gesture("Nollie"), Some(Flick::NOLLIE));
        assert_eq!(Flick::from_gesture("FrontFlip"), None);
        // Every built-in shape is a trick.
        for s in built_in() {
            assert!(Flick::from_gesture(&s.name).is_some(), "{}", s.name);
        }
    }
}
