//! Camera shots: where the camera sits (distance, elevation, heading round the skater), how it frames
//! the skater (yaw, pitch, roll, look-at point, lens) and how quickly it follows; blend shots mix two of
//! their (up to three) children by a value from the skater (speed, slope, steering, air time ...).
//!
//! After SK8-ENGINE/skate-3-rust-engine (GPL-3.0) crates/skate-core/src/camera/shot.rs and
//! shot_manager.rs: a shot change keeps the shot being shown as the "previous" one and eases every field
//! over the new shot's transition time with a sine curve; a blend shot's value is filtered
//! (`(1 - smoothing)^2` of the way each tick), placed between its blend points and sine-eased.
//!
//! Skate 3's shots come from the player's disc (`fg_import rules`: `rules/camera/shots`, the
//! `camera_shots` collections the camera graph uses); [`ShotSet::built_in`] is flatground's own set.
//!
//! Entry `rules/camera/shots` (big-endian):
//! ```text
//! "FGCS", u32 version (1), u32 shot count, then per shot 160 bytes:
//!   u64 key (Skate 3's collection name hash, `name_hash`), u8 kind (0 shot, 1 blend), u8 blend type,
//!   u8 transition units, u8 options (1 follow in air, 2 mirror for stance, 4 use previous shot,
//!   8 use drop predictor, 16 avoidance override, 32 snap to reference, 64 free camera stick), u8 anchor,
//!   u8 compass north, u8 collision hint, u8 0, u16 children[3] (indices, 0xFFFF none), u16 0,
//!   f32[34]: distance, lens, smoothing direction/elevation/yaw/pitch, reference weights[10], board
//!   offset, heading, elevation, framing roll, yaw, pitch (degrees), transition time, blend points[3],
//!   blend value, blend smoothing, world heading (deg), blur, transition blur, opacity, 0, 0
//! ```

use alloc::vec::Vec;

use crate::math::{PI, sinf, wrap_angle};

use super::tracker::{RAD, blend_angle};

pub const MAGIC: [u8; 4] = *b"FGCS";
pub const VERSION: u32 = 1;
pub const RECORD: usize = 24 + 34 * 4;
pub const NONE: u16 = 0xFFFF;
pub const ENTRY: &str = "rules/camera/shots";

pub const OPT_FOLLOW: u8 = 1;
pub const OPT_MIRROR: u8 = 2;
pub const OPT_PREVIOUS: u8 = 4;
pub const OPT_DROP: u8 = 8;
pub const OPT_AVOID: u8 = 16;
pub const OPT_SNAP: u8 = 32;
/// Skate 3's `OptionUseFreeCamStick`: the look stick moves the camera (free look) in this shot.
pub const OPT_LOOK: u8 = 64;

/// Reference points a shot's look-at mixes (`ReferenceWeight*`, in Skate 3's order).
pub const REF_HEAD: usize = 0;
pub const REF_HIPS: usize = 1;
pub const REF_GRIND: usize = 2;
pub const REF_COM: usize = 3;
pub const REF_BOARD: usize = 4;
pub const REF_TRAJECTORY: usize = 5;
pub const REF_ANCHOR: usize = 6;
pub const REF_FEET: usize = 7;
pub const REF_DAMPED_COM: usize = 8;
pub const REFS: usize = 10;

