//! Riding on Skate 3's rigid board (`physics::board`): the ground behaviours hand the board forces and
//! deck turns, as SK8's ground update queues them (riding/grounded/state/ordinary.rs), the steering's
//! deck tilt becomes the trucks' targets (riding/steering.rs `TruckSteeringState`), and the board's
//! seven bodies are solved together; the skater's ground state (`pos`, `fwd`, `speed`, `slip`,
//! `contact_n`) is then read off the deck.
//!
//! What still turns the board kinematically (kick turns, reverts, powerslides) runs the older ground
//! model, and the rigid board is put where that leaves it.

use super::*;
use crate::math::Mat3;
use crate::physics::board::{self as pb, Board, BoardInput};
use crate::tuning::Curve;

/// `physics_feet`: the feet's force points along the deck (m), the force against the rider's up per
/// m/s of speed /8 (N), the landing force (N) and how long it lasts (s): SK8 riding/ground_force.rs and
/// grounded/state/ordinary.rs `ground_forces` (`FootForceOffset`, `MinForce`, `MaxLandingForce`,
/// `LandingForceTime`).
const FOOT_OFFSET: f32 = 0.22;
const FOOT_MIN_FORCE: f32 = 20.0;
const FOOT_LANDING_FORCE: f32 = 100.0;
const FOOT_LANDING_TIME: f32 = 0.1;
/// Skate 3's manual corrective drag (N per m/s, SK8 manual/controller.rs `apply_correction`).
const MANUAL_CORRECTIVE_DRAG: f32 = 400.0;

/// Sideways friction (SK8 riding/slide_friction.rs): against the ground's angle (quarter turns),
/// `physics_friction/SlideVsNormal`; against the speed / 10, `SlideVsSpeed`; building over the
/// seconds on the ground / `StraightenOut_ForceTime` (0.5 on smooth concrete),
/// `physics_heading/SideFrictionFactor`; the wheels' hardness (0.7 while riding, measured in SK8)
/// between 1 and `SoftestWheelSideFrictionScalar` (2).
const SLIDE_VS_NORMAL: Curve = Curve::new(&[
    (0.0, 0.0),
    (0.048_859_94, 0.0),
    (0.133_550_5, 0.131_782_9),
    (0.193_811_1, 0.472_868_2),
    (0.229_641_7, 0.616_279_1),
    (0.276_873, 0.767_441_9),
    (0.338_762_2, 0.879_845),
    (0.405_537_5, 0.961_240_3),
    (0.480_456, 1.0),
    (0.543_973_9, 1.0),
    (0.620_521_2, 1.0),
    (0.700_325_7, 1.0),
    (0.757_329, 1.0),
    (0.820_846_9, 1.0),
    (0.903_908_8, 1.0),
    (1.0, 1.0),
]);
const SLIDE_VS_SPEED: Curve = Curve::new(&[
    (0.0, 1.0),
    (0.047_231_27, 1.0),
    (0.118_892_5, 0.949_324_3),
    (0.201_954_4, 0.722_973),
    (0.381_107_5, 0.314_189_2),
    (0.5, 0.179_054_1),
    (0.741_042_3, 0.0),
    (1.0, 0.0),
]);
const SIDE_FRICTION_FACTOR: Curve = Curve::new(&[
    (0.0, 0.0),
    (0.0625, 0.0),
    (0.125, 0.0),
    (0.1875, 0.0),
    (0.25, 0.0),
    (0.324_742_3, 0.019_379_85),
    (0.381_443_3, 0.050_387_62),
    (0.445_876_3, 0.100_775_2),
    (0.512_886_6, 0.182_170_6),
    (0.587_628_8, 0.294_573_7),
    (0.672_680_4, 0.468_992_2),
    (0.770_618_6, 0.655_038_8),
    (0.855_670_1, 0.856_589_1),
    (0.909_793_8, 0.961_240_3),
    (0.948_453_6, 1.0),
    (1.0, 1.0),
]);
const SIDE_FRICTION_TIME: f32 = 0.5;
const SOFTEST_WHEEL_SIDE_FRICTION: f32 = 2.0;
const WHEEL_HARDNESS: f32 = 0.7;

