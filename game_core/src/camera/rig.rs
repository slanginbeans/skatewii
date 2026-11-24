//! The camera's rig: the state its shot choice reads (air, slopes, stance, steering), the compass the
//! shots' headings turn with, the arm (heading, elevation, distance) and its trackers, the anchor and its
//! push lag, the look-at height through the air, collision, avoidance and the drop predictor.
//!
//! After SK8-ENGINE/skate-3-rust-engine (GPL-3.0) crates/skate-core/src/camera: manager_state.rs,
//! manager_controls.rs, manager_incline.rs, manager_air.rs, manager_rig.rs, compass.rs,
//! compass_movement.rs, rig.rs, rig_tracking.rs, rig_framing.rs, rig_positioning.rs, positioner.rs,
//! avoidance.rs and drop_predictor.rs. Every update runs once per 1/60 s tick, as Skate 3's do.
//!
//! Angles: elevations up positive; headings in Skate 3's sense, the negative of flatground's yaw (an arm
//! of heading `h` points along yaw `-h`).

use libm::{acosf, asinf, atanf};

use crate::math::{PI, Vec3, atan2f, cosf, sinf, v3, wrap_angle};
use crate::park::Terrain;
use crate::tuning::CamTuning;
use crate::tuning::camera::AngleTrack;

use super::shot::{OPT_AVOID, OPT_FOLLOW, OPT_MIRROR, Shot};
use super::subject::Subject;
use super::tracker::{Angle, Params, Point, RAD, Scalar, blend_angle};

pub const STEP: f32 = 1.0 / 60.0;
/// Radius of the camera's collision lines (m).
pub const LINE_RADIUS: f32 = 0.13;
/// Most the arm's elevation goes (rad, 80 degrees).
const MAX_ELEVATION: f32 = 80.0 * RAD;

/// The arm's direction from its anchor: elevation `e`, heading `h`.
pub fn arm(e: f32, h: f32) -> Vec3 {
    let c = cosf(e);
    v3(c * sinf(-h), sinf(e), c * cosf(-h))
}

/// Elevation and yaw of a direction (zero when it points straight up or down).
pub fn angles_of(d: Vec3) -> (f32, f32) {
    let hl = (d.x * d.x + d.z * d.z).max(0.0);
    if hl < 1e-8 {
        return (0.0, 0.0);
    }
    let l = d.length().max(1e-6);
    (asinf((d.y / l).clamp(-1.0, 1.0)), atan2f(d.x, d.z))
}

/// A compass heading (Skate 3's sense) of a direction; straight up reads 0.
pub fn heading_of(v: Vec3) -> f32 {
    if v.y > 0.9998 { 0.0 } else { atan2f(-v.x, v.z) }
}

/// Where a line from `a` to `b` first comes within `radius` of the park, as a fraction of the way
/// (none: clear).
pub fn sweep(park: &dyn Terrain, a: Vec3, b: Vec3, radius: f32) -> Option<f32> {
    let d = b - a;
    let len = d.length();
    if len < 1e-4 {
        return None;
    }
    let dir = d * (1.0 / len);
    park.raycast(a, dir, len + radius)
        .map(|h| ((h.t - radius) / len).clamp(0.0, 1.0))
}

fn angle_params(t: &AngleTrack, smoothing: f32) -> Params {
    Params {
        umbra: t.umbra * RAD,
        penumbra: t.penumbra * RAD,
        speed: t.speed * RAD,
        accel_far: t.accel_far * RAD,
        accel_near: t.accel_near * RAD,
        smooth_far: smoothing,
        smooth_near: smoothing,
        overshoot_stops: true,
    }
}

/// What the shot choice and the blends read, kept from tick to tick (SK8 ManagerState).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub anchor: Vec3,
    pub anchor_vel: Vec3,
    pub selected_anchor: u8,
    pub frontside: f32,
    pub velocity_incline: f32,
    pub direction_incline: f32,
    pub launch_incline: f32,
    pub landing_incline: f32,
    pub apex_height: f32,
    pub apex_time: f32,
    pub landing_delta: f32,
    pub apex_ratio: f32,
    pub transfer: f32,
    pub trajectory_distance: f32,
    pub steering_time: f32,
    pub centred_time: f32,
    pub collision_elevation: f32,
    /// A flight is being followed.
    pub in_flight: bool,
    /// The flight goes up a wall and comes back down one (Skate 3's reentry).
    pub reentry: bool,
    /// The camera was on the ground last tick.
    pub was_ground: bool,
    /// Just reset (cleared on the ground).
    pub fresh: bool,
    pub mirrored: bool,
    pub heading_mirror: Scalar,
    pub framing_mirror: Scalar,
    pub prev_vel: Vec3,
}