/// Skate 3's collection-name hash (Bob Jenkins' 64-bit lookup8, as its tuning database keys and
/// `RefSpec`s use): shots are found by the hash of their lowercase name.
pub fn name_hash(name: &str) -> u64 {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return 0;
    }
    fn mix(a: &mut u64, b: &mut u64, c: &mut u64) {
        for (x, y, z) in [(43, 9, 8), (38, 23, 5), (35, 49, 11), (12, 18, 22)] {
            *a = a.wrapping_sub(*b).wrapping_sub(*c) ^ (*c >> x);
            *b = b.wrapping_sub(*c).wrapping_sub(*a) ^ (*a << y);
            *c = c.wrapping_sub(*a).wrapping_sub(*b) ^ (*b >> z);
        }
    }
    let lower = |b: u8| b.to_ascii_lowercase() as u64;
    let word = |s: &[u8]| {
        s.iter()
            .enumerate()
            .fold(0u64, |w, (i, &b)| w | lower(b) << (8 * i))
    };
    let mut a = 0xabcd_ef00_1122_3344_u64;
    let mut b = a;
    let mut c = 0x9e37_79b9_7f4a_7c13_u64;
    let mut blocks = bytes.chunks_exact(24);
    for k in &mut blocks {
        a = a.wrapping_add(word(&k[..8]));
        b = b.wrapping_add(word(&k[8..16]));
        c = c.wrapping_add(word(&k[16..]));
        mix(&mut a, &mut b, &mut c);
    }
    c = c.wrapping_add(bytes.len() as u64);
    for (i, &byte) in blocks.remainder().iter().enumerate() {
        match i {
            0..=7 => a = a.wrapping_add(lower(byte) << (8 * i)),
            8..=15 => b = b.wrapping_add(lower(byte) << (8 * (i - 8))),
            _ => c = c.wrapping_add(lower(byte) << (8 * (i - 15))),
        }
    }
    mix(&mut a, &mut b, &mut c);
    c
}

/// One shot's values (angles in radians; heading in Skate 3's sense: 180 degrees is straight behind,
/// positive round to the skater's left before mirroring).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub distance: f32,
    pub lens: f32,
    /// Smoothing of the direction (heading), elevation, framing yaw and framing pitch.
    pub smoothing: [f32; 4],
    pub weights: [f32; REFS],
    pub board_offset: f32,
    pub heading: f32,
    pub elevation: f32,
    /// Framing roll, yaw, pitch.
    pub framing: [f32; 3],
    pub options: u8,
    pub anchor: u8,
    pub compass: u8,
    pub collision: u8,
    /// The arm the camera sits on, worked out each tick: heading (including the compass north) and
    /// elevation.
    pub arm_heading: f32,
    pub arm_elevation: f32,
}

impl Shot {
    /// Before any shot was chosen.
    pub const START: Shot = Shot {
        distance: 2.0,
        lens: 12.0,
        smoothing: [0.8; 4],
        weights: [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        board_offset: 0.0,
        heading: PI,
        elevation: 0.0,
        framing: [0.0; 3],
        options: OPT_FOLLOW | OPT_MIRROR,
        anchor: 0,
        compass: 0,
        collision: 0,
        arm_heading: PI,
        arm_elevation: 0.0,
    };

    pub fn has(&self, o: u8) -> bool {
        self.options & o != 0
    }

    /// A plain shot's arm for this tick: its heading mirrored for the stance (`mirror` -1) and turned to
    /// the compass north. Shots that keep the previous shot keep its arm.
    pub fn place_arm(&mut self, north: f32, mirror: f32) {
        if self.has(OPT_PREVIOUS) {
            return;
        }
        let from = self.heading;
        let h = blend_angle(from, -from, (1.0 - mirror) * 0.5);
        self.arm_heading = wrap_angle(h + north);
        self.arm_elevation = self.elevation;
    }

    /// `from` to `to` by `k` (sine-eased); the options come from `to` (Skate 3: even at 0).
    pub fn blend(from: &Shot, to: &Shot, k: f32) -> Shot {
        let e = ease(k);
        let l = |a: f32, b: f32| a + (b - a) * e;
        let mut s = *to;
        s.distance = l(from.distance, to.distance);
        s.lens = l(from.lens, to.lens);
        for i in 0..4 {
            s.smoothing[i] = l(from.smoothing[i], to.smoothing[i]);
        }
        for i in 0..REFS {
            s.weights[i] = l(from.weights[i], to.weights[i]);
        }
        s.board_offset = l(from.board_offset, to.board_offset);
        s.elevation = l(from.elevation, to.elevation);
        // The authored heading is not blended (the arm carries it).
        s.heading = to.heading;
        for i in 0..3 {
            s.framing[i] = l(from.framing[i], to.framing[i]);
        }
        s.arm_heading = blend_angle(from.arm_heading, to.arm_heading, e);
        s.arm_elevation = l(from.arm_elevation, to.arm_elevation);
        s
    }
}

/// Skate 3's sine ease: 0..1 to 0..1, flat at both ends.
pub fn ease(k: f32) -> f32 {
    sinf(k * PI - PI * 0.5) * 0.5 + 0.5
}

/// A shot as authored: a plain shot, or a blend of up to three children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotDef {
    pub key: u64,
    pub blend: bool,
    pub shot: Shot,
    pub transition_time: f32,
    /// 0 seconds, 1 ticks, 2 metres a second (the camera's travel over its speed).
    pub transition_units: u8,
    pub children: [u16; 3],
    pub points: [f32; 3],
    pub blend_value: f32,
    pub blend_type: u8,
    pub blend_smoothing: f32,
}

