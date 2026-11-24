//! The board off the rider's feet (Skate 3's SkateboardController possession, as SK8-ENGINE/
//! skate-3-rust-engine ports it, GPL-3.0: skate-core player/offboard/board_possession lifecycle.rs and
//! motion.rs, skate-game physics/offboard/board_manager): under the feet riding, in the hands on foot,
//! free on the ground (dropped or thrown down: its own body, rolling, settling, stopped by walls), hidden
//! once the rider is far from it, and flying back to the hands when he picks it up.
//!
//! - **Let go** (SK8 `LetGo`): from the hands. A drop keeps the hands' velocity; a throw launches it
//!   `ThrownSkateboardLaunchPitch` (18 degrees) up from the facing at the rider's speed plus
//!   `ThrownSkateboardVelocity` of it (3 m/s standing), and for 8 ticks turns it towards
//!   `ThrownSkateboardPitchTarget` and its heading, rolled level (SK8 `throw_torques`, by their scalars).
//! - **Free** (SK8 state 2, `UpdateFreeBoard`): flatground's own rigid board over the park's ground and
//!   wall queries. In the air it falls; it lands with the released board's restitution (0) and friction
//!   (0.36, `physics_wipeout/SkateboardFriction`); on the ground it rolls along its nose with the riding
//!   board's rolling resistance (no rider input) and the slope, and slides sideways against that friction.
//! - **Hidden** (state 3): further than `MaxDistance` (30 m) from the rider it is out of play until picked
//!   up (Skate 3 parks it `DistanceToHideSkateboard`, 1000 m, away).
//! - **Returning** (state 4, `Retrieve`): from where it is (moved in to `DistanceForBoardReturn`, 30 m,
//!   or `DistanceForMountedReturn`, 5 m, getting on) to the hands over `RetrievalTimeVsDist` of the
//!   distance (getting on, at most `MinTimeToRetrieveBoardWhileMounting`), weighted by
//!   `RetrievalSpeedCurve`; then **held** again.

use crate::math::{Vec3, atan2f, clamp, cosf, sinf, sqrtf, v3};
use crate::park::Terrain;
use crate::tuning::Tuning;

use super::DECK_HEIGHT;

/// Where the board is (SK8 SkateboardController state 448: 0 off, 1 held, 2 free, 3 hidden, 4
/// returning).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Under the rider's feet: riding (the board is the skater's own body).
    Underfoot,
    /// In the hands, on foot.
    Held,
    /// Its own body in the park.
    Free,
    /// Out of play, far from the rider.
    Hidden,
    /// Flying back to the hands.
    Returning,
}

/// The board as its own body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeBoard {
    /// The deck's centre.
    pub pos: Vec3,
    pub vel: Vec3,
    /// Its nose and up (unit, at right angles).
    pub nose: Vec3,
    pub up: Vec3,
    /// On its wheels on the ground (else in the air).
    pub on_ground: bool,
}

impl FreeBoard {
    pub const fn at(pos: Vec3, nose: Vec3) -> FreeBoard {
        FreeBoard {
            pos,
            vel: Vec3::ZERO,
            nose,
            up: Vec3::UP,
            on_ground: false,
        }
    }

    /// The rolling speed along the nose.
    pub fn speed(&self) -> f32 {
        self.vel.dot(self.nose)
    }
}

/// The rider as the possession reads him: where he stands, his facing (unit, level) and velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiderFrame {
    pub pos: Vec3,
    pub forward: Vec3,
    pub vel: Vec3,
}

impl RiderFrame {
    /// The hands' grip on the carried board (flatground's own: at the rider's side, hip high, as the
    /// off-board clips carry it).
    pub fn hand(&self) -> Vec3 {
        let right = self.forward.cross(Vec3::UP).normalize_or(v3(1.0, 0.0, 0.0));
        self.pos + Vec3::UP * HAND_HEIGHT + right * HAND_SIDE
    }
}

