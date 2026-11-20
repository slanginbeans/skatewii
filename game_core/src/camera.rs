//! Chase camera after Skate 3's low riding camera: close behind the board and a little to one side
//! (mirrored for the stance), nearly level with it, following the direction of *travel* (so riding fakie
//! or switch doesn't swing it round).
//!
//! What keeps it steady: the travel direction comes from a leash that only moves once the skater is far
//! enough from it (further at low speed), so small wobbles don't turn the camera; when a wall comes
//! between the skater and the lens the camera snaps straight in to where it can see (no spring to bounce
//! back and forth) and only eases back out slowly once the wall has gone. The shot values come from
//! [`crate::tuning::Tuning`] (Skate 3's `camera_shots` when the player's disc supplied them).

use crate::gfx::Camera;
use crate::math::{PI, Vec3, atan2f, cosf, sinf, smooth, v3, wrap_angle, yaw_dir};
use crate::park::Terrain;
use crate::skater::{Mode, Skater};
use crate::tuning::CamShot;

/// Leash length (m) at speed and at a standstill: how far the skater moves before the camera turns.
const LEASH_FAST: f32 = 0.25;
const LEASH_SLOW: f32 = 1.2;
const LEASH_SPEED: f32 = 4.0;
/// Stopped this long, the camera lines up behind the board.
const LINEUP_TIME: f32 = 1.0;
/// How quickly the camera swings round to the travel direction (1/s), on the ground and in the air.
const HEADING_RATE: f32 = 4.0;
const AIR_HEADING_RATE: f32 = 1.5;
/// The shot's distance changes this quickly (m/s) when nothing is in the way.
const DISTANCE_RATE: f32 = 2.5;
/// Seconds to ease back out to full distance after a wall stops blocking the view.
const RECOVER_TIME: f32 = 2.5;
/// Never closer than this (m).
const MIN_DISTANCE: f32 = 0.6;
/// Lens clearance from walls and the ground (m).
const CLEARANCE: f32 = 0.25;

pub struct ChaseCam {
    eye: Vec3,
    target: Vec3,
    /// Yaw the camera looks along (towards the skater).
    yaw: f32,
    /// Smoothed elevation (deg) and look-at height.
    elevation: f32,
    height: f32,
    /// Current boom length, after any wall pull-in.
    distance: f32,
    /// Boom length wanted from the shot.
    shot_distance: f32,
    leash: Vec3,
    stopped: f32,
    /// Which side the camera sits on (+1 / -1), eased when the stance changes.
    side: f32,
    pitch: f32,
}

impl ChaseCam {
    pub fn new(s: &Skater) -> ChaseCam {
        let shot = shot(s);
        let yaw = s.travel_yaw();
        let mut c = ChaseCam {
            eye: s.pos,
            target: s.pos,
            yaw,
            elevation: shot.elevation,
            height: shot.board_offset,
            distance: shot.distance,
            shot_distance: shot.distance,
            leash: s.pos - yaw_dir(yaw) * LEASH_SLOW,
            stopped: 0.0,
            side: side(s),
            pitch: shot.pitch,
        };
        c.place(s, &shot);
        c
    }

    /// Where the camera would be with the current yaw, elevation and distance.
    fn place(&mut self, s: &Skater, shot: &CamShot) {
        self.target = s.pos + v3(0.0, self.height, 0.0);
        let back = self.boom_dir(shot);
        self.eye = self.target + back * self.distance;
    }

    /// Unit vector from the look-at point towards the lens.
    fn boom_dir(&self, shot: &CamShot) -> Vec3 {
        // Heading 180 is straight behind; Skate 3's shots sit a few degrees to one side.
        let off = (shot.heading.abs() - 180.0).abs() * PI / 180.0 * self.side;
        let yaw = self.yaw + PI + off;
        let el = self.elevation * PI / 180.0;
        let h = yaw_dir(yaw);
        v3(h.x * cosf(el), sinf(el), h.z * cosf(el))
    }

    pub fn update(&mut self, s: &Skater, park: &dyn Terrain, dt: f32) {
        let shot = shot(s);
        // Travel direction from the leash.
        let speed = match s.mode {
            Mode::Rolling | Mode::Grind => s.speed.abs(),
            _ => s.vel.horizontal().length(),
        };
        let k = (speed / LEASH_SPEED).min(1.0);
        let leash = LEASH_SLOW + (LEASH_FAST - LEASH_SLOW) * k;
        let d = (s.pos - self.leash).horizontal();
        if d.length() > leash {
            self.leash = s.pos - d.normalize_or(yaw_dir(self.yaw)) * leash;
        }
        let travel = match s.mode {
            Mode::Grind | Mode::Air { .. } => s.travel_yaw(),
            _ if d.length() > 0.05 && speed > 0.3 => atan2f(d.x, d.z),
            _ => self.yaw,
        };
        self.stopped = if speed < 0.5 { self.stopped + dt } else { 0.0 };
        let want_yaw = if self.stopped > LINEUP_TIME && s.mode == Mode::Rolling {
            // Lined up behind the board, facing where the rider would push to.
            s.heading
        } else {
            travel
        };
        let rate = if s.is_airborne() {
            AIR_HEADING_RATE
        } else {
            HEADING_RATE
        };
        self.yaw = wrap_angle(self.yaw + wrap_angle(want_yaw - self.yaw) * smooth(rate, dt));
        self.side += (side(s) - self.side) * smooth(2.0, dt);
        self.elevation += (shot.elevation - self.elevation) * smooth(3.0, dt);
        self.pitch += (shot.pitch - self.pitch) * smooth(3.0, dt);
        // Follow height more loosely in the air so jumps read as jumps.
        let follow = if s.is_airborne() { 4.0 } else { 10.0 };
        let target_h = s.pos.y + shot.board_offset;
        let cur_h = self.target.y;
        self.height = shot.board_offset;
        let y = cur_h + (target_h - cur_h) * smooth(follow, dt);
        self.shot_distance +=
            (shot.distance - self.shot_distance).clamp(-DISTANCE_RATE * dt, DISTANCE_RATE * dt);

        // Walls: the longest boom (up to the shot's) with a clear view of the skater. Pulled in at once,
        // let out slowly.
        let target = v3(s.pos.x, y, s.pos.z);
        let back = self.boom_dir(&shot);
        let clear = self.clear_distance(park, target, back);
        if clear < self.distance {
            self.distance = clear;
        } else {
            let step = self.shot_distance / RECOVER_TIME * dt;
            self.distance = (self.distance + step).min(clear).min(self.shot_distance);
        }
        if self.distance > self.shot_distance {
            self.distance = self.shot_distance;
        }
        let mut eye = target + back * self.distance;
        // Keep the lens out of the ground and ramps.
        let floor = park.ground(eye.x, eye.z, eye.y + 1.0).height + CLEARANCE;
        eye.y = eye.y.max(floor);
        self.eye = eye;
        self.target = target;
    }

