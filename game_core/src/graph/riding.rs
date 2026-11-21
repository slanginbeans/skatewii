//! The riding behaviours' animation side: what Skate 3's on-board motion graph feeds its riding blend
//! trees (`BTREE_RIDING`: crouch by `DISTTOCOG`, carve lean by `ANGLE`, `DIR`, `QUICKNESS` and `HOLDING`,
//! speed tuck by `SPEEDTUCK`, wall riding by `TILT_X`; the manual trees by `MANUAL_ANGLE`; the landing
//! trees by `AVGVELY` and `SPIN`).
//!
//! Ports of SK8-ENGINE/skate-3-rust-engine (GPL-3.0): crates/skate-core/src/input/turn_conditioner.rs (the
//! physics' animation turn conditioner), input/set_turning.rs and input/turn_remap.rs (SetTurning),
//! animation/body_tilt.rs (SettingBodyTilt), animation/crouching.rs (Crouching, with the auto pump),
//! animation/manual.rs (SetManualAngle), and the landing data of crates/skate-game/src/graph_host
//! motion_native.rs (StoreLandingData) and motion_landing_execute.rs (SetLandingData). SK8 matches the
//! original's fused multiply-adds bit for bit; this port uses plain arithmetic.
//!
//! The tuning (`tuning::AnimTuning`) comes from the player's disc through the rules pack, or is
//! flatground's own.

use libm::{atan2f, cosf, sinf, sqrtf};

use crate::math::PI;
use crate::tuning::{AnimTuning, Curve};

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// What the physics tells the turn conditioner each update.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnInput {
    /// Speed (m/s).
    pub speed: f32,
    /// Speed wobble (0..1).
    pub wobble: f32,
    /// The body's lean into the turn the physics aims for (rad).
    pub lean: f32,
    /// The processed turn input (-1..1).
    pub turn: f32,
    /// The wheels are down (the stick's movement counts towards quickness and holding).
    pub ground: bool,
    /// The rider's poses are mirrored.
    pub mirrored: bool,
}

/// The conditioner's outputs, in the original's order: smoothed turn, target, base, turn and base
/// with the stance sign, quickness, direction, holding.
pub type TurnOutput = [f32; 8];

/// The physics' animation turn conditioner (SK8 turn_conditioner.rs): turns the steering and the lean
/// into the carve's turn, direction, quickness and holding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Conditioner {
    history: [f32; 8],
    filters: [[f32; 9]; 3],
}

fn filter(state: &mut [f32; 9], c: [f32; 4], input: f32) -> f32 {
    state[..4].copy_from_slice(&c);
    let difference = input - state[4];
    let input_difference = input - state[5];
    let complement = 1.0 - state[3];
    state[5] = input;
    state[8] = input_difference * state[3] + state[8] * complement;
    state[6] = difference * state[3] + complement * state[6];
    let term = (state[8] - state[7]) * state[2] + state[1] * state[6];
    let term = difference * state[0] + term;
    let result = term + state[4];
    state[7] = result - state[4];
    state[4] = result;
    result
}

/// A divisor that is never zero.
fn div(v: f32) -> f32 {
    if v.abs() < 1e-6 { 1e-6 } else { v }
}