impl Default for State {
    fn default() -> Self {
        State {
            anchor: Vec3::ZERO,
            anchor_vel: Vec3::ZERO,
            selected_anchor: 0,
            frontside: 0.0,
            velocity_incline: 0.0,
            direction_incline: 0.0,
            launch_incline: 0.0,
            landing_incline: 0.0,
            apex_height: 0.0,
            apex_time: 0.0,
            landing_delta: 0.0,
            apex_ratio: 0.0,
            transfer: PI,
            trajectory_distance: 0.0,
            steering_time: 0.0,
            centred_time: 0.0,
            collision_elevation: 0.0,
            in_flight: false,
            reentry: false,
            was_ground: true,
            fresh: true,
            mirrored: false,
            heading_mirror: Scalar::at(1.0),
            framing_mirror: Scalar::at(1.0),
            prev_vel: Vec3::ZERO,
        }
    }
}

impl State {
    /// Steering clocks and the stance (SK8 manager_controls.rs `update_timing`).
    pub fn update_timing(&mut self, s: &Subject, t: &CamTuning) {
        if s.reset {
            self.trajectory_distance = 0.0;
            self.centred_time = 0.0;
            self.steering_time = 0.0;
            self.collision_elevation = 0.0;
            self.anchor_vel = Vec3::ZERO;
            self.in_flight = false;
        }
        self.steering_time = if s.steering.abs() <= t.centred_deadzone {
            0.0
        } else {
            self.steering_time + STEP
        };
        self.centred_time = if s.steering_for_turn.abs() >= t.centred_deadzone {
            0.0
        } else {
            self.centred_time + STEP
        };
        self.mirrored = s.mirrored;
    }

    /// The flight's numbers and the look-at height's ramp from take-off to landing (SK8 manager_air.rs).
    ///
    /// Returns the ground's normal and the camera's velocity a tick ago when the camera's ground state
    /// just came on moving into the ground: a landing, for the shake (SK8 manager_air.rs, the pair it
    /// hands `ShakeEffect82E055D8`).
    pub fn update_air(
        &mut self,
        s: &Subject,
        rig: &mut Rig,
        ground: bool,
        camera: Vec3,
    ) -> Option<(Vec3, Vec3)> {
        let previous_delta = self.landing_delta;
        let traj = s.trajectory;
        if let Some(tr) = traj {
            self.landing_delta = tr.landing.y - tr.launch.y;
            self.apex_height = tr.apex.y - tr.launch.y;
            self.apex_time = tr.apex_time;
            self.launch_incline = acosf(tr.launch_normal.y.clamp(-1.0, 1.0));
        }
        self.apex_ratio = if self.trajectory_distance != 0.0 {
            self.apex_height / self.trajectory_distance
        } else {
            0.0
        };
        self.reentry = false;
        match traj {
            Some(tr) if !ground => {
                if self.in_flight {
                    if (self.landing_delta - previous_delta).abs() > 0.001 {
                        rig.landing_moved = true;
                    }
                } else {
                    let a = rig.anchors_pos[self.selected_anchor as usize];
                    self.trajectory_distance = (camera - a).length();
                    rig.landing_moved = false;
                }
                self.in_flight = true;
                self.landing_incline = acosf(tr.landing_normal.y.clamp(-1.0, 1.0));
                if self.launch_incline > 70.0 * RAD && self.landing_incline > 30.0 * RAD {
                    self.reentry = true;
                }
                self.transfer = PI;
                if tr.launch_normal.y < 0.99 && tr.landing_normal.y < 0.99 {
                    let a = heading_of(tr.launch_normal);
                    let b = heading_of(tr.landing_normal);
                    self.transfer = wrap_angle(b - a).abs();
                }
            }
            _ => self.in_flight = false,
        }
        rig.height_weight = match traj {
            Some(tr) if !ground => (tr.time / tr.duration).clamp(0.0, 1.0),
            _ => 0.0,
        };
        if !ground
            && let Some(tr) = traj
            && rig.height_time >= rig.height_duration
            && self.landing_delta > -3.0
            && !self.fresh
            && rig.previous_height != f32::MAX
            && rig.reference_height != f32::MAX
        {
            rig.height_start = rig.reference_height;
            rig.height_end = self.landing_delta + rig.reference_height;
            rig.height_duration = tr.duration - tr.time;
            rig.height_time = 0.0;
        }
        let mut landing = None;
        if ground {
            if !self.was_ground && self.prev_vel.dot(s.ground_up) < 0.0 {
                landing = Some((s.ground_up, self.prev_vel));
            }
            self.landing_delta = 0.0;
        }
        self.was_ground = ground;
        self.prev_vel = rig.velocity;
        landing
    }