/// The rigid board and what the skater's state was when it was last read off it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    pub board: Board,
    /// Seconds on the ground since the board was put down (SK8 Processed+2752).
    pub ground_time: f32,
    pos: Vec3,
    fwd: Vec3,
    velocity: Vec3,
    /// The velocity change last brought in from outside the ride (diagnostics).
    pub last_sync: Vec3,
}

impl Skater {
    /// Whether this tick's ride runs on the rigid board.
    pub(super) fn rides_rigid(d: &Drive) -> bool {
        d.slide.is_none() && !d.pivot && !d.keep_velocity
    }

    fn deck_axes(&self) -> Mat3 {
        let n = self.contact_n;
        let f = tangent(self.fwd, n);
        Mat3::from_cols(n.cross(f), n, f)
    }

    /// Brings the rigid board in line with the skater's ground state: new when riding starts, put where
    /// the skater is if something moved or turned him, given any velocity change made from outside.
    pub(super) fn sync_rig(&mut self, park: &dyn Terrain) {
        let n = self.contact_n;
        let velocity = self.ground_velocity() + n * self.vn;
        let deck = self.pos + n * pb::DECK_HEIGHT;
        let axes = self.deck_axes();
        let spin = n * self.yaw_rate;
        match &mut self.rig {
            Some(r) if r.pos == self.pos && r.fwd == self.fwd => {
                let dv = velocity - r.velocity;
                r.last_sync = dv;
                if dv.dot(dv) > 0.0 {
                    for p in &mut r.board.parts {
                        p.velocity += dv;
                    }
                }
            }
            Some(r) => {
                r.last_sync = v3(
                    (r.pos - self.pos).length(),
                    (r.fwd - self.fwd).length(),
                    99.0,
                );
                r.board.place(deck, &axes, velocity, spin);
                r.board.settle(park);
            }
            None => {
                // Entering the ground: Skate 3 starts its wobble afresh.
                self.wobble.reset();
                let mut board = Board::new(deck, &axes);
                board.place(deck, &axes, velocity, spin);
                board.settle(park);
                self.rig = Some(Rig {
                    board,
                    ground_time: 0.0,
                    pos: self.pos,
                    fwd: self.fwd,
                    velocity,
                    last_sync: Vec3::ZERO,
                });
            }
        }
    }

    /// Remembers the ground state as read off the board.
    fn mark_rig(&mut self) {
        let velocity = self.ground_velocity() + self.contact_n * self.vn;
        if let Some(r) = &mut self.rig {
            r.pos = self.pos;
            r.fwd = self.fwd;
            r.velocity = velocity;
        }
    }

    /// The rigid board is tipping over an edge: its leading truck's wheels are off and its leading
    /// end dips below the surface's plane (SK8's board pivots over a coping on its back wheels
    /// and rolls into the transition; a rail under it then is not ridden onto).
    pub(super) fn tipping_over_edge(&self) -> bool {
        let Some(r) = &self.rig else { return false };
        let w = &r.board.wheels;
        let ahead = if self.speed >= 0.0 { 1.0 } else { -1.0 };
        let lead = if ahead > 0.0 {
            [pb::RIGHT_FRONT_WHEEL, pb::LEFT_FRONT_WHEEL]
        } else {
            [pb::RIGHT_BACK_WHEEL, pb::LEFT_BACK_WHEEL]
        };
        let down = |i: usize| w[i].is_some_and(|c| c.load > 0.0);
        let nose = r.board.deck().axes.cols[2] * ahead;
        // Off its leading wheels with the nose dipping (still on the trailing ones or just off them).
        !lead.iter().any(|&i| down(i)) && nose.dot(self.contact_n) < -0.05
    }

    /// Puts the rigid board where the kinematic ground model left the skater.
    pub(super) fn follow_rig(&mut self, park: &dyn Terrain) {
        if self.mode == Mode::Rolling {
            self.sync_rig(park);
            self.mark_rig();
        }
    }

