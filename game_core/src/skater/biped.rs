//! Walking off the board: Skate 3's biped controller as SK8-ENGINE/skate-3-rust-engine reads it
//! (GPL-3.0; skate-core player/offboard: ground_input.rs, movement_intent.rs, movement_velocity.rs,
//! cadence.rs, air_launch/jump.rs).
//!
//! - The stick, turned into the world by the camera, is read against the rider's facing: its angle away
//!   from straight back (0..1 of pi) gives a move amount (`move_vs_stick_angle`) and a turn
//!   (`turn_vs_stick_angle`), both scaled by how far it is pushed. Pulled straight back the rider turns
//!   round on the spot; pushed ahead he walks without turning.
//! - The move amount is a speed (`speed_vs_input`), blended towards the sprint's while the sprint is
//!   held, for a burst that fades (`sprint_blend` against the time sprinting).
//! - The rider always moves along his facing: the velocity chases facing x target speed by at most
//!   0.2 m/s a tick, slowing by at most 2 m/s^2 while still walking (and as gravity allows uphill); the
//!   turn rate chases `turn_vs_speed` x turn by at most `turn_delta_vs_speed` a tick.
//! - Walk, run or sprint by the speed against the halfway points between the clips' own speeds.
//! - The jump: up at sqrt(2 g height), ahead at the ground speed's forward part x `jump_speed_scalar`.
//!
//! flatground keeps the ground simple: the rider stands on the highest surface under him no more than
//! [`STEP_UP`] above his feet, a wall is anything higher, and a drop of more than [`STEP_DOWN`] is a fall.

use crate::math::{PI, Vec3, clamp, cosf, sinf, sqrtf, v3, wrap_angle};
use crate::park::Terrain;
use crate::tuning::biped::BipedTuning;
use libm::asinf;

/// Skate 3's fixed tick, which the controller's per-tick limits are written for.
const TICK: f32 = 1.0 / 60.0;
const DEG: f32 = PI / 180.0;
/// The most the velocity changes in a tick (m/s), and the most it slows a second while still walking.
const MAX_DV: f32 = 0.2;
const MAX_SLOW: f32 = 2.0;
/// A sprint press keeps counting this long after the button comes up (s).
const SPRINT_GRACE: f32 = 0.3;
/// Highest step walked up, deepest walked down (m).
pub const STEP_UP: f32 = 0.45;
pub const STEP_DOWN: f32 = 0.35;
/// The clips' speeds (m/s: walk, run and sprint cycles' travel over their length), for walk / run /
/// sprint: Skate 3's NB_WALK_FWD_CYC, NB_RUN_FWD_CYC, NB_SPRINT_FWD_CYC.
pub const CLIP_SPEEDS: [f32; 3] = [1.683_4, 5.369_8, 9.101_9];
/// Those cycles' lengths (s): one step cycle travels its clip speed times this.
pub const CYCLE_TIMES: [f32; 3] = [56.0 / 60.0, 38.0 / 60.0, 30.0 / 60.0];
/// Below this (m/s) the rider stands.
const STAND_SPEED: f32 = 0.01;
/// Skate 3's ground profile ahead of the walker (SK8 contact_toolkit generation.rs): a rise or drop over
/// 0.05 m in it is a step (`segment.length > 0.05`), and drops are looked for within 0.75 m.
pub const PROFILE_STEP: f32 = 0.05;
pub const PROFILE_LENGTH: f32 = 0.75;

