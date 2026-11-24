//! The wipeout: the ragdoll (`skater::ragdoll`) run as Skate 3 runs its physical wipeout state
//! (SK8-ENGINE/skate-3-rust-engine, GPL-3.0: `skate-game/src/physics/wipeout_states/update.rs`,
//! `lifecycle.rs`, `prediction.rs`; `skate-core/src/player/wipeout_state/`: `profiles.rs`,
//! `control_profile_air.rs`, `control_ground.rs`, `response.rs`, `weights.rs`, `recovery.rs`).
//!
//! - The wipeout gesture (the C-stick, blended by the motion graph's `Wipeout` behaviour, or here
//!   without the graphs) picks one of Skate 3's five control profiles (`tuning::wipeout`): centred free
//!   fall, up cannon ball (a tuck), right judo kick, down swan dive, left torpedo; the profile is
//!   `GestureType` to the graphs and `IsWipeoutBodyTweak` to the camera.
//! - The control (the main stick, `WipeoutControlX/Y`, in the world as the camera sees it) steers the
//!   fall: in the air the profile's spin about the body and across the travel, its tilt, its alignment
//!   with the travel and its drift; on the ground its roll along the ground and alignment.
//! - The push off (`WipeOutPushOff`, R fully in) kicks the body along the stick off what it lies on,
//!   with Skate 3's response strength.
//! - Drives pull the body towards the animated pose (the imported rider's clips, else the pose at the
//!   bail) fading out over `TimeToRemoveDrives`, and towards the gesture's pose by the controlled weight
//!   (`DynamicDriveWeightVelControlled` a tick), the collision weight going limp on contact.
//! - Over (`IsDoneWipingOut`; the recovery's settling): the hips and head under `WipeoutOverSpeed` after
//!   `WipeoutOverMinTime` with no response strength left; `WipeoutTimeToLand` (the centre of mass's fall
//!   to the ground) and `WipeoutTimeSinceContact` for the graphs.

use super::meat::Meat;
use super::ragdoll::{self, Body, Frame, PARTS, POINTS, Settings};
use crate::math::{PI, TAU, Vec3, v3, wrap_angle};
use crate::park::Terrain;
use crate::tuning::Tuning;
use crate::tuning::wipeout::Profile;

/// The share of the gap to the drives' targets closed a tick at full weight (ours: Skate 3's drive
/// strengths are in its physical skeleton's bank).
pub const DRIVE_GAIN: f32 = 0.35;
/// Speeds are capped at the bail (SK8 `WipeoutEnter`: `limit_velocity(20)`).
const ENTER_MAX_SPEED: f32 = 20.0;
/// The landing prediction's reach and step (s) and the centre of mass's radius (m) (SK8's query: 3 s,
/// 0.3 m).
const PREDICT_TIME: f32 = 3.0;
const PREDICT_STEP: f32 = 0.1;
const PREDICT_RADIUS: f32 = 0.3;
/// Skate 3's Wipeout behaviour lets the gesture through after this many ticks anyway.
const GESTURE_RELEASE_TICKS: u32 = 80;