impl Conditioner {
    pub fn update(&mut self, i: TurnInput, t: &AnimTuning) -> TurnOutput {
        let p = t.cond;
        let sign = if i.mirrored { -1.0 } else { 1.0 };
        let scaled = i.turn * sign;
        let speed_fraction = clamp(i.speed / div(p[11]), 0.0, 1.0);
        let other_fraction = clamp(i.lean / div(p[10]) * sign, -1.0, 1.0);
        let base = t.turn_vs_speed.eval(speed_fraction) * scaled;
        let attenuated = (1.0 - (1.0 - p[8]) * i.wobble) * other_fraction;
        let target = attenuated * p[9] + (1.0 - p[9]) * base;
        let epsilon = 0.01;
        let first = filter(&mut self.filters[0], t.filter_lean_slow, target);
        let first = if first.abs() > epsilon { first } else { 0.0 };
        let second = filter(&mut self.filters[1], t.filter_lean, target);
        let second = if second.abs() > epsilon { second } else { 0.0 };
        let third = filter(
            &mut self.filters[2],
            t.filter_dir,
            (target - second) / div(p[12]),
        );
        let h = &mut self.history;
        // Quickness: how much the stick has been moving lately.
        let movement = if i.ground { scaled } else { 0.0 };
        let difference = movement - h[0];
        h[0] = movement;
        h[2] = difference.abs() * p[2] + (1.0 - p[2]) * h[2];
        let ratio = h[2] / div(p[1]);
        let bounded = (ratio * ratio).min(1.0);
        h[3] = clamp(bounded, h[3] - p[0], h[3] + p[0]);
        let quick = clamp(t.quickness_curve.eval(h[3]), 0.0, 1.0);
        let quick = t.quickness_vs_speed.eval(i.speed) * quick;
        // Holding: the lean has stayed put.
        let movement = if i.ground { second } else { 0.0 };
        let difference = movement - h[1];
        h[1] = movement;
        h[5] = difference.abs() * p[5] + (1.0 - p[5]) * h[5];
        let hold: f32 = if h[5] < p[3] { 1.0 } else { 0.0 };
        let alpha = (1.0 - hold) * p[7] + p[6] * hold;
        let next = (1.0 - alpha) * h[4] + alpha * hold;
        let difference = next - h[4];
        h[6] = clamp(difference, h[6] - p[4], h[6] + p[4]);
        h[4] = clamp(h[4] + h[6], 0.0, 1.0);
        let alpha = t.wobble_blend.eval(i.wobble);
        h[7] = (1.0 - alpha) * h[7] + alpha * first;
        let turn = clamp(h[7], -1.0, 1.0);
        [
            turn,
            clamp(target, -1.0, 1.0),
            base,
            turn * sign,
            base * sign,
            quick,
            clamp(third, -1.0, 1.0),
            clamp(h[4], 0.0, 1.0),
        ]
    }
}

/// An angle of (x, y), 0 for a vector too short to have one.
fn angle(x: f32, y: f32, magnitude: f32) -> f32 {
    if magnitude > 1.0 / 65536.0 {
        atan2f(y, x)
    } else {
        0.0
    }
}

/// SetTurning's vector remap (SK8 turn_remap.rs): the vector scaled by a curve against its angle, then
/// its angle remapped by a curve plus an offset. Curves read angles folded into 0..pi.
pub fn remap(magnitude: &Curve, angles: &Curve, offset: f32, input: [f32; 2]) -> [f32; 2] {
    let one = |v: f32| clamp(v, -1.0, 1.0);
    let m = sqrtf(input[0] * input[0] + input[1] * input[1]);
    let a = angle(input[0], input[1], m);
    let scaled = m * magnitude.eval(if a >= 0.0 { a } else { a + PI });
    let (x, y) = (one(cosf(a) * scaled), one(sinf(a) * scaled));
    let m = sqrtf(x * x + y * y);
    let a = angle(x, y, m);
    let mapped = angles.eval(if a >= 0.0 { a } else { a + PI });
    let mapped = if a >= 0.0 { mapped } else { mapped + PI };
    let r = offset + mapped;
    [one(cosf(r) * m), one(sinf(r) * m)]
}

/// SetTurning's tree parameters and packet values for one update.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnValues {
    pub angle: f32,
    pub dir: f32,
    pub quickness: f32,
    pub speed_tuck: f32,
    pub holding: f32,
    /// The `Turn` and `Slide` packet values, when set.
    pub turn: Option<f32>,
    pub slide: Option<f32>,
}

/// SetTurning (SK8 set_turning.rs): the carve lean from the conditioner's turn and direction, through
/// the quick and slow remaps mixed by quickness, smoothed in by `lean_in` and limited per update; a
/// powerslide holds the lean fully to its side. Also the speed tuck past `tuck_start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turning {
    elapsed: f32,
    smoothed: f32,
    /// 0 sliding left, 1 sliding right, 2 carving.
    mode: u8,
}

impl Default for Turning {
    fn default() -> Self {
        Turning {
            elapsed: 0.0,
            smoothed: 0.0,
            mode: 2,
        }
    }
}