    /// Slopes along the travel and the frontside angle (SK8 manager_incline.rs).
    pub fn update_incline(&mut self, s: &Subject) {
        let n = s.ground_up;
        if n.y > 0.99999 && !s.grinding {
            self.velocity_incline = 0.0;
            self.frontside = 0.0;
            self.direction_incline = 0.0;
            return;
        }
        let dir = if self.anchor_vel.length() > 1e-6 {
            self.anchor_vel.normalize_or(s.fwd)
        } else {
            s.fwd
        };
        self.velocity_incline = asinf(dir.y.clamp(-1.0, 1.0));
        // Uphill along the slope, and across it.
        let tangent = (Vec3::UP - n * n.dot(Vec3::UP)).normalize_or(Vec3::ZERO);
        let across = n.cross(tangent);
        self.frontside = atan2f(dir.dot(across), dir.dot(tangent));
        if self.mirrored {
            self.frontside = -self.frontside;
        }
        if self.anchor_vel.length() > 1e-7 {
            let d = if s.grinding { s.grind_dir } else { dir };
            self.direction_incline = asinf(d.y.clamp(-1.0, 1.0));
        }
    }

    /// The stance mirror, eased through two trackers (quickly for headings, slowly for framing), only
    /// while the camera is on the ground (SK8 manager_controls.rs `update_mirrors`).
    pub fn update_mirrors(&mut self, reset: bool, ground: bool, shot: &Shot) {
        if !reset && !ground {
            return;
        }
        let target = if shot.has(OPT_MIRROR) && self.mirrored {
            -1.0
        } else {
            1.0
        };
        if reset {
            self.heading_mirror.snap(target);
            self.framing_mirror.snap(target);
        } else {
            self.heading_mirror
                .update(STEP, target, &Params::fixed(100.0, 500.0, 0.85));
            self.framing_mirror
                .update(STEP, target, &Params::fixed(15.0, 50.0, 0.85));
        }
    }
}

/// The compass norths a shot's heading turns with (SK8 compass.rs, compass_movement.rs): 0 the
/// movement heading, 1 the ground's slope heading from take-off to landing, 3 the board's heading, 4
/// the flight's, 6 and 7 towards the skater from the camera (7 follows the body's motion), 8 the camera
/// round the board.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compass {
    pub headings: [f32; 9],
    pub movement: f32,
    prev_pos: Vec3,
    stopped: f32,
    vel: Vec3,
    fresh: bool,
}

impl Default for Compass {
    fn default() -> Self {
        Compass {
            headings: [0.0; 9],
            movement: 0.0,
            prev_pos: Vec3::ZERO,
            stopped: 0.0,
            vel: Vec3::ZERO,
            fresh: true,
        }
    }
}

impl Compass {
    pub fn update(&mut self, s: &Subject, t: &CamTuning, camera: Vec3, dt: f32, selected: u8) {
        let reset = dt == 0.0 || self.fresh || s.reset;
        if reset {
            self.prev_pos = s.pos;
            self.stopped = 0.0;
        }
        // The movement heading: frozen through a flight.
        if s.trajectory.is_none() {
            if reset {
                self.vel = Vec3::ZERO;
            } else {
                self.vel = (s.pos - self.prev_pos) * (1.0 / dt);
                self.prev_pos = s.pos;
            }
            let speed = self.vel.horizontal().length();
            if reset {
                self.movement = -angles_of(s.fwd).1;
            } else if s.grinding {
                self.movement = -angles_of(s.grind_dir).1;
            } else {
                let mut target = self.movement;
                let mut weight = 1.0;
                self.stopped = if s.wiping_out || speed > 0.5 {
                    0.0
                } else {
                    self.stopped + dt
                };
                if speed > 0.5 {
                    weight = t.heading_response.clamp(0.0, 1.0);
                    target = -angles_of(self.vel).1;
                } else if self.stopped >= t.lineup_time {
                    weight = t.lineup_speed;
                    target = -angles_of(s.fwd).1;
                }
                self.movement = blend_angle(self.movement, target, weight);
            }
        } else {
            self.prev_pos = s.pos;
        }
        let mut launch = heading_of(s.ground_up);
        let mut landing = launch;
        let mut fraction = 0.0;
        if let Some(tr) = s.trajectory {
            launch = heading_of(tr.launch_normal);
            landing = heading_of(tr.landing_normal);
            fraction = (tr.time / tr.duration).clamp(0.0, 1.0);
        }
        self.headings[1] = blend_angle(launch, landing, fraction);
        self.headings[0] = self.movement;
        if let Some(tr) = s.trajectory
            && fraction > 0.0
            && (tr.landing - tr.launch).horizontal().length() > 1.0
        {
            self.headings[4] = -angles_of(tr.dir).1;
            self.headings[0] = blend_angle(self.headings[0], self.headings[4], fraction);
        }
        self.headings[3] = heading_of(s.fwd);
        let to_subject = (s.pos - camera).horizontal();
        self.headings[6] = heading_of(to_subject);
        if selected != 7 {
            self.headings[7] = heading_of(to_subject);
        } else {
            let v = self.vel.horizontal();
            let speed = v.length();
            if speed > 0.7 {
                let most = ((speed - 0.7) * 0.435).clamp(0.0, 1.0) * 0.6;
                let target = heading_of(v);
                let previous = heading_of(to_subject);
                self.headings[7] =
                    previous + (wrap_angle(target - previous) * 0.25).clamp(-most, most);
            }
        }
        let forward = heading_of(s.fwd);
        let cam = heading_of(to_subject);
        let delta = wrap_angle(cam - forward);
        let least = 70.0 * RAD;
        self.headings[8] = if delta.abs() <= least {
            if delta >= 0.0 {
                forward + least
            } else {
                forward - least
            }
        } else {
            cam
        };
        self.fresh = dt == 0.0;
    }
}