/// The carried board's centre over the feet and to the side (m; flatground's own, from the off-board
/// clips' hold).
pub const HAND_HEIGHT: f32 = 0.75;
pub const HAND_SIDE: f32 = 0.3;
/// SK8 `LetGo`: a throw's attitude is turned for this many ticks.
pub const THROW_TICKS: u8 = 8;
/// Landing on the board from the air: the rider's feet within this of the deck's centre (m, seen from
/// above) land on it (SK8 landing_deck/update.rs: the IK offset under 0.5, 0.55 with support).
pub const LAND_REACH: f32 = 0.55;
/// Below this rolling speed (m/s) on flat ground a free board stops.
const REST_SPEED: f32 = 0.02;
/// How far the ground may fall away under a rolling free board before it leaves it (m).
const GROUND_STICK: f32 = 0.06;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Possession {
    pub hold: Hold,
    pub board: FreeBoard,
    /// The retrieval: where it started, seconds into it, how long it takes, its progress and weight
    /// (0..1).
    pub from: Vec3,
    pub elapsed: f32,
    pub duration: f32,
    pub progress: f32,
    pub weight: f32,
    /// Ticks of the throw's attitude turn left (SK8 word 444).
    throw_ticks: u8,
    /// The throw's heading (level, unit).
    throw_heading: Vec3,
    /// The thrown board's pitch (rad, nose up) while in the air.
    pub pitch: f32,
}

impl Default for Possession {
    fn default() -> Self {
        Possession::new()
    }
}

impl Possession {
    pub const fn new() -> Possession {
        Possession {
            hold: Hold::Underfoot,
            board: FreeBoard::at(Vec3::ZERO, v3(0.0, 0.0, 1.0)),
            from: Vec3::ZERO,
            elapsed: 0.0,
            duration: 0.0,
            progress: 0.0,
            weight: 0.0,
            throw_ticks: 0,
            throw_heading: v3(0.0, 0.0, 1.0),
            pitch: 0.0,
        }
    }

    pub fn held(&self) -> bool {
        self.hold == Hold::Held
    }

    /// Skate 3's `free_board` (OffBoard312): free, hidden or returning, not in the hands or under the
    /// feet (`IsDeckFree`).
    pub fn free(&self) -> bool {
        matches!(self.hold, Hold::Free | Hold::Hidden | Hold::Returning)
    }

    /// The board is a body of its own in the park (drawn there): free, or flying back.
    pub fn loose(&self) -> bool {
        matches!(self.hold, Hold::Free | Hold::Returning)
    }

    /// Under the feet again (riding; SK8 `Stop`).
    pub fn under_feet(&mut self) {
        self.hold = Hold::Underfoot;
        self.throw_ticks = 0;
        self.pitch = 0.0;
    }

    /// In the hands (SK8 `Hold`).
    pub fn hold(&mut self) {
        self.hold = Hold::Held;
        self.board.vel = Vec3::ZERO;
        self.throw_ticks = 0;
        self.pitch = 0.0;
    }

    /// Lets go of the board in the hands (SK8 `LetGo`): dropped with the hands' velocity, or thrown.
    pub fn let_go(&mut self, t: &Tuning, rider: RiderFrame, throw: bool) {
        if self.hold != Hold::Held {
            return;
        }
        let p = &t.possession;
        let forward = rider.forward.horizontal().normalize_or(v3(0.0, 0.0, 1.0));
        self.board = FreeBoard {
            pos: rider.hand(),
            vel: rider.vel,
            nose: forward,
            up: Vec3::UP,
            on_ground: false,
        };
        self.pitch = 0.0;
        self.throw_ticks = 0;
        if throw {
            self.board.vel = throw_velocity(t, forward, rider.vel);
            self.throw_ticks = THROW_TICKS;
            self.throw_heading = forward;
        }
        let _ = p;
        self.hold = Hold::Free;
    }

    /// Lets go of the board from under the feet in the air (getting on in the air: the rider throws it
    /// down under him), at the feet with the rider's speed along the ground.
    pub fn release_under(&mut self, rider: RiderFrame) {
        let forward = rider.forward.horizontal().normalize_or(v3(0.0, 0.0, 1.0));
        self.board = FreeBoard {
            pos: rider.pos + Vec3::UP * DECK_HEIGHT,
            vel: rider.vel.horizontal(),
            nose: forward,
            up: Vec3::UP,
            on_ground: false,
        };
        self.pitch = 0.0;
        self.throw_ticks = 0;
        self.hold = Hold::Free;
    }