/// What SetTurning reads besides the conditioner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnContext {
    /// The powerslide's side by the graph's (authored) reckoning: `Some(true)` right.
    pub slide: Option<bool>,
    /// Riding fakie (the slide's captured stance) and mirrored.
    pub fakie: bool,
    pub mirrored: bool,
    /// Speed along the board (m/s).
    pub forward_speed: f32,
    /// `FakieTurn`, `LeftSlide` and `RightSlide` motion intents.
    pub fakie_turn: Option<f32>,
    pub left_slide: Option<f32>,
    pub right_slide: Option<f32>,
    /// `anim_motion/power_slide/slide_turn_value`.
    pub slide_turn_value: f32,
}

impl Turning {
    pub fn update(
        &mut self,
        c: &TurnOutput,
        x: TurnContext,
        dt: f32,
        t: &AnimTuning,
    ) -> TurnValues {
        let a = remap(&t.quick_mag, &t.quick_angle, t.quick_offset, [c[0], c[6]]);
        let b = remap(&t.slow_mag, &t.slow_angle, t.slow_offset, [c[0], c[6]]);
        let q = c[5];
        let mut target = -(a[0] * q + b[0] * (1.0 - q));
        let direction = -(a[1] * q + b[1] * (1.0 - q));
        let mode = match x.slide {
            Some(true) => 1,
            Some(false) => 0,
            None => 2,
        };
        if mode <= 1 {
            target = if (mode == 1) != x.mirrored { 1.0 } else { -1.0 };
            if x.fakie {
                target = -target;
            }
        }
        if mode != self.mode {
            self.elapsed = 0.0;
        }
        self.mode = mode;
        let blend = t.lean_in.eval(self.elapsed);
        let (lower, upper) = (
            self.smoothed - t.lean_max_delta,
            self.smoothed + t.lean_max_delta,
        );
        let value = (1.0 - blend) * self.smoothed + blend * target;
        self.elapsed += dt;
        self.smoothed = clamp(value, lower, upper);
        let speed = x.forward_speed.abs();
        let speed_tuck = if speed > t.tuck_start {
            t.speed_tuck.eval(speed - t.tuck_start)
        } else {
            0.0
        };
        let mut out = TurnValues {
            angle: self.smoothed,
            dir: direction,
            quickness: q,
            speed_tuck,
            holding: c[7],
            turn: None,
            slide: None,
        };
        if mode <= 1 {
            let flip = x.fakie;
            out.turn = Some(if (mode == 1) != flip {
                x.slide_turn_value
            } else {
                -x.slide_turn_value
            });
            let slide = if mode == 0 {
                x.left_slide
            } else {
                x.right_slide
            }
            .unwrap_or(0.0);
            out.slide = Some(if flip {
                let a = slide.abs();
                let v = if a > 0.5 { 1.0 - a } else { a };
                if -slide >= 0.0 { v } else { -v }
            } else {
                slide
            });
        } else if let Some(v) = x.fakie_turn {
            out.turn = Some(v);
        }
        out
    }
}

/// SettingBodyTilt (SK8 body_tilt.rs): `TILT_X` eases towards how far the board leans sideways
/// against gravity (0 on flat ground, about 1 on a wall), less while the body spins, rate-limited per
/// update. Restarts from 0 each time ApplyingBodyTilt switches it on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyTilt {
    value: f32,
    velocity: f32,
    was_enabled: bool,
}

impl BodyTilt {
    pub fn disable(&mut self) {
        self.was_enabled = false;
    }

    /// `lateral`: world up along the board's right side; `spin`: body spin speed (rad/s).
    pub fn update(
        &mut self,
        mirrored: bool,
        lateral: f32,
        spin: f32,
        air: bool,
        t: &AnimTuning,
    ) -> f32 {
        if !self.was_enabled {
            self.value = 0.0;
            self.velocity = 0.0;
        }
        self.was_enabled = true;
        let tilt = if mirrored { lateral } else { -lateral };
        let target = t.tilt_spin_factor.eval(spin.abs()) * tilt;
        let (vel, acc) = if air {
            (t.tilt_air_vel, t.tilt_air_acc)
        } else {
            (t.tilt_ground_vel, t.tilt_ground_acc)
        };
        let desired = clamp(target - self.value, -vel, vel);
        let change = clamp(desired - self.velocity, -acc, acc);
        self.velocity += change;
        self.value += self.velocity;
        self.value
    }
}