/// The gesture poses in the pelvis's frame (x right, y up, z forwards; m, ours): where the drives pull
/// a cannon ball (a tuck), a judo kick (one leg out), a swan dive (arms spread, legs straight) and a
/// torpedo (arms at the sides) without the imported clips. Free fall holds the pose of the bail.
const GESTURE_POSES: [[Vec3; POINTS]; 4] = [
    [
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.4, 0.14),
        v3(0.0, 0.62, 0.24),
        v3(-0.1, -0.02, 0.0),
        v3(0.1, -0.02, 0.0),
        v3(-0.17, 0.42, 0.12),
        v3(0.17, 0.42, 0.12),
        v3(-0.24, 0.24, 0.3),
        v3(-0.12, 0.1, 0.45),
        v3(0.24, 0.24, 0.3),
        v3(0.12, 0.1, 0.45),
        v3(-0.11, 0.24, 0.36),
        v3(-0.1, -0.1, 0.3),
        v3(0.11, 0.24, 0.36),
        v3(0.1, -0.1, 0.3),
    ],
    [
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.44, 0.05),
        v3(0.0, 0.72, 0.08),
        v3(-0.1, -0.02, 0.0),
        v3(0.1, -0.02, 0.0),
        v3(-0.17, 0.47, 0.04),
        v3(0.17, 0.47, 0.04),
        v3(-0.42, 0.52, 0.0),
        v3(-0.66, 0.55, 0.05),
        v3(0.3, 0.3, 0.2),
        v3(0.25, 0.42, 0.42),
        v3(-0.12, 0.12, 0.32),
        v3(-0.1, -0.28, 0.24),
        v3(0.12, -0.06, 0.44),
        v3(0.12, -0.08, 0.88),
    ],
    [
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.45, -0.02),
        v3(0.0, 0.73, 0.04),
        v3(-0.1, -0.02, 0.0),
        v3(0.1, -0.02, 0.0),
        v3(-0.17, 0.47, -0.02),
        v3(0.17, 0.47, -0.02),
        v3(-0.42, 0.47, -0.08),
        v3(-0.66, 0.44, -0.18),
        v3(0.42, 0.47, -0.08),
        v3(0.66, 0.44, -0.18),
        v3(-0.07, -0.47, -0.02),
        v3(-0.06, -0.9, -0.06),
        v3(0.07, -0.47, -0.02),
        v3(0.06, -0.9, -0.06),
    ],
    [
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.45, 0.0),
        v3(0.0, 0.74, 0.0),
        v3(-0.1, -0.02, 0.0),
        v3(0.1, -0.02, 0.0),
        v3(-0.17, 0.47, 0.0),
        v3(0.17, 0.47, 0.0),
        v3(-0.22, 0.2, 0.0),
        v3(-0.22, -0.06, 0.02),
        v3(0.22, 0.2, 0.0),
        v3(0.22, -0.06, 0.02),
        v3(-0.06, -0.47, 0.0),
        v3(-0.05, -0.92, 0.0),
        v3(0.06, -0.47, 0.0),
        v3(0.05, -0.92, 0.0),
    ],
];

/// The players' input to the wipeout for a tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// `WipeoutControlX/Y`: the main stick in the world (x, z), as the camera sees it.
    pub control: [f32; 2],
    /// `WipeoutGestureX/Y` as the motion graph's Wipeout behaviour blended it (or this module's own
    /// blend of the C-stick without the graphs, [`Wipeout::gesture_from_stick`]).
    pub gesture: [f32; 2],
    /// `WipeOutPushOff` this tick.
    pub push_off: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wipeout {
    pub body: Body,
    pub active: bool,
    /// Seconds since the bail.
    pub time: f32,
    /// The control profile the gesture picked (0 free fall .. 4 torpedo).
    pub profile: usize,
    forward: Vec3,
    right: Vec3,
    direction_set: bool,
    forward_input: f32,
    sideways_input: f32,
    retained_forward: f32,
    retained_sideways: f32,
    retained_tilt: Vec3,
    control_time: f32,
    collision_weight: f32,
    controlled_weight: f32,
    /// The drives' strength this tick (0..`DRIVE_GAIN`).
    pub drive: f32,
    /// Settled (Skate 3's `over`): what the recovery and `IsDoneWipingOut` read.
    pub over: bool,
    slow_time: f32,
    maximum_speed: f32,
    response_time: f32,
    response_count: u32,
    airborne_frames: i32,
    /// `WipeoutTimeSinceContact` (s), `WipeoutTimeToLand` (s; `f32::MAX` none found).
    pub no_support_time: f32,
    pub predicted_time: f32,
    /// The pose at the bail in the pelvis's frame (free fall's targets).
    seed_targets: [Vec3; POINTS],
    /// The imported rider's animated pose to drive towards this tick (world), when the game has one.
    pub anim: Option<[Vec3; POINTS]>,
    /// The wipeout gesture blended from the C-stick without the graphs (Skate 3's Wipeout behaviour),
    /// its release and ticks.
    pub gesture: [f32; 2],
    gesture_released: bool,
    gesture_ticks: u32,
    /// Hall of Meat: the injuries so far.
    pub meat: Meat,
}