impl ShotDef {
    pub fn children(&self) -> impl Iterator<Item = usize> + '_ {
        self.children
            .iter()
            .take_while(|&&c| c != NONE)
            .map(|&c| c as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    BadVersion(u32),
    Truncated,
    /// A child index outside the set, or a cycle.
    Bad,
}

/// Every shot the camera can choose, found by name.
#[derive(Clone, Debug, PartialEq)]
pub struct ShotSet {
    pub defs: Vec<ShotDef>,
    /// The set came from the player's disc.
    pub imported: bool,
}

impl ShotSet {
    pub fn find(&self, name: &str) -> Option<usize> {
        let k = name_hash(name);
        self.defs.iter().position(|d| d.key == k)
    }

    pub fn parse(b: &[u8]) -> Result<ShotSet, Error> {
        if b.get(..4) != Some(&MAGIC[..]) {
            return Err(Error::BadMagic);
        }
        let be32 = |at: usize| -> Result<u32, Error> {
            b.get(at..at + 4)
                .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
                .ok_or(Error::Truncated)
        };
        let version = be32(4)?;
        if version != VERSION {
            return Err(Error::BadVersion(version));
        }
        let count = be32(8)? as usize;
        if b.len() < 12 + count * RECORD {
            return Err(Error::Truncated);
        }
        let mut defs = Vec::with_capacity(count);
        for i in 0..count {
            let r = &b[12 + i * RECORD..12 + (i + 1) * RECORD];
            let u16_at = |at: usize| u16::from_be_bytes([r[at], r[at + 1]]);
            let f = |n: usize| {
                let at = 24 + n * 4;
                let v = f32::from_be_bytes([r[at], r[at + 1], r[at + 2], r[at + 3]]);
                if v.is_finite() { v } else { 0.0 }
            };
            let mut key = [0u8; 8];
            key.copy_from_slice(&r[..8]);
            let mut weights = [0.0; REFS];
            for (k, w) in weights.iter_mut().enumerate() {
                *w = f(6 + k);
            }
            let shot = Shot {
                distance: f(0),
                lens: f(1),
                smoothing: [f(2), f(3), f(4), f(5)],
                weights,
                board_offset: f(16),
                heading: f(17) * RAD,
                elevation: f(18) * RAD,
                framing: [f(19) * RAD, f(20) * RAD, f(21) * RAD],
                options: r[11],
                anchor: r[12],
                compass: r[13],
                collision: r[14],
                arm_heading: f(17) * RAD,
                arm_elevation: f(18) * RAD,
            };
            let children = [u16_at(16), u16_at(18), u16_at(20)];
            if children.iter().any(|&c| c != NONE && c as usize >= count) {
                return Err(Error::Bad);
            }
            defs.push(ShotDef {
                key: u64::from_be_bytes(key),
                blend: r[8] == 1,
                shot,
                transition_time: f(22),
                transition_units: r[10],
                children,
                points: [f(23), f(24), f(25)],
                blend_value: f(26),
                blend_type: r[9],
                blend_smoothing: f(27),
            });
        }
        let set = ShotSet {
            defs,
            imported: true,
        };
        // No cycles: every blend reaches leaves within a few levels.
        for i in 0..set.defs.len() {
            if set.depth(i, 0) > 8 {
                return Err(Error::Bad);
            }
        }
        Ok(set)
    }

    fn depth(&self, i: usize, d: usize) -> usize {
        if d > 8 {
            return d;
        }
        let def = &self.defs[i];
        if !def.blend {
            return d;
        }
        def.children()
            .map(|c| self.depth(c, d + 1))
            .max()
            .unwrap_or(d)
    }