    /// Lets go of the board in a bail (Skate 3's wipeout frees the board's drives): from under the feet
    /// or the hands, at the deck in `frame` (nose along its z, up its y) with the rider's velocity.
    pub fn release_with(&mut self, rider: RiderFrame, frame: crate::math::Mat3, vel: Vec3) {
        let up = frame.cols[1].normalize_or(Vec3::UP);
        let nose = frame.cols[2].normalize_or(rider.forward);
        let pos = match self.hold {
            Hold::Held => rider.hand(),
            _ => rider.pos + up * DECK_HEIGHT,
        };
        self.board = FreeBoard {
            pos,
            vel,
            nose,
            up,
            on_ground: false,
        };
        self.pitch = 0.0;
        self.throw_ticks = 0;
        self.hold = Hold::Free;
    }

    /// Starts picking the board up (SK8 `Retrieve`): from where it is, moved in to the return distance,
    /// to the hands. `mounting`: getting on (the nearer distance, the time capped).
    pub fn retrieve(&mut self, t: &Tuning, rider: RiderFrame, mounting: bool) {
        if !matches!(self.hold, Hold::Free | Hold::Hidden) {
            return;
        }
        let p = &t.possession;
        let target = rider.hand();
        let mut delta = self.board.pos - target;
        let mut distance = delta.length();
        if distance < 0.001 {
            distance = 0.001;
            delta = rider.forward * 0.001;
        }
        let max = if mounting {
            p.mounted_return_distance
        } else {
            p.return_distance
        };
        let mut from = self.board.pos;
        if distance > max || self.hold == Hold::Hidden {
            let d = distance.min(max);
            from = target + delta * (d / distance);
            distance = d;
        }
        self.from = from;
        self.board.pos = from;
        self.board.vel = Vec3::ZERO;
        self.elapsed = 0.0;
        self.progress = 0.0;
        self.weight = 0.0;
        self.duration = p.retrieval_time.eval(distance);
        if mounting {
            self.duration = self.duration.min(p.mounting_time);
        }
        self.duration = self.duration.max(1e-3);
        self.throw_ticks = 0;
        self.hold = Hold::Returning;
    }

    /// The board's direction from the rider in his frame (rad: 0 ahead, + to his right; SK8 Fill's
    /// `angle_36`, what the retrieve clips aim at).
    pub fn angle(&self, rider: RiderFrame) -> f32 {
        let d = self.board.pos - rider.pos;
        let right = rider.forward.cross(Vec3::UP);
        atan2f(d.dot(right), d.dot(rider.forward))
    }

    /// One tick of the board off the feet.
    pub fn step(&mut self, t: &Tuning, park: &dyn Terrain, rider: RiderFrame, dt: f32) {
        match self.hold {
            Hold::Free => {
                self.step_free(t, park, dt);
                let far = (self.board.pos - rider.pos).length() > t.possession.hide_distance;
                if far {
                    // SK8 UpdateState 2: too far from the rider, hidden.
                    self.board.vel = Vec3::ZERO;
                    self.hold = Hold::Hidden;
                }
            }
            Hold::Returning => {
                self.elapsed += dt;
                self.progress = (self.elapsed / self.duration).min(1.0);
                self.weight = t.possession.retrieval_weight.eval(self.progress);
                let target = rider.hand();
                self.board.pos = self.from.lerp(target, self.weight);
                self.board.nose = self
                    .board
                    .nose
                    .lerp(rider.forward, self.weight)
                    .normalize_or(rider.forward);
                self.board.up = self
                    .board
                    .up
                    .lerp(Vec3::UP, self.weight)
                    .normalize_or(Vec3::UP);
                if self.progress >= 1.0 {
                    self.hold();
                }
            }
            _ => {}
        }
    }