impl Default for Wipeout {
    fn default() -> Self {
        Wipeout {
            body: Body::default(),
            active: false,
            time: 0.0,
            profile: 0,
            forward: v3(0.0, 0.0, 1.0),
            right: v3(1.0, 0.0, 0.0),
            direction_set: false,
            forward_input: 0.0,
            sideways_input: 0.0,
            retained_forward: 0.0,
            retained_sideways: 0.0,
            retained_tilt: Vec3::ZERO,
            control_time: 0.0,
            collision_weight: 0.0,
            controlled_weight: 0.0,
            drive: 0.0,
            over: false,
            slow_time: 0.0,
            maximum_speed: 10.0,
            response_time: 0.0,
            response_count: 0,
            airborne_frames: 15,
            no_support_time: 0.0,
            predicted_time: f32::MAX,
            seed_targets: ragdoll::rest_local(),
            anim: None,
            gesture: [0.0; 2],
            gesture_released: false,
            gesture_ticks: 0,
            meat: Meat::default(),
        }
    }
}

/// The standing rider's points placed by `frame` (the placeholder's board space: goofy, facing +x)
/// and turned round for a mirrored rider.
pub fn built_in_pose(place: &crate::math::Xform, mirrored: bool) -> [Vec3; POINTS] {
    let mut out = ragdoll::REST;
    for p in &mut out {
        let q = if mirrored { v3(-p.x, p.y, -p.z) } else { *p };
        *p = place.point(q);
    }
    // A mirrored placeholder is turned half round, not reflected: its left and right swap places.
    out
}

impl Wipeout {
    /// Starts the wipeout with the body at `points` (world) moving at `vel`.
    pub fn start(&mut self, points: &[Vec3; POINTS], vel: Vec3, dt: f32) {
        *self = Wipeout::default();
        self.body = Body::new(points, vel, dt);
        self.body.limit_speed(ENTER_MAX_SPEED, dt);
        self.seed_targets = self.body.target;
        self.active = true;
    }

    /// Skate 3's Wipeout behaviour without the graphs: the C-stick blended into the gesture once let
    /// go to the middle (or after 80 ticks, or on a bail request), at `anim_wipeout`'s blends.
    pub fn gesture_from_stick(&mut self, stick: [f32; 2], request: bool, blend: [f32; 2]) {
        if request || self.gesture_ticks > GESTURE_RELEASE_TICKS {
            self.gesture_released = true;
        }
        if !self.gesture_released && stick[0].abs() < 0.1 && stick[1].abs() < 0.1 {
            self.gesture_released = true;
        }
        if self.gesture_released {
            for k in 0..2 {
                self.gesture[k] = (1.0 - blend[k]) * self.gesture[k] + stick[k] * blend[k];
            }
        }
        self.gesture_ticks += 1;
    }

    /// Skate 3's response strength (SK8 `State::response_strength`): what a push off still has.
    pub fn response_strength(&self) -> f32 {
        let speed = self.maximum_speed * 0.1;
        let count = (4i32 - self.response_count as i32) as f32;
        let remaining = (count + 1.0) * 0.5;
        let r = speed.max(remaining);
        if self.slow_time > 1.0 || r <= 0.2 {
            0.0
        } else {
            r
        }
    }

    /// The body's lean and twist for the bail trees (SK8 `output`: the hips' right against the
    /// surface's up, and the hips' up about their right against it).
    pub fn hips(&self) -> [f32; 2] {
        let n = if self.body.contacts.any() {
            self.body.contacts.normal()
        } else {
            Vec3::UP
        };
        let [r, u, _] = self.body.axes();
        let right_angle = signed_angle(r, n, Vec3::ZERO);
        let projected = n - r * n.dot(r);
        let up_angle = wrap_angle(signed_angle(projected, u, r) + PI);
        [right_angle, up_angle]
    }