    /// Writes the entry (the importer, tests).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.defs.len() * RECORD);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(&(self.defs.len() as u32).to_be_bytes());
        for d in &self.defs {
            let s = &d.shot;
            out.extend_from_slice(&d.key.to_be_bytes());
            out.extend_from_slice(&[
                u8::from(d.blend),
                d.blend_type,
                d.transition_units,
                s.options,
                s.anchor,
                s.compass,
                s.collision,
                0,
            ]);
            for c in d.children {
                out.extend_from_slice(&c.to_be_bytes());
            }
            out.extend_from_slice(&[0, 0]);
            let deg = 1.0 / RAD;
            let mut v = alloc::vec![s.distance, s.lens];
            v.extend_from_slice(&s.smoothing);
            v.extend_from_slice(&s.weights);
            v.extend_from_slice(&[
                s.board_offset,
                s.heading * deg,
                s.elevation * deg,
                s.framing[0] * deg,
                s.framing[1] * deg,
                s.framing[2] * deg,
                d.transition_time,
            ]);
            v.extend_from_slice(&d.points);
            v.extend_from_slice(&[
                d.blend_value,
                d.blend_smoothing,
                0.0,
                1.0,
                1.0,
                1.0,
                0.0,
                0.0,
            ]);
            debug_assert_eq!(v.len(), 34);
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        out
    }
}

/// What a blend shot can blend by, this tick (Skate 3's blend types).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlendInputs {
    /// Speed along the board (m/s, 0 or more): type 1.
    pub speed: f32,
    /// Acceleration along the travel: type 2.
    pub accel: f32,
    /// The predicted air's height over the take-off (m): type 4; progress through it: 5.
    pub apex_height: f32,
    pub air_progress: f32,
    /// Slope of the travel (deg, + uphill): type 6; its size: 9.
    pub incline: f32,
    /// The previous shot's distance: type 7.
    pub previous_distance: f32,
    /// Progress to the apex (0..1) and past it (1..2): type 8.
    pub apex_progress: f32,
    /// The landing surface's slope (deg): type 10.
    pub landing_incline: f32,
    /// Steering, + towards the shots' authored left: types 11, 12, 13.
    pub steering: f32,
    /// Frontside angle: type 15.
    pub frontside: f32,
    /// The known landing's flight time (0 without one): type 16.
    pub air_time: f32,
    /// Slope along the travel direction (deg; on rails the rail's): type 20.
    pub direction_incline: f32,
    /// The stance mirror for headings (+1 / -1) and the compass norths (rad).
    pub mirror: f32,
    pub compass: [f32; 9],
}

impl BlendInputs {
    fn raw(&self, kind: u8, authored: f32, dt: f32) -> f32 {
        match kind {
            0 => authored,
            1 => self.speed,
            2 => self.accel,
            4 => self.apex_height,
            5 => self.air_progress,
            6 => self.incline,
            7 => self.previous_distance,
            8 => self.apex_progress,
            9 => self.incline.abs(),
            10 => self.landing_incline,
            11..=13 => {
                if dt == 0.0 {
                    0.0
                } else {
                    self.steering
                }
            }
            15 => self.frontside,
            16 => self.air_time,
            20 => self.direction_incline,
            // Race and skitching preferences flatground doesn't have.
            21 | 22 => 0.0,
            // An effect weight that stays 1.
            23 => 1.0,
            _ => 0.5,
        }
    }
}

/// One shot of the chosen tree, as it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Node {
    def: usize,
    shot: Shot,
    value: f32,
    /// Children nodes (indices into the tree), `usize::MAX` none.
    children: [usize; 3],
}

/// The chosen shot and the one before it, and the transition between them.
#[derive(Clone, Debug, PartialEq)]
pub struct ShotManager {
    nodes: Vec<Node>,
    /// The chosen shot's index in the set (none before the first choice).
    pub current: Option<usize>,
    pub previous: Shot,
    pub interpolated: Shot,
    pub elapsed: f32,
    pub transition_time: f32,
    pub transition_units: u8,
    pub transition_distance: f32,
    /// The last graph shot's way-out transition, used by the next shot without one of its own.
    pub transition_out: f32,
    /// Shot changes cut straight (after a reset).
    pub instant: bool,
}

impl Default for ShotManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ShotManager {
    pub fn new() -> ShotManager {
        ShotManager {
            nodes: Vec::new(),
            current: None,
            previous: Shot::START,
            interpolated: Shot::START,
            elapsed: 0.0,
            transition_time: 0.0,
            transition_units: 0,
            transition_distance: 0.0,
            transition_out: -1.0,
            instant: true,
        }
    }

