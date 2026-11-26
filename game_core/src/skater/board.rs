//! The board on the ground: one rigid board on two trucks, with a full velocity.
//!
//! The velocity is kept in the surface's frame while the wheels are down: `speed` along the nose,
//! `slip` sideways (+ towards the board's right) and `vn` away from the surface (non-zero only while the
//! wheels have lifted a little without the board being airborne yet). Leaving the ground hands the same
//! velocity over to the air as a world vector.
//!
//! What a tick does, in Skate 3's order of ideas (SK8-ENGINE/skate-3-rust-engine, GPL-3.0, `riding/`):
//! - **Steering** (`steering.rs`, `truck_frames.rs`): the deck tilt eases towards the stick's tilt, each
//!   truck turns about its kingpin (31 degrees) and the board follows the trucks' curve, curvature
//!   `2 tan(sin 31 deg * tilt) / wheelbase`; the yaw rate is speed times that, so it reverses rolling
//!   backwards and vanishes when stopped. Grip limits it.
//! - **Heading** (`heading.rs`): on a slope the board swings towards the fall line (strong when slow, gone
//!   at speed); on steep faces the stick adds its own turn, which overrides the swing.
//! - **Side friction and straightening** (`slide_friction.rs`, `straighten.rs`): sideways speed is
//!   damped (no more than the wheels' grip allows) and a torque turns the board back in line with its
//!   travel unless the stick is turning hard.
//! - **Speed model** (`speed_model.rs`): gravity along the slope (capped), rolling resistance (uphill the
//!   larger of the two, not their sum), all off while a foot is pushing or braking.
//! - **Feet** (`push.rs`, `braking.rs`): a pushing foot chases its target speed by a per-tick limit; a
//!   braking foot slows the board.
//! - **Pumping** (`pumping/controller.rs`): standing up while the ground under the wheels curves upwards
//!   (a concave transition) adds speed, more the tighter the curve and the faster the extension.
//! - **Contact** (`selector/conditions.rs`): the board only follows the ground while the ground holds it
//!   up. Where the surface falls away faster than the board would fall (a crest, a lip, a roller, a
//!   bank-to-flat edge) it flies on its own path and is airborne once the gap passes Skate 3's natural
//!   air distances. Walls take away only the speed into them and turn the board along them
//!   (`collision_response.rs`).
//! - **Wallrides** (`ground_contact_response.rs`): a steep face with a real floor under it; the board
//!   falls along it in an arc and jumps off when low, slow or after a while.
//!
//! Rolling, carving, pushing, braking, pumping and manuals now ride Skate 3's rigid board
//! (`skater::rig`, `physics::board`): `ride` hands those ticks to `ride_rigid`. What follows here is the
//! older kinematic model, still used for kick turns, reverts and powerslides: masses, inertia and wheel
//! forces are not simulated there, and where Skate 3's value is a force, a flatground calibration turns
//! it into an acceleration (marked "ours").

use super::*;