/// What Crouching reads of the physics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrouchPhysical {
    /// Vertical speed (m/s).
    pub vel_y: f32,
    /// Speed along the ground (m/s).
    pub ground_speed: f32,
    /// Pumping potential (0..1).
    pub pump_potential: f32,
    /// The board's and the ground's push up into the rider, the least crouch the physics asks for, and
    /// the crouch the deck's angle asks for.
    pub force: f32,
    pub ground_force: f32,
    pub min_crouch: f32,
    pub deck_angle: f32,
    /// The rider's centre of mass over the board (m).
    pub height: f32,
}

/// What Crouching reads of the motion intents.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrouchIntents {
    pub auto_pump_angle: Option<f32>,
    pub auto_pump_mag: Option<f32>,
    pub crouch: Option<f32>,
    pub hard_turn_crouch: Option<f32>,
    pub manual: Option<f32>,
    /// Powersliding.
    pub sliding: bool,
}

/// Crouching's result for one update.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrouchOut {
    /// The centre-of-mass height to pose (the `crouchName` parameter, m).
    pub height: f32,
    pub new_auto_pump: bool,
    pub player_pump: bool,
}

/// Crouching (SK8 crouching.rs): the rider's centre-of-mass height over the board, between
/// `crouch_min_height` and the speed's highest, from the crouch intents (triggers, hard turns, a manual's
/// back stick), the auto pump and the board's absorption, with rate limits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crouch {
    absorbed_velocity: f32,
    filtered_input: f32,
    pub fraction: f32,
    delta_fraction: f32,
    player_pumping: bool,
    auto_fraction: f32,
    auto_state: u8,
    auto_time: f32,
    previous_angle: f32,
    previous_magnitude: f32,
    filtered_potential: f32,
    pump_idle_time: f32,
}

fn unit(v: f32) -> f32 {
    clamp(v, 0.0, 1.0)
}

impl Crouch {
    pub fn begin(p: CrouchPhysical, t: &AnimTuning) -> Crouch {
        let max = t.crouch_max_height.eval(p.ground_speed);
        let range = max - t.crouch_min_height;
        Crouch {
            fraction: 1.0 - unit((p.height - t.crouch_min_height) / div(range)),
            pump_idle_time: 0.5,
            ..Crouch::default()
        }
    }

