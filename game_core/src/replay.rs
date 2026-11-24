//! The replay: the last 30 s, watched back with play and pause, scrubbing, slow motion and a few
//! cameras (Skate 3's replay; SK8-ENGINE/skate-3-rust-engine, GPL-3.0: `crates/skate-game/src/replay.rs`,
//! a 30 s rolling presentation recording).
//!
//! As SK8 does, it records what was drawn, not a second simulation: per displayed frame that ran a tick
//! (one 1/60 s tick at 60 Hz) the rider's frame and the board's (the placeholder's), the gameplay
//! camera and, with the imported skater, his posed skeleton (every bone's character-space transform,
//! packed). Watching it back draws those, so the simulation never runs or rewinds: on leaving, the game
//! goes on from exactly where it stopped. (Re-simulating from recorded input would need snapshots of the
//! state graphs' runtime, the camera and the streamed district, which aren't cheap to copy on the Wii.)
//!
//! Memory: a fixed ring of [`FRAMES`] frames (30 s at 60 Hz, 36 s on PAL50), allocated once: nothing is
//! allocated while recording. A frame is a [`Header`] (168 bytes, on the Wii as on the PC) plus 14 bytes
//! a bone ([`Packed`]: the rotation as a quaternion in 16-bit fixed point, the position in 1/1024 m), so
//! with Skate 3's 36-bone skater 672 bytes a frame, 40 KB a second, 1.2 MB (1 209 600 bytes) for the
//! 30 s ([`Recording::bytes`], [`Recording::budget`]); with the placeholder rider 302 KB. Recording copies
//! the header and packs the bones once a frame (a matrix-to-quaternion each): 2.3 us a frame on a PC,
//! about 3% of the skinning's time.

use alloc::vec;
use alloc::vec::Vec;

use crate::character::Rows;
use crate::gfx::Camera;
use crate::input::{Pad, btn};
use crate::math::{Vec3, Xform, sqrtf};

/// How much is kept (s), and so many ticks.
pub const SECONDS: usize = 30;
pub const FRAMES: usize = SECONDS * 60;

/// What is drawn of the rider in a tick: the rider's frame (the imported character's placement, or the
/// placeholder rider's), the placeholder's board (none for the imported skater, whose board is in his
/// skeleton, and none on foot), and the placeholder's crouch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Figure {
    pub body: Xform,
    pub board: Option<Xform>,
    pub tucked: bool,
}

/// One recorded frame, apart from the bones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    /// The game's tick count when it was recorded (its last tick), and the frame's length (s).
    pub tick: u64,
    pub dt: f32,
    pub figure: Figure,
    pub cam: Camera,
    /// The imported skater's skin was mirrored (a regular rider without the state graphs).
    pub mirror: bool,
}

/// One bone: quaternion x, y, z, w in units of 1/32767 (w kept at or over 0; stored as `-w - 1` when the
/// bone's axes are mirrored, the first row flipped), position in 1/1024 m (about +-32 m).
pub type Packed = [i16; 7];

const Q_SCALE: f32 = 32767.0;
const T_SCALE: f32 = 1024.0;

fn quantise(v: f32, scale: f32) -> i16 {
    let x = v * scale;
    let x = if x >= 0.0 { x + 0.5 } else { x - 0.5 };
    x.clamp(-32768.0, 32767.0) as i16
}

/// A bone's transform packed (see [`Packed`]).
pub fn pack(m: &Rows) -> Packed {
    let mut r = [m[0], m[1], m[2]];
    let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
        - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
        + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
    let mirrored = det < 0.0;
    if mirrored {
        r[0] = [-r[0][0], -r[0][1], -r[0][2]];
    }
    // Rows are the transposed rotation of `character::from_rt`'s quaternion.
    let tr = r[0][0] + r[1][1] + r[2][2];
    let (x, y, z, w);
    if tr > 0.0 {
        let s = sqrtf(tr + 1.0) * 2.0;
        w = 0.25 * s;
        x = (r[1][2] - r[2][1]) / s;
        y = (r[2][0] - r[0][2]) / s;
        z = (r[0][1] - r[1][0]) / s;
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = sqrtf(1.0 + r[0][0] - r[1][1] - r[2][2]) * 2.0;
        w = (r[1][2] - r[2][1]) / s;
        x = 0.25 * s;
        y = (r[1][0] + r[0][1]) / s;
        z = (r[2][0] + r[0][2]) / s;
    } else if r[1][1] > r[2][2] {
        let s = sqrtf(1.0 + r[1][1] - r[0][0] - r[2][2]) * 2.0;
        w = (r[2][0] - r[0][2]) / s;
        x = (r[1][0] + r[0][1]) / s;
        y = 0.25 * s;
        z = (r[2][1] + r[1][2]) / s;
    } else {
        let s = sqrtf(1.0 + r[2][2] - r[0][0] - r[1][1]) * 2.0;
        w = (r[0][1] - r[1][0]) / s;
        x = (r[2][0] + r[0][2]) / s;
        y = (r[2][1] + r[1][2]) / s;
        z = 0.25 * s;
    }
    let len = sqrtf(x * x + y * y + z * z + w * w).max(1e-9);
    let sign = if w < 0.0 { -1.0 } else { 1.0 } / len;
    let qw = quantise(w * sign, Q_SCALE).max(0);
    let t = m[3];
    [
        quantise(x * sign, Q_SCALE),
        quantise(y * sign, Q_SCALE),
        quantise(z * sign, Q_SCALE),
        if mirrored { -qw - 1 } else { qw },
        quantise(t[0], T_SCALE),
        quantise(t[1], T_SCALE),
        quantise(t[2], T_SCALE),
    ]
}