/// Distance from the board's middle to each truck (m) and the wheelbase.
pub const AXLE: f32 = 0.243;
/// Skate 3's board (SK8 physics/board.rs `RETAIL_WHEEL_RADIUS`, `RETAIL_TRUCK_Y_POSITION`): the wheels'
/// radius and how far the trucks' axles are under the deck's centre (m).
pub const WHEEL_RADIUS: f32 = 0.031;
pub const TRUCK_DROP: f32 = 0.0565;
/// The deck's centre over the board's contact point (`pos`) with the wheels on flat ground: SK8
/// drive_frames.rs `RETAIL_LEVEL_GROUND_DECK_CENTER_HEIGHT`, the wheel radius plus the truck drop. The
/// clips' `SKATEBOARD_ROOT` is the deck's centre (Skate 3 puts the animation's board on the physical
/// deck, SK8 skeleton_root.rs), so the imported rider is drawn this far over `pos`.
pub const DECK_HEIGHT: f32 = WHEEL_RADIUS + TRUCK_DROP;
/// The trucks' hangers' underside under the deck's centre (m, ours: the axle plus half the hanger): what
/// rests on the rail in a 50-50 or a 5-0.
pub const HANGER_DROP: f32 = 0.064;
/// The deck's underside under its centre (m, ours: half a 10 mm deck): what rests on the rail in a slide.
pub const DECK_UNDERSIDE: f32 = 0.005;
const WHEELBASE: f32 = 2.0 * AXLE;
/// The share of the trucks' geometric curve the board follows, the most it turns (rad/s, at a standstill
/// and per m/s of speed) and how long its yaw rate takes to build (s). Ours, measured against SK8's
/// simulation of the same deck tilt: half a stick at 6.65 m/s turns 0.42 rad/s, a full one 1.75 rad/s at
/// 6.5 m/s and 1.83 at 8.3; the turn builds in about 0.1 s at full stick and 0.35 s at half.
const TRUCK_FOLLOW: f32 = 0.88;
const TRUCK_YAW_MAX: f32 = 1.5;
const TRUCK_YAW_MAX_PER_SPEED: f32 = 0.04;
const TRUCK_YAW_LAG: f32 = 0.05;
/// sin(31 deg): the trucks' kingpin angle turns a deck tilt into a truck turn.
const KINGPIN: f32 = 0.515_038;
/// The trucks look for the surface from this far above the board, and this far below it.
const PROBE_UP: f32 = 0.35;
const PROBE_DOWN: f32 = 0.25;
/// Walls are looked for at about wheel height: anything higher is hit, not climbed.
const WALL_RAY: f32 = 0.04;
/// A truck climbs at most this much more than it did last tick (m): a curb is hit, not rolled up.
const CLIMB: f32 = 0.04;
/// A surface turned further than this from the one under the board (cosine) is a wall.
const WALL_COS: f32 = 0.64;
/// After landing each truck's turn comes back in over this long (s).
const TRUCK_RAMP: f32 = 0.165;
/// Wheel grip sideways (share of the load) and how quickly side friction damps sideways speed per unit
/// of `side_friction` (1/s). Ours: SK8's board holds a full carve at 8.3 m/s (about 1.55 g) with 8
/// degrees of slip.
const SIDE_GRIP: f32 = 1.6;
/// The share of a push tick's speed change the board keeps: SK8's push force (the gap to the target, at
/// most `MaxPushDVStart`..`End`) moves the whole rigid board and rider, and the deck gains 0.955 to 0.97
/// of it (measured, 0 to 7 m/s).
pub const PUSH_TRANSFER: f32 = 0.96;
const SIDE_DAMP: f32 = 0.4;
/// Skate 3's manual corrective drag (N per m/s: SK8 manual/controller.rs, -400 x the ground velocity at
/// the dragging truck) and the mass it moves (kg, ours: SK8's deck loses 54% of 5.43 m/s to it in one
/// tick, its rider then running out).
const MANUAL_CORRECTIVE_DRAG: f32 = 400.0;
const MANUAL_CORRECTIVE_MASS: f32 = 12.33;
/// The trucks may turn the board a little faster than grip lets the travel turn (a drift), up to a
/// sideways angle of `MAX_SLIP` (rad). Ours.
const GRIP_OVER: f32 = 1.1;
const MAX_SLIP: f32 = 0.35;
/// Powerslide: wheel friction sliding sideways (share of g at 90 degrees). Ours: SK8's slide slows the
/// board about 5.5 m/s^2 through its middle.
const SLIDE_GRIP: f32 = 0.6;
/// Powerslide: how hard the sliding wheels pull the travel round (m/s^2) and how long that takes to
/// build (s). Ours, measured against SK8's simulation: a slide from 6.65 m/s turns the travel about 150
/// degrees in its second, at about 1 g sideways, after a slow first 0.2 s.
const SLIDE_SIDE_ACCEL: f32 = 9.8;
const SLIDE_SIDE_BUILD: f32 = 0.25;
/// Full crouch lowers the rider's centre of mass this much (m, ours): the pumping extension.
const CROUCH_DEPTH: f32 = 0.25;
/// Wallrides: how far down the floor is looked for, and the least height above it (m).
const FLOOR_PROBE: f32 = 4.0;
pub const WALLRIDE_MIN_HEIGHT: f32 = 0.5;
const WALLRIDE_POINTS: u32 = 250;
/// A wall hit this soon after the last one is not a single hit (s).
const REHIT_TIME: f32 = 1.0;
/// Closing on a wall faster than the bail limit but slower than this times it, upright and on a single
/// hit: the rider steps off (a runout), not a full bail. Ours.
const RUNOUT_RATIO: f32 = 1.6;

/// What the rider does to the board this tick: what the ground physics takes from the state logic. A
/// state graph driven by Skate 3's clips (their foot-contact events) can produce the same inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drive {
    /// Smoothed turn input (-1..1, + right) and the hard-turn amount, as Skate 3's steering reads them.
    pub turn: f32,
    pub hard: f32,
    /// The stick's raw turn (for the steep-face turn torque).
    pub raw_turn: f32,
    /// The pushing foot is on the ground: chase this speed (m/s along the nose).
    pub push_target: Option<f32>,
    /// A push is under way (its clips' push contact events, the foot down or coming back): Skate 3's
    /// steering weakens to `PushScalarMin` meanwhile (SK8 steering.rs reads processed bit 27).
    pub pushing: bool,
    /// A braking foot or tail is on the ground: its force (N).
    pub brake: f32,
    /// On two wheels.
    pub manual: bool,
    /// A manual's Normal corrective drag acts this tick (Skate 3's `correction_active`): a drag force
    /// against the velocity at the dragging truck replaces the brake, push and ground forces.
    pub manual_correction: bool,
    /// Powerslide: the board's angle from travel (rad); the path keeps following the trucks.
    pub slide: Option<f32>,
    /// Seconds into the powerslide.
    pub slide_time: f32,
    /// A move turns the board itself (kick turn, revert): no truck or heading turn.
    pub pivot: bool,
    /// The wheels don't scrub sideways speed (a revert: Skate 3 drops the board's drag while it turns).
    pub keep_velocity: bool,
    /// The rider is pumping on purpose (crouching and standing from the stick).
    pub intentional_pump: bool,
}

/// Pumping state (Skate 3's controller): smoothed extension, time extending, the ground's smoothed turn
/// rate along travel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pump {
    pub effect: f32,
    pub time: f32,
    pub turn_rate: f32,
    /// The speed the pump added this tick (m/s; Skate 3's pumping acceleration the pump layers read).
    pub dv: f32,
    /// What the pumping tells the animation's crouch (SK8 PhysOutAnimation's crouching feedback):
    /// the pumping (turn rate times `PumpVsVel`), the board's absorption (-speed x turn rate), the
    /// ground's and the deck's compression and the least crouch for the ground's angle.
    pub pumping: f32,
    pub absorption: f32,
    pub ground_absorption: f32,
    pub min_crouch: f32,
    pub deck_absorption: f32,
    pub(super) last_height: f32,
    pub(super) prev_n: Option<Vec3>,
}

impl Skater {
    /// The velocity along the surface.
    pub fn ground_velocity(&self) -> Vec3 {
        let n = self.contact_n;
        let f = tangent(self.fwd, n);
        f * self.speed + n.cross(f) * self.slip
    }