    /// One tick.
    pub fn step(&mut self, park: &dyn Terrain, t: &Tuning, input: Input, dt: f32) {
        let w = &t.wipeout;
        let contact = self.body.contacts.any();
        self.airborne_frames = if contact {
            -1
        } else {
            self.airborne_frames.saturating_add(1)
        };
        self.time += dt;
        self.response_time += dt;
        self.slow_time += dt;
        let vel = self.body.com_vel(dt);
        let speed = vel.length();
        self.maximum_speed = self.maximum_speed.max(speed);
        if speed > 2.0 {
            self.slow_time = 0.0;
        }
        self.prepare(w.profiles.as_ref(), input, vel, dt);
        let profile = w.profiles[self.profile];
        let normal = self.body.contacts.normal();
        let mut control_applied = false;
        if input.push_off && contact && self.response_time >= 0.1 {
            self.push_off(normal, input.control, dt);
        } else if self.airborne_frames > 15 && !self.over {
            self.control_air(&profile, input.control, vel, dt);
            self.drift(&profile, vel, dt);
            control_applied = true;
        } else if !self.over && contact {
            self.control_ground(&profile, normal, vel, dt);
            control_applied = true;
        }
        // The weights (SK8 `weights::update`): the drives fade with the time, the collision weight
        // grows on contact with the gesture centred, the controlled weight while controlled.
        let centred = input.gesture[0].abs() < 0.2 && input.gesture[1].abs() < 0.2;
        if centred {
            let step = if contact {
                w.collision_step
            } else {
                -w.collision_step
            };
            self.collision_weight = (self.collision_weight + step).clamp(0.0, 1.0);
        } else {
            self.collision_weight = 0.0;
        }
        let fade = fade(w.remove_drives_time, self.time);
        let step = if control_applied {
            w.controlled_step
        } else {
            -w.controlled_step
        };
        self.controlled_weight = (self.controlled_weight + step).clamp(0.0, 1.0);
        let available = 1.0 - self.collision_weight;
        let start = available * fade * fade;
        let weight = start + (available - start) * self.controlled_weight;
        self.drive = DRIVE_GAIN * weight.clamp(0.0, 1.0);
        // Where the drives pull: the animated pose, else the gesture's, else the pose of the bail.
        if let Some(a) = self.anim {
            self.body.set_targets(&a);
        } else if self.profile > 0 {
            self.body
                .set_local_targets(&GESTURE_POSES[self.profile - 1]);
        } else {
            self.body.set_local_targets(&self.seed_targets);
        }
        let settings = Settings {
            gravity: t.gravity,
            angular_drag: w.angular_drag,
            linear_drag: w.linear_drag,
            friction: w.friction,
            restitution: w.restitution,
            head_friction: w.head_friction,
            head_restitution: w.head_restitution,
            drive: self.drive,
        };
        self.body.step(park, &settings, dt);
        self.meat.take(&self.body.contacts);
        // Settled (SK8 `post_physics`).
        let over = t.settled_speed;
        let hips = self.body.vel(ragdoll::PELVIS, dt).length();
        let neck = self.body.vel(ragdoll::HEAD, dt).length();
        self.over = hips < over
            && neck < over
            && self.time > t.settled_min_time
            && self.response_strength() == 0.0;
        let touching = self.body.contacts.any();
        self.no_support_time = if touching {
            0.0
        } else {
            self.no_support_time + dt
        };
        self.predicted_time = if touching {
            0.0
        } else {
            predict(park, self.body.com(), self.body.com_vel(dt), t.gravity)
        };
    }