/// A packed bone's quaternion (unnormalised), position and whether its axes are mirrored.
fn unpack_parts(p: &Packed) -> ([f32; 4], [f32; 3], bool) {
    let mirrored = p[3] < 0;
    let w = if mirrored {
        -(p[3] as i32) - 1
    } else {
        p[3] as i32
    };
    (
        [
            p[0] as f32 / Q_SCALE,
            p[1] as f32 / Q_SCALE,
            p[2] as f32 / Q_SCALE,
            w as f32 / Q_SCALE,
        ],
        [
            p[4] as f32 / T_SCALE,
            p[5] as f32 / T_SCALE,
            p[6] as f32 / T_SCALE,
        ],
        mirrored,
    )
}

fn rows(q: [f32; 4], t: [f32; 3], mirrored: bool) -> Rows {
    let len = sqrtf(q.iter().map(|v| v * v).sum::<f32>()).max(1e-9);
    let mut m = crate::character::from_rt(q.map(|v| v / len), t);
    if mirrored {
        m[0] = [-m[0][0], -m[0][1], -m[0][2]];
    }
    m
}

/// A packed bone back as a transform.
pub fn unpack(p: &Packed) -> Rows {
    let (q, t, mirrored) = unpack_parts(p);
    rows(q, t, mirrored)
}

/// Between two packed bones by `k` (0..1): positions blended, rotations along the shorter way.
pub fn unpack_blend(a: &Packed, b: &Packed, k: f32) -> Rows {
    let (qa, ta, ma) = unpack_parts(a);
    let (qb, tb, mb) = unpack_parts(b);
    if ma != mb || k <= 0.0 {
        return rows(qa, ta, ma);
    }
    let d: f32 = (0..4).map(|i| qa[i] * qb[i]).sum();
    let s = if d < 0.0 { -1.0 } else { 1.0 };
    let q = core::array::from_fn(|i| qa[i] + (qb[i] * s - qa[i]) * k);
    let t = core::array::from_fn(|i| ta[i] + (tb[i] - ta[i]) * k);
    rows(q, t, ma)
}

/// The rolling recording: the last [`FRAMES`] frames, oldest first.
#[derive(Clone, Debug, Default)]
pub struct Recording {
    heads: Vec<Header>,
    bones: Vec<Packed>,
    bone_count: usize,
    /// Where the oldest tick is in the ring, and how many are kept.
    start: usize,
    len: usize,
    /// Ticks recorded since the bones were last filled in.
    unposed: usize,
}

impl Recording {
    pub fn new() -> Recording {
        Recording {
            heads: Vec::with_capacity(FRAMES),
            ..Recording::default()
        }
    }

