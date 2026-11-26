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
use crate::math::Xform;
use crate::physics::board::{self as pb, Board, BoardInput};
use crate::physics::rider_body::{RiderBody, RiderInput};
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
/// A board further than this (m) from the skater's frame was left behind by a move from outside:
/// put where the skater is rather than adopted or flown to.
const LANDING_REACH: f32 = 0.5;

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
    /// Flying: the board follows the flight on SK8's hook drive and lands on its own physics.
    pub airborne: bool,
    /// The skater's frame is turned end for end against the deck (a switch or a revert's turn).
    pub flip: bool,
}

impl Skater {
    /// Whether this tick's ride runs on the rigid board.
    pub(super) fn rides_rigid(d: &Drive) -> bool {
        d.slide.is_none() && !d.pivot && !d.keep_velocity
    }

    /// The physical deck's axes for the skater's frame: turned end for end while the frame is (the
    /// board itself never turns round, SK8 board_motion_output.rs `board_flipped`).
    fn deck_axes(&self) -> Mat3 {
        let n = self.contact_n;
        let f = tangent(self.fwd, n) * self.rig_sign();
        Mat3::from_cols(n.cross(f), n, f)
    }

    /// -1 while the skater's frame is turned end for end against the physical deck.
    fn rig_sign(&self) -> f32 {
        if self.rig.as_ref().is_some_and(|r| r.flip) {
            -1.0
        } else {
            1.0
        }
    }