/// Skate 3's locomotion states (the motion graph's `LocoState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Loco {
    #[default]
    Stand = 0,
    Walk = 1,
    Run = 2,
    Sprint = 3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Biped {
    /// Facing (yaw, rad) and its turn rate (rad/s).
    pub heading: f32,
    pub turn: f32,
    /// Velocity (m/s): along the ground on foot, free in the air.
    pub vel: Vec3,
    pub speed: f32,
    pub air: Option<f32>,
    pub sprint_time: f32,
    sprint_grace: f32,
    pub loco: Loco,
    /// This tick's stick reading: move amount (0..1), turn (-1..1, + right) and the stick in the
    /// world (x, z), as the graphs' `OB_SteerMagnitude`, `OB_Steer`, `OB_BipedWorldX/Z` read them.
    pub mag: f32,
    pub steer: f32,
    pub world_stick: [f32; 2],
    /// The ground's up under the feet.
    pub normal: Vec3,
    /// Jumped (rather than walked off something) this time in the air.
    pub jumped: bool,
    /// In the air: where it left the ground, and the flight's predicted time left (s) and travel
    /// (m, world) to the ground under it.
    pub takeoff: Vec3,
    pub air_remaining: f32,
    pub air_travel: Vec3,
    /// Skate 3's step cadence (SK8 offboard/cadence.rs, `BipedPhase`): where the walk, run or sprint
    /// cycle is (0..1). It moves on by the distance covered over the cycle's own travel, so the feet
    /// keep pace with the ground; the phase-controlled locomotion clips are drawn at it.
    pub phase: f32,
    /// Skate 3's ground kind along the facing (the motion graph's `GroundSlopeType`; SK8
    /// contact_toolkit classification.rs): 0 flat, 1 stairs up shallow, 2 up steep, 3 down shallow, 4
    /// down steep, 5..8 the same on a ramp. A new kind is taken once the last three samples agree.
    pub slope_kind: u8,
    slope_samples: [u8; 3],
    slope_cursor: u8,
}

/// What the controller reads each tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct BipedInput {
    /// The main stick (x right, y forward) and the camera's yaw it is relative to.
    pub stick: [f32; 2],
    pub cam_yaw: f32,
    pub sprint: bool,
    pub jump: bool,
}

/// The slope classes' bounds, compared with the tangent's rise over its run: `tan` of Skate 3's globals
/// 822F8FCC / 822F8FD0 (0.1047 and 0.5585 rad: 6 and 32 degrees).
const SLOPE_LOW_TAN: f32 = 0.105_104_24;
const SLOPE_HIGH_TAN: f32 = 0.624_869_35;

/// The ground ahead along `forward` from the feet at `pos` (on ground of up `normal`): its tangent's rise
/// over run, and whether it steps (stairs). Skate 3 builds a profile of the ground ahead and marks a
/// rise or drop over 0.05 m as a step (SK8 contact_toolkit generation.rs); flatground samples the ground
/// every [`PROFILE_STEP`] up to [`PROFILE_LENGTH`] ahead, and a height change that the surfaces' own
/// slope doesn't account for by over 0.05 m is a step. On stairs the tangent joins the first and last
/// steps seen (SK8 joins the steps' ends: `between_slope`), or the feet and the one step; elsewhere it
/// is the ground's own slope.
pub fn ground_ahead(park: &dyn Terrain, pos: Vec3, forward: Vec3, normal: Vec3) -> (f32, bool) {
    let f = Vec3 { y: 0.0, ..forward }.normalize_or(v3(0.0, 0.0, 1.0));
    let along = |n: Vec3| {
        if n.y > 0.1 {
            -(n.x * f.x + n.z * f.z) / n.y
        } else {
            0.0
        }
    };
    let own = along(normal);
    let (mut h, mut n) = (pos.y, normal);
    let mut first: Option<(f32, f32)> = None;
    let mut last = (0.0, pos.y);
    let steps = libm::roundf(PROFILE_LENGTH / PROFILE_STEP) as usize;
    for i in 1..=steps {
        let d = i as f32 * PROFILE_STEP;
        let p = pos + f * d;
        let g = park.ground(p.x, p.z, h + STEP_UP);
        if g.height <= f32::MIN / 2.0 || g.height > h + STEP_UP || g.height < h - STEP_DOWN {
            break;
        }
        let expected = (along(n) + along(g.normal)) * 0.5 * PROFILE_STEP;
        if (g.height - h - expected).abs() > PROFILE_STEP {
            first.get_or_insert((d, g.height));
            last = (d, g.height);
        }
        h = g.height;
        n = g.normal;
    }
    match first {
        // Two steps or more: the pitch between the first and last step seen.
        Some(a) if last.0 > a.0 => ((last.1 - a.1) / (last.0 - a.0), true),
        Some(_) => ((last.1 - pos.y) / last.0, true),
        None => (own, false),
    }
}