    /// One tick of riding on the rigid board, driven by `d`.
    pub(super) fn ride_rigid(
        &mut self,
        park: &dyn Terrain,
        d: &Drive,
        dt: f32,
        events: &mut Vec<Event>,
    ) {
        let t = self.tuning;
        let ticks = dt * 60.0;
        let n = self.contact_n;
        self.sync_rig(park);

        // The speed model's target (SK8 speed_model.rs), as `ride`.
        let bound = t.friction_max_change.max(0.0);
        // The speed model and rolling resistance read the board's travel speed (sideways slip
        // included), signed as the speed along it.
        let travel = self.ground_velocity().length();
        let travel_speed = if travel > 1e-3 {
            travel * self.speed.signum()
        } else {
            self.speed
        };
        let start = travel_speed.abs();
        if (start - self.model_speed).abs() > 1e-4 {
            self.target_speed = start;
        }
        self.target_speed = self
            .target_speed
            .clamp((start - bound).max(0.0), start + bound);

        // Steering: the deck tilt, ramped in after landing; the trucks take it (opposite in sign to
        // Skate 3's tilt, whose positive turns the board to its left).
        self.push_steer = if d.pushing {
            (self.push_steer - t.push_steer_drop * ticks).max(t.push_steer_min)
        } else {
            (self.push_steer + t.push_steer_rise * ticks).min(1.0)
        };
        let manual_k = if d.manual { t.manual_tilt } else { 1.0 };
        let want = t.steer_tilt(d.turn, self.speed, d.hard, self.push_steer) * manual_k;
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
        self.truck_ramp = (self.truck_ramp + dt).min(0.165);
        let tilt = self.deck_tilt * (self.truck_ramp / 0.165);

        let vel = self.ground_velocity();
        let v = vel.length();
        let mut input = BoardInput {
            trucks: [-tilt, -tilt],
            wheel_friction: [pb::WHEEL_FRICTION[0], pb::WHEEL_FRICTION[1]],
            ..BoardInput::default()
        };

        // Heading and the stick's turn torque on slopes, straightening, wall-hit turns: limited deck
        // turns about the surface's normal (SK8 heading.rs, straighten.rs, `apply_vector_82c07000`).
        let mut turn = 0.0;
        let slope = (libm::acosf(n.y.clamp(-1.0, 1.0)) / (PI / 2.0)).clamp(0.0, 1.0);
        let downhill = (Vec3::UP * -1.0 + n * n.y).normalize_or(Vec3::ZERO);
        if v > 0.05 {
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
            turn += if d.raw_turn != 0.0 {
                if torque * heading > 0.0 && heading.abs() > torque.abs() {
                    heading
                } else {
                    torque
                }
            } else {
                heading
            };
        }
        if v > 0.3 {
            let f = tangent(self.fwd, n);
            let f = if f.dot(vel) < 0.0 { -f } else { f };
            let err = n.dot(f.cross(vel));
            let mut s = t.straighten * err;
            if -d.turn * s < 0.0 {
                let lim = t.straighten_max_turn.max(0.01);
                s *= 1.0 - d.turn.abs().min(lim) / lim;
            }
            turn += s;
        }
        if let Some((rate, time)) = &mut self.collide {
            turn += *rate / 60.0;
            *rate *= libm::powf(t.collision_torque_fade, ticks);
            *time -= dt;
            if *time <= 0.0 {
                self.collide = None;
            }
        }
        input.limited = n * turn;

        // The speed along the board: rolling resistance, the feet, the speed model; the force that
        // makes the board's speed change so, gravity being the solver's (SK8 queues each as a force).
        let g = t.gravity;
        let gravity = Vec3::UP * -g + n * (g * n.y);
        let f = tangent(self.fwd, n);
        let fdir = if v > 0.1 {
            vel * (self.speed.signum() / v)
        } else {
            f
        };
        let along = clamp(gravity.dot(fdir), -t.max_slope_accel, t.max_slope_accel);
        let foot_down = d.push_target.is_some() || d.brake > 0.0;
        let mass = pb::total_mass();
        let mut speed = travel_speed;
        if d.manual_correction {
            // The Normal corrective drag replaces the tick's other board forces.
            // At the dragging truck, against the velocity there (SK8 `apply_correction`'s
            // correction points): it brakes the board and turns it about the truck.
            let nose = self.manual.is_some_and(|m| m.nose);
            let z = if nose { pb::TRUCK_Z } else { -pb::TRUCK_Z };
            let at = v3(0.0, pb::TRUCK_Y - pb::FORCE_Y_OFFSET, z);
            let deck = self.rig.as_ref().map(|r| *r.board.deck());
            let velocity = deck.map_or(vel + n * self.vn, |b| {
                b.point_velocity(b.axes.apply(v3(0.0, pb::TRUCK_Y, z)))
            });
            input.forces.push((-velocity * MANUAL_CORRECTIVE_DRAG, at));
            self.target_speed = self.speed.abs();
        } else {
            let before_model = speed.abs();
            let mut friction = 0.0;
            if !foot_down {
                let sp = speed.abs();
                friction = t.friction.eval(sp);
                if self.idle > t.no_input_time {
                    friction += t.friction_no_input.eval(sp);
                }
                if d.manual {
                    friction += t.friction_manual.eval(sp);
                }
                friction = friction.min(t.corrector_limit());
            }
            if along * speed < 0.0 {
                if along.abs() >= friction {
                    speed += along * dt;
                } else {
                    speed = approach(speed, 0.0, friction * dt);
                }
            } else {
                speed += along * dt;
                speed = approach(speed, 0.0, friction * dt);
            }
            self.target_speed = (self.target_speed + speed.abs() - before_model).max(0.0);
            // The feet's forces (SK8 push.rs, braking.rs): the push the gap to its target, at most
            // the tick's change, times the board's mass; the brake its force (`brake_scale` turns
            // it into the deceleration the older model used). The board's wheels take their share,
            // as SK8's do.
            let mut feet = 0.0;
            if let Some(target) = d.push_target
                && self.speed < target
            {
                feet += mass
                    * (target - self.speed).min(t.push_tick_limit(self.speed.max(0.0)) * ticks)
                    / dt;
            }
            if d.brake > 0.0 {
                if self.speed.abs() < 2.0 * t.brake_slow_speed {
                    // SK8 braking.rs `calculate_linear_drag`: a brake on a slow board sets the
                    // parts' linear drag to 1 x 60, which stops them.
                    speed = 0.0;
                    input.linear_drag = 60.0;
                } else {
                    feet -= self.speed.signum() * d.brake / t.brake_scale.max(1e-6);
                }
            }
            input.forces.push((f * feet, Vec3::ZERO));
            if foot_down {
                self.target_speed = speed.abs();
            } else if speed.abs() > 0.01 {
                let error = clamp(self.target_speed - speed.abs(), -bound, bound);
                speed += speed.signum() * t.speed_general_amount * error * ticks;
            }
            let gravity_now = pb::GRAVITY.dot(fdir) * dt;
            let push = mass * (speed - travel_speed - gravity_now) / dt;
            input.forces.push((fdir * push, Vec3::ZERO));
            // The feet (SK8 ground_force.rs): against the rider's up, square to the travel, at each
            // foot; the landing's force first.
            let r = self.rig.as_ref().map_or(0.0, |r| r.ground_time);
            let fraction = (r / FOOT_LANDING_TIME).clamp(0.0, 1.0);
            let rest = if r != 0.0 {
                (1.0 - fraction) * FOOT_LANDING_FORCE
            } else {
                0.0
            };
            let magnitude = self.speed.abs() * 0.125 * FOOT_MIN_FORCE * fraction + rest;
            let mut foot = self.reckoning.up * -magnitude;
            if v > 0.01 {
                let dir = vel * (1.0 / v);
                foot -= dir * dir.dot(foot);
            }
            for z in [FOOT_OFFSET, -FOOT_OFFSET] {
                input.forces.push((foot, v3(0.0, 0.0, z)));
            }
            // Sideways friction across the deck.
            let angle = (libm::acosf(n.y.clamp(-1.0, 1.0)) * (2.0 / PI)).clamp(0.0, 1.0);
            let by_angle = 1.0 - SLIDE_VS_NORMAL.eval(angle);
            let by_speed = 1.0 - SLIDE_VS_SPEED.eval(self.speed.abs() * 0.1);
            let time =
                SIDE_FRICTION_FACTOR.eval(r.clamp(0.0, SIDE_FRICTION_TIME) / SIDE_FRICTION_TIME);
            let blend =
                (1.0 - SOFTEST_WHEEL_SIDE_FRICTION) * WHEEL_HARDNESS + SOFTEST_WHEEL_SIDE_FRICTION;
            let scalar = -(time * blend * by_angle.max(by_speed) * t.side_friction);
            let side = self
                .rig
                .as_ref()
                .map_or(self.deck_axes(), |r| r.board.deck().axes)
                .cols[0];
            let along_surface = vel;
            input
                .forces
                .push((side * (along_surface.dot(side) * scalar), Vec3::ZERO));
        }

        // The manual's deck turn (SK8 manual/controller.rs, `apply_angular_displacement`): the nose
        // lifted about the deck's left-right axis.
        if let Some(m) = &self.manual {
            let lift = if m.nose { -m.last_step } else { m.last_step } * ticks;
            let axes = self
                .rig
                .as_ref()
                .map_or(self.deck_axes(), |r| r.board.deck().axes);
            input.angular = axes.cols[0] * -lift;
        }

        // Walls at wheel height ahead, as the older ground model meets them.
        let motion = vel * dt;
        let len = motion.length();
        // (Riding a transition, the rigid board meets what is ahead with its own contacts.)
        if len > 1e-6 && n.y > 0.7 {
            let dir = motion * (1.0 / len);
            let lead = (dir.dot(f).abs() * AXLE).max(0.1);
            if let Some(h) = park.raycast(self.pos + n * 0.04, dir, len + lead)
                && h.normal.dot(n) < 0.64
                // (A transition curving up ahead is ridden into by the wheels, not hit.)
                && h.normal.y < 0.3
                && h.normal.dot(dir) < -0.2
                && let Some(vv) = self.hit_wall(h.normal, vel, false, events)
            {
                if self.mode != Mode::Rolling {
                    return;
                }
                self.set_ground_velocity(vv);
                self.pos += h.normal * (t.wall_push_away * dt);
                self.follow_rig(park);
                return;
            }
        }

        let Some(rig) = self.rig.as_mut() else {
            return;
        };
        rig.board.step(park, &input);
        rig.ground_time += dt;
        self.read_rig(park, d, dt, events);
    }