    /// Brings the rigid board in line with the skater's ground state: new when riding starts, put where
    /// the skater is if something moved or turned him, given any velocity change made from outside.
    pub(super) fn sync_rig(&mut self, park: &dyn Terrain) {
        let n = self.contact_n;
        let velocity = self.ground_velocity() + n * self.vn;
        let deck = self.pos + n * pb::DECK_HEIGHT;
        let axes = self.deck_axes();
        let spin = n * self.yaw_rate;
        // Landing: the skater's ground state comes off the board, which flew on and met the ground
        // itself (it isn't put where the flight lands), unless the flight took it far from it.
        if let Some(r) = &mut self.rig
            && r.airborne
        {
            r.airborne = false;
            if (r.board.deck().position - deck).length() < LANDING_REACH {
                self.adopt_rig();
                self.mark_rig();
                return;
            }
        }
        if let Some(r) = &mut self.rig
            && r.pos == self.pos
            && r.fwd.dot(self.fwd) < -0.9999
        {
            // The frame turned round, not the board.
            r.flip = !r.flip;
            r.fwd = self.fwd;
        }
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
                    airborne: false,
                    flip: false,
                });
            }
        }
    }

    /// The skater's ground state from the board as it is (no step): its loaded wheels' surface, the
    /// point under the deck on it, the nose and the travel.
    fn adopt_rig(&mut self) {
        let Some(r) = &self.rig else { return };
        let deck = *r.board.deck();
        let down: Vec<pb::WheelContact> = r
            .board
            .wheels
            .iter()
            .flatten()
            .filter(|w| w.normal.dot(self.contact_n) > 0.5)
            .copied()
            .collect();
        let nn = if down.is_empty() {
            self.contact_n
        } else {
            down.iter()
                .fold(Vec3::ZERO, |a, w| a + w.normal)
                .normalize_or(self.contact_n)
        };
        let f = tangent(deck.axes.cols[2] * self.rig_sign(), nn);
        let v = r.board.velocity();
        self.contact_n = nn;
        self.fwd = f;
        self.pos = deck.position - nn * pb::DECK_HEIGHT;
        self.speed = v.dot(f);
        self.slip = v.dot(nn.cross(f));
        self.vn = 0.0;
    }

    /// One air tick of the rigid board: no ground behaviours, the deck hooked to where the flight
    /// draws it (SK8's hook drive while the animation holds the board). SK8's hook targets the animated
    /// skateboard root, which carries the board adjust: the target is the adjusted deck
    /// ([`Skater::frame`]), so the board flies adjusted and the landing adopts it as lined up. The
    /// target is the physical deck's way round (end for end while the skater's frame is turned).
    pub(super) fn fly_rig(&mut self, park: &dyn Terrain, landing: bool) {
        let f = self.frame();
        let sign = self.rig_sign();
        let target = (f.deck, Mat3::from_cols(f.right * sign, f.up, f.fwd * sign));
        let vel = self.vel;
        let Some(r) = &mut self.rig else { return };
        r.airborne = true;
        // Moved far from the flight (a respawn, a scripted drop): put there, as `sync_rig` does.
        if (r.board.deck().position - target.0).length() > LANDING_REACH {
            r.board.place(target.0, &target.1, vel, Vec3::ZERO);
        }
        // SK8 flies the board on its own physics (the hook only turns it) and places the rider on
        // it; flatground's flight is the skater's, so the hook moves with it: its damping holds the
        // deck to the flight's velocity instead of dragging it behind (it would land late).
        // The tick the flight lands, the board comes down on its own physics as it flew (the landed
        // frame is the ground's, not where the flight put the board).
        let input = BoardInput {
            hook: (!landing).then_some((target.0 - vel * crate::physics::STEP, target.1)),
            hook_velocity: vel,
            hook_gravity: pb::GRAVITY,
            wheel_friction: [pb::WHEEL_FRICTION[0], pb::WHEEL_FRICTION[1]],
            ..BoardInput::default()
        };
        self.step_board(park, &input, true);
    }

    /// The rider's body's input this tick: the animated pose at the deck where it will be at the end
    /// of the tick (SK8 drives to the predicted board), and the ground under the wheels.
    fn rider_input(&self, airborne: bool) -> Option<RiderInput> {
        let local = self.rider_pose.as_ref()?;
        let d = self.rig.as_ref()?.board.deck();
        let deck = Xform::new(
            self.rider_axes,
            d.position + d.velocity * crate::physics::STEP,
        );
        let wheels = &self.rig.as_ref()?.board.wheels;
        let ground = wheels.iter().flatten().next().map_or(
            (
                d.position - d.axes.cols[1] * pb::DECK_HEIGHT,
                d.axes.cols[1],
            ),
            |w| (w.point, w.normal),
        );
        Some(RiderInput {
            pose: local.map(|x| deck.then(&x)),
            airborne,
            ground,
            board_contacts: false,
            clock: self.clock.0,
        })
    }

    /// Steps the rigid board, with the rider's body in its solve when there is one (timed apart).
    fn step_board(&mut self, park: &dyn Terrain, input: &BoardInput, airborne: bool) {
        let ri = self.rider_input(airborne);
        if ri.is_none() {
            self.rider_body = None;
        }
        let t0 = self.clock.now();
        let Some(rig) = self.rig.as_mut() else { return };
        let body = match &ri {
            Some(ri) => Some((
                &mut **self
                    .rider_body
                    .get_or_insert_with(|| alloc::boxed::Box::new(RiderBody::new(&ri.pose))),
                ri,
            )),
            None => None,
        };
        rig.board.step_with(park, input, body);
        self.body_us = self.rider_body.as_ref().map_or(0, |b| b.cost_us);
        self.board_us = self
            .clock
            .now()
            .wrapping_sub(t0)
            .saturating_sub(self.body_us);
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
        if self.stays_rolling() {
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
            trucks: [-tilt * self.rig_sign(), -tilt * self.rig_sign()],
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
        let foot_down = d.push_target.is_some() || d.brake > 0.0;
        let mass = pb::total_mass();
        if d.manual_correction {
            // The Normal corrective drag replaces the tick's other board forces.
            // At the dragging truck (SK8 `apply_correction`'s correction points: the truck's base
            // transform, which the step lifts by `DeckForceYOffset` as every queued force), against
            // the deck's ground velocity (its velocity off the ground's normal, SK8
            // board_motion_output.rs `ground_velocity`).
            let nose = self.manual.is_some_and(|m| m.nose);
            let z = if nose { pb::TRUCK_Z } else { -pb::TRUCK_Z };
            let at = v3(0.0, pb::TRUCK_Y, z);
            let deck = self.rig.as_ref().map(|r| *r.board.deck());
            let velocity = deck.map_or(vel, |b| b.velocity - n * b.velocity.dot(n));
            input.forces.push((-velocity * MANUAL_CORRECTIVE_DRAG, at));
            self.target_speed = self.speed.abs();
        } else {
            // SK8 speed_model.rs `update`: the target speed moves by the slope's gravity (capped) and
            // the surface's rolling resistance, whichever wins uphill; the no-input and manual
            // frictions take theirs. The physical gravity acts in full on the board meanwhile.
            let sp = travel_speed.abs();
            let travel_dir = if v > 0.1 {
                vel * (1.0 / v)
            } else {
                f * self.speed.signum()
            };
            let slope = clamp(
                gravity.dot(travel_dir),
                -t.max_slope_accel,
                t.max_slope_accel,
            ) * dt;
            let friction = -t.friction.eval(sp) * dt;
            let surface = if slope > 0.0 {
                friction + slope
            } else if friction - slope >= 0.0 {
                slope
            } else {
                friction
            };
            let mut target = self.target_speed + surface;
            if self.idle > t.no_input_time {
                target -= t.friction_no_input.eval(sp) * dt;
            }
            if d.manual {
                target -= t.friction_manual.eval(sp) * dt;
            }
            self.target_speed = target.max(0.0);
            // The feet's forces (SK8 push.rs, braking.rs): the push the gap to its target, at most
            // the tick's change, times the board's mass; the brake its force. The board's wheels
            // take their share, as SK8's do.
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
                    input.linear_drag = 60.0;
                } else {
                    feet -= self.speed.signum() * d.brake;
                }
            }
            input.forces.push((f * feet, Vec3::ZERO));
            // Its force: the board's mass times the gain of the error, along the travel, with two
            // wheels down and no foot on the ground (the feet restart the model from the speed).
            let wheels = self.rig.as_ref().map_or(4, |r| r.board.wheels_down());
            if foot_down {
                self.target_speed = sp;
            } else if wheels >= 2 && sp > 0.01 {
                let error = clamp(self.target_speed - sp, -bound, bound);
                let push = mass * t.speed_general_amount * error / dt;
                input.forces.push((travel_dir * push, Vec3::ZERO));
            }
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
            input.angular = axes.cols[0] * (-lift * self.rig_sign());
        }

        // Walls at wheel height ahead, as the older ground model meets them.
        let motion = vel * dt;
        let len = motion.length();
        // (Riding a transition, the rigid board meets what is ahead with its own contacts.)
        // A terrain of triangles meets the board's volumes in the solve itself (walls included).
        let physical = park.triangles_near(self.pos, 0.0, &mut Vec::new());
        if len > 1e-6 && n.y > 0.7 && !physical {
            let dir = motion * (1.0 / len);
            let lead = (dir.dot(f).abs() * AXLE).max(0.1);
            if let Some(h) = park.raycast(self.pos + n * 0.04, dir, len + lead)
                && h.normal.dot(n) < 0.64
                // (A transition curving up ahead is ridden into by the wheels, not hit.)
                && h.normal.y < 0.3
                && h.normal.dot(dir) < -0.2
                && let Some(vv) = self.hit_wall(h.normal, vel, false, events)
            {
                if !self.stays_rolling() {
                    return;
                }
                self.set_ground_velocity(vv);
                self.pos += h.normal * (t.wall_push_away * dt);
                self.follow_rig(park);
                return;
            }
        }

        if self.rig.is_none() {
            return;
        }
        self.step_board(park, &input, false);
        let Some(rig) = self.rig.as_mut() else {
            return;
        };
        rig.ground_time += dt;
        self.read_rig(park, d, dt, events);
    }

    /// The skater's ground state off the board after its step; leaving the ground when its wheels
    /// have been off it far or long enough.
    fn read_rig(&mut self, park: &dyn Terrain, d: &Drive, dt: f32, events: &mut Vec<Event>) {
        let t = self.tuning;
        // The board's volumes met a wall (SK8 collision_response.rs on its collision reports): a
        // hard hit takes the rider off, a softer one turns the board along it; the solve has
        // already stopped the board going into it.
        if let Some((wall, _)) = self.rig.as_ref().and_then(|r| r.board.wall) {
            let before = self.ground_velocity();
            if self.hit_wall(wall, before, false, events).is_none() || !self.stays_rolling() {
                return;
            }
        }
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
        let f = tangent(deck.axes.cols[2] * self.rig_sign(), nn);
        let sign = self.rig_sign();
        if let Some(manual) = &mut self.manual {
            // The manual's pitch is the physical deck's (SK8's measured angle).
            let lift = libm::asinf((deck.axes.cols[2] * sign).dot(nn).clamp(-1.0, 1.0));
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
        // (Only on level ground: at the top of a transition a board stops for a moment and rolls
        // back.)
        if v.length() < 1e-3 && d.push_target.is_none() && nn.y > 0.99 {
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
        // SK8 board_ground.rs `advance_contact_time`: the time with no wheel touching.
        let loaded = board
            .wheels
            .iter()
            .flatten()
            .filter(|w| w.load > 0.0)
            .count();
        // The wheels touching, for Skate 3's state selector (SkateboardBody+869).
        self.states.facts.wheels = loaded as u8;
        if loaded > 0 {
            self.gap_time = 0.0;
        } else {
            self.gap_time += dt;
        }
        if down.is_empty() {
            self.vn = v.dot(nn);
            // Skate 3's off-ground test (SK8 player/selector/conditions.rs `condition_is_off_ground`,
            // TU3 0x82D8BD50, fed by player_state/selection.rs): the nearest surface along the four
            // wheel lines (board_ground.rs, 0.2 m down the reckoning's up from each wheel) further
            // than NaturalAirMaxDist, or further than NaturalAirMinDist with no wheel touching for
            // NaturalAirTime.
            let gap = wheel_line_gap(park, board, self.normal);
            let leave = off_ground(gap, self.gap_time, &t);
            if leave {
                // The flight starts from the board as it is: a board tipped over an edge leaves it
                // pitched, its deck below the edge.
                let up = deck.axes.cols[1];
                self.pos = deck.position - up * pb::DECK_HEIGHT;
                self.normal = up;
                self.contact_n = up;
                self.fwd = deck.axes.cols[2] * self.rig_sign();
                self.publish_off_ground(gap, self.gap_time);
                self.leave_ground(v, false, events);
                return;
            }
        } else {
            self.vn = 0.0;
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
            self.leave_ground(v, true, events);
            return;
        }
        self.update_wallride(park, dt, events);
    }
}

/// SK8 board_ground.rs `WHEEL_LINE_LENGTH`: each wheel's line down the reckoning's up (m).
pub const WHEEL_LINE_LENGTH: f32 = 0.2;

/// SK8 board_ground.rs `wheel_lines` / `WheelLineState::publish`: the nearest surface along the four
/// wheel lines (from each wheel's centre, `WHEEL_LINE_LENGTH` down `up`); a miss counts the whole line.
fn wheel_line_gap(park: &dyn Terrain, board: &Board, up: Vec3) -> f32 {
    board.parts[..4].iter().fold(WHEEL_LINE_LENGTH, |gap, w| {
        park.raycast(w.position, -up, WHEEL_LINE_LENGTH)
            .map_or(gap, |h| gap.min(h.t))
    })
}

/// SK8 player/selector/conditions.rs `condition_is_off_ground` (TU3 0x82D8BD50): the wheel lines'
/// gap beyond NaturalAirMaxDist, or beyond NaturalAirMinDist with no wheel touching for longer than
/// NaturalAirTime (all strict).
fn off_ground(gap: f32, no_wheel_time: f32, t: &crate::tuning::Tuning) -> bool {
    gap > t.natural_air_max_dist
        || (gap > t.natural_air_min_dist && no_wheel_time > t.natural_air_time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::Park;
    use crate::physics::skeleton::ANIMATED_PARTS;

    /// SK8 `condition_is_off_ground`: far beyond the max distance at once, between the two distances
    /// only after the natural-air time without a wheel touching (all strict), never within the min.
    #[test]
    fn off_ground_is_skate_3s_two_stage_test() {
        let t = crate::tuning::Tuning::BUILT_IN;
        assert_eq!(
            (
                t.natural_air_max_dist,
                t.natural_air_min_dist,
                t.natural_air_time
            ),
            (0.12, 0.06, 0.2),
            "physics_airstates/default"
        );
        assert!(off_ground(0.13, 0.0, &t));
        assert!(!off_ground(0.1, 0.2, &t));
        assert!(off_ground(0.1, 0.21, &t));
        assert!(!off_ground(0.05, 1.0, &t));
        // A miss on every wheel line counts the whole line: off.
        assert!(off_ground(WHEEL_LINE_LENGTH, 0.0, &t));
    }

    /// A board at rest overhanging a ledge (its middle past the edge, its back wheels over the top)
    /// stays on the ground: the wheel lines still find the ledge under the back wheels (the deck's
    /// middle ray found the floor below and the rider went in and out of the air every tick).
    #[test]
    fn a_board_hanging_over_a_ledge_at_rest_does_not_loop_in_and_out_of_the_air() {
        let park = Park {
            shapes: alloc::vec![crate::park::Shape::Block {
                min: v3(-2.0, 0.0, -6.0),
                max: v3(2.0, 0.5, 0.0),
                rgba: 0,
            }],
            half_size: 50.0,
            wall_height: 2.0,
            spawn: v3(0.0, 0.5, -3.0),
            spawn_yaw: 0.0,
            rails: Vec::new(),
        };
        for over in [0.02, 0.06, 0.1] {
            let mut s = Skater::spawn(&park);
            s.since_spawn = 10.0;
            // On the ledge, then the board slid out over its edge.
            s.put_at(&park, v3(0.0, 0.5, -0.5), 0.0, 0.0);
            s.sync_rig(&park);
            // Its wheels just off the top (no wheel touching), as a board left hanging by a grind.
            let r = s.rig.as_mut().unwrap();
            let d = *r.board.deck();
            r.board.place(
                d.position + v3(0.0, 0.015, 0.5 + over),
                &d.axes,
                Vec3::ZERO,
                Vec3::ZERO,
            );
            s.pos += v3(0.0, 0.015, 0.5 + over);
            s.mark_rig();
            let mut ev = Vec::new();
            let step = |s: &mut Skater, ev: &mut Vec<Event>| {
                s.step(
                    &park,
                    &crate::input::Pad::default(),
                    &crate::flick::FlickOut::default(),
                    1.0 / 60.0,
                    ev,
                )
            };
            step(&mut s, &mut ev);
            assert!(!s.is_airborne(), "{over} m over: off the ground at rest");
            let mut was = s.is_airborne();
            let (mut flips, mut last) = (0, 0);
            for tick in 0..180 {
                step(&mut s, &mut ev);
                if s.is_airborne() != was {
                    flips += 1;
                    last = tick;
                    was = s.is_airborne();
                }
            }
            // Off the ledge and down onto the floor (a bounce off the ledge's face at most), then
            // settled on the ground.
            assert!(
                flips <= 4,
                "{over} m over: {flips} flips between ground and air"
            );
            assert!(last < 60, "{over} m over: still flipping at tick {last}");
            assert!(s.rolling(), "{over} m over");
        }
    }

    /// SK8 `SkeletonRootFrames::update`: the body's animated pose stands on the rider's axes (the
    /// reckoning's up) at the predicted deck, whatever the physical deck's pitch.
    #[test]
    fn the_rider_pose_stands_on_the_reckoning_not_the_deck() {
        let park = Park {
            shapes: Vec::new(),
            half_size: 50.0,
            wall_height: 2.0,
            spawn: Vec3::ZERO,
            spawn_yaw: 0.0,
            rails: Vec::new(),
        };
        let mut s = Skater::spawn(&park);
        s.sync_rig(&park);
        // The deck pitched nose down half a radian (over a lip), the reckoning upright.
        let pitched = Mat3::from_cols(
            v3(1.0, 0.0, 0.0),
            v3(0.0, libm::cosf(0.5), libm::sinf(0.5)),
            v3(0.0, -libm::sinf(0.5), libm::cosf(0.5)),
        );
        let r = s.rig.as_mut().unwrap();
        let at = r.board.deck().position;
        r.board.place(at, &pitched, Vec3::ZERO, Vec3::ZERO);
        s.rider_axes = Mat3::IDENTITY;
        let local = Xform::new(Mat3::IDENTITY, v3(0.0, 0.9, 0.0));
        s.rider_pose = Some([local; ANIMATED_PARTS]);
        let input = s.rider_input(false).unwrap();
        let hips = input.pose[23];
        let deck = s.rig.as_ref().unwrap().board.deck().position;
        assert!(
            (hips.pos - (deck + v3(0.0, 0.9, 0.0))).length() < 1e-4,
            "{:?}",
            hips.pos
        );
        assert!((hips.rot.cols[1] - Vec3::UP).length() < 1e-6);
    }
}