    /// How hard the wheels can push sideways (m/s^2): grip times the load (gravity into the surface plus
    /// the curve of a transition pressing the board into it).
    fn grip(&self) -> f32 {
        let load = self.tuning.gravity * self.contact_n.y
            + self.pump.turn_rate.max(0.0) * self.speed.abs();
        SIDE_GRIP * load.max(0.0)
    }

    /// The rider's centre of mass over the board (m): the posed rider's, else between the riding
    /// animation's highest (at this speed) and lowest by the crouch (Skate 3's `anim_motion/crouching`).
    pub fn riding_com(&self) -> f32 {
        self.body_com.unwrap_or_else(|| {
            let a = &self.tuning.anim;
            let high = a.crouch_max_height.eval(self.speed.abs());
            high - self.crouch.clamp(0.0, 1.0) * (high - a.crouch_min_height)
        })
    }

    /// One tick of riding on the ground, driven by `d`.
    pub(super) fn ride(&mut self, park: &dyn Terrain, d: &Drive, dt: f32, events: &mut Vec<Event>) {
        if Self::rides_rigid(d) {
            self.ride_rigid(park, d, dt, events);
        } else {
            self.ride_kinematic(park, d, dt, events);
            self.follow_rig(park);
        }
    }

    /// The older ground model: kick turns, reverts and powerslides turn the board kinematically.
    fn ride_kinematic(&mut self, park: &dyn Terrain, d: &Drive, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let ticks = dt * 60.0;
        let n = self.contact_n;
        // The speed model's target stays within `friction_max_change` of the speed (SK8 speed_model.rs).
        let bound = t.friction_max_change.max(0.0);
        let start = self.speed.abs();
        if (start - self.model_speed).abs() > 1e-4 {
            // Set from outside the ride (a landing, put down, a revert): the model starts from it, as
            // Skate 3's does on entering the ground.
            self.target_speed = start;
        }
        self.target_speed = self
            .target_speed
            .clamp((start - bound).max(0.0), start + bound);

        // Steering: deck tilt eased in, trucks ramped in after landing, then the trucks' curve.
        self.push_steer = if d.pushing {
            (self.push_steer - t.push_steer_drop * ticks).max(t.push_steer_min)
        } else {
            (self.push_steer + t.push_steer_rise * ticks).min(1.0)
        };
        let manual_k = if d.manual { t.manual_tilt } else { 1.0 };
        let want = t.steer_tilt(d.turn, self.speed, d.hard, self.push_steer) * manual_k;
        // Skate 3's speed wobble shakes the tilt before the trucks take it (SK8 Ground Update).
        let want = self.wobble.update(
            &t.wobble,
            wobble::WobbleInput {
                tilt: want,
                speed: self.speed,
                com_height: self.riding_com(),
                truck_tightness: t.truck_tightness,
            },
        );
        let blend = 1.0 - libm::powf(1.0 - t.steer_tilt_blend.clamp(0.0, 1.0), ticks);
        self.deck_tilt += (want - self.deck_tilt) * blend;
        self.truck_ramp = (self.truck_ramp + dt).min(TRUCK_RAMP);
        let tilt = self.deck_tilt * (self.truck_ramp / TRUCK_RAMP);
        let curvature = 2.0 * libm::tanf(KINGPIN * tilt) / WHEELBASE;
        let vel = self.ground_velocity();
        let v = vel.length();
        // The board follows its trucks' curve less than fully and never turns faster than
        // `TRUCK_YAW_MAX` (+ `TRUCK_YAW_MAX_PER_SPEED` x speed), and its yaw rate builds up over
        // `TRUCK_YAW_LAG` (the rider leaning the deck over, its wheels' slip): what SK8's rigid board
        // does with the same deck tilt (ours, measured against SK8's simulation).
        let cap = TRUCK_YAW_MAX + TRUCK_YAW_MAX_PER_SPEED * v;
        let truck = clamp(-self.speed * curvature * TRUCK_FOLLOW, -cap, cap);
        self.truck_yaw += (truck - self.truck_yaw) * (1.0 - libm::expf(-dt / TRUCK_YAW_LAG));
        let mut yaw = self.truck_yaw;
        let grip = self.grip();
        if v > 0.5 {
            // The board can't turn much faster than grip lets its travel turn.
            let lim = grip / v * GRIP_OVER;
            yaw = clamp(yaw, -lim, lim);
        }
        if v > 0.5 && (self.slip / v).abs() > libm::sinf(MAX_SLIP) && yaw * self.slip > 0.0 {
            // Drifted as far as it goes: the board turns no further from its travel.
            yaw = 0.0;
        }

        // Heading: towards the fall line on a slope, or the stick's own turn on a steep face.
        let slope = (libm::acosf(n.y.clamp(-1.0, 1.0)) / (PI / 2.0)).clamp(0.0, 1.0);
        let downhill = (Vec3::UP * -1.0 + n * n.y).normalize_or(Vec3::ZERO);
        if v > 0.05 && d.slide.is_none() && !d.pivot {
            let dir = vel * (1.0 / v);
            let toward = n.dot(dir.cross(downhill)) * v;
            let heading = t.heading_adjust
                * toward
                * t.heading_vs_slope.eval(slope)
                * t.heading_vs_speed
                    .eval((v / t.heading_adjust_max_speed.max(0.01)).clamp(0.0, 1.0));
            let opposing = (self.yaw_rate * d.raw_turn.signum()).max(0.0) / 60.0;
            let torque = -t.turn_torque
                * d.raw_turn
                * t.turn_torque_vs_speed.eval((v / 8.0).clamp(0.0, 1.0))
                * t.turn_torque_vs_slope.eval(slope)
                * t.turn_torque_vs_neg_ang_vel.eval(opposing);
            // The stick's turn replaces the swing unless both turn the same way and the swing is bigger.
            let chosen = if d.raw_turn != 0.0 {
                if torque * heading > 0.0 && heading.abs() > torque.abs() {
                    heading
                } else {
                    torque
                }
            } else {
                heading
            };
            yaw += chosen * 60.0;
        }

        // Straightening: back in line with the travel, unless the stick turns the other way hard.
        if v > 0.3 && d.slide.is_none() && !d.pivot {
            let f = tangent(self.fwd, n);
            let f = if f.dot(vel) < 0.0 { -f } else { f };
            let err = n.dot(f.cross(vel));
            let mut s = t.straighten * err * 60.0;
            if -d.turn * s < 0.0 {
                let lim = t.straighten_max_turn.max(0.01);
                s *= 1.0 - d.turn.abs().min(lim) / lim;
            }
            yaw += s;
        }
        // A wall hit turns the board along the wall for a moment.
        if let Some((rate, time)) = &mut self.collide {
            yaw += *rate;
            *rate *= libm::powf(t.collision_torque_fade, ticks);
            *time -= dt;
            if *time <= 0.0 {
                self.collide = None;
            }
        }
        if d.pivot {
            yaw = 0.0;
        }
        self.yaw_rate = yaw;
        if yaw != 0.0 {
            self.fwd = rotate(self.fwd, n, yaw * dt);
            // The velocity stays where it was in the world: re-read it in the turned board's frame.
            let f = tangent(self.fwd, n);
            self.speed = vel.dot(f);
            self.slip = vel.dot(n.cross(f));
        }

        // Speed model: gravity along the surface, rolling resistance, feet.
        let g = t.gravity;
        let mut gravity = Vec3::UP * -g + n * (g * n.y);
        if let Some(w) = &self.wallride {
            gravity = gravity * (1.0 - t.wallride_antigravity.eval(w.time));
        }
        let f = tangent(self.fwd, n);
        let side = n.cross(f);
        let along = clamp(gravity.dot(f), -t.max_slope_accel, t.max_slope_accel);
        let foot_down = d.push_target.is_some() || d.brake > 0.0;
        let before_model = self.speed.abs();
        if d.manual_correction {
            // Skate 3's Normal corrective drag (SK8 manual/controller.rs `apply_correction`, queued as
            // the tick's only board force by grounded/state/ordinary.rs): -400 N s/m of the velocity at
            // the dragging truck; no brake, push, friction or speed model this tick, gravity still.
            let keep = (1.0 - MANUAL_CORRECTIVE_DRAG * dt / MANUAL_CORRECTIVE_MASS).max(0.0);
            self.speed = self.speed * keep + along * dt;
            self.slip = self.slip * keep + gravity.dot(side) * dt;
            self.target_speed = self.speed.abs();
            self.move_on_ground(park, d, dt, events);
            self.model_speed = self.speed.abs();
            return;
        }
        let mut friction = 0.0;
        if !foot_down && d.slide.is_none() {
            let sp = self.speed.abs();
            friction = t.friction.eval(sp);
            if self.idle > t.no_input_time {
                friction += t.friction_no_input.eval(sp);
            }
            if d.manual {
                friction += t.friction_manual.eval(sp);
            }
            friction = friction.min(t.corrector_limit());
        }
        if along * self.speed < 0.0 {
            // Uphill: whichever slows the board more, gravity or friction (not both).
            if along.abs() >= friction {
                self.speed += along * dt;
            } else {
                self.speed = approach(self.speed, 0.0, friction * dt);
            }
        } else {
            self.speed += along * dt;
            self.speed = approach(self.speed, 0.0, friction * dt);
        }
        // The target follows the slope and the rolling resistance, nothing else.
        self.target_speed = (self.target_speed + self.speed.abs() - before_model).max(0.0);
        self.slip += gravity.dot(side) * dt;
        if let Some(target) = d.push_target
            && self.speed < target
        {
            // SK8 push.rs: the gap to the target, at most the tick's change, as a force on the board.
            let step = (target - self.speed).min(t.push_tick_limit(self.speed.max(0.0)) * ticks);
            self.speed += step * PUSH_TRANSFER;
        }
        if d.brake > 0.0 {
            let decel = d.brake / crate::physics::board::total_mass();
            self.speed = approach(self.speed, 0.0, decel * dt);
            self.slip = approach(self.slip, 0.0, decel * dt);
            if self.speed.abs() < 2.0 * t.brake_slow_speed {
                self.speed = 0.0;
                self.slip = 0.0;
            }
        }
        if let Some(angle) = d.slide {
            // Sideways wheels scrub speed, more the further round the board is.
            let decel = SLIDE_GRIP * g * libm::sinf(angle).abs();
            self.speed = approach(self.speed, 0.0, decel * dt);
            // The sliding wheels' side force (SK8 slide_state/forces.rs `sliding_force`, its
            // `force_vs_angle` part) curves the travel the way the slide turns: the board comes round
            // with it rather than skidding straight on.
            let v = self.speed.abs();
            if v > 0.5 {
                let build = (d.slide_time / SLIDE_SIDE_BUILD).clamp(0.0, 1.0);
                let rate = -angle.signum() * SLIDE_SIDE_ACCEL * build / v;
                self.fwd = rotate(self.fwd, n, rate * dt);
            }
        }
        // Side friction: sideways speed damped, no faster than grip allows.
        if !d.keep_velocity {
            let keep = libm::expf(-t.side_friction * SIDE_DAMP * dt);
            let removal = clamp(self.slip * (1.0 - keep), -grip * dt, grip * dt);
            self.slip -= removal;
        }
        // Speed conservation (SK8 speed_model.rs `update`, Skate 3's UpdateSpeedModel): a force pulls the
        // speed towards the model's target by `speed_general_amount` of the difference a tick, so turning
        // and scrubbing sideways cost no speed; off while a foot pushes or brakes and in a powerslide.
        if foot_down || d.slide.is_some() {
            // The feet and the slide set the speed; the model starts again from it (Skate 3 resets
            // the target to the speed, SK8's `flags_1360` reset).
            self.target_speed = self.speed.abs();
        } else if self.speed.abs() > 0.01 {
            let error = clamp(self.target_speed - self.speed.abs(), -bound, bound);
            self.speed += self.speed.signum() * t.speed_general_amount * error * ticks;
        }

        self.move_on_ground(park, d, dt, events);
        self.model_speed = self.speed.abs();
    }