/// The camera's distance from its anchor along the arm, after collision (SK8 positioner.rs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Positioner {
    pub distance: Scalar,
    pub position: Vec3,
    pub velocity: Vec3,
    /// The full shot distance would be clear up to here (m).
    pub available: f32,
    /// Pulled in this tick (all three lines blocked), and too close to keep the shot.
    pub hit: bool,
    pub too_close: bool,
    pub clear_time: f32,
}

impl Default for Positioner {
    fn default() -> Self {
        Positioner {
            distance: Scalar::at(2.0),
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            available: 2.0,
            hit: false,
            too_close: false,
            clear_time: 5.0,
        }
    }
}

/// The inputs of one positioner update.
pub struct Placement {
    pub anchor: Vec3,
    pub heading: f32,
    pub elevation: f32,
    pub reference_heading: f32,
    pub reference_elevation: f32,
    pub distance: f32,
    pub min_distance: f32,
    pub collision: bool,
    pub bypass: bool,
    /// Where the three lines start: centre of mass, head, feet (lifted).
    pub starts: [Vec3; 3],
}

impl Positioner {
    pub fn update(&mut self, park: &dyn Terrain, p: &Placement, t: &CamTuning, dt: f32) {
        if p.bypass {
            self.distance.target = p.distance;
            self.distance.pos = p.distance;
        } else {
            let accel = if self.distance.pos < 1.05 {
                100.0
            } else {
                t.distance_accel
            };
            self.distance.update(
                dt,
                p.distance,
                &Params::fixed(t.distance_speed, accel, t.distance_smoothing),
            );
        }
        let previous = self.position;
        self.available = p.distance;
        let d = self.distance.pos;
        self.position = p.anchor + arm(p.elevation, p.heading) * d;
        self.hit = false;
        self.too_close = false;
        if p.collision {
            // The unobstructed shot position: how much of it is clear.
            let reference = p.anchor + arm(p.reference_elevation, p.reference_heading) * p.distance;
            self.available = 0.0;
            for &a in &p.starts {
                match sweep(park, a, reference, LINE_RADIUS) {
                    Some(f) => {
                        let point = p.starts[0] + (reference - p.starts[0]) * f;
                        self.available = self.available.max((point - p.anchor).length());
                    }
                    None => self.available = p.distance,
                }
            }
            // Pulled in only when the camera can't see any of the three points.
            let hits = p.starts.map(|a| sweep(park, a, self.position, LINE_RADIUS));
            if let [Some(f0), Some(_), Some(_)] = hits {
                self.position = p.starts[0] + (self.position - p.starts[0]) * f0;
                self.hit = true;
                self.too_close =
                    (self.position - p.starts[0]).length() < LINE_RADIUS + p.min_distance;
                let d = (self.position - p.anchor).length();
                self.distance = Scalar::at(d);
            }
        }
        if dt > 0.0 {
            self.velocity = (self.position - previous) * (1.0 / dt);
        }
        self.clear_time = if self.hit { 0.0 } else { self.clear_time + dt };
    }
}

/// Raises the camera ahead of drops: ledges, stair sets, the lip of a bowl (SK8 drop_predictor.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DropPredictor {
    pub elevation: f32,
    pub target: f32,
    /// The deepest drop found ahead and how far (m).
    pub depth: f32,
    pub ahead: f32,
}

impl DropPredictor {
    pub fn reset(&mut self) {
        *self = DropPredictor::default();
    }

    pub fn update(
        &mut self,
        park: &dyn Terrain,
        pos: Vec3,
        vel: Vec3,
        enabled: bool,
        offboard: bool,
        t: &CamTuning,
    ) {
        let h = vel.horizontal();
        let mut angle = 0.0;
        self.depth = 0.0;
        if enabled {
            let span = (h.length() * t.drop_test_time).clamp(0.0, t.drop_max_test);
            let spacing = span / 5.0;
            let dir = h.normalize_or(Vec3::ZERO);
            let mut best = (0.0f32, 0usize);
            if spacing > 0.0 {
                for i in 0..5 {
                    let point = pos + dir * (spacing * (i + 1) as f32);
                    // Something in the way ahead: no probes past it.
                    if sweep(park, pos, point, 0.01).is_some() {
                        break;
                    }
                    let below = point - Vec3::UP * t.drop_max_depth;
                    let depth = sweep(park, point, below, 0.1)
                        .map_or(t.drop_max_depth, |f| f * t.drop_max_depth);
                    if depth > best.0 {
                        best = (depth, i);
                    }
                }
                if best.0 > 0.0 {
                    let ahead = (best.1 + 1) as f32 * spacing;
                    angle = atanf(best.0 / ahead).min(PI * 0.5);
                    self.depth = best.0;
                    self.ahead = ahead;
                }
            }
        }
        let limit = if offboard { 40.0 } else { 70.0 } * RAD;
        angle = angle.min(limit);
        self.target = if enabled && angle >= 15.0 * RAD {
            angle
        } else {
            0.0
        };
        let degrees = (self.target - self.elevation).abs() / RAD;
        let step = ((degrees - 7.0) * 0.0455 + 0.1).clamp(0.1, 1.6);
        if degrees > step {
            self.elevation += if self.elevation >= self.target {
                -step * RAD
            } else {
                step * RAD
            };
        } else {
            self.elevation = self.target;
        }
    }
}