    /// The free board's body: the throw's attitude turn, flight, landing, rolling.
    fn step_free(&mut self, t: &Tuning, park: &dyn Terrain, dt: f32) {
        let p = t.possession;
        let g = t.gravity;
        let b = &mut self.board;
        if self.throw_ticks > 0 {
            // SK8 throw_torques: the angles left to the target pitch (nose up), the throw's heading and
            // level, each turned by its scalar.
            self.throw_ticks -= 1;
            let target = p.throw_target_pitch.to_radians();
            self.pitch += (target - self.pitch) * p.throw_pitch_scalar;
            let heading = self.throw_heading;
            b.nose = b
                .nose
                .lerp(heading, p.throw_yaw_scalar)
                .horizontal()
                .normalize_or(heading);
        }
        if !b.on_ground {
            b.vel.y -= g * dt;
            let next = b.pos + b.vel * dt;
            let (x, z) = slide(park, b.pos, next, &mut b.vel);
            b.pos = v3(x, next.y, z);
            let ground = park.ground(b.pos.x, b.pos.z, b.pos.y + super::STEP);
            if ground.height > f32::MIN / 2.0 && b.pos.y - DECK_HEIGHT <= ground.height {
                // Touching down on its wheels: the speed into the ground goes (restitution), and the
                // friction of the impact takes some of the sideways speed.
                let n = ground.normal.normalize_or(Vec3::UP);
                b.pos.y = ground.height + DECK_HEIGHT;
                let vn = b.vel.dot(n);
                if vn < 0.0 {
                    b.vel -= n * (vn * (1.0 + p.restitution));
                    let nose = tangent(b.nose, n);
                    let side = n.cross(nose);
                    let vs = b.vel.dot(side);
                    let loss = (p.friction * -vn).min(vs.abs());
                    b.vel -= side * (loss * vs.signum());
                }
                b.up = n;
                b.nose = tangent(b.nose, n);
                b.on_ground = true;
                self.pitch = 0.0;
                self.throw_ticks = 0;
            }
            return;
        }
        // Rolling on the ground.
        let ground = park.ground(b.pos.x, b.pos.z, b.pos.y + super::STEP);
        let bottom = b.pos.y - DECK_HEIGHT;
        if ground.height < f32::MIN / 2.0 || bottom - ground.height > GROUND_STICK {
            b.on_ground = false;
            return;
        }
        let n = ground.normal.normalize_or(Vec3::UP);
        b.up = n;
        b.nose = tangent(b.nose, n);
        let side = n.cross(b.nose);
        let mut along = b.vel.dot(b.nose);
        let mut sideways = b.vel.dot(side);
        // Gravity along the surface (as the riding board's, capped the same), the rolling resistance of a
        // board nobody stands on (Skate 3's, with no input), and the sideways slide against the released
        // board's friction.
        let gravity = Vec3::UP * -g + n * (g * n.y);
        let slope = clamp(gravity.dot(b.nose), -t.max_slope_accel, t.max_slope_accel);
        let sp = along.abs();
        let friction =
            (t.friction.eval(sp) + t.friction_no_input.eval(sp)).min(t.corrector_limit());
        along += slope * dt;
        along = approach(along, 0.0, friction * dt);
        sideways += gravity.dot(side) * dt;
        sideways = approach(sideways, 0.0, p.friction * g * n.y.max(0.0) * dt);
        if slope.abs() < friction && along.abs() < REST_SPEED {
            along = 0.0;
        }
        b.vel = b.nose * along + side * sideways;
        let next = b.pos + b.vel * dt;
        let (x, z) = slide(park, b.pos, next, &mut b.vel);
        let top = park.ground(x, z, b.pos.y + super::STEP);
        b.pos.x = x;
        b.pos.z = z;
        if top.height > f32::MIN / 2.0 && b.pos.y - DECK_HEIGHT - top.height <= GROUND_STICK {
            b.pos.y = top.height + DECK_HEIGHT;
        } else {
            b.on_ground = false;
        }
    }