    /// The chosen tree's root shot this tick (before the transition).
    pub fn current_shot(&self) -> Shot {
        self.nodes.first().map_or(self.interpolated, |n| n.shot)
    }

    /// The chosen tree's first leaf (Skate 3 reads options from it on a reset).
    pub fn first_leaf(&self) -> Shot {
        let mut i = 0;
        loop {
            match self.nodes.get(i) {
                Some(n) if n.children[0] != usize::MAX => i = n.children[0],
                Some(n) => return n.shot,
                None => return self.interpolated,
            }
        }
    }

    /// Chooses shot `def` of `set` (unless it is the current one and not `force`d). The shot being shown
    /// becomes the previous one; a shot that keeps the previous shot takes over its distance, lens,
    /// arm and framing. Returns whether the shot changed.
    pub fn choose(&mut self, set: &ShotSet, def: usize, force: bool, inputs: &BlendInputs) -> bool {
        if !force && self.current == Some(def) {
            return false;
        }
        self.previous = self.interpolated;
        self.nodes.clear();
        self.build(set, def, inputs, 0);
        if let Some(root) = self.nodes.first_mut()
            && root.shot.has(OPT_PREVIOUS)
        {
            let p = &self.interpolated;
            root.shot.distance = p.distance;
            root.shot.lens = p.lens;
            root.shot.heading = p.heading;
            root.shot.elevation = p.elevation;
            root.shot.framing = p.framing;
            root.shot.arm_heading = p.arm_heading;
            root.shot.arm_elevation = p.arm_elevation;
        }
        self.current = Some(def);
        let d = &set.defs[def];
        self.transition_time = d.transition_time;
        self.transition_units = d.transition_units;
        if self.instant {
            self.make_instant();
        } else {
            self.elapsed = 0.0;
        }
        true
    }

    /// The graph's transition times for the shot it just chose: its own way in, else the last shot's
    /// way out.
    pub fn graph_transition(&mut self, transition_in: f32, transition_out: f32) {
        if transition_in >= 0.0 {
            self.transition_time = transition_in;
        } else if self.transition_out >= 0.0 {
            self.transition_time = self.transition_out;
        }
        self.transition_out = if transition_out >= 0.0 {
            transition_out
        } else {
            -1.0
        };
    }

    fn build(&mut self, set: &ShotSet, def: usize, inputs: &BlendInputs, depth: usize) -> usize {
        let d = &set.defs[def];
        let at = self.nodes.len();
        self.nodes.push(Node {
            def,
            shot: d.shot,
            value: d.blend_value,
            children: [usize::MAX; 3],
        });
        if d.blend && depth < 8 && self.nodes.len() < 64 {
            let kids: Vec<usize> = d.children().collect();
            for (k, c) in kids.into_iter().enumerate() {
                let n = self.build(set, c, inputs, depth + 1);
                self.nodes[at].children[k] = n;
            }
            self.nodes[at].value = inputs.raw(d.blend_type, d.blend_value, 0.0);
        }
        at
    }