    /// Forgets everything (a new location, another skater).
    pub fn clear(&mut self) {
        self.start = 0;
        self.len = 0;
        self.unposed = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn bone_count(&self) -> usize {
        self.bone_count
    }

    /// One tick (its bones follow with [`Recording::pose`]). Past [`FRAMES`] the oldest is dropped.
    pub fn push(&mut self, h: Header) {
        let slot = if self.len < FRAMES {
            self.len += 1;
            (self.start + self.len - 1) % FRAMES
        } else {
            let s = self.start;
            self.start = (self.start + 1) % FRAMES;
            s
        };
        if slot < self.heads.len() {
            self.heads[slot] = h;
        } else {
            self.heads.push(h);
        }
        self.unposed = (self.unposed + 1).min(self.len);
    }

    /// The skeleton posed for the ticks recorded since the last call (once a frame, after posing).
    /// Another bone count than before starts the recording over.
    pub fn pose(&mut self, world: &[Rows]) {
        if world.len() != self.bone_count {
            self.bone_count = world.len();
            self.bones = vec![[0; 7]; FRAMES * world.len()];
            self.clear();
            return;
        }
        let n = self.bone_count;
        if n == 0 {
            self.unposed = 0;
            return;
        }
        for back in 0..self.unposed {
            let slot = (self.start + self.len - 1 - back) % FRAMES;
            let dst = &mut self.bones[slot * n..(slot + 1) * n];
            if back == 0 {
                for (d, m) in dst.iter_mut().zip(world) {
                    *d = pack(m);
                }
            } else {
                let last = (self.start + self.len - 1) % FRAMES;
                let (a, b) = if slot < last {
                    let (lo, hi) = self.bones.split_at_mut(last * n);
                    (&mut lo[slot * n..(slot + 1) * n], &hi[..n])
                } else {
                    let (lo, hi) = self.bones.split_at_mut(slot * n);
                    (&mut hi[..n], &lo[last * n..(last + 1) * n])
                };
                a.copy_from_slice(b);
            }
        }
        self.unposed = 0;
    }

    fn slot(&self, i: usize) -> usize {
        (self.start + i) % FRAMES
    }

    /// The `i`th tick kept (0: the oldest).
    pub fn header(&self, i: usize) -> Option<&Header> {
        (i < self.len).then(|| &self.heads[self.slot(i)])
    }

    /// The `i`th tick's bones (empty without the imported skater).
    pub fn bones(&self, i: usize) -> &[Packed] {
        let n = self.bone_count;
        if i >= self.len || n == 0 {
            return &[];
        }
        let s = self.slot(i);
        &self.bones[s * n..(s + 1) * n]
    }

    /// The index of the frame recorded at game tick `tick`, if it is still kept.
    pub fn find(&self, tick: u64) -> Option<usize> {
        let (mut lo, mut hi) = (0, self.len);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.heads[self.slot(mid)].tick < tick {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (self.header(lo)?.tick == tick).then_some(lo)
    }

    /// Memory the recording holds (bytes): fixed once the bone count is known.
    pub fn bytes(&self) -> usize {
        self.heads.capacity() * core::mem::size_of::<Header>()
            + self.bones.capacity() * core::mem::size_of::<Packed>()
    }

    /// The most it can hold with `bones` bones (bytes).
    pub fn budget(bones: usize) -> usize {
        FRAMES * (core::mem::size_of::<Header>() + bones * core::mem::size_of::<Packed>())
    }
}

/// The replay's cameras.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    /// The gameplay camera as it was.
    Game,
    /// Standing still where it was put (the gameplay camera's eye when chosen), turning to follow the
    /// rider.
    Tripod(Option<Vec3>),
    /// Round the rider: the C-stick turns it round and up and down.
    Orbit { yaw: f32, pitch: f32 },
}

impl View {
    pub fn name(&self) -> &'static str {
        match self {
            View::Game => "GAME CAMERA",
            View::Tripod(_) => "TRIPOD",
            View::Orbit { .. } => "ORBIT",
        }
    }
}

/// Playback speeds (d-pad up / down): full speed and two slow motions.
pub const SPEEDS: [f32; 3] = [1.0, 0.5, 0.25];
/// Scrubbing at full trigger (s of replay a second).
const SCRUB: f32 = 4.0;
/// The orbit camera's turn rate at full C-stick (rad/s) and its pitch range.
const ORBIT_RATE: f32 = 2.0;
const ORBIT_PITCH: (f32, f32) = (-0.2, 1.2);

/// The replay being watched: where in the recording (ticks from the oldest, fractional), playing or not,
/// how fast, which camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub cursor: f32,
    pub playing: bool,
    pub speed: usize,
    pub view: View,
    prev: u32,
}

impl Player {
    /// Opened on the latest tick of a recording `len` ticks long, paused (as SK8 opens it).
    pub fn open(len: usize, held: u32) -> Player {
        Player {
            cursor: len.saturating_sub(1) as f32,
            playing: false,
            speed: 0,
            view: View::Game,
            prev: held,
        }
    }

    fn end(len: usize) -> f32 {
        len.saturating_sub(1) as f32
    }

    pub fn seek(&mut self, at: f32, len: usize) {
        self.cursor = at.clamp(0.0, Self::end(len));
    }