    /// Skate 3's landing assist onto the free board from the air (SK8 landing_deck `consider_board` and
    /// `assist`): the flight (rider `pos`, `vel`, under `gravity`) that brings the feet down onto the
    /// deck, and how long it takes, if the board is upright and the time is found. `None`: no landing on
    /// it from here.
    pub fn landing_flight(&self, t: &Tuning, pos: Vec3, vel: Vec3) -> Option<(Vec3, f32)> {
        if self.hold != Hold::Free || self.board.up.y <= t.possession.deck_min_up {
            return None;
        }
        let g = t.gravity;
        let b = &self.board;
        let mut vb = b.vel;
        let airborne = !b.on_ground;
        if airborne {
            vb.y = 0.0;
        }
        // The rider against the board: SK8 halves gravity against a board in the air (both fall).
        let a = if airborne { -0.5 * g } else { -g };
        let n = b.up;
        let target = b.pos;
        let rel = vel - vb;
        // n . (p + v t + a/2 t^2 up) = n . target: the later root.
        let qa = 0.5 * a * n.y;
        let qb = n.dot(rel);
        let qc = n.dot(pos) - n.dot(target);
        let disc = qb * qb - 4.0 * qa * qc;
        if disc < 0.0 || qa == 0.0 {
            return None;
        }
        let root = sqrtf(disc);
        let time = ((-qb + root) / (2.0 * qa)).max((-qb - root) / (2.0 * qa));
        if time <= 0.0 {
            return None;
        }
        let distance = (pos - target).length();
        let optimal = sqrtf(2.0 * distance / 9.8);
        let proposed = (time * 1.3).min((time * 0.75).max(optimal));
        let flight = (target - pos) * (1.0 / proposed) + Vec3::UP * (0.5 * g * proposed) + b.vel;
        Some((flight, proposed))
    }

    /// The feet are over the deck (seen from above) to land on it.
    pub fn under(&self, pos: Vec3) -> bool {
        self.hold == Hold::Free && (self.board.pos - pos).horizontal().length() < LAND_REACH
    }
}

/// SK8 `throw_velocity`: the facing pitched up by `ThrownSkateboardLaunchPitch`, at the rider's speed
/// plus `ThrownSkateboardVelocity` of it.
pub fn throw_velocity(t: &Tuning, forward: Vec3, rider_vel: Vec3) -> Vec3 {
    let p = &t.possession;
    let pitch = p.throw_pitch.to_radians();
    let f = forward.horizontal().normalize_or(v3(0.0, 0.0, 1.0));
    let dir = f * cosf(pitch) + Vec3::UP * sinf(pitch);
    let speed = rider_vel.length();
    dir * (speed + p.throw_velocity.eval(speed))
}

/// Skate 3's landing assist's limit on the change of the flight (m/s) against the time in the air (SK8
/// biped_air/orientation.rs `landing_assist_limit`): 3.5 for the jump's first 0.034 s, down to 0.5 by
/// 0.194 s.
pub fn landing_assist_limit(time: f32) -> f32 {
    let t = (time - 0.034).max(0.0);
    let blend = clamp(1.0 - t * 6.25, 0.0, 1.0);
    (1.0 - blend) * 0.5 + blend * 3.5
}

/// Moves from `a` towards `b` along the ground as far as the walls let it: a blocked axis stops (the
/// released board's restitution is 0).
fn slide(park: &dyn Terrain, a: Vec3, b: Vec3, vel: &mut Vec3) -> (f32, f32) {
    let ok = |x: f32, z: f32| {
        park.ground(x, z, a.y + super::STEP).height <= a.y - DECK_HEIGHT + super::STEP
            && !park.blocked(a, v3(x, a.y, z))
    };
    if ok(b.x, b.z) {
        (b.x, b.z)
    } else if ok(b.x, a.z) {
        vel.z = 0.0;
        (b.x, a.z)
    } else if ok(a.x, b.z) {
        vel.x = 0.0;
        (a.x, b.z)
    } else {
        vel.x = 0.0;
        vel.z = 0.0;
        (a.x, a.z)
    }
}

fn tangent(v: Vec3, n: Vec3) -> Vec3 {
    (v - n * v.dot(n)).normalize_or(v3(0.0, 0.0, 1.0))
}