/// The move amount and turn from the stick in the world against the facing (SK8 ground_input.rs).
pub fn read_stick(t: &BipedTuning, heading: f32, world: [f32; 2]) -> (f32, f32) {
    let (fx, fz) = (sinf(heading), cosf(heading));
    // The walker's right (forward x up), as `world_stick`'s.
    let (rx, rz) = (-cosf(heading), sinf(heading));
    let (x, z) = (world[0] * rx + world[1] * rz, world[0] * fx + world[1] * fz);
    let mag = sqrtf(x * x + z * z).min(1.0);
    if mag <= 0.0 {
        return (0.0, 0.0);
    }
    // 0 straight back, +-1 straight ahead (+ to the right).
    let angle = crate::math::atan2f(x, -z) / PI;
    let sign = if angle >= 0.0 { 1.0 } else { -1.0 };
    let a = angle.abs();
    (
        t.move_vs_stick_angle.eval(a) * mag,
        t.turn_vs_stick_angle.eval(a) * mag * sign,
    )
}

/// The stick turned into the world by the camera's yaw (x, z): forward is where the camera looks, right is
/// the screen's right (forward x up: -x when the camera looks along +z, as Y is up and the axes right-handed).
pub fn world_stick(stick: [f32; 2], cam_yaw: f32) -> [f32; 2] {
    let (fx, fz) = (sinf(cam_yaw), cosf(cam_yaw));
    let (rx, rz) = (-cosf(cam_yaw), sinf(cam_yaw));
    [stick[0] * rx + stick[1] * fx, stick[0] * rz + stick[1] * fz]
}

/// Walk, run or sprint for a speed (SK8 cadence.rs: halfway between the clips' speeds).
pub fn loco_for(speed: f32) -> Loco {
    let [walk, run, sprint] = CLIP_SPEEDS;
    if speed <= STAND_SPEED {
        Loco::Stand
    } else if speed <= (walk + run) * 0.5 {
        Loco::Walk
    } else if speed <= (run + sprint) * 0.5 {
        Loco::Run
    } else {
        Loco::Sprint
    }
}

impl Biped {
    /// Steps off the board at `pos` facing `heading`, moving at `vel`.
    pub fn new(heading: f32, vel: Vec3, normal: Vec3) -> Biped {
        let v = Vec3 { y: 0.0, ..vel };
        Biped {
            heading,
            vel: v,
            speed: v.length(),
            normal,
            loco: loco_for(v.length()),
            ..Biped::default()
        }
    }

    pub fn forward(&self) -> Vec3 {
        v3(sinf(self.heading), 0.0, cosf(self.heading))
    }

    /// One tick. Moves `pos` (the feet) over `park`.
    pub fn step(
        &mut self,
        t: &BipedTuning,
        gravity: f32,
        park: &dyn Terrain,
        pos: &mut Vec3,
        input: BipedInput,
        dt: f32,
    ) {
        let ticks = dt / TICK;
        self.world_stick = world_stick(input.stick, input.cam_yaw);
        let (mag, steer) = read_stick(t, self.heading, self.world_stick);
        self.mag = mag;
        self.steer = steer;
        if let Some(time) = self.air {
            self.fly(gravity, park, pos, time + dt, dt);
            return;
        }
        // The speed the stick asks for (SK8 movement_intent.rs).
        self.sprint_grace = if input.sprint {
            SPRINT_GRACE
        } else {
            self.sprint_grace - dt
        };
        let sprinting = input.sprint || self.sprint_grace > 0.0;
        self.sprint_time = clamp(
            self.sprint_time + if sprinting { dt } else { -dt },
            0.0,
            t.sprint_time_cap,
        );
        let normal_speed = t.speed_vs_input.eval(mag);
        let blend = if sprinting {
            t.sprint_blend.eval(self.sprint_time)
        } else {
            0.0
        };
        let desired = normal_speed + (t.sprint_speed.eval(mag) - normal_speed) * blend;

        // Velocity and turning (SK8 movement_velocity.rs).
        let f = self.forward();
        let n = self.normal;
        let dir = (f - n * f.dot(n)).normalize_or(f);
        let slope = asinf(clamp(dir.y, -1.0, 1.0)) / DEG;
        let mut target_speed = t.slope_speed.eval(slope) * desired;
        if desired > 0.5 && target_speed < self.speed {
            let uphill = if self.vel.y > 0.0 && dir.y > 0.0 {
                -gravity * dir.y * dt
            } else {
                0.0
            };
            let slowing = (target_speed - self.speed).max(-MAX_SLOW) * dt;
            target_speed = self.speed + slowing.min(uphill);
        }
        let target = dir * target_speed;
        let change = target - self.vel;
        let most = MAX_DV * ticks;
        let len = change.length();
        self.vel = if len > most {
            self.vel + change * (most / len)
        } else {
            target
        };
        let turn_error = t.turn_vs_speed.eval(self.speed) * steer * DEG - self.turn;
        let turn_most = t.turn_delta_vs_speed.eval(self.speed) * DEG * ticks;
        self.turn += clamp(turn_error, -turn_most, turn_most);
        // Turning right (positive) is towards -x when facing +z: the yaw goes down.
        self.heading = wrap_angle(self.heading - self.turn * dt);

        if input.jump {
            self.jump(t, gravity);
            self.takeoff = *pos;
            self.fly(gravity, park, pos, 0.0, dt);
            return;
        }
        self.walk(gravity, park, pos, dt);
        self.speed = self.vel.length();
        self.loco = loco_for(self.speed);
        self.step_cadence(dt);
    }