    pub fn update(
        &mut self,
        p: CrouchPhysical,
        i: CrouchIntents,
        dt: f32,
        t: &AnimTuning,
    ) -> CrouchOut {
        let restoring = -t.crouch_upforce.eval(1.0 - self.fraction);
        let mut board = clamp(-p.force / div(t.crouch_max_force), -1.0, 1.0);
        self.absorbed_velocity += p.vel_y;
        self.absorbed_velocity *= t.crouch_board_damping;
        if self.absorbed_velocity < 0.0 {
            board = -board;
        }
        let ground = clamp(-p.ground_force / div(t.crouch_max_ground_force), -1.0, 1.0);
        let mut absorption = if board < 0.0 {
            if ground > 0.0 { ground + board } else { board }
        } else {
            board.max(ground)
        };
        if absorption < 0.0 {
            if absorption >= restoring {
                absorption = restoring;
            }
        } else {
            absorption += restoring;
        }
        // The auto pump: a stick pushed forward from rest starts a crouch, then a pump.
        let a = &t.auto_pump;
        let (sufficient, standing, rise, pump_speed) = (a[0], a[1], a[2], a[3]);
        let (potential_thresh, potential_blend, mag_start, angle_region) = (a[4], a[5], a[6], a[7]);
        let (crouch_time, crouch_speed) = (a[8], a[9]);
        let angle = i.auto_pump_angle.unwrap_or(0.0);
        let magnitude = i.auto_pump_mag.unwrap_or(0.0);
        let started = self.previous_angle.abs() < angle_region
            && self.previous_magnitude < mag_start
            && angle.abs() < angle_region
            && magnitude > mag_start;
        self.previous_angle = angle;
        self.previous_magnitude = magnitude;
        self.filtered_potential =
            p.pump_potential * potential_blend + (1.0 - potential_blend) * self.filtered_potential;
        let sufficient_potential = self.filtered_potential >= potential_thresh;
        if started {
            self.auto_time = 0.0;
            self.auto_state = 1;
        }
        if self.auto_state == 1 && self.auto_time > crouch_time {
            self.auto_state = 2;
        }
        let new_auto_pump =
            matches!(self.auto_state, 1 | 2) && self.fraction > sufficient && sufficient_potential;
        if new_auto_pump {
            self.auto_state = 3;
        }
        if matches!(self.auto_state, 2 | 3) && self.fraction <= standing + p.min_crouch {
            self.auto_state = 0;
        }
        match self.auto_state {
            0 => self.auto_fraction = 0.0,
            1 => {
                self.auto_fraction = t
                    .auto_pump_max_crouch
                    .eval(p.ground_speed)
                    .min(crouch_speed + self.auto_fraction);
                self.auto_time = (crouch_time + 1.0).min(self.auto_time + dt);
            }
            2 => self.auto_fraction = (self.auto_fraction - rise).max(0.0),
            _ => self.auto_fraction = (self.auto_fraction - pump_speed).max(0.0),
        }
        // The player's crouch: triggers, a hard turn (not while sliding), a manual's back stick while
        // sliding.
        let mut crouch = i.crouch.unwrap_or(0.0);
        if !i.sliding {
            crouch = crouch.max(i.hard_turn_crouch.unwrap_or(0.0));
        }
        if i.sliding && i.manual.is_some_and(|m| m < 0.0) {
            crouch = crouch.max(0.4);
        }
        let blend = unit(t.crouch_blend_input);
        self.filtered_input =
            crouch.max(self.auto_fraction) * blend + (1.0 - blend) * self.filtered_input;
        if self.filtered_input.abs() < 0.005 {
            self.filtered_input = 0.0;
        }
        let mut delta = t.crouch_absorption * absorption;
        if crouch > 0.0
            || self.auto_fraction > 0.0
            || (self.player_pumping && self.fraction > p.min_crouch)
        {
            self.player_pumping = true;
            delta = (p.min_crouch.max(self.filtered_input).min(1.0) - self.fraction)
                .max(-t.crouch_max_delta);
            self.pump_idle_time = 0.0;
        } else {
            self.player_pumping = false;
            // A fixed 1/60 s, as the original (not the graph's frame time).
            self.pump_idle_time += 1.0 / 60.0;
        }
        let player_pump = self.pump_idle_time < 0.5;
        let lo = -(t.crouch_pump_speed.eval(1.0 - self.fraction) * t.crouch_pump_speed_y);
        let hi = t.crouch_pump_speed.eval(self.fraction) * t.crouch_pump_speed_y;
        let delta = clamp(delta, lo, hi);
        self.delta_fraction = clamp(
            delta,
            self.delta_fraction - t.crouch_max_delta_delta,
            self.delta_fraction + t.crouch_max_delta_delta,
        );
        self.fraction = clamp(self.fraction + self.delta_fraction, p.min_crouch, 1.0);
        let from_deck = t.crouch_from_deck * p.deck_angle;
        if from_deck > self.fraction {
            self.fraction = from_deck;
        }
        let ratio = unit(1.0 - t.crouch_max_ratio * self.fraction);
        let max = t.crouch_max_height.eval(p.ground_speed);
        CrouchOut {
            height: (max - t.crouch_min_height) * ratio + t.crouch_min_height,
            new_auto_pump,
            player_pump,
        }
    }
}

/// GrindControlFade's twist (SK8 animation/grind_control.rs `Fade`): the grind trees' `twist` (here
/// across its range, 0..1, the middle straight) follows the balance intent (`GrindBalanceX` times
/// `twist_sensitivity` a tick into a target), eased by `twist_smoothing`, its step changing by at most
/// `twist_max_delta_delta` and never more than `twist_max_delta` a tick. The first update only clears
/// the begin latch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrindFade {
    just_began: bool,
    pub elapsed: f32,
    pub value: f32,
    target: f32,
    step: f32,
}

impl GrindFade {
    pub fn begin(&mut self, twist: f32) -> f32 {
        let v = twist.clamp(0.0, 1.0);
        *self = GrindFade {
            just_began: true,
            value: v,
            target: v,
            ..GrindFade::default()
        };
        v
    }