    /// The profile from the gesture, the travel's frame and the stick in it (SK8 `profiles::prepare`).
    fn prepare(&mut self, profiles: &[Profile], input: Input, vel: Vec3, dt: f32) {
        let [x, z] = input.gesture;
        let selected = if x.abs() < 0.2 && z.abs() < 0.2 {
            0
        } else if x.abs() <= z.abs() {
            if z <= 0.0 { 3 } else { 1 }
        } else if x <= 0.0 {
            4
        } else {
            2
        };
        self.control_time += dt;
        if selected != self.profile {
            self.control_time = 0.0;
            self.profile = selected;
            self.retained_sideways = 0.0;
            self.retained_forward = 0.0;
        }
        let flat = v3(vel.x, 0.0, vel.z).normalize_or(Vec3::ZERO);
        self.forward = if self.direction_set {
            let k = profiles[selected].direction_follow;
            (flat * k + self.forward * (1.0 - k)).normalize_or(Vec3::ZERO)
        } else {
            self.direction_set = true;
            flat
        };
        self.right = Vec3::UP.cross(self.forward);
        let c = v3(input.control[0], 0.0, input.control[1]);
        self.forward_input = c.dot(self.forward);
        self.sideways_input = c.dot(self.right);
    }

    /// The body's frame as Skate 3's effective transform: right, up, front.
    fn frame(&self) -> [Vec3; 3] {
        self.body.axes()
    }

    /// SK8 `control_profile_air::update`.
    fn control_air(&mut self, p: &Profile, control: [f32; 2], vel: Vec3, dt: f32) {
        let omega = self.body.angular_velocity(dt);
        let rad = PI / 180.0;
        let tilt_input = v3(
            control[1] * p.tilt_degrees * rad,
            0.0,
            control[0] * p.tilt_degrees * -rad,
        );
        self.retained_tilt = tilt_input * 0.05 + self.retained_tilt * 0.95;
        let tilt = quat_of(self.retained_tilt);
        let f = self.frame();
        let up = alignment_up(p.align_euler[0], p.align_euler[1]);
        let desired = f[0] * up.x + f[1] * up.y + f[2] * up.z;
        let side = self.right.normalize_or(Vec3::ZERO);
        let projected = vel - side * vel.dot(side);
        let direction = if self.forward.dot(projected) <= 0.0 {
            (desired + v3(0.0, -100.0, 0.0)).normalize_or(-Vec3::UP)
        } else {
            projected.normalize_or(desired)
        };
        let tilted = rotate(tilt, direction);
        let axis = tilted.cross(desired).normalize_or(desired);
        let angle = wrap_angle(signed_angle(tilted, desired, axis));
        let distance = if p.align_with_velocity {
            axis * (angle * p.torque_distance)
        } else {
            Vec3::ZERO
        };
        let side_axis = rotate(tilt, desired);
        let forward_axis = rotate(tilt, direction.cross(Vec3::UP).normalize_or(Vec3::ZERO));
        self.retained_sideways =
            retain(self.sideways_input, self.retained_sideways, p.spin_inertia);
        self.retained_forward = retain(self.forward_input, self.retained_forward, p.spin_inertia);
        let side_spin = p.sideways_spin * self.retained_sideways;
        let forward_spin = p.forward_spin * self.retained_forward;
        let spin = forward_axis * forward_spin + side_axis * side_spin;
        if spin.dot(omega) <= 0.0 {
            self.control_time = 0.0;
        }
        let spin = spin * p.spin_vs_time.eval(self.control_time);
        let horizontal = if side_spin.abs() <= 0.1 {
            horizontal_alignment(p, &f, direction, tilt)
        } else {
            Vec3::ZERO
        };
        let control = (spin - omega) * p.torque_velocity + distance + horizontal;
        self.body.add_spin(control, dt);
    }

    /// SK8 `profiles::drift`: the stick pushes the fall across and along the travel, with drag.
    fn drift(&mut self, p: &Profile, vel: Vec3, dt: f32) {
        let desired = self.right * (p.drift_sideways * self.sideways_input)
            + self.forward * (p.drift_forward * self.forward_input);
        let mut change = vel * -p.drift_drag;
        change.x *= 0.02;
        change.z *= 0.02;
        if vel.dot(desired.normalize_or(Vec3::ZERO)) < p.drift_max_speed {
            change += desired;
        }
        self.body.add_velocity(change * dt, dt);
    }