    /// The step cadence on the ground (SK8 BipedCadence::update): on by the speed over the cycle's
    /// travel; standing, it settles at the nearer of the cycle's two standing points (0 and 0.5) at 5 a
    /// second.
    fn step_cadence(&mut self, dt: f32) {
        let i = match self.loco {
            Loco::Stand => {
                let target = if (self.phase - 0.5).abs() < 0.25 {
                    0.5
                } else {
                    0.0
                };
                let mut d = target - self.phase;
                if d > 0.5 {
                    d -= 1.0;
                } else if d < -0.5 {
                    d += 1.0;
                }
                let step = 5.0 * dt;
                let next = self.phase + d.clamp(-step, step);
                self.phase = next - libm::floorf(next);
                return;
            }
            Loco::Walk => 0,
            Loco::Run => 1,
            Loco::Sprint => 2,
        };
        let travel = CLIP_SPEEDS[i] * CYCLE_TIMES[i];
        let next = self.phase + self.speed / travel * dt;
        self.phase = next - libm::floorf(next);
    }

    /// Skate 3's jump (SK8 air_launch/jump.rs): up for `jump_height`, ahead with the ground speed's
    /// forward part (nothing backwards) x `jump_speed_scalar`.
    fn jump(&mut self, t: &BipedTuning, gravity: f32) {
        let f = self.forward();
        let h = Vec3 { y: 0.0, ..self.vel };
        let ahead = h.dot(f).max(0.0);
        let side = h - f * h.dot(f);
        let h = (f * ahead + side) * t.jump_speed_scalar;
        self.vel = Vec3 {
            y: sqrtf(2.0 * gravity * t.jump_height.max(0.0)),
            ..h
        };
        self.air = Some(0.0);
        self.jumped = true;
    }

    /// On foot: along the ground, up steps, stopped by walls, off edges into the air.
    fn walk(&mut self, gravity: f32, park: &dyn Terrain, pos: &mut Vec3, dt: f32) {
        let delta = self.vel * dt;
        let here = *pos;
        let ok = |x: f32, z: f32| {
            let g = park.ground(x, z, here.y + STEP_UP);
            g.height > f32::MIN / 2.0
                && g.height <= here.y + STEP_UP
                && !park.blocked(here, Vec3 { x, y: here.y, z })
        };
        let (nx, nz) = (pos.x + delta.x, pos.z + delta.z);
        if ok(nx, nz) {
            pos.x = nx;
            pos.z = nz;
        } else if ok(nx, pos.z) {
            pos.x = nx;
            self.vel.z = 0.0;
        } else if ok(pos.x, nz) {
            pos.z = nz;
            self.vel.x = 0.0;
        } else {
            self.vel = Vec3::ZERO;
        }
        let g = park.ground(pos.x, pos.z, pos.y + STEP_UP);
        if g.height < pos.y - STEP_DOWN || g.height <= f32::MIN / 2.0 {
            // Walked off something: falling, with the walk's speed.
            self.vel.y = 0.0;
            self.air = Some(0.0);
            self.jumped = false;
            self.takeoff = *pos;
            self.predict(gravity, park, *pos);
        } else {
            pos.y = g.height;
            self.normal = g.normal;
            let (slope, stairs) = ground_ahead(park, *pos, self.forward(), self.normal);
            self.classify(slope, stairs);
        }
    }