    pub fn update(&mut self, dt: f32, intent: f32, t: &crate::tuning::GrindTuning) -> Option<f32> {
        if self.just_began {
            self.just_began = false;
            return None;
        }
        self.elapsed += dt;
        self.target = (self.target + t.twist_sensitivity * intent).clamp(0.0, 1.0);
        let desired = self.value * (1.0 - t.twist_smoothing) + self.target * t.twist_smoothing;
        let step =
            (desired - self.value).clamp(self.step - t.twist_accel, self.step + t.twist_accel);
        self.step = step.clamp(-t.twist_step, t.twist_step);
        self.value = (self.value + self.step).clamp(0.0, 1.0);
        Some(self.value)
    }
}

/// ControlGrindCrouch (SK8 `crouch`): the grind trees' height between `min_grind_disttocog` and
/// `max_grind_disttocog`, the lower of what the crouch intent and the physics' grind crouch ask, moving
/// at most `grind_disttocog_speed` a second.
pub fn grind_crouch(
    previous: f32,
    intent: f32,
    physical: f32,
    t: &crate::tuning::GrindTuning,
    dt: f32,
) -> f32 {
    let (lo, hi) = (t.height_min, t.height_max.max(t.height_min));
    let input = (1.0 - intent).clamp(lo, hi);
    let body = (1.0 - physical).clamp(lo, hi);
    let desired = input.min(body);
    let step = dt * t.height_speed;
    desired.clamp(previous - step, previous + step)
}

/// SetManualAngle (SK8 manual.rs): the manual's pose angle eases towards the balance curve of the
/// C-stick (signed), rate-limited per update.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ManualAngle {
    angle: f32,
    velocity: f32,
}

impl ManualAngle {
    pub fn update(&mut self, manual: Option<f32>, t: &AnimTuning) -> f32 {
        let manual = manual.unwrap_or(0.0);
        let target = t.manual_balance.eval(manual.abs());
        let target = if manual >= 0.0 { target } else { -target };
        let desired = clamp(target - self.angle, -t.manual_vel, t.manual_vel);
        let acc = clamp(desired - self.velocity, -t.manual_acc, t.manual_acc);
        self.velocity += acc;
        self.angle += self.velocity;
        self.angle
    }
}

/// StoreLandingData (SK8 motion_native.rs LandingData): the fastest the centre of mass has come down
/// since the rider left the ground (m/s), for the landing trees' `AVGVELY`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LandingStore {
    previous: f32,
    was_air: bool,
    pub landing_velocity: f32,
}