    /// Moves along the ground: walls, curbs, the trucks finding the surface, and leaving it.
    fn move_on_ground(&mut self, park: &dyn Terrain, d: &Drive, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let n = self.contact_n;
        let f = tangent(self.fwd, n);
        let mut vel = self.ground_velocity();
        self.wall_clock += dt;

        // Walls at wheel height ahead.
        let motion = vel * dt;
        let len = motion.length();
        if len > 1e-6 {
            let dir = motion * (1.0 / len);
            let lead = (dir.dot(f).abs() * AXLE).max(0.1);
            if let Some(h) = park.raycast(self.pos + n * WALL_RAY, dir, len + lead)
                && h.normal.dot(n) < WALL_COS
                && h.normal.dot(dir) < -0.2
                && let Some(v) = self.hit_wall(h.normal, vel, false, events)
            {
                vel = v;
                if !self.stays_rolling() {
                    return;
                }
                // This tick only moves away from the wall.
                self.set_ground_velocity(vel);
                self.pos += h.normal * (t.wall_push_away * dt);
                return;
            }
        }

        // Where the board would be if nothing held it: along its velocity, falling towards the surface.
        let g = t.gravity;
        let p = self.pos + vel * dt + n * (self.vn * dt - 0.5 * g * n.y * dt * dt);
        let ahead = if self.speed >= 0.0 { 1.0 } else { -1.0 };
        let front = Self::probe(park, p + f * AXLE, n);
        let back = Self::probe(park, p - f * AXLE, n);
        let (lead, trail) = if ahead > 0.0 {
            (front, back)
        } else {
            (back, front)
        };
        // Curb: the leading truck would climb more than a wheel's height at once.
        if let Some(h) = lead {
            let rise = (h.point - (p + f * (AXLE * ahead))).dot(n);
            let jump = rise - self.lead_rise;
            self.lead_rise = rise.max(0.0);
            // A step up onto a surface parallel to this one (not the start of a ramp).
            if jump > CLIMB && h.normal.dot(n) > 0.97 && self.speed.abs() > 0.05 {
                let wall = (-(f * ahead)).normalize_or(Vec3::UP);
                if let Some(v) = self.hit_wall(wall, vel, false, events) {
                    if self.stays_rolling() {
                        self.set_ground_velocity(v);
                    }
                    return;
                }
            }
        }
        let contact = match (front, back) {
            (Some(a), Some(b)) => {
                let nn = (a.normal + b.normal).normalize_or(n);
                let chord = (a.point - b.point).normalize_or(f);
                Some(((a.point + b.point) * 0.5, nn, Some(chord)))
            }
            _ if lead.is_none() && (trail.is_none() || Self::probe(park, p, n).is_none()) => None,
            _ => {
                // One truck on: carry on at its height.
                let (h, side) = match lead {
                    Some(h) => (h, ahead),
                    None => (trail.expect("one truck has a surface"), -ahead),
                };
                let c = h.point - f * (AXLE * side);
                Some((p + n * (c - p).dot(n), n, None))
            }
        };
        let Some((c, nn, chord)) = contact else {
            // Rolled off an edge: a lip, a ledge, the coping.
            self.pos = p + n * 0.02;
            self.publish_off_ground(super::rig::WHEEL_LINE_LENGTH, self.gap_time);
            self.leave_ground(vel + n * self.vn, false, events);
            return;
        };
        let gap = (p - c).dot(nn);
        if gap <= 0.0 {
            // The ground holds the board up: settle on it, the velocity turned into its plane (kept whole
            // while it was already pressed on; coming back down, only the part along the surface stays).
            let pressed = self.vn == 0.0;
            let v3d = vel + n * self.vn;
            self.pos = c;
            self.contact_n = nn;
            // Drawn along the chord between the trucks' contacts, so both wheels sit on the surface
            // over a kink too (the physics' nose follows the surfaces' mean normal).
            self.chord = chord.map(|ch| if ch.dot(self.fwd) < 0.0 { -ch } else { ch });
            if let Some(ch) = chord {
                self.fwd = tangent(ch, nn);
            } else {
                self.fwd = tangent(self.fwd, nn);
            }
            self.vn = 0.0;
            self.gap_time = 0.0;
            self.publish_wheels(if chord.is_some() { 4 } else { 2 });
            let f2 = tangent(self.fwd, nn);
            let along = v3d - nn * v3d.dot(nn);
            let v3d = if pressed {
                along.normalize_or(f2) * vel.length()
            } else {
                along
            };
            self.speed = v3d.dot(f2);
            self.slip = v3d.dot(nn.cross(f2));
        } else {
            // The surface falls away faster than the board falls: it flies on its own path, and is in
            // the air once the gap is big enough (or has lasted).
            self.pos = p;
            self.vn -= g * n.y * dt;
            if gap > t.natural_air_max_dist {
                self.publish_off_ground(gap, self.gap_time);
                self.leave_ground(vel + n * self.vn, false, events);
                return;
            }
            if gap > t.natural_air_min_dist {
                self.gap_time += dt;
                if self.gap_time > t.natural_air_time {
                    self.publish_off_ground(gap, self.gap_time);
                    self.leave_ground(vel + n * self.vn, false, events);
                    return;
                }
            } else {
                self.gap_time = 0.0;
            }
        }
        self.update_pumping(d, dt);
        // Hanging under an overhang: fall off it.
        if self.contact_n.y < -0.2 {
            let v = self.ground_velocity() + self.contact_n * 0.5;
            self.pos += self.contact_n * 0.02;
            self.leave_ground(v, true, events);
            return;
        }
        self.update_wallride(park, dt, events);
    }