/// The arm, its anchor and their trackers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rig {
    /// The selected anchor's position and velocity, and all anchors' positions (for the flight).
    pub anchor: Vec3,
    pub velocity: Vec3,
    pub acceleration: Vec3,
    pub anchors_pos: [Vec3; 7],
    pub heading: Angle,
    pub elevation: Angle,
    pub heading_target: f32,
    pub heading_reference: f32,
    pub elevation_target: f32,
    pub elevation_reference: f32,
    pub elevation_offset: f32,
    pub additional_elevation: f32,
    pub avoid_heading: f32,
    pub avoid_elevation: f32,
    pub heading_weight: f32,
    pub elevation_weight: f32,
    /// The avoidance found the subject's paths no clearer than the camera's.
    pub obstructed: bool,
    pub heading_smoothing: f32,
    pub elevation_smoothing: f32,
    pub distance: f32,
    pub tracked: Point,
    pub reset_time: f32,
    pub disable_avoid: f32,
    /// The look-at height through the air: from, to, progress, and the height itself.
    pub height_start: f32,
    pub height_end: f32,
    pub height_weight: f32,
    pub height_duration: f32,
    pub height_time: f32,
    pub reference_height: f32,
    pub previous_height: f32,
    pub height_offset: f32,
    pub landing_moved: bool,
    /// 0 riding, 1 a flight, 2 grinding; the one before and how long since the change.
    pub mode: u8,
    pub previous_mode: u8,
    pub mode_time: f32,
    pub latch_time: f32,
    pub input_time: f32,
    pub push_latched: bool,
    /// Flags from the shot: follow the board up in the air, a transition is running, avoidance is off.
    pub follow_in_air: bool,
    pub transitioning: bool,
    pub avoid_override: bool,
    /// The heading tracks (the shot doesn't hold its own compass).
    pub heading_on: bool,
    /// The anchor changed: re-aim from where the camera is.
    pub rebind: bool,
    pub reangle: bool,
    pub positioner: Positioner,
    pub breadcrumbs: [Vec3; 2],
    pub crumb: usize,
}

impl Default for Rig {
    fn default() -> Self {
        Rig {
            anchor: Vec3::ZERO,
            velocity: Vec3::ZERO,
            acceleration: Vec3::ZERO,
            anchors_pos: [Vec3::ZERO; 7],
            heading: Angle::at(PI),
            elevation: Angle::at(0.0),
            heading_target: PI,
            heading_reference: PI,
            elevation_target: 0.0,
            elevation_reference: 0.0,
            elevation_offset: 0.0,
            additional_elevation: 0.0,
            avoid_heading: 0.0,
            avoid_elevation: 0.0,
            heading_weight: 0.0,
            elevation_weight: 0.0,
            obstructed: false,
            heading_smoothing: 1.0,
            elevation_smoothing: 1.0,
            distance: 1.0,
            tracked: Point::default(),
            reset_time: 2.0,
            disable_avoid: 0.0,
            height_start: 0.0,
            height_end: 0.0,
            height_weight: 0.0,
            height_duration: 0.0,
            height_time: 0.0,
            reference_height: 1000.0,
            previous_height: f32::MAX,
            height_offset: 0.0,
            landing_moved: false,
            mode: 0,
            previous_mode: 0,
            mode_time: 0.0,
            latch_time: 0.0,
            input_time: 0.0,
            push_latched: false,
            follow_in_air: true,
            transitioning: false,
            avoid_override: false,
            heading_on: true,
            rebind: false,
            reangle: true,
            positioner: Positioner::default(),
            breadcrumbs: [Vec3::ZERO; 2],
            crumb: 0,
        }
    }
}

impl Rig {
    /// Straight to the anchor and the shot (a reset).
    pub fn teleport(&mut self) {
        self.avoid_heading = 0.0;
        self.avoid_elevation = 0.0;
        self.heading_weight = 0.0;
        self.elevation_weight = 0.0;
        self.obstructed = false;
        self.heading.snap(self.heading_target);
        self.elevation.snap(self.elevation_target);
        self.tracked = Point::at(self.anchor);
        self.reference_height = self.anchor.y;
        self.height_offset = 0.0;
        self.positioner.distance = Scalar::at(self.distance);
        self.reset_time = 0.1;
        self.disable_avoid = 2.0;
        self.height_duration = 0.0;
        self.rebind = false;
        self.latch_time = 0.0;
        self.input_time = 0.0;
        self.reangle = true;
        self.breadcrumbs = [self.anchor; 2];
    }