    /// The longest boom length (up to the shot's) from `target` along `back` with nothing in the way.
    fn clear_distance(&self, park: &dyn Terrain, target: Vec3, back: Vec3) -> f32 {
        let full = self.shot_distance + CLEARANCE;
        if !park.occluded(target, target + back * full) {
            return self.shot_distance;
        }
        // Step in until clear.
        let mut d = full;
        while d > MIN_DISTANCE + CLEARANCE {
            d -= 0.2;
            if !park.occluded(target, target + back * d) {
                return (d - CLEARANCE).max(MIN_DISTANCE);
            }
        }
        MIN_DISTANCE
    }

    pub fn camera(&self) -> Camera {
        // Framing pitch: look a little above or below the board.
        let look = self.target - self.eye;
        let dist = look.length().max(1e-3);
        let lift = dist * sinf(self.pitch * PI / 180.0);
        Camera {
            eye: self.eye,
            target: self.target + v3(0.0, lift, 0.0),
            fov_y_deg: 58.0,
            near: 0.1,
            far: 120.0,
        }
    }

    /// Boom length right now (after any wall pull-in).
    pub fn distance(&self) -> f32 {
        self.distance
    }
}

/// The shot for what the skater is doing: grinding, or riding (slow to fast by speed).
fn shot(s: &Skater) -> CamShot {
    let t = &s.tuning;
    if s.mode == Mode::Grind {
        return t.cam_grind;
    }
    let speed = match s.mode {
        Mode::Rolling => s.speed.abs(),
        _ => s.vel.horizontal().length(),
    };
    t.cam_slow.lerp(
        &t.cam_fast,
        (speed / t.cam_fast_speed.max(0.1)).clamp(0.0, 1.0),
    )
}

/// Which side of the skater the camera sits: behind the rider's back, mirrored with the stance.
fn side(s: &Skater) -> f32 {
    if s.mirrored() { 1.0 } else { -1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::Park;
    use alloc::vec::Vec;

    fn flat() -> Park {
        Park {
            shapes: Vec::new(),
            half_size: 50.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: Vec::new(),
        }
    }

    #[test]
    fn sits_behind_the_travel_direction_even_fakie() {
        let park = flat();
        let mut s = Skater::spawn(&park);
        s.speed = -4.0; // rolling tail first along -z
        let mut c = ChaseCam::new(&s);
        for _ in 0..240 {
            s.pos.z += s.speed / 60.0;
            c.update(&s, &park, 1.0 / 60.0);
        }
        // Travelling towards -z: the camera is on the +z side of the skater.
        assert!(c.eye.z > s.pos.z + 1.0, "{:?} {:?}", c.eye, s.pos);
        let d = (c.eye - c.target).length();
        assert!((d - s.tuning.cam_fast.lerp(&s.tuning.cam_slow, 0.5).distance).abs() < 0.6);
    }

    /// Flat ground with a wall across z = -1 that blocks the view.
    struct Wall;
    impl Terrain for Wall {
        fn ground(&self, _x: f32, _z: f32, _max: f32) -> crate::park::Surface {
            crate::park::Surface {
                height: 0.0,
                normal: Vec3::UP,
            }
        }
        fn occluded(&self, p: Vec3, q: Vec3) -> bool {
            (p.z > -1.0) != (q.z > -1.0)
        }
        fn spawn(&self) -> (Vec3, f32) {
            (Vec3::ZERO, 0.0)
        }
    }

    #[test]
    fn a_wall_pulls_it_in_at_once_and_it_eases_back_out() {
        let park = Wall;
        let mut s = Skater::spawn(&park);
        s.speed = 0.0;
        let mut c = ChaseCam::new(&s);
        let mut dists = Vec::new();
        for _ in 0..30 {
            c.update(&s, &park, 1.0 / 60.0);
            dists.push(c.distance());
        }
        assert!(dists[0] < 1.2, "{dists:?}");
        // Steady: no bouncing while the wall stays.
        let spread = dists[5..]
            .iter()
            .fold(0.0f32, |m, d| m.max((d - dists[5]).abs()));
        assert!(spread < 0.01, "{dists:?}");
        // Ride away: it lets the boom out gradually, not in one jump.
        let before = c.distance();
        s.pos.z = 6.0;
        c.update(&s, &park, 1.0 / 60.0);
        assert!(c.distance() - before < 0.1);
        for _ in 0..240 {
            c.update(&s, &park, 1.0 / 60.0);
        }
        assert!(c.distance() > 1.5, "{}", c.distance());
    }
}