    /// The skater's ground state off the board after its step; leaving the ground when its wheels
    /// have been off it far or long enough.
    fn read_rig(&mut self, park: &dyn Terrain, d: &Drive, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        let Some(rig) = &self.rig else { return };
        let board = &rig.board;
        let deck = *board.deck();
        // The wheels pressing on the ground; failing those, the ones within reach of a face turned less
        // than 60 degrees from the last surface (not a wall a tipped board's wheel comes near).
        let mut down: Vec<pb::WheelContact> = board
            .wheels
            .iter()
            .flatten()
            .filter(|w| w.load > 0.0)
            .copied()
            .collect();
        if down.is_empty() {
            let n = self.contact_n;
            down = board
                .wheels
                .iter()
                .flatten()
                .filter(|w| w.normal.dot(n) > 0.5)
                .copied()
                .collect();
        }
        let old_fwd = self.fwd;
        let old_n = self.contact_n;
        let nn = if down.is_empty() {
            old_n
        } else {
            down.iter()
                .fold(Vec3::ZERO, |a, w| a + w.normal)
                .normalize_or(old_n)
        };
        let f = tangent(deck.axes.cols[2], nn);
        if let Some(manual) = &mut self.manual {
            // The manual's pitch is the physical deck's (SK8's measured angle).
            let lift = libm::asinf(deck.axes.cols[2].dot(nn).clamp(-1.0, 1.0));
            manual.pitch = if manual.nose { -lift } else { lift };
        }
        self.contact_n = nn;
        self.fwd = f;
        let ground = if down.is_empty() {
            deck.position - nn * pb::DECK_HEIGHT
        } else {
            let p = down.iter().fold(Vec3::ZERO, |a, w| a + w.point) * (1.0 / down.len() as f32);
            deck.position - nn * (deck.position - p).dot(nn)
        };
        self.pos = ground;
        // The board's travel: its parts' centre of mass (the deck alone runs a little ahead of a
        // force it takes until its joints hand it on).
        let v = board.velocity();
        self.speed = v.dot(f);
        self.slip = v.dot(nn.cross(f));
        if v.length() < 1e-3 && d.push_target.is_none() {
            // At rest (Skate 3 freezes bodies under `FreezingEnergy`; SK8's host never sleeps them):
            // a standing board's micrometre-a-second jitter is no speed; the next tick's sync stops
            // its parts.
            self.speed = 0.0;
            self.slip = 0.0;
        }
        self.chord = (down.len() >= 3).then(|| {
            let ch = board.parts[pb::FRONT_TRUCK].position - board.parts[pb::BACK_TRUCK].position;
            tangent(ch, nn)
        });
        let turned = nn.dot(tangent(old_fwd, nn).cross(f));
        self.yaw_rate = libm::asinf(turned.clamp(-1.0, 1.0)) / dt;
        self.wall_clock += dt;
        if down.is_empty() {
            self.vn = v.dot(nn);
            // How far the surface is below the deck along its up (a board tipped over a coping looks
            // down the transition, not to the pipe's floor).
            let up = deck.axes.cols[1];
            let gap = park
                .raycast(deck.position, -up, 1.0)
                .map_or(f32::MAX, |h| h.t - pb::DECK_HEIGHT);
            // A board tipping nose first over an edge stays on its physics for Skate 3's natural-air
            // time (SK8's rigid board falls onto the transition below a coping on its own and lands
            // there, a quarter second later, without the flight's kinematic path).
            let nose = deck.axes.cols[2];
            let lead = if deck.velocity.dot(nose) >= 0.0 {
                nose
            } else {
                -nose
            };
            let tipping = lead.y < -0.2;
            let leave = if tipping {
                gap == f32::MAX
            } else {
                gap > t.natural_air_max_dist || {
                    if gap > t.natural_air_min_dist {
                        self.gap_time += dt;
                    } else {
                        self.gap_time = 0.0;
                    }
                    self.gap_time > t.natural_air_time
                }
            };
            if leave {
                // The flight starts from the board as it is: a board tipped over an edge leaves it
                // pitched, its deck below the edge.
                let up = deck.axes.cols[1];
                self.pos = deck.position - up * pb::DECK_HEIGHT;
                self.normal = up;
                self.contact_n = up;
                self.fwd = deck.axes.cols[2];
                self.leave_ground(v, events);
                return;
            }
        } else {
            self.vn = 0.0;
            self.gap_time = 0.0;
        }
        let before = self.speed;
        self.update_pumping(d, dt);
        let pumped = self.speed - before;
        if pumped != 0.0
            && let Some(r) = &mut self.rig
        {
            for p in &mut r.board.parts {
                p.velocity += f * pumped;
            }
        }
        self.mark_rig();
        // A foot on the ground resets the speed model: it starts again from the speed the board
        // comes out of this tick with (SK8's `flags_1360` reset).
        self.model_speed = if d.push_target.is_some() || d.brake > 0.0 || d.manual_correction {
            -1.0
        } else {
            self.ground_velocity().length()
        };
        if self.contact_n.y < -0.2 {
            let v = self.ground_velocity() + self.contact_n * 0.5;
            self.pos += self.contact_n * 0.02;
            self.leave_ground(v, events);
            return;
        }
        self.update_wallride(park, dt, events);
    }
}

fn approach(v: f32, target: f32, by: f32) -> f32 {
    if v > target {
        (v - by).max(target)
    } else {
        (v + by).min(target)
    }
}