    /// Looks for the surface under a point of the board: a ray along -`n` from a little above it.
    fn probe(park: &dyn Terrain, at: Vec3, n: Vec3) -> Option<crate::park::Hit> {
        park.raycast(at + n * PROBE_UP, -n, PROBE_UP + PROBE_DOWN)
    }

    /// Sets the ground velocity from a vector along the surface.
    pub(super) fn set_ground_velocity(&mut self, v: Vec3) {
        let n = self.contact_n;
        let f = tangent(self.fwd, n);
        self.speed = v.dot(f);
        self.slip = v.dot(n.cross(f));
    }

    /// The board leaves the ground with velocity `v` (a world vector). Rolling off vert without a pop
    /// keeps only part of the speed away from the face. `jump`: a jump off the surface (a wallride's
    /// jump off, falling off an overhang), which starts Skate 3's state frames over; else the board
    /// has rolled off the ground (the off-ground facts are published), and leaves it only if Skate 3's
    /// state selector takes it into the air (false: it stays on the ground).
    pub(super) fn leave_ground(
        &mut self,
        mut v: Vec3,
        jump: bool,
        events: &mut Vec<Event>,
    ) -> bool {
        if !jump && !matches!(self.selects(|_| {}), State::PhysicsAir | State::KnownAir) {
            return false;
        }
        let t = self.tuning;
        let n = self.contact_n;
        if n.y < t.vert_align_max_ny {
            let out = v.dot(n);
            if out > 0.0 {
                v -= n * (out * (1.0 - t.natural_air_off_verts));
            }
        }
        if let Some(w) = self.wallride.take() {
            self.score_wallride(w.time, events);
        }
        self.vel = v;
        self.take_off(None);
        if !jump {
            // Rolled off: the selector's off-ground test takes it into the air, not a jump.
            self.states.facts.jumped = false;
        }
        events.push(Event::Launched);
        true
    }