    /// One displayed frame of `dt` s with the pad (by position, as the riding: A play / pause, L / R
    /// scrub back / forward, d-pad left / right a frame back / on, d-pad up / down faster / slower, Y
    /// the next camera), over a recording `len` frames long whose frames last `frame_dt` s. True: leave
    /// the replay (X, Skate 3's B, or START).
    pub fn control(&mut self, pad: &Pad, buttons: u32, dt: f32, len: usize, frame_dt: f32) -> bool {
        let rate = dt / frame_dt.max(1e-3);
        let pressed = buttons & !self.prev;
        self.prev = buttons;
        if pressed & (btn::X | btn::START) != 0 {
            return true;
        }
        let end = Self::end(len);
        if pressed & btn::A != 0 {
            if self.cursor >= end {
                self.cursor = 0.0;
            }
            self.playing = !self.playing;
        }
        if pressed & btn::Y != 0 {
            self.view = match self.view {
                View::Game => View::Tripod(None),
                View::Tripod(_) => View::Orbit {
                    yaw: 0.0,
                    pitch: 0.35,
                },
                View::Orbit { .. } => View::Game,
            };
        }
        if pressed & btn::DPAD_UP != 0 {
            self.speed = self.speed.saturating_sub(1);
        }
        if pressed & btn::DPAD_DOWN != 0 {
            self.speed = (self.speed + 1).min(SPEEDS.len() - 1);
        }
        if pressed & btn::DPAD_LEFT != 0 {
            self.playing = false;
            let at = libm::ceilf(self.cursor) - 1.0;
            self.seek(at, len);
        }
        if pressed & btn::DPAD_RIGHT != 0 {
            self.playing = false;
            let at = libm::floorf(self.cursor) + 1.0;
            self.seek(at, len);
        }
        let scrub = pad.trig_r - pad.trig_l;
        if pad.trig_l > 0.0 || pad.trig_r > 0.0 {
            self.playing = false;
            self.seek(self.cursor + scrub * SCRUB * rate, len);
        } else if self.playing {
            self.seek(self.cursor + SPEEDS[self.speed] * rate, len);
            if self.cursor >= end {
                self.playing = false;
            }
        }
        if let View::Orbit { yaw, pitch } = &mut self.view {
            *yaw += pad.flick[0] * ORBIT_RATE * dt;
            *pitch = (*pitch - pad.flick[1] * ORBIT_RATE * dt).clamp(ORBIT_PITCH.0, ORBIT_PITCH.1);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Mat3, v3};

    fn header(tick: u64) -> Header {
        Header {
            tick,
            dt: 1.0 / 60.0,
            figure: Figure {
                body: Xform::new(Mat3::IDENTITY, v3(tick as f32, 0.0, 0.0)),
                board: None,
                tucked: false,
            },
            cam: Camera {
                eye: v3(0.0, 2.0, -4.0),
                target: v3(0.0, 1.0, 0.0),
                up: Vec3::UP,
                fov_y_deg: 60.0,
                near: 0.1,
                far: 400.0,
            },
            mirror: false,
        }
    }

    fn bone(r: f32, t: f32) -> Rows {
        let m = Mat3::rot_y(r).mul(&Mat3::rot_x(r * 0.5));
        let c = m.cols;
        [
            [c[0].x, c[0].y, c[0].z],
            [c[1].x, c[1].y, c[1].z],
            [c[2].x, c[2].y, c[2].z],
            [t, -t * 0.5, 1.0],
        ]
    }

    fn close(a: &Rows, b: &Rows, tol: f32) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() < tol)
    }

    #[test]
    fn bones_pack_and_unpack_within_a_millimetre() {
        for i in 0..64 {
            let r = i as f32 * 0.37 - 10.0;
            let m = bone(r, r * 0.3);
            assert!(close(&unpack(&pack(&m)), &m, 2e-3), "{r}");
            // A mirrored bone (one axis flipped) comes back mirrored.
            let mut f = m;
            f[0] = [-f[0][0], -f[0][1], -f[0][2]];
            assert!(close(&unpack(&pack(&f)), &f, 2e-3), "mirrored {r}");
        }
        // Halfway between two poses is between them.
        let (a, b) = (bone(0.2, 0.0), bone(0.4, 1.0));
        let mid = unpack_blend(&pack(&a), &pack(&b), 0.5);
        assert!(close(&mid, &bone(0.3, 0.5), 2e-2));
    }

    #[test]
    fn the_ring_keeps_the_last_30_seconds_and_wraps() {
        let mut r = Recording::new();
        let world = [bone(0.0, 0.0); 3];
        r.pose(&world);
        for tick in 0..(FRAMES as u64 + 700) {
            r.push(header(tick));
            let w = [
                bone(tick as f32 * 0.01, 0.0),
                bone(0.0, 0.0),
                bone(0.0, 1.0),
            ];
            r.pose(&w);
        }
        assert_eq!(r.len(), FRAMES);
        assert_eq!(r.header(0).unwrap().tick, 700);
        assert_eq!(r.header(FRAMES - 1).unwrap().tick, FRAMES as u64 + 699);
        assert_eq!(r.find(699), None);
        let i = r.find(1000).unwrap();
        assert_eq!(r.header(i).unwrap().tick, 1000);
        assert!(close(&unpack(&r.bones(i)[0]), &bone(10.0, 0.0), 2e-3));
        assert!(r.header(FRAMES).is_none());
    }

    #[test]
    fn frames_recorded_before_a_pose_share_it() {
        let mut r = Recording::new();
        r.pose(&[bone(0.0, 0.0)]);
        r.push(header(0));
        r.push(header(1));
        r.pose(&[bone(1.0, 0.0)]);
        assert_eq!(r.bones(0), r.bones(1));
        assert!(close(&unpack(&r.bones(0)[0]), &bone(1.0, 0.0), 2e-3));
    }

    #[test]
    fn the_memory_is_bounded_and_allocated_once() {
        let mut r = Recording::new();
        let world = vec![bone(0.0, 0.0); 36];
        r.pose(&world);
        let before = r.bytes();
        for tick in 0..(FRAMES as u64 * 2) {
            r.push(header(tick));
            r.pose(&world);
        }
        assert_eq!(r.bytes(), before, "no allocation once recording");
        assert!(r.bytes() <= Recording::budget(36));
        assert!(core::mem::size_of::<Header>() <= 168);
        assert_eq!(core::mem::size_of::<Packed>(), 14);
        // Skate 3's 36-bone skater: under 1.5 MB for the 30 s, 50 KB a second.
        assert!(
            Recording::budget(36) < 1_500_000,
            "{}",
            Recording::budget(36)
        );
        assert!(Recording::budget(36) / SECONDS < 50_000);
    }

    fn pad(buttons: u32, prev: u32) -> Pad {
        Pad {
            held: buttons,
            pressed: buttons & !prev,
            ..Pad::default()
        }
    }

    #[test]
    fn the_player_plays_pauses_scrubs_steps_and_slows_down() {
        let len = 600;
        let dt = 1.0 / 60.0;
        let mut p = Player::open(len, 0);
        assert_eq!(p.cursor, 599.0);
        assert!(!p.playing);
        // A at the end: from the start, playing.
        assert!(!p.control(&pad(btn::A, 0), btn::A, dt, len, dt));
        assert!(p.playing);
        assert!((p.cursor - 1.0).abs() < 1e-4);
        p.control(&pad(0, btn::A), 0, dt, len, dt);
        assert!((p.cursor - 2.0).abs() < 1e-4);
        // Half speed.
        p.control(&pad(btn::DPAD_DOWN, 0), btn::DPAD_DOWN, dt, len, dt);
        assert!((p.cursor - 2.5).abs() < 1e-4);
        // Paused: stays.
        p.control(&pad(btn::A, 0), btn::A, dt, len, dt);
        assert!(!p.playing);
        p.control(&pad(0, btn::A), 0, dt, len, dt);
        assert!((p.cursor - 2.5).abs() < 1e-4);
        // A tick on and back.
        p.control(&pad(btn::DPAD_RIGHT, 0), btn::DPAD_RIGHT, dt, len, dt);
        assert_eq!(p.cursor, 3.0);
        p.control(&pad(btn::DPAD_LEFT, 0), btn::DPAD_LEFT, dt, len, dt);
        assert_eq!(p.cursor, 2.0);
        // R fully in: forward at four times.
        let mut r = pad(0, 0);
        r.trig_r = 1.0;
        p.control(&r, 0, dt, len, dt);
        assert_eq!(p.cursor, 6.0);
        // Not past either end.
        let mut l = pad(0, 0);
        l.trig_l = 1.0;
        for _ in 0..100 {
            p.control(&l, 0, dt, len, dt);
        }
        assert_eq!(p.cursor, 0.0);
        // Y: the next camera; X leaves.
        p.control(&pad(btn::Y, 0), btn::Y, dt, len, dt);
        assert_eq!(p.view, View::Tripod(None));
        assert!(p.control(&pad(btn::X, btn::Y), btn::X, dt, len, dt));
    }
}