    /// The elevation the arm aims for, pulled towards the avoidance's when it is obstructed.
    pub fn set_elevation(&mut self, e: f32, avoidance: bool) {
        self.elevation_target = e;
        self.elevation_reference = e;
        if avoidance && self.obstructed {
            self.elevation_target = blend_angle(e, self.avoid_elevation, self.elevation_weight);
        }
        self.elevation_target = wrap_angle(self.elevation_target);
    }

    /// Ground, flight or grind (SK8 rig_framing.rs `RigMode`): what the subject is doing decides.
    fn update_mode(&mut self, s: &Subject) {
        let next = if s.ground {
            0
        } else if s.grinding {
            2
        } else if s.trajectory.is_some() {
            1
        } else {
            return;
        };
        if next != self.mode {
            self.previous_mode = self.mode;
            self.mode = next;
            self.mode_time = 0.0;
        }
    }

    /// One tick of the arm (SK8 rig.rs `Rig::update`).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        park: &dyn Terrain,
        s: &Subject,
        t: &CamTuning,
        dt: f32,
        min_distance: f32,
        collision: bool,
    ) {
        self.reset_time = (self.reset_time - dt).max(0.0);
        self.disable_avoid = (self.disable_avoid - dt).max(0.0);
        self.mode_time += dt;
        self.update_mode(s);
        self.avoid(park, s, t, dt);
        self.update_height(s, dt);
        self.update_heading(t, dt);
        self.update_elevation(t, dt);
        self.update_anchor(s, t, dt);
        let crumb = v3(
            self.tracked.pos.x,
            self.reference_height,
            self.tracked.pos.z,
        );
        if (crumb - self.breadcrumbs[self.crumb]).length() > 1.0 {
            self.crumb = (self.crumb + 1) % 2;
            self.breadcrumbs[self.crumb] = crumb;
        }
        let anchor = v3(
            self.tracked.pos.x,
            self.reference_height,
            self.tracked.pos.z,
        );
        if self.rebind {
            // The anchor changed: keep the camera where it is, re-aimed from the new anchor.
            let from = self.positioner.position - anchor;
            let (e, yaw) = angles_of(from);
            self.heading_target = -yaw;
            self.elevation_target = e - self.elevation_offset - self.additional_elevation;
            self.distance = from.length();
            self.heading.snap(self.heading_target);
            self.elevation.snap(self.elevation_target);
        }
        let reference_heading = self.heading.pos();
        let reference_elevation =
            (self.elevation_offset + self.elevation.pos()).clamp(-MAX_ELEVATION, MAX_ELEVATION);
        let elevation = (self.additional_elevation + self.elevation_offset + self.elevation.pos())
            .clamp(-MAX_ELEVATION, MAX_ELEVATION);
        if self.reset_time > 0.0 {
            self.positioner.distance = Scalar::at(self.distance);
        }
        let body_up = s.offset_dir;
        let placement = Placement {
            anchor,
            heading: self.heading.pos(),
            elevation,
            reference_heading,
            reference_elevation,
            distance: if self.distance.is_nan() {
                1.0
            } else {
                self.distance
            },
            min_distance,
            collision,
            bypass: self.rebind,
            starts: [
                s.refs[super::shot::REF_COM],
                s.refs[super::shot::REF_HEAD],
                s.refs[super::shot::REF_FEET] + body_up * 0.15,
            ],
        };
        self.rebind = false;
        self.positioner.update(park, &placement, t, dt);
    }

    /// Avoidance mode 1: when the camera's path ahead runs into something before the skater's does,
    /// swing the arm towards the skater's breadcrumb trail (SK8 avoidance.rs).
    fn avoid(&mut self, park: &dyn Terrain, s: &Subject, t: &CamTuning, dt: f32) {
        let speed = self.velocity.dot(s.fwd);
        if speed < 0.5 {
            return;
        }
        if self.avoid_override || self.disable_avoid > 0.0 {
            self.heading_weight = 0.0;
            self.elevation_weight = 0.0;
            return;
        }
        let anchor = v3(
            self.tracked.pos.x,
            self.reference_height,
            self.tracked.pos.z,
        );
        let crumb = self.breadcrumbs[(self.crumb + 1) % 2];
        let to = crumb - anchor;
        if to.length() > 1e-4 {
            let (e, yaw) = angles_of(to);
            self.avoid_heading = -yaw;
            self.avoid_elevation = e;
        } else {
            return;
        }
        let horizon = t.avoid_horizon.max(0.05);
        let path = |from: Vec3, radius: f32| -> f32 {
            let to = from + self.velocity * horizon;
            sweep(park, from, to, radius).map_or(f32::MAX, |f| f * horizon)
        };
        let camera_time = path(self.positioner.position, t.avoid_padding + LINE_RADIUS);
        let second = if self.mode == 1 {
            self.anchor + arm(self.avoid_elevation, self.avoid_heading) * self.distance
        } else {
            v3(self.anchor.x, self.positioner.position.y, self.anchor.z)
        };
        let second_time = path(second, 0.02);
        let third_time = path(s.refs[super::shot::REF_HIPS], 0.02);
        let other = second_time.max(third_time);
        if third_time >= second_time {
            self.obstructed = true;
        }
        if s.off_board {
            // Nothing.
        } else if self.positioner.too_close {
            self.heading_weight += dt * 0.5;
            self.elevation_weight += dt * 0.5;
        } else if self.mode != 1 && other > camera_time {
            let target = (1.0 - camera_time / horizon).clamp(0.0, 1.0);
            let h = self.heading_weight.max(target);
            let e = self.elevation_weight.max(target);
            self.heading_weight += (1.0 - t.avoid_heading_smoothing) * (h - self.heading_weight);
            self.elevation_weight +=
                (1.0 - t.avoid_elevation_smoothing) * (e - self.elevation_weight);
        } else if (speed > 0.15 && self.reset_time <= 0.0) || s.wiping_out {
            let ease = dt / t.avoid_ease_out.max(1e-3);
            self.heading_weight = (self.heading_weight - ease).max(0.0);
            self.elevation_weight = (self.elevation_weight - ease).max(0.0);
            self.obstructed = false;
        }
        self.heading_weight = self.heading_weight.clamp(0.0, 1.0);
        self.elevation_weight = self.elevation_weight.clamp(0.0, 1.0);
    }

    /// The look-at height (SK8 rig_tracking.rs `ReferenceHeightTracking`): with the board on the ground
    /// or a shot that follows it into the air, the anchor's height; otherwise a ramp from the take-off
    /// to the landing height over the flight, never above the anchor.
    fn update_height(&mut self, s: &Subject, dt: f32) {
        let anchor_height = self.anchor.y;
        let immediate = self.follow_in_air;
        if immediate {
            self.height_duration = 0.0;
        }
        let old = self.reference_height;
        let mut target = old;
        let blending = !immediate && self.height_time < self.height_duration;
        if blending {
            let k = (self.height_time / self.height_duration).clamp(0.0, 1.0);
            target = self.height_start + (self.height_end - self.height_start) * k;
            if self.landing_moved {
                let w = self.height_weight * self.height_weight;
                target = target * (1.0 - w) + anchor_height * w;
            }
        } else if self.mode == 0 || self.mode == 2 {
            self.previous_height = old;
            target = anchor_height;
        } else if immediate || old > anchor_height {
            target = anchor_height;
        }
        self.reference_height = old + (target - old);
        if s.off_board {
            let d = self.reference_height - anchor_height;
            if d.abs() > 1e-4 {
                self.height_offset = d;
                if d > 0.0 {
                    self.reference_height = anchor_height;
                    self.height_offset = 0.0;
                }
            } else if self.height_offset.abs() > 0.005 {
                self.height_offset *= 0.9;
                self.reference_height += self.height_offset;
            } else {
                self.height_offset = 0.0;
            }
        } else {
            self.height_offset = self.reference_height - anchor_height;
        }
        self.height_time += dt;
    }

    /// The heading tracker: smoothing that lets go as the error grows (1 - `h^2` near the target, at
    /// most 0.8 from 45 degrees off), acceleration clamped harder near the target (SK8 rig_tracking.rs
    /// `update_heading`).
    fn update_heading(&mut self, t: &CamTuning, dt: f32) {
        if !self.heading_on && self.heading_weight == 0.0 {
            return;
        }
        let err = wrap_angle(self.heading_target - self.heading.pos()).abs();
        let smoothing = 1.0 - self.heading_smoothing;
        let capped = smoothing.min(0.8);
        let smoothing = if err < PI / 4.0 {
            smoothing + (capped - smoothing) * err * (4.0 / PI)
        } else {
            capped
        };
        if self.reset_time > 0.0 {
            self.heading.snap(self.heading_target);
        }
        let mut p = angle_params(&t.heading, smoothing);
        if self.heading_weight > 0.0 {
            p.accel_far *= 2.0;
            p.accel_near *= 2.0;
        }
        p.overshoot_stops = false;
        self.heading.update(dt, self.heading_target, &p);
    }

    fn update_elevation(&mut self, t: &CamTuning, dt: f32) {
        if self.reset_time > 0.0 {
            self.elevation.snap(self.elevation_target);
        }
        let p = angle_params(&t.elevation, 1.0 - self.elevation_smoothing);
        self.elevation.update(dt, self.elevation_target, &p);
    }

    /// The anchor's tracker: tight, but lagging while pushing (and half a second after) or pumping, less
    /// so from 8 m/s and not at all from 12 (SK8 rig_tracking.rs `AnchorRigTracking`).
    fn update_anchor(&mut self, s: &Subject, t: &CamTuning, dt: f32) {
        self.latch_time = (self.latch_time - dt).max(0.0);
        if !s.pushing {
            self.push_latched = false;
        } else if s.at_pushable_speed {
            self.push_latched = true;
        }
        if self.push_latched {
            self.latch_time = 0.5;
        }
        let latch = t
            .push_curve
            .eval(1.0 - 2.0 * self.latch_time)
            .clamp(0.0, 1.0)
            * t.push_max;
        self.input_time = (self.input_time - dt).max(0.0);
        if s.pumping > t.pump_min_accel {
            self.input_time = 0.5;
        }
        let input = t
            .pump_curve
            .eval(1.0 - 2.0 * self.input_time)
            .clamp(0.0, 1.0)
            * t.pump_max;
        let smoothing = if s.wiping_out || self.rebind {
            0.0
        } else {
            latch.max(input) * ((self.velocity.length() - 8.0) * -0.25).clamp(0.0, 1.0)
        };
        let p = Params::fixed(t.subject_speed, t.subject_accel, smoothing);
        let target = v3(self.anchor.x, self.reference_height, self.anchor.z);
        self.tracked.update(dt, target, &p);
    }
}