    /// Running into a wall (unit `normal` facing the board) with ground velocity `vel`: too fast is a
    /// bail or a runout (returns `None`); otherwise only the speed into it goes, the board is pushed off
    /// it and turned along it. Returns the new velocity.
    pub(super) fn hit_wall(
        &mut self,
        normal: Vec3,
        vel: Vec3,
        air: bool,
        events: &mut Vec<Event>,
    ) -> Option<Vec3> {
        let t = self.tuning;
        let n = if air { Vec3::UP } else { self.contact_n };
        // The wall's normal along the surface (or level, in the air).
        let wn = (normal - n * normal.dot(n)).normalize_or(normal);
        let closing = -vel.dot(wn);
        if closing <= 0.0 {
            return Some(vel);
        }
        let limit = if air {
            t.wipe_air_closing
        } else {
            t.wipe_ground_closing
        };
        let single = self.wall_clock > REHIT_TIME;
        self.wall_clock = 0.0;
        if closing > limit {
            let upright = self.normal.y > 0.7;
            self.vel = vel * 0.3;
            if upright && single && closing < limit * RUNOUT_RATIO {
                self.run_out("ran into a wall", events);
            } else {
                self.bail(
                    if air {
                        "hit a wall in the air"
                    } else {
                        "ran into a wall"
                    },
                    events,
                );
            }
            return None;
        }
        // Keep the speed along the wall; push off it.
        let mut dv = wn * (closing + t.wall_push_away);
        if dv.length() > t.wall_max_vel_delta {
            dv = dv * (t.wall_max_vel_delta / dv.length());
        }
        let out = vel + dv;
        // Turn the board along the wall: more for glancing hits, none head on.
        let v = vel.length();
        if !air && v > 0.1 {
            let dir = vel * (1.0 / v);
            let x = libm::acosf(dir.dot(wn).clamp(-1.0, 1.0)) / PI;
            let along = dir - wn * dir.dot(wn);
            let f = tangent(self.fwd, n);
            let f = if f.dot(dir) < 0.0 { -f } else { f };
            let sign = n.dot(f.cross(along)).signum();
            let rate = t.collision_torque.eval(x) * 60.0 * sign;
            if rate != 0.0 {
                self.collide = Some((rate, t.collision_torque_time));
            }
        }
        self.slide = None;
        if let Some(w) = self.wallride.take() {
            self.score_wallride(w.time, events);
        }
        Some(out)
    }