    /// SK8 `control_ground::update`: rolling along the ground with the stick (hands together) and
    /// lining up with the travel.
    fn control_ground(&mut self, p: &Profile, normal: Vec3, vel: Vec3, dt: f32) {
        let omega = self.body.angular_velocity(dt);
        let current = omega.normalize_or(Vec3::ZERO);
        let speed = vel.length();
        let f = self.frame();
        let mut rolling_axis =
            (f[0] * p.roll_axis[0] + f[1] * p.roll_axis[1] + f[2] * p.roll_axis[2])
                .normalize_or(Vec3::ZERO);
        let b = &self.body.pos;
        let hands_close = (b[ragdoll::L_HAND] - b[ragdoll::R_HAND]).length() <= 0.6;
        let mut alignment = Vec3::ZERO;
        if p.align_ground_with_velocity {
            let axis = normal.cross(vel).normalize_or(Vec3::ZERO);
            if rolling_axis.dot(axis) < 0.0 {
                rolling_axis = -rolling_axis;
            }
            let perpendicular = rolling_axis.cross(axis);
            let sine = perpendicular.length().clamp(-0.9999, 0.9999);
            let angle = libm::asinf(sine);
            let magnitude = (angle * 1.2).clamp(0.0, 1.0);
            if speed > 2.0 {
                let a = perpendicular.normalize_or(Vec3::ZERO);
                alignment = (a * magnitude - a * current.dot(a)) * p.ground_align_torque;
            }
        }
        let mut rolling = Vec3::ZERO;
        if hands_close && p.roll_on_ground && speed > 2.0 {
            rolling = (rolling_axis * (1.5 * self.forward_input)
                - rolling_axis * current.dot(rolling_axis))
                * p.ground_roll_torque;
        }
        self.body.add_spin(alignment + rolling, dt);
    }

    /// SK8 `response::trigger`: a push off the surface along the stick (1.25 m/s at full strength).
    fn push_off(&mut self, normal: Vec3, control: [f32; 2], dt: f32) {
        let strength = self.response_strength();
        if self.maximum_speed * 0.1 < 1.0 {
            self.response_count += 1;
        } else {
            self.response_count = 0;
        }
        self.maximum_speed = 0.0;
        if strength == 0.0 {
            return;
        }
        self.response_time = 0.0;
        let strength = strength.min(1.0);
        let mut c = v3(control[0], 0.0, control[1]);
        let into = normal.dot(c);
        if into < 0.0 {
            c -= normal * into;
        }
        self.body.add_velocity(c * (1.25 * strength), dt);
    }

    /// The body parts' frames between the last two ticks (`k` 0: a tick ago, 1: now).
    pub fn frames(&self, k: f32) -> [Frame; PARTS] {
        let p = self.body.between(k);
        self.body.frames(&p)
    }
}

fn fade(duration: f32, time: f32) -> f32 {
    let d = if duration == 0.0 { 0.01 } else { duration };
    ((duration - time) / d).clamp(0.0, 1.0)
}

fn retain(input: f32, previous: f32, inertia: f32) -> f32 {
    if input.abs() >= previous.abs() {
        input
    } else {
        (1.0 - inertia) * input + previous * inertia
    }
}

/// SK8 `orientation::signed_angle`: the angle from `a` to `b`, the far way round (2 pi less) when it
/// turns against `axis`.
fn signed_angle(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    if a.dot(a) <= 1e-4 || b.dot(b) <= 1e-4 {
        return 0.0;
    }
    let (a, b) = (a.normalize_or(Vec3::UP), b.normalize_or(Vec3::UP));
    let angle = libm::acosf(a.dot(b).clamp(-1.0, 1.0));
    if a.cross(b).dot(axis) < 0.0 {
        TAU - angle
    } else {
        angle
    }
}

/// A rotation vector as a quaternion (x, y, z, w).
fn quat_of(v: Vec3) -> [f32; 4] {
    let l = v.length();
    if l <= 0.01 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let (s, c) = (libm::sinf(l * 0.5), libm::cosf(l * 0.5));
    let a = v * (s / l);
    [a.x, a.y, a.z, c]
}