    /// Skate 3's slope class from the tangent's rise over run along the facing, on stairs or a ramp, and
    /// its three-sample acceptance (SK8 contact_toolkit classification.rs, TU3 82D848D0): flat is taken
    /// at once and clears the history; another kind once the three samples before it were the same.
    pub fn classify(&mut self, slope: f32, stairs: bool) {
        let a = slope.abs();
        let proposed = if a <= SLOPE_LOW_TAN {
            0
        } else if a <= SLOPE_HIGH_TAN {
            if slope > 0.0 { 1 } else { 3 }
        } else if slope > 0.0 {
            2
        } else {
            4
        };
        let proposed = if proposed != 0 && !stairs {
            proposed + 4
        } else {
            proposed
        };
        if proposed == 0 {
            self.slope_kind = 0;
            self.slope_samples = [0; 3];
            self.slope_cursor = 0;
        } else if self.slope_samples.iter().all(|&k| k == proposed) {
            self.slope_kind = proposed;
        } else {
            self.slope_cursor = (self.slope_cursor + 1) % 3;
            self.slope_samples[self.slope_cursor as usize] = proposed;
        }
    }

    /// In the air: gravity until the feet meet the ground.
    fn fly(&mut self, gravity: f32, park: &dyn Terrain, pos: &mut Vec3, time: f32, dt: f32) {
        self.vel.y -= gravity * dt;
        let h = Vec3 { y: 0.0, ..self.vel };
        let here = *pos;
        let to = here + h * dt;
        if park.blocked(here, Vec3 { y: here.y, ..to })
            || park.ground(to.x, to.z, here.y + STEP_UP).height > here.y + STEP_UP
        {
            self.vel.x = 0.0;
            self.vel.z = 0.0;
        } else {
            pos.x = to.x;
            pos.z = to.z;
        }
        pos.y += self.vel.y * dt;
        let g = park.ground(pos.x, pos.z, pos.y + STEP_UP.min(0.1 - self.vel.y * dt));
        if self.vel.y <= 0.0 && g.height > f32::MIN / 2.0 && pos.y <= g.height {
            pos.y = g.height;
            self.normal = g.normal;
            self.vel.y = 0.0;
            self.air = None;
        } else {
            self.air = Some(time);
            self.predict(gravity, park, *pos);
        }
        self.speed = Vec3 { y: 0.0, ..self.vel }.length();
        self.loco = loco_for(self.speed);
    }

    /// In the air: a new flight from here (Skate 3's landing assist onto the board replacing the jump's).
    pub fn set_flight(&mut self, vel: Vec3, gravity: f32, park: &dyn Terrain, pos: Vec3) {
        if self.air.is_some() {
            self.vel = vel;
            self.predict(gravity, park, pos);
        }
    }

    /// The flight's time left and travel to the ground under the feet (a fall straight down to it:
    /// flatground's own estimate of Skate 3's landing prediction).
    fn predict(&mut self, gravity: f32, park: &dyn Terrain, pos: Vec3) {
        let g = park.ground(pos.x, pos.z, pos.y + 0.05);
        let above = if g.height > f32::MIN / 2.0 {
            (pos.y - g.height).max(0.0)
        } else {
            0.0
        };
        self.air_remaining = self.time_to_land(gravity, above).unwrap_or(0.0);
        let end = pos + Vec3 { y: 0.0, ..self.vel } * self.air_remaining;
        self.air_travel = v3(end.x, pos.y - above, end.z) - self.takeoff;
    }