    /// Skate 3's pumping (SK8 pumping/controller.rs `update`, `calculate`): the rider's centre of mass
    /// rising over the board times how fast the ground turns upward along the travel. The centre of
    /// mass is the animation's (the imported rider's), which the transition itself compresses through
    /// the crouch this reports back (`absorption`, `ground_absorption`, `min_crouch`): riding out of a
    /// transition the rider springs up and the pump pays it out, intentional or not. Without the rider,
    /// the stick's crouch.
    pub(super) fn update_pumping(&mut self, d: &Drive, dt: f32) {
        let t = self.tuning;
        let n = self.contact_n;
        let deck_angle = self.deck_angle();
        let height = self.body_com.unwrap_or(-self.crouch * CROUCH_DEPTH);
        let vel = self.ground_velocity();
        let p = &mut self.pump;
        if p.prev_n.is_none() {
            // The first sample only records (SK8's `record_valid`).
            p.last_height = height;
        }
        let rising = (height - p.last_height).max(0.0);
        p.last_height = height;
        p.effect = rising * t.pump_damping + (1.0 - t.pump_damping) * p.effect;
        let mut effect = if p.effect < t.pump_min_change {
            0.0
        } else {
            p.effect
        };
        p.time = if effect > 0.0 { p.time + dt } else { 0.0 };
        effect = effect.min(t.pump_max_change) * t.pump_vs_time.eval(p.time);
        // How fast the ground's normal turns along the travel (+: concave, the board pressed in).
        let v = vel.length();
        let measured = match p.prev_n {
            Some(prev) if v > 0.1 => {
                let axis = vel.cross(n).normalize_or(Vec3::ZERO);
                // The normal tipping back against the travel (the surface rising ahead) is concave.
                prev.cross(n).dot(axis) / dt
            }
            _ => 0.0,
        };
        // The ground's angle (quarter turns) under the last tick's normal.
        let angle = p.prev_n.map_or(0.0, |prev| {
            libm::acosf(prev.y.clamp(0.0, 1.0)) * (2.0 / core::f32::consts::PI)
        });
        p.prev_n = Some(n);
        p.turn_rate = measured * t.pump_ang_damping + (1.0 - t.pump_ang_damping) * p.turn_rate;
        p.absorption = -(v * p.turn_rate);
        p.ground_absorption =
            t.compression_vs_ground_angle.eval(angle.clamp(0.0, 1.0)) * t.compression_ground_scale;
        p.min_crouch = t.min_crouch_vs_ground_angle.eval(angle.clamp(0.0, 1.0));
        // The deck's angle (SK8 ground_runtime/pumping.rs `update`): when the physical deck is
        // pitched further from the rider's up than the posed board, by how much (about the ground
        // frame's right); it compresses the rider by `CompressionVsDeckAngle` x
        // `CompressionDeckScalar` when that outweighs the ground's compression.
        p.deck_absorption = t
            .compression_vs_deck_angle
            .eval(deck_angle * (2.0 / core::f32::consts::PI));
        let deck_compression = t.compression_deck_scale * p.deck_absorption;
        if deck_compression * deck_compression > p.ground_absorption * p.ground_absorption {
            p.ground_absorption = deck_compression;
        }
        let pumping = p.turn_rate * t.pump_vs_vel.eval(v);
        p.pumping = pumping;
        p.dv = 0.0;
        // Rising with the ground turning up along the travel pumps (`PumpEffectFactor`); rising with it
        // turning away takes speed (`PumpEffectFactorAbsorption`), at most the mode's
        // `pump_max_absorption` a second (SK8 pumping/controller.rs `update`; none on Easy and Normal).
        if pumping != 0.0 && effect > 0.0 && d.slide.is_none() {
            let factor = if pumping * effect > 0.0 {
                t.pump_factor
            } else {
                t.pump_absorption
            };
            let raw = pumping * factor * effect;
            let mut dv = raw
                .max(-(t.pump_max_absorption * dt))
                .min(t.pump_max_accel * dt);
            if !d.intentional_pump {
                dv *= t.pump_unintentional;
            }
            if dv != 0.0 {
                self.speed += if self.speed < 0.0 { -dv } else { dv };
                self.pump.dv = dv;
            }
        }
    }

