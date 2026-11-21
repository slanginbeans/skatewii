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
//! Masses, inertia and wheel forces are not simulated; where Skate 3's value is a force, a flatground
//! calibration turns it into an acceleration (marked "ours").

use super::*;

/// Distance from the board's middle to each truck (m) and the wheelbase.
pub const AXLE: f32 = 0.243;
const WHEELBASE: f32 = 2.0 * AXLE;
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
/// of `side_friction` (1/s). Ours.
const SIDE_GRIP: f32 = 0.85;
const SIDE_DAMP: f32 = 0.4;
/// The trucks may turn the board a little faster than grip lets the travel turn (a drift), up to a
/// sideways angle of `MAX_SLIP` (rad). Ours.
const GRIP_OVER: f32 = 1.1;
const MAX_SLIP: f32 = 0.35;
/// Powerslide: wheel friction sliding sideways (share of g at 90 degrees). Ours.
const SLIDE_GRIP: f32 = 0.5;
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
    /// A braking foot or tail is on the ground: slow down at this (m/s^2).
    pub brake: f32,
    /// On two wheels.
    pub manual: bool,
    /// Powerslide: the board's angle from travel (rad); the path keeps following the trucks.
    pub slide: Option<f32>,
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

    /// One tick of riding on the ground, driven by `d`.
    pub(super) fn ride(&mut self, park: &dyn Terrain, d: &Drive, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let ticks = dt * 60.0;
        let n = self.contact_n;

        // Steering: deck tilt eased in, trucks ramped in after landing, then the trucks' curve.
        let push_contact = d.push_target.is_some();
        self.push_steer = if push_contact {
            (self.push_steer - t.push_steer_drop * ticks).max(t.push_steer_min)
        } else {
            (self.push_steer + t.push_steer_rise * ticks).min(1.0)
        };
        let manual_k = if d.manual { t.manual_tilt } else { 1.0 };
        let want = t.steer_tilt(d.turn, self.speed, d.hard, self.push_steer) * manual_k;
        let blend = 1.0 - libm::powf(1.0 - t.steer_tilt_blend.clamp(0.0, 1.0), ticks);
        self.deck_tilt += (want - self.deck_tilt) * blend;
        self.truck_ramp = (self.truck_ramp + dt).min(TRUCK_RAMP);
        let tilt = self.deck_tilt * (self.truck_ramp / TRUCK_RAMP);
        let curvature = 2.0 * libm::tanf(KINGPIN * tilt) / WHEELBASE;
        let vel = self.ground_velocity();
        let v = vel.length();
        let mut yaw = -self.speed * curvature;
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
        let foot_down = push_contact || d.brake > 0.0;
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
        self.slip += gravity.dot(side) * dt;
        if let Some(target) = d.push_target
            && self.speed < target
        {
            self.speed = (self.speed + t.push_tick_limit(self.speed.max(0.0)) * ticks).min(target);
        }
        if d.brake > 0.0 {
            self.speed = approach(self.speed, 0.0, d.brake * dt);
            self.slip = approach(self.slip, 0.0, d.brake * dt);
            if self.speed.abs() < 2.0 * t.brake_slow_speed {
                self.speed = 0.0;
                self.slip = 0.0;
            }
        }
        if let Some(angle) = d.slide {
            // Sideways wheels scrub speed, more the further round the board is.
            let decel = SLIDE_GRIP * g * libm::sinf(angle).abs();
            self.speed = approach(self.speed, 0.0, decel * dt);
        }
        // Side friction: sideways speed damped, no faster than grip allows.
        if !d.keep_velocity {
            let keep = libm::expf(-t.side_friction * SIDE_DAMP * dt);
            let removal = clamp(self.slip * (1.0 - keep), -grip * dt, grip * dt);
            self.slip -= removal;
        }

        self.move_on_ground(park, d, dt, events);
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
                if self.mode != Mode::Rolling {
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
                    if self.mode == Mode::Rolling {
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
            self.leave_ground(vel + n * self.vn, events);
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
            if let Some(ch) = chord {
                self.fwd = tangent(ch, nn);
            } else {
                self.fwd = tangent(self.fwd, nn);
            }
            self.vn = 0.0;
            self.gap_time = 0.0;
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
                self.leave_ground(vel + n * self.vn, events);
                return;
            }
            if gap > t.natural_air_min_dist {
                self.gap_time += dt;
                if self.gap_time > t.natural_air_time {
                    self.leave_ground(vel + n * self.vn, events);
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
            self.leave_ground(v, events);
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
    /// keeps only part of the speed away from the face.
    fn leave_ground(&mut self, mut v: Vec3, events: &mut Vec<Event>) {
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
        events.push(Event::Launched);
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

    /// Skate 3's pumping: the rider's extension times how fast the ground turns upward along the travel.
    fn update_pumping(&mut self, d: &Drive, dt: f32) {
        let t = self.tuning;
        let n = self.contact_n;
        let height = -self.crouch * CROUCH_DEPTH;
        let vel = self.ground_velocity();
        let p = &mut self.pump;
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
        p.prev_n = Some(n);
        p.turn_rate = measured * t.pump_ang_damping + (1.0 - t.pump_ang_damping) * p.turn_rate;
        let pumping = p.turn_rate * t.pump_vs_vel.eval(v);
        p.dv = 0.0;
        if pumping > 0.0 && effect > 0.0 && d.slide.is_none() {
            let mut dv = (pumping * t.pump_factor * effect).min(t.pump_max_accel * dt);
            if !d.intentional_pump {
                dv *= t.pump_unintentional;
            }
            self.speed += if self.speed < 0.0 { -dv } else { dv };
            self.pump.dv = dv;
        }
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
    fn update_wallride(&mut self, park: &dyn Terrain, dt: f32, events: &mut Vec<Event>) {
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
            self.leave_ground(v, events);
        }
    }

    /// Starts a wallride on the face under the board.
    pub(super) fn start_wallride(&mut self, from_air: bool) {
        self.wallride = Some(WallRide {
            time: 0.0,
            from_air,
        });
        self.manual = None;
        self.push = None;
        self.brake = None;
        self.slide = None;
    }

    fn score_wallride(&self, time: f32, events: &mut Vec<Event>) {
        if time > 0.2 {
            events.push(Event::Landed {
                name: "WALLRIDE".into(),
                points: WALLRIDE_POINTS + (time * 200.0) as u32,
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