impl LandingStore {
    /// `vel_up`: the centre of mass's speed along the surface's up.
    pub fn update(&mut self, air: bool, vel_up: f32) {
        if !self.was_air && air {
            self.landing_velocity = 0.0;
        }
        self.was_air = air;
        if vel_up < 0.0 && vel_up < self.previous {
            self.landing_velocity = vel_up.abs();
        }
        self.previous = vel_up;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: AnimTuning = AnimTuning::BUILT_IN;

    fn steady(turn: f32, ticks: usize) -> TurnOutput {
        let mut c = Conditioner::default();
        let mut out = [0.0; 8];
        for _ in 0..ticks {
            out = c.update(
                TurnInput {
                    speed: 5.0,
                    turn,
                    lean: turn * 0.3,
                    ground: true,
                    ..TurnInput::default()
                },
                &T,
            );
        }
        out
    }

    #[test]
    fn riding_straight_is_centred_and_held() {
        let c = steady(0.0, 120);
        assert_eq!(c[0], 0.0);
        assert!(c[7] > 0.9, "holding {}", c[7]);
        let mut s = Turning::default();
        let mut v = TurnValues::default();
        for _ in 0..60 {
            v = s.update(&c, TurnContext::default(), 1.0 / 60.0, &T);
        }
        assert_eq!(v.angle, 0.0);
        assert_eq!(v.speed_tuck, 0.0);
    }

    #[test]
    fn a_held_carve_leans_one_way_and_the_other_carve_the_other() {
        let left = steady(1.0, 240);
        let right = steady(-1.0, 240);
        assert!(left[0] > 0.2, "{left:?}");
        assert!((left[0] + right[0]).abs() < 1e-4, "symmetric");
        let lean = |c: &TurnOutput| {
            let mut s = Turning::default();
            let mut v = TurnValues::default();
            for _ in 0..120 {
                v = s.update(c, TurnContext::default(), 1.0 / 60.0, &T);
            }
            v.angle
        };
        let (l, r) = (lean(&left), lean(&right));
        assert!(l.abs() > 0.2 && l * r < 0.0, "{l} {r}");
        // Mirrored riders lean the same way for the same physical turn: the stance sign comes in and
        // goes out again.
        let mut c = Conditioner::default();
        let mut m = [0.0; 8];
        for _ in 0..240 {
            m = c.update(
                TurnInput {
                    speed: 5.0,
                    turn: 1.0,
                    lean: 0.3,
                    ground: true,
                    mirrored: true,
                    ..TurnInput::default()
                },
                &T,
            );
        }
        assert!((m[3] - left[3]).abs() < 1e-4 && m[0] * left[0] < 0.0);
    }

    #[test]
    fn powerslides_hold_the_lean_to_their_side_and_fast_riding_tucks() {
        let c = steady(0.0, 10);
        let mut s = Turning::default();
        let x = TurnContext {
            slide: Some(true),
            slide_turn_value: 0.4,
            forward_speed: 14.0,
            ..TurnContext::default()
        };
        let mut v = TurnValues::default();
        for _ in 0..60 {
            v = s.update(&c, x, 1.0 / 60.0, &T);
        }
        assert!((v.angle - 1.0).abs() < 1e-4, "{}", v.angle);
        assert_eq!(v.turn, Some(0.4));
        assert!(v.speed_tuck > 0.0);
    }

    #[test]
    fn identity_remaps_keep_the_vector() {
        let r = remap(
            &Curve::constant(1.0),
            &Curve::new(&[(0.0, 0.0), (PI, PI)]),
            0.0,
            [0.3, -0.4],
        );
        assert!(
            (r[0] - 0.3).abs() < 1e-5 && (r[1] + 0.4).abs() < 1e-5,
            "{r:?}"
        );
    }

    #[test]
    fn body_tilt_stays_flat_on_flat_ground_and_reaches_a_wall() {
        let mut b = BodyTilt::default();
        for _ in 0..60 {
            assert_eq!(b.update(true, 0.0, 0.0, false, &T), 0.0);
        }
        let mut v = 0.0;
        for _ in 0..600 {
            v = b.update(true, 1.0, 0.0, false, &T);
        }
        assert!((v - 1.0).abs() < 0.05, "{v}");
        // Not mirrored: the other way.
        let mut b = BodyTilt::default();
        for _ in 0..600 {
            v = b.update(false, 1.0, 0.0, false, &T);
        }
        assert!(v < -0.9);
    }

    #[test]
    fn crouch_follows_the_trigger_and_stands_back_up() {
        let p = CrouchPhysical {
            ground_speed: 4.0,
            height: 0.96,
            ..CrouchPhysical::default()
        };
        let mut c = Crouch::begin(p, &T);
        let mut h = 0.0;
        for _ in 0..30 {
            h = c.update(p, CrouchIntents::default(), 1.0 / 60.0, &T).height;
        }
        assert!((h - 0.96).abs() < 0.01, "standing {h}");
        for _ in 0..120 {
            h = c
                .update(
                    p,
                    CrouchIntents {
                        crouch: Some(1.0),
                        ..CrouchIntents::default()
                    },
                    1.0 / 60.0,
                    &T,
                )
                .height;
        }
        assert!((h - T.crouch_min_height).abs() < 0.02, "crouched {h}");
        for _ in 0..240 {
            h = c.update(p, CrouchIntents::default(), 1.0 / 60.0, &T).height;
        }
        assert!(h > 0.9, "back up {h}");
    }

    #[test]
    fn manual_angle_eases_to_the_balance_curve() {
        let mut m = ManualAngle::default();
        let mut a = 0.0;
        for _ in 0..200 {
            a = m.update(Some(-1.0), &T);
        }
        assert!((a + 1.0).abs() < 0.01, "{a}");
    }

    #[test]
    fn landing_velocity_is_the_fastest_descent_since_leaving_the_ground() {
        let mut l = LandingStore::default();
        l.update(false, 0.0);
        l.update(true, 2.0);
        for v in [1.0, -1.0, -3.0, -2.5] {
            l.update(true, v);
        }
        assert_eq!(l.landing_velocity, 3.0);
        l.update(false, 0.0);
        l.update(true, 1.0);
        assert_eq!(l.landing_velocity, 0.0, "reset on leaving the ground");
    }
}