/// SK8 `orientation::rotate`.
fn rotate(q: [f32; 4], v: Vec3) -> Vec3 {
    let qv = v3(q[0], q[1], q[2]);
    let first = qv.cross(v);
    let middle = v * q[3] + first;
    v + qv.cross(middle) * 2.0
}

/// SK8 `orientation::alignment_up`: the up axis turned by Euler x then y (degrees).
fn alignment_up(x_deg: f32, y_deg: f32) -> Vec3 {
    let rad = PI / 180.0;
    let (sx, cx) = (libm::sinf(x_deg * rad * 0.5), libm::cosf(x_deg * rad * 0.5));
    let (sy, cy) = (libm::sinf(y_deg * rad * 0.5), libm::cosf(y_deg * rad * 0.5));
    let q = [cy * sx, sy * cx, -sy * sx, cy * cx];
    rotate(q, Vec3::UP)
}

/// SK8 `control_profile_air::horizontal_alignment` (its cross product of the profile's local axis with
/// the world target kept as Skate 3 has it).
fn horizontal_alignment(p: &Profile, f: &[Vec3; 3], direction: Vec3, tilt: [f32; 4]) -> Vec3 {
    let a = p.horizontal_axis;
    let local = v3(a[0], a[1], a[2]).normalize_or(v3(1.0, 0.0, 0.0));
    let world = f[0] * local.x + f[1] * local.y + f[2] * local.z;
    let mut target = rotate(tilt, direction.cross(Vec3::UP).normalize_or(Vec3::ZERO));
    if !p.horizontal_directed && world.dot(target) <= 0.0 {
        target = -target;
    }
    let axis = local.cross(target).normalize_or(Vec3::ZERO);
    if axis.dot(axis) <= 0.9 {
        return Vec3::ZERO;
    }
    let pw = world - axis * axis.dot(world);
    let pt = target - axis * axis.dot(target);
    if world.dot(world) * target.dot(target) <= 1.5e-5 {
        return Vec3::ZERO;
    }
    let angle = wrap_angle(signed_angle(pw, pt, axis));
    let limit = PI / 4.0;
    axis * (angle.clamp(-limit, limit) * 0.8)
}