/// The shot's arm and options handed to the rig (SK8 manager_rig.rs `bind_shot`): heading and elevation
/// targets, drop, free look and collision raises, smoothing. Returns the heading mirror for the next
/// tick's shot.
#[allow(clippy::too_many_arguments)]
pub fn bind_shot(
    state: &mut State,
    rig: &mut Rig,
    shot: &Shot,
    s: &Subject,
    drop: &DropPredictor,
    look_elevation: f32,
    drop_on: bool,
    reset: bool,
    ground: bool,
    transitioning: bool,
) -> (f32, f32) {
    rig.transitioning = transitioning;
    rig.follow_in_air = shot.has(OPT_FOLLOW);
    rig.avoid_override = shot.has(OPT_AVOID);
    let elevation = shot.arm_elevation;
    let heading = shot.arm_heading;
    let distance = shot.distance;
    let mut drop_elevation = 0.0;
    if drop_on {
        let speed = state.anchor_vel.length();
        let scale = (1.0 - (speed - 4.0) * 0.025).clamp(0.9, 1.0);
        drop_elevation = drop.elevation * scale;
    }
    let collision_elevation = if reset {
        0.0
    } else {
        let most = distance.min(2.5);
        let fraction = ((most - rig.positioner.available) / most).clamp(0.0, 1.0);
        let angle = if s.off_board { 60.0 } else { 50.0 } * RAD;
        let mut target = (angle * fraction - drop_elevation).max(0.0);
        if ground && s.ground_normal.y < -0.2 {
            target = -target;
        }
        let outward = (target > 0.0 && state.collision_elevation < target)
            || (target < 0.0 && state.collision_elevation > target);
        let step = angle * if outward { 1.0 / 15.0 } else { 0.01 };
        state.collision_elevation = if (target - state.collision_elevation).abs() > step {
            if target >= state.collision_elevation {
                state.collision_elevation + step
            } else {
                state.collision_elevation - step
            }
        } else {
            target
        };
        state
            .collision_elevation
            .clamp(-MAX_ELEVATION, MAX_ELEVATION)
    };
    rig.distance = distance;
    rig.heading_reference = wrap_angle(heading);
    rig.heading_target = if reset {
        rig.heading_reference
    } else {
        blend_angle(
            rig.avoid_heading,
            rig.heading_reference,
            1.0 - rig.heading_weight,
        )
    };
    rig.set_elevation(elevation, !reset);
    rig.elevation_offset = drop_elevation + look_elevation;
    rig.additional_elevation = collision_elevation;
    rig.heading_on = shot.compass != 5;
    let heading_mirror = if shot.compass == 2 {
        1.0
    } else {
        state.heading_mirror.pos
    };
    let framing_mirror = if shot.compass == 2 {
        1.0
    } else {
        state.framing_mirror.pos
    };
    let h = 1.0 - shot.smoothing[0];
    let e = 1.0 - shot.smoothing[1];
    rig.heading_smoothing = h * h;
    rig.elevation_smoothing = e * e;
    (heading_mirror, framing_mirror)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arm_points_behind_for_heading_180() {
        // Heading 180 with north 0 (facing +z): the arm points back along -z.
        let a = arm(0.0, PI);
        assert!(a.z < -0.99, "{a:?}");
        // Elevation lifts it.
        assert!(arm(0.3, PI).y > 0.29);
        // Heading -h is flatground's yaw.
        let (_, yaw) = angles_of(arm(0.0, 0.5));
        assert!((yaw + 0.5).abs() < 1e-5);
    }
}