fn approach(v: f32, target: f32, by: f32) -> f32 {
    if v > target {
        (v - by).max(target)
    } else {
        (v + by).min(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::{Axis, Park, Shape};
    use alloc::vec;
    use alloc::vec::Vec;

    const DT: f32 = 1.0 / 60.0;

    fn park(shapes: Vec<Shape>) -> Park {
        Park {
            shapes,
            half_size: 50.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: Vec::new(),
        }
    }

    fn flat() -> Park {
        park(Vec::new())
    }

    fn rider(pos: Vec3, vel: Vec3) -> RiderFrame {
        RiderFrame {
            pos,
            forward: v3(0.0, 0.0, 1.0),
            vel,
        }
    }

    fn held() -> Possession {
        let mut p = Possession::new();
        p.hold();
        p
    }

    #[test]
    fn a_dropped_board_falls_to_its_wheels_and_rests() {
        let (t, park) = (Tuning::BUILT_IN, flat());
        let r = rider(Vec3::ZERO, Vec3::ZERO);
        let mut p = held();
        p.let_go(&t, r, false);
        assert_eq!(p.hold, Hold::Free);
        assert!(p.free() && p.loose() && !p.held());
        assert!((p.board.pos.y - HAND_HEIGHT).abs() < 1e-5);
        for _ in 0..60 {
            p.step(&t, &park, r, DT);
        }
        assert!(p.board.on_ground, "lands");
        assert!(
            (p.board.pos.y - DECK_HEIGHT).abs() < 1e-4,
            "{:?}",
            p.board.pos
        );
        assert!(p.board.vel.length() < 1e-4, "stays where it fell");
    }

    #[test]
    fn a_throw_launches_at_skate_3s_pitch_and_speed_and_rolls_on() {
        let (t, park) = (Tuning::BUILT_IN, flat());
        // Standing: 3 m/s (`ThrownSkateboardVelocity` at 0) at 18 degrees up.
        let v = throw_velocity(&t, v3(0.0, 0.0, 1.0), Vec3::ZERO);
        assert!((v.length() - 3.0).abs() < 1e-4, "{v:?}");
        assert!((libm::atan2f(v.y, v.z).to_degrees() - 18.0).abs() < 1e-3);
        // Running at 6 m/s: plus 0.92.
        let v = throw_velocity(&t, v3(0.0, 0.0, 1.0), v3(0.0, 0.0, 6.0));
        assert!((v.length() - 6.0 - t.possession.throw_velocity.eval(6.0)).abs() < 1e-4);
        let r = rider(Vec3::ZERO, Vec3::ZERO);
        let mut p = held();
        p.let_go(&t, r, true);
        let mut landed_at = None;
        for i in 0..120 {
            p.step(&t, &park, r, DT);
            if p.board.on_ground && landed_at.is_none() {
                landed_at = Some((i, p.board.pos.z));
            }
        }
        let (_, z) = landed_at.expect("comes down");
        assert!(z > 0.5, "ahead: {z}");
        assert!(
            p.board.on_ground && p.board.speed() > 1.5,
            "rolls on: {:?}",
            p.board.vel
        );
        assert!(
            p.board.pos.z > z + 2.0,
            "rolled from {z} to {}",
            p.board.pos.z
        );
        assert!(p.board.vel.x.abs() < 1e-4);
    }

    #[test]
    fn a_free_board_rolls_down_a_slope() {
        let t = Tuning::BUILT_IN;
        // A 10 degree bank rising along +z (the board starts facing up it and rolls back down).
        let rise = libm::tanf(10f32.to_radians()) * 20.0;
        let park = park(vec![Shape::Ramp {
            min: v3(-5.0, 0.0, -10.0),
            max: v3(5.0, rise, 10.0),
            axis: Axis::Z,
            h0: 0.0,
            h1: rise,
            rgba: 0,
        }]);
        let mut p = Possession::new();
        p.hold = Hold::Free;
        let y = park.ground(0.0, 0.0, 10.0).height;
        p.board = FreeBoard::at(v3(0.0, y + DECK_HEIGHT, 0.0), v3(0.0, 0.0, 1.0));
        p.board.on_ground = true;
        let r = rider(v3(0.0, y, -2.0), Vec3::ZERO);
        for _ in 0..60 {
            p.step(&t, &park, r, DT);
        }
        assert!(p.board.on_ground);
        assert!(p.board.vel.length() > 1.0, "{:?}", p.board.vel);
    }

    #[test]
    fn far_from_the_rider_it_hides_and_is_picked_up_from_the_return_distance() {
        let (t, park) = (Tuning::BUILT_IN, flat());
        let mut p = Possession::new();
        p.hold = Hold::Free;
        p.board = FreeBoard::at(v3(0.0, DECK_HEIGHT, 0.0), v3(0.0, 0.0, 1.0));
        p.board.on_ground = true;
        let r = rider(v3(0.0, 0.0, 40.0), Vec3::ZERO);
        p.step(&t, &park, r, DT);
        assert_eq!(p.hold, Hold::Hidden, "more than 30 m away");
        assert!(p.free() && !p.loose());
        p.retrieve(&t, r, false);
        assert_eq!(p.hold, Hold::Returning);
        let from = (p.from - r.hand()).length();
        assert!((from - 30.0).abs() < 1e-3, "starts 30 m out: {from}");
        assert!((p.duration - t.possession.retrieval_time.eval(30.0)).abs() < 1e-5);
        let mut ticks = 0;
        while p.hold == Hold::Returning && ticks < 200 {
            p.step(&t, &park, r, DT);
            ticks += 1;
        }
        assert_eq!(p.hold, Hold::Held, "back in the hands");
        assert!((ticks as f32 * DT - p.duration).abs() < 2.0 * DT);
    }

    #[test]
    fn getting_on_picks_it_up_quickly_from_five_metres_at_most() {
        let (t, park) = (Tuning::BUILT_IN, flat());
        let mut p = Possession::new();
        p.hold = Hold::Free;
        p.board = FreeBoard::at(v3(0.0, DECK_HEIGHT, 9.0), v3(0.0, 0.0, 1.0));
        p.board.on_ground = true;
        let r = rider(Vec3::ZERO, Vec3::ZERO);
        p.retrieve(&t, r, true);
        assert!(((p.from - r.hand()).length() - 5.0).abs() < 1e-3);
        assert!((p.duration - 0.1).abs() < 1e-6, "{}", p.duration);
        for _ in 0..7 {
            p.step(&t, &park, r, DT);
        }
        assert!(p.held());
    }

    #[test]
    fn the_landing_flight_comes_down_on_the_deck() {
        let (t, park) = (Tuning::BUILT_IN, flat());
        let mut p = Possession::new();
        p.hold = Hold::Free;
        p.board = FreeBoard::at(v3(0.0, DECK_HEIGHT, 2.0), v3(0.0, 0.0, 1.0));
        p.board.vel = v3(0.0, 0.0, 4.0);
        p.board.on_ground = true;
        let (vel, time) = p
            .landing_flight(&t, Vec3::ZERO, v3(0.0, 4.0, 4.0))
            .expect("a flight");
        assert!(time > 0.4, "{time}");
        // Follow it: the feet end on the deck.
        let (mut pos, mut v) = (Vec3::ZERO, vel);
        let r = rider(Vec3::ZERO, Vec3::ZERO);
        let mut s = 0.0;
        while s < time {
            v.y -= t.gravity * DT;
            pos += v * DT;
            p.step(&t, &park, r, DT);
            s += DT;
        }
        assert!(p.under(pos), "{pos:?} vs {:?}", p.board.pos);
        // An upside-down board can't be landed on.
        p.board.up = -Vec3::UP;
        assert!(
            p.landing_flight(&t, Vec3::ZERO, v3(0.0, 4.0, 4.0))
                .is_none()
        );
    }

    #[test]
    fn the_assist_limit_narrows_after_take_off() {
        assert_eq!(landing_assist_limit(0.0), 3.5);
        assert!((landing_assist_limit(0.114) - 2.0).abs() < 1e-4);
        assert_eq!(landing_assist_limit(0.5), 0.5);
    }

    #[test]
    fn walls_stop_a_free_board() {
        let t = Tuning::BUILT_IN;
        let park = park(vec![Shape::Block {
            min: v3(-5.0, 0.0, 3.0),
            max: v3(5.0, 1.5, 4.0),
            rgba: 0,
        }]);
        let mut p = Possession::new();
        p.hold = Hold::Free;
        p.board = FreeBoard::at(v3(0.0, DECK_HEIGHT, 0.0), v3(0.0, 0.0, 1.0));
        p.board.vel = v3(0.0, 0.0, 4.0);
        p.board.on_ground = true;
        let r = rider(Vec3::ZERO, Vec3::ZERO);
        for _ in 0..120 {
            p.step(&t, &park, r, DT);
        }
        assert!(p.board.pos.z < 3.0, "{:?}", p.board.pos);
        assert!(p.board.vel.length() < 1e-3);
    }
}