    /// One tick: the tree's blends and the transition from the previous shot. Returns the transition's
    /// progress (0..1).
    pub fn update(&mut self, set: &ShotSet, dt: f32, inputs: &BlendInputs) -> f32 {
        self.elapsed += dt;
        if !self.nodes.is_empty() {
            self.update_node(set, 0, dt, inputs);
        }
        let duration = self.duration();
        let k = if duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / duration).clamp(0.0, 1.0)
        };
        self.interpolated = Shot::blend(&self.previous, &self.current_shot(), k);
        k
    }

    fn update_node(&mut self, set: &ShotSet, i: usize, dt: f32, inputs: &BlendInputs) {
        let d = &set.defs[self.nodes[i].def];
        if !d.blend {
            let north = inputs.compass[(self.nodes[i].shot.compass as usize).min(8)];
            self.nodes[i].shot.place_arm(north, inputs.mirror);
            return;
        }
        let kids: Vec<usize> = self.nodes[i]
            .children
            .iter()
            .copied()
            .take_while(|&c| c != usize::MAX)
            .collect();
        if kids.is_empty() {
            return;
        }
        let raw = inputs.raw(d.blend_type, d.blend_value, dt);
        let r = 1.0 - d.blend_smoothing;
        let prev = self.nodes[i].value;
        let v = (prev + r * r * (raw - prev))
            .max(d.points[0])
            .min(d.points[kids.len() - 1]);
        self.nodes[i].value = v;
        let (a, b, k) = interval(&d.points[..kids.len()], v);
        let (ka, kb) = (kids[a], kids[b]);
        self.update_node(set, ka, dt, inputs);
        if kb != ka {
            self.update_node(set, kb, dt, inputs);
        }
        let blended = Shot::blend(&self.nodes[ka].shot, &self.nodes[kb].shot, k);
        let n = &mut self.nodes[i].shot;
        let heading = n.heading;
        *n = blended;
        // A blend keeps its own authored heading; its options follow the child (as `blend` does).
        n.heading = heading;
    }

    /// Seconds the transition takes.
    pub fn duration(&self) -> f32 {
        match self.transition_units {
            1 => self.transition_time / 60.0,
            2 => self.transition_distance / self.transition_time.max(1e-4),
            _ => self.transition_time,
        }
    }

    pub fn make_instant(&mut self) {
        self.transition_out = -1.0;
        self.elapsed = self.duration();
    }

    pub fn transitioning(&self) -> bool {
        self.elapsed < self.duration()
    }

    /// The framing's own sine-eased transition progress.
    pub fn framing_progress(&self) -> f32 {
        let d = self.duration();
        if d <= 0.0 {
            1.0
        } else {
            ease((self.elapsed / d).clamp(0.0, 1.0))
        }
    }

    /// The chosen tree's blend values, root first (tests, log).
    pub fn blend_value(&self, depth_first_index: usize) -> Option<f32> {
        self.nodes.get(depth_first_index).map(|n| n.value)
    }
}

/// Which two children `value` falls between and how far (Skate 3: below the first point the first
/// child alone, past the last the last two at 1).
fn interval(points: &[f32], value: f32) -> (usize, usize, f32) {
    let n = points.len();
    for i in 0..n {
        if points[i] > value {
            if i == 0 {
                return (0, 0, 1.0);
            }
            let w = points[i] - points[i - 1];
            let k = if w > 0.0 {
                (value - points[i - 1]) / w
            } else {
                1.0
            };
            return (i - 1, i, ease(k));
        }
    }
    (n.saturating_sub(2), n - 1, ease(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_hash_is_lookup8_of_the_lowercase_name() {
        // Known vector: lookup8 of an empty string is 0, and case doesn't matter.
        assert_eq!(name_hash(""), 0);
        assert_eq!(name_hash("Chase"), name_hash("chase"));
        assert_ne!(name_hash("chase_a"), name_hash("chase_b"));
        // Long names go through the 24-byte blocks.
        assert_ne!(
            name_hash("a_rather_long_camera_shot_name_1"),
            name_hash("a_rather_long_camera_shot_name_2")
        );
    }

    #[test]
    fn the_shot_entry_round_trips() {
        let set = ShotSet::built_in();
        let back = ShotSet::parse(&set.encode()).unwrap();
        assert_eq!(back.defs.len(), set.defs.len());
        for (a, b) in set.defs.iter().zip(&back.defs) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.children, b.children);
            assert!((a.shot.distance - b.shot.distance).abs() < 1e-6);
            assert!((a.shot.heading - b.shot.heading).abs() < 1e-5);
        }
        // A child outside the set is refused.
        let mut bytes = set.encode();
        bytes[12 + 16] = 0x7f;
        assert!(ShotSet::parse(&bytes).is_err());
    }

    #[test]
    fn interval_eases_between_points() {
        assert_eq!(interval(&[10.0, 20.0], 5.0), (0, 0, 1.0));
        let (a, b, k) = interval(&[10.0, 20.0], 15.0);
        assert_eq!((a, b), (0, 1));
        assert!((k - 0.5).abs() < 1e-5);
        let (a, b, k) = interval(&[10.0, 20.0], 25.0);
        assert_eq!((a, b, k), (0, 1, 1.0));
        // Near the ends the sine ease is flat.
        let (_, _, k) = interval(&[10.0, 20.0], 11.0);
        assert!(k < 0.05, "{k}");
    }
}