    /// Seconds until the feet meet the ground at this height along a free fall (`None` on foot or
    /// rising past it).
    pub fn time_to_land(&self, gravity: f32, above: f32) -> Option<f32> {
        self.air?;
        // above + vy t - g t^2 / 2 = 0
        let (a, b, c) = (0.5 * gravity, -self.vel.y, -above);
        let d = b * b - 4.0 * a * c;
        if d < 0.0 || a <= 0.0 {
            return None;
        }
        Some((-b + sqrtf(d)) / (2.0 * a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: BipedTuning = BipedTuning::BUILT_IN;

    #[test]
    fn the_step_cadence_covers_a_cycle_per_cycle_of_travel_and_settles_standing() {
        // Running at the run clip's own speed: one cycle per cycle length, the feet keeping pace.
        let mut b = Biped {
            speed: CLIP_SPEEDS[1],
            loco: Loco::Run,
            phase: 0.25,
            ..Biped::default()
        };
        let ticks = libm::roundf(CYCLE_TIMES[1] * 60.0) as usize;
        let mut wrapped = 0;
        for _ in 0..ticks {
            let before = b.phase;
            b.step_cadence(TICK);
            wrapped += usize::from(b.phase < before);
        }
        assert!((b.phase - 0.25).abs() < 0.02 && wrapped == 1, "{}", b.phase);
        // Twice as fast: twice the cycles.
        b.speed *= 2.0;
        for _ in 0..ticks {
            b.step_cadence(TICK);
        }
        assert!((b.phase - 0.25).abs() < 0.03, "{}", b.phase);
        // Standing: to the nearer standing point (0.25 is between, 0.3 goes to 0.5), then holds.
        b.loco = Loco::Stand;
        b.phase = 0.3;
        for _ in 0..60 {
            b.step_cadence(TICK);
        }
        assert!((b.phase - 0.5).abs() < 1e-4, "{}", b.phase);
        b.phase = 0.9;
        for _ in 0..60 {
            b.step_cadence(TICK);
        }
        assert!(b.phase < 1e-4 || b.phase > 1.0 - 1e-4, "{}", b.phase);
    }

    #[test]
    fn the_stick_ahead_moves_without_turning_and_back_turns_hard() {
        // Facing +z with the stick pushed that way: full move, no turn.
        let (mag, steer) = read_stick(&T, 0.0, [0.0, 1.0]);
        assert!(
            (mag - 1.0).abs() < 1e-4 && steer.abs() < 1e-4,
            "{mag} {steer}"
        );
        // Pulled straight back: no move, Skate 3's hardest turn (1.7).
        let (mag, steer) = read_stick(&T, 0.0, [0.0, -1.0]);
        assert!(
            mag.abs() < 1e-4 && (steer - 1.7).abs() < 1e-3,
            "{mag} {steer}"
        );
        // To the right (-x when facing +z): half a turn's angle, turning right (+).
        let (mag, steer) = read_stick(&T, 0.0, [-1.0, 0.0]);
        assert!(mag > 0.0 && steer > 0.0, "{mag} {steer}");
        let (_, left) = read_stick(&T, 0.0, [1.0, 0.0]);
        assert!((left + steer).abs() < 1e-4);
        // Half pushed scales both.
        let (half, _) = read_stick(&T, 0.0, [0.0, 0.5]);
        assert!((half - 0.5).abs() < 1e-4);
    }

    #[test]
    fn the_stick_is_read_from_the_camera() {
        use crate::math::v3;
        for yaw in [0.0, core::f32::consts::FRAC_PI_2, 2.0, -2.5] {
            // Forward is where the camera looks.
            let w = world_stick([0.0, 1.0], yaw);
            assert!(
                (w[0] - sinf(yaw)).abs() < 1e-5 && (w[1] - cosf(yaw)).abs() < 1e-5,
                "{w:?}"
            );
            // Right is the screen's right, as the renderer draws it: the view's forward x up.
            let screen_right = v3(sinf(yaw), 0.0, cosf(yaw)).cross(v3(0.0, 1.0, 0.0));
            let w = world_stick([1.0, 0.0], yaw);
            assert!(
                (w[0] - screen_right.x).abs() < 1e-5 && (w[1] - screen_right.z).abs() < 1e-5,
                "yaw {yaw}: {w:?} vs {screen_right:?}"
            );
        }
        // The camera behind a walker facing +z: stick right is a right turn (positive), towards -x.
        let (_, steer) = read_stick(&T, 0.0, world_stick([1.0, 0.3], 0.0));
        assert!(steer > 0.0, "{steer}");
        let (_, steer) = read_stick(&T, 0.0, world_stick([-1.0, 0.3], 0.0));
        assert!(steer < 0.0, "{steer}");
    }

    /// Stairs up +z from z = 1: `count` steps of `rise` over `tread`, then a landing; or a plain slope of
    /// `rise / tread` (a ramp) when `ramp`.
    struct Stairs {
        rise: f32,
        tread: f32,
        count: f32,
        ramp: bool,
    }

    impl Terrain for Stairs {
        fn ground(&self, _x: f32, z: f32, _max: f32) -> crate::park::Surface {
            let top = self.count * self.tread;
            let along = (z - 1.0).clamp(0.0, top);
            if self.ramp {
                let k = self.rise / self.tread;
                let on = z > 1.0 && z < 1.0 + top;
                return crate::park::Surface {
                    height: along * k,
                    normal: if on {
                        v3(0.0, 1.0, -k).normalize_or(Vec3::UP)
                    } else {
                        Vec3::UP
                    },
                };
            }
            let steps = if z < 1.0 {
                0.0
            } else {
                (libm::floorf(along / self.tread) + 1.0).min(self.count)
            };
            crate::park::Surface {
                height: steps * self.rise,
                normal: Vec3::UP,
            }
        }
        fn spawn(&self) -> (Vec3, f32) {
            (Vec3::ZERO, 0.0)
        }
    }

    /// Walks the stick straight ahead along +z from z = 0 (or back down along -z from the top) and
    /// returns the ground kind held longest off the flat, and the kind at the end.
    fn walk_over(park: &Stairs, down: bool, stick: f32) -> (u8, u8) {
        let top = park.count * park.tread;
        let mut pos = if down {
            v3(0.0, park.count * park.rise, 1.0 + top + 1.0)
        } else {
            Vec3::ZERO
        };
        let heading = if down { PI } else { 0.0 };
        let mut b = Biped::new(heading, Vec3::ZERO, Vec3::UP);
        let input = BipedInput {
            stick: [0.0, stick],
            cam_yaw: heading,
            ..BipedInput::default()
        };
        let mut ticks = [0; 9];
        for _ in 0..600 {
            b.step(&T, 9.81, park, &mut pos, input, TICK);
            ticks[b.slope_kind as usize] += 1;
            if (down && pos.z < -0.5) || (!down && pos.z > top + 2.5) {
                break;
            }
        }
        let most = (1..9).max_by_key(|&k| ticks[k]).unwrap() as u8;
        (most, b.slope_kind)
    }

    #[test]
    fn stairs_and_ramps_are_told_apart_by_their_steps() {
        // Shallow stairs (0.15 m over 0.3 m: 27 degrees), steep stairs (0.2 over 0.25: 39 degrees).
        let shallow = Stairs {
            rise: 0.15,
            tread: 0.3,
            count: 8.0,
            ramp: false,
        };
        let steep = Stairs {
            rise: 0.2,
            tread: 0.25,
            count: 8.0,
            ramp: false,
        };
        assert_eq!(walk_over(&shallow, false, 1.0), (1, 0));
        assert_eq!(walk_over(&steep, false, 0.5), (2, 0));
        assert_eq!(walk_over(&shallow, true, 1.0), (3, 0));
        assert_eq!(walk_over(&steep, true, 0.5), (4, 0));
        // The same slopes as ramps: no steps, the ramp's own kinds.
        let ramp = Stairs {
            ramp: true,
            ..shallow
        };
        assert_eq!(walk_over(&ramp, false, 1.0), (5, 0));
        assert_eq!(walk_over(&ramp, true, 1.0), (7, 0));
        let ramp = Stairs {
            ramp: true,
            ..steep
        };
        assert_eq!(walk_over(&ramp, false, 0.5), (6, 0));
    }

    #[test]
    fn a_new_ground_kind_waits_for_three_agreeing_samples_and_flat_is_at_once() {
        let mut b = Biped::default();
        for expected in [0, 0, 0, 1] {
            b.classify(0.3, true);
            assert_eq!(b.slope_kind, expected);
        }
        // Under 6 degrees is flat at once.
        b.classify(0.1, true);
        assert_eq!(b.slope_kind, 0);
        // Over 32 degrees steep; a ramp without steps.
        for _ in 0..4 {
            b.classify(-0.7, false);
        }
        assert_eq!(b.slope_kind, 8);
    }

    #[test]
    fn walk_run_and_sprint_are_halfway_between_the_cycles() {
        assert_eq!(loco_for(0.0), Loco::Stand);
        assert_eq!(loco_for(2.5), Loco::Walk);
        assert_eq!(loco_for(3.6), Loco::Run);
        assert_eq!(loco_for(6.0), Loco::Run);
        assert_eq!(loco_for(7.3), Loco::Sprint);
    }
}