    /// The physical deck's pitch against the posed board's, both from the rider's (reckoning) up about
    /// the ground frame's right, when the deck is the steeper of the two (SK8 ground_runtime/pumping.rs
    /// `update`, its `deck_angle`): a deck pitched over a kink or a transition's start while the
    /// rider's up lags. The posed board is the imported rider's (`GraphControl::pose_deck`, in the
    /// rider's frame), else level in it.
    fn deck_angle(&self) -> f32 {
        // All in the board's effective frame (the rider's), turned end for end with it.
        let r = &self.reckoning;
        let turn = if self.frame_turned { -1.0 } else { 1.0 };
        let up = r.up;
        let axis = r.ground_right() * turn;
        // The rider's frame: his heading square to his up.
        let right = up.cross(r.heading() * turn).normalize_or(axis);
        let heading = right.cross(up).normalize_or(r.heading() * turn);
        let posed = self.graph.pose_deck.unwrap_or(Vec3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        });
        let animated = right * posed.x + up * posed.y + heading * posed.z;
        // The physical deck (SK8's toolkit deck), its nose the skater's way; the kinematic board's
        // nose lies along the ground and never pitches over a lip.
        if let Some(rig) = self.rig.as_ref().filter(|_| self.rolling()) {
            let deck = rig.board.deck().axes.cols[2] * self.rig_sign();
            if deck.dot(up).abs() <= animated.dot(up).abs() {
                return 0.0;
            }
            let physical = super::reckoning::wrapped_angle(deck, up, axis);
            let posed = super::reckoning::wrapped_angle(animated, up, axis);
            return (physical - posed).abs();
        }
        let frame = self.board_frame();
        let mut deck = frame.cols[2];
        if let Some(m) = &self.manual {
            // The manual pitches the deck about the wheels left down: a tail manual's nose up.
            let lift = if m.nose { -m.pitch } else { m.pitch };
            deck = deck * libm::cosf(lift) + frame.cols[1] * libm::sinf(lift);
        }
        if deck.dot(up).abs() <= animated.dot(up).abs() {
            return 0.0;
        }
        let physical = super::reckoning::wrapped_angle(deck, up, axis);
        let posed = super::reckoning::wrapped_angle(animated, up, axis);
        (physical - posed).abs()
    }

    /// The floor under a point on a steep face: its normal and how far below (m), looking straight down.
    pub(super) fn floor_under(park: &dyn Terrain, at: Vec3, wall: Vec3) -> Option<(Vec3, f32)> {
        let from = at + wall * 0.1;
        park.raycast(from, -Vec3::UP, FLOOR_PROBE)
            .map(|h| (h.normal, from.y - h.point.y))
    }

    /// Wallrides (Skate 3's model): a face steeper than 60 degrees with a real floor under it (one that
    /// doesn't curve up into it, as a quarter pipe or bowl does). The board falls along the wall in an
    /// arc and jumps off it when it gets low, slow, or has been on it a while.
    pub(super) fn update_wallride(&mut self, park: &dyn Terrain, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let n = self.contact_n;
        let steep = n.y.abs() < 0.5;
        if !steep {
            if let Some(w) = self.wallride.take() {
                self.score_wallride(w.time, events);
            }
            return;
        }
        let floor = Self::floor_under(park, self.pos, n);
        let Some(w) = &mut self.wallride else {
            let v = self.ground_velocity().length();
            if let Some((fl, h)) = floor
                && fl.dot(n) < t.wallride_max_dot_floor
                && h >= WALLRIDE_MIN_HEIGHT
                && v > 1.0
            {
                self.start_wallride(false);
            }
            return;
        };
        w.time += dt;
        let time = w.time;
        let vel = self.ground_velocity();
        let speed = vel.length();
        let (floor_n, height) = floor.unwrap_or((Vec3::UP, 0.0));
        let dot = floor_n.dot(n);
        let predicted = height + vel.y * t.wallride_lookahead;
        let slow = speed < 3.0 && time > 0.1;
        if dot < t.wallride_max_dot_floor
            && (slow || predicted < t.wallride_jump_height || time > t.wallride_max_time)
        {
            // Jump off: along the wall (falling speed partly kept) and away from it.
            let mut along = vel - n * vel.dot(n);
            if along.y < 0.0 && along.y > -6.0 {
                along.y *= t.wallride_jump_ydown;
            }
            let v = along + n * t.wallride_jump_force;
            self.pos += n * 0.05;
            self.leave_ground(v, true, events);
        }
    }

    /// Starts a wallride on the face under the board.
    pub(super) fn start_wallride(&mut self, from_air: bool) {
        self.wallride = Some(WallRide {
            time: 0.0,
            from_air,
        });
        self.manual = None;
        self.brake = None;
        self.slide = None;
    }

    fn score_wallride(&self, time: f32, events: &mut Vec<Event>) {
        if time > 0.2 {
            events.push(Event::Landed {
                name: "WALLRIDE".into(),
                points: 0,
                score: crate::scoring::Scored::Flat(WALLRIDE_POINTS + (time * 200.0) as u32),
            });
        }
    }

    /// The direction and speed of a pop off the surface under the board (Skate 3's ground jump): the
    /// jump along the normal, weaker the steeper the face; a lift on near-vertical faces; the left stick
    /// (`stick`, at the pop) nudging it; and on vert, the launch re-aimed straight up, leaned in or out
    /// over the coping with the main stick.
    pub(super) fn pop_velocity(&self, up_speed: f32, stick: [f32; 2]) -> Vec3 {
        let t = self.tuning;
        let n = self.contact_n;
        let slope = (libm::acosf(n.y.clamp(-1.0, 1.0)) / (PI / 2.0)).clamp(0.0, 1.0);
        let response = t.vertical_response.eval(slope).max(t.jump_min_scalar);
        let along = up_speed * response;
        let mut vel = self.ground_velocity() + n * along;
        vel.y += ((t.jump_y_vs_normal_y.eval(n.y) - 1.0) * vel.y).clamp(0.0, t.jump_y_bonus_max);
        // Left stick: sideways (right +) and forwards.
        let fh = self.fwd.horizontal().normalize_or(yaw_dir(self.heading));
        let fh = if self.speed < 0.0 { -fh } else { fh };
        // Up x travel points to the rider's left (+X when travelling +Z): the stick to the right takes
        // the pop the other way (SK8 ground_jump: `side * -x`).
        let left = Vec3::UP.cross(fh).normalize_or(Vec3::ZERO);
        vel += left * (-stick[0] * t.jump_adjust_x * along)
            + fh * (stick[1] * t.jump_adjust_z * along);
        if n.y < t.vert_align_max_ny {
            // The face's horizontal out direction: the launch's up-and-out part gets re-aimed.
            let out = v3(n.x, 0.0, n.z).normalize_or(Vec3::ZERO);
            let part = Vec3::UP * vel.y + out * vel.dot(out);
            let len = part.length();
            if len > 1e-3 && part.y / len > t.vert_align_min_dir_y {
                let lean = clamp(stick[1] - 0.25, -1.0, 1.0);
                let aim = (Vec3::UP - n * (lean * t.vert_align_max_angle)).normalize_or(Vec3::UP);
                let k = t.vert_align_factor.clamp(0.0, 1.0);
                vel = vel - part + part.lerp(aim * len, k);
            }
        }
        vel
    }
}

/// A wallride in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallRide {
    pub time: f32,
    pub from_air: bool,
}