/// Seconds until the centre of mass (`PREDICT_RADIUS` round) falls onto the ground from `p` at `v`
/// (SK8's wipeout trajectory query), `f32::MAX` when nothing is met within 3 s.
fn predict(park: &dyn Terrain, p: Vec3, v: Vec3, g: f32) -> f32 {
    let at = |t: f32| p + v * t + Vec3::UP * (-0.5 * g * t * t);
    let mut prev = (0.0, p);
    let mut t = PREDICT_STEP;
    while t <= PREDICT_TIME + 1e-4 {
        let q = at(t);
        let top = prev.1.y.max(q.y) + PREDICT_RADIUS;
        let h = park.ground(q.x, q.z, top).height;
        let under = q.y - PREDICT_RADIUS - h;
        if h > f32::MIN / 2.0 && under <= 0.0 {
            // Between the last step and this one.
            let h0 = park.ground(prev.1.x, prev.1.z, top).height;
            let above = (prev.1.y - PREDICT_RADIUS - h0).max(0.0);
            let k = if above - under > 1e-6 {
                above / (above - under)
            } else {
                1.0
            };
            return prev.0 + (t - prev.0) * k;
        }
        prev = (t, q);
        t += PREDICT_STEP;
    }
    f32::MAX
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Xform;
    use crate::park::{Park, Surface};

    const DT: f32 = 1.0 / 60.0;

    struct Flat;
    impl Terrain for Flat {
        fn ground(&self, _x: f32, _z: f32, _max: f32) -> Surface {
            Surface {
                height: 0.0,
                normal: Vec3::UP,
            }
        }
        fn spawn(&self) -> (Vec3, f32) {
            (Vec3::ZERO, 0.0)
        }
    }

    fn start(at: Vec3, vel: Vec3) -> Wipeout {
        let mut w = Wipeout::default();
        w.start(
            &built_in_pose(&Xform::new(crate::math::Mat3::IDENTITY, at), false),
            vel,
            DT,
        );
        w
    }

    #[test]
    fn the_gesture_picks_skate_3s_profiles() {
        let t = Tuning::BUILT_IN;
        for (g, want) in [
            ([0.0, 0.0], 0),
            ([0.0, 1.0], 1),
            ([1.0, 0.0], 2),
            ([0.0, -1.0], 3),
            ([-1.0, 0.0], 4),
        ] {
            let mut w = start(v3(0.0, 5.0, 0.0), v3(0.0, 0.0, 5.0));
            w.step(
                &Flat,
                &t,
                Input {
                    gesture: g,
                    ..Input::default()
                },
                DT,
            );
            assert_eq!(w.profile, want, "{g:?}");
        }
        // Without the graphs the C-stick waits to be centred, then blends in.
        let mut w = start(v3(0.0, 5.0, 0.0), Vec3::ZERO);
        w.gesture_from_stick([0.0, 1.0], false, [0.2, 0.2]);
        assert_eq!(w.gesture, [0.0, 0.0]);
        w.gesture_from_stick([0.0, 0.0], false, [0.2, 0.2]);
        for _ in 0..20 {
            w.gesture_from_stick([0.0, 1.0], false, [0.2, 0.2]);
        }
        assert!(w.gesture[1] > 0.95, "{:?}", w.gesture);
    }

    #[test]
    fn a_fall_predicts_its_landing_and_settles() {
        let t = Tuning::BUILT_IN;
        let mut w = start(v3(0.0, 5.0, 0.0), v3(3.0, 0.0, 0.0));
        w.step(&Flat, &t, Input::default(), DT);
        // About sqrt(2 x 4.4 / g) s to the ground for the centre of mass 1 m up.
        let expect = libm::sqrtf(2.0 * (w.body.com().y - 0.3) / t.gravity);
        assert!(
            (w.predicted_time - expect).abs() < 0.08,
            "{} vs {expect}",
            w.predicted_time
        );
        assert!(w.no_support_time > 0.0);
        let mut settled = None;
        for i in 0..600 {
            w.step(&Flat, &t, Input::default(), DT);
            if w.over && settled.is_none() {
                settled = Some(i);
            }
        }
        assert!(settled.is_some(), "never settled");
        assert_eq!(w.predicted_time, 0.0);
        assert!(w.meat.hits > 0, "a 5 m fall hurts");
    }

    #[test]
    fn the_control_steers_and_spins_the_fall() {
        let t = Tuning::BUILT_IN;
        let fall = |control: [f32; 2], gesture: [f32; 2]| {
            let mut w = start(v3(0.0, 30.0, 0.0), v3(0.0, 0.0, 6.0));
            for _ in 0..60 {
                w.step(
                    &Flat,
                    &t,
                    Input {
                        control,
                        gesture,
                        push_off: false,
                    },
                    DT,
                );
            }
            w
        };
        // A swan dive drifts with the stick across the travel.
        let left = fall([-1.0, 0.0], [0.0, -1.0]).body.com();
        let right = fall([1.0, 0.0], [0.0, -1.0]).body.com();
        assert!(right.x - left.x > 1.0, "{left:?} {right:?}");
        // A cannon ball pulled back somersaults.
        let w = fall([0.0, -1.0], [0.0, 1.0]);
        assert!(w.body.angular_velocity(DT).length() > 3.0);
        assert!(w.drive > 0.0);
    }

    #[test]
    fn a_push_off_kicks_the_body_along_the_stick() {
        let t = Tuning::BUILT_IN;
        let park = Park::demo();
        let (spawn, _) = park.spawn();
        let mut w = start(spawn, Vec3::ZERO);
        for _ in 0..40 {
            w.step(&park, &t, Input::default(), DT);
        }
        assert!(w.body.contacts.any());
        let before = w.body.com_vel(DT).x;
        w.step(
            &park,
            &t,
            Input {
                control: [1.0, 0.0],
                push_off: true,
                ..Input::default()
            },
            DT,
        );
        assert!(
            w.body.com_vel(DT).x - before > 0.5,
            "{:?}",
            w.body.com_vel(DT)
        );
    }
}
