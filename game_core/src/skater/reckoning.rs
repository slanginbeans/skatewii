//! Skate 3's reckoning on the ground: the rider's up vector, filtered from the ground's normal and the
//! deck's tilt, and his lean into a turn (the angle from that up to the ground's normal about his
//! heading), which the riding animation's turn conditioner reads.
//!
//! Port of SK8-ENGINE/skate-3-rust-engine (GPL-3.0): crates/skate-core/src/riding/ground_orientation.rs
//! (`GroundOrientation::update`, TU3 Reckoning 82D8C8F0), riding/reckoning_frames.rs
//! (`calculate_transform`, `calculate_dynamic_lean`) and air/ground_normal.rs (the vector filter
//! 82D7A018). SK8 matches the original's fused multiply-adds bit for bit; this port uses plain
//! arithmetic. The body-flip frame and the coffin's up-behind-the-board restriction are left out (the
//! lean is cleared off the ground, as SK8's air and grind reckonings clear it).

use crate::math::{PI, Vec3};
use crate::tuning::reckoning::ReckoningTuning;

const UP: Vec3 = Vec3 {
    x: 0.0,
    y: 1.0,
    z: 0.0,
};

/// Skate 3's vector filter: the output follows the input by gain `c[0]` of the error, `c[1]` of its
/// running blend and `c[2]` of the input's change against the output's, the blends taking `c[3]` a
/// step (SK8 `GroundNormalFilter::filter`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Filter {
    current: Vec3,
    previous_input: Vec3,
    error_blend: Vec3,
    output_delta: Vec3,
    input_blend: Vec3,
}

impl Filter {
    const fn new(seed: Vec3) -> Filter {
        let zero = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        Filter {
            current: seed,
            previous_input: seed,
            error_blend: zero,
            output_delta: zero,
            input_blend: zero,
        }
    }

    fn filter(&mut self, c: [f32; 4], input: Vec3) -> Vec3 {
        let blend = c[3];
        let retained = 1.0 - blend;
        let error = input - self.current;
        self.error_blend = self.error_blend * retained + error * blend;
        let input_delta = input - self.previous_input;
        self.input_blend = self.input_blend * retained + input_delta * blend;
        let predicted = self.current + self.error_blend * c[1] + error * c[0];
        let output = predicted + (self.input_blend - self.output_delta) * c[2];
        self.output_delta = output - self.current;
        self.current = output;
        self.previous_input = input;
        output
    }
}

/// Skate 3's dynamic normal (TU3 82C02388, SK8 physics/board_dynamic_normal.rs, its Ground64): the
/// direction of the wheels' acceleration against gravity (the way the ground pushes the board, leaning
/// into a turn), followed by a damped, speed-limited step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynamicNormal {
    pub normal: Vec3,
    delta: Vec3,
}

impl Default for DynamicNormal {
    fn default() -> Self {
        // The original's constructor really starts the step at up.
        DynamicNormal {
            normal: UP,
            delta: UP,
        }
    }
}

impl DynamicNormal {
    /// One tick with the wheels down (`wheels` > 0): `acceleration` is the board's (m/s^2), `gravity`
    /// its (m/s^2, down), `previous_speed` the last tick's (m/s).
    pub fn update(
        &mut self,
        t: &ReckoningTuning,
        wheels: i32,
        acceleration: Vec3,
        gravity: f32,
        previous_speed: f32,
    ) {
        if wheels == 0 {
            return;
        }
        let target = normalize_or(acceleration + UP * gravity, Vec3::ZERO);
        self.delta = self.delta * t.dynamic_speed_damping;
        let error = (target - self.normal) * t.dynamic_up_damping;
        let magnitude = error.length();
        let direction = normalize_or(error, Vec3::ZERO);
        let max = t
            .dynamic_max_delta_vs_speed
            .eval(previous_speed.abs() / t.dynamic_speed_scale)
            * t.dynamic_max_delta;
        self.delta += direction * magnitude.clamp(0.0, max.max(0.0));
        self.normal = normalize_or(self.normal + self.delta, Vec3::ZERO);
    }
}

/// What the ground physics tells the reckoning each tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReckoningInput {
    /// From the rider's centre of mass to the deck (world).
    pub com_to_deck: Vec3,
    /// The wheels' ground normal and Skate 3's dynamic up (`DynamicNormal`).
    pub ground_normal: Vec3,
    pub dynamic_up: Vec3,
    /// The deck's speed (m/s).
    pub speed: f32,
    /// Wheels on the ground (4 rolling, 2 in a manual, 0 lifted).
    pub wheels: i32,
    /// A manual's balance (the deck's tilt is used only with none).
    pub balance: f32,
    /// The deck's up (with its tilt) and nose.
    pub board_up: Vec3,
    pub board_forward: Vec3,
    /// The rider's heading (his frame's forward).
    pub heading: Vec3,
}

/// The ground reckoning's state (SK8 `GroundOrientation` and the frames' heading, right and lean).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reckoning {
    pub up: Vec3,
    pub up_velocity: Vec3,
    pub target: Vec3,
    pub ground_normal: Vec3,
    pub dynamic_up: Vec3,
    ground_blend: f32,
    ground_filter: Filter,
    slow_filter: Filter,
    fast_filter: Filter,
    heading: Vec3,
    right: Vec3,
    /// The lean (rad, SK8 `target_lean_angle`): from the rider's up to the ground's normal about his
    /// heading.
    pub lean: f32,
}

impl Default for Reckoning {
    fn default() -> Self {
        Reckoning {
            up: UP,
            up_velocity: Vec3::ZERO,
            target: UP,
            ground_normal: UP,
            dynamic_up: UP,
            ground_blend: 0.0,
            ground_filter: Filter::new(UP),
            slow_filter: Filter::new(UP),
            fast_filter: Filter::new(UP),
            heading: Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            right: Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            lean: 0.0,
        }
    }
}

fn normalize_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let len = v.length();
    if len > 1e-6 {
        v * (1.0 / len)
    } else {
        fallback
    }
}

fn blend(from: Vec3, to: Vec3, amount: f32) -> Vec3 {
    to * amount + from * (1.0 - amount)
}

fn angle_between(a: Vec3, b: Vec3) -> f32 {
    let a = normalize_or(a, Vec3::ZERO);
    let b = normalize_or(b, Vec3::ZERO);
    libm::acosf(a.dot(b).clamp(-1.0, 1.0))
}

/// The angle from `a` to `b` turning about `axis`, 0..2 pi (SK8 collision_response.rs `signed_angle`).
fn signed_angle(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    if a.dot(a) <= 1e-4 || b.dot(b) <= 1e-4 {
        return 0.0;
    }
    let angle = angle_between(a, b);
    if a.cross(b).dot(axis) < 0.0 {
        2.0 * PI - angle
    } else {
        angle
    }
}

/// The signed angle from `a` to `b` about `axis`, -pi..pi (SK8 ground_runtime/pumping.rs's
/// `wrap(signed_angle(..))`).
pub fn wrapped_angle(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    wrap(signed_angle(a, b, axis))
}

/// Into -pi..pi (SK8 reckoning_frames.rs `wrap_fraction`).
fn wrap(angle: f32) -> f32 {
    let turns = angle / (2.0 * PI);
    let mut fraction = turns - libm::floorf(turns);
    if fraction > 0.5 {
        fraction -= 1.0;
    }
    fraction * 2.0 * PI
}

fn clamp_length(v: Vec3, max: f32) -> Vec3 {
    let len = v.length();
    if len < 1.525_878_9e-5 || len <= max {
        v
    } else {
        v * (max / len)
    }
}

impl Reckoning {
    /// The rider's heading, square to his up.
    pub fn heading(&self) -> Vec3 {
        self.heading
    }

    /// The ground frame's right (SK8 `ReckoningFrames::ground[0]`: the smoothed ground normal across
    /// the heading).
    pub fn ground_right(&self) -> Vec3 {
        normalize_or(self.ground_normal.cross(self.heading), self.right)
    }

    /// One ground tick (SK8 `GroundOrientation::update`, then the frames' transform and dynamic lean).
    pub fn update(&mut self, t: &ReckoningTuning, i: ReckoningInput) {
        let com = normalize_or(i.com_to_deck, Vec3::ZERO);
        let amount = t.dynamic_up_vs_ground_y.eval(i.ground_normal.y);
        let prediction = normalize_or(
            blend(self.ground_normal, UP + (i.dynamic_up - com), amount),
            Vec3::ZERO,
        );
        self.dynamic_up = i.dynamic_up;
        self.ground_normal = normalize_or(
            self.ground_filter
                .filter(t.ground_normal_smoothing, i.ground_normal),
            UP,
        );
        self.ground_filter.current = self.ground_normal;
        let horizontal = UP.cross(self.ground_normal);
        if horizontal.dot(horizontal) > 0.001 {
            let axis = normalize_or(horizontal, Vec3::ZERO);
            let projected = prediction - axis * axis.dot(prediction);
            let prediction_to_ground = angle_between(projected, self.ground_normal);
            let prediction_to_up = angle_between(projected, UP);
            let ground_to_up = angle_between(self.ground_normal, UP);
            let old_up_to_up = angle_between(self.up, UP);
            let mut candidate = if prediction_to_ground < prediction_to_up {
                self.ground_normal
            } else {
                UP
            };
            if prediction_to_ground < ground_to_up && prediction_to_up < ground_to_up {
                candidate = projected;
            }
            let alpha = (i.speed + 1.0) * 0.001;
            if !(old_up_to_up > ground_to_up || prediction_to_up > old_up_to_up) {
                candidate = self.up * (1.0 - alpha) + candidate * alpha;
            }
            let slope = (ground_to_up / PI).clamp(0.0, 1.0);
            let mut target_blend = t.ground_vector_blend.eval(slope);
            if i.wheels <= t.min_wheels_for_ground_blend {
                target_blend = 0.0;
            }
            let delta = (target_blend - self.ground_blend)
                .clamp(-t.ground_blend_max_delta, t.ground_blend_max_delta);
            self.ground_blend += delta;
            self.target = normalize_or(
                blend(candidate, self.ground_normal, self.ground_blend),
                Vec3::ZERO,
            );
        } else {
            self.target = self.ground_normal;
        }
        if i.wheels >= 2 && i.balance == 0.0 {
            // Part of the deck's own tilt, more the faster (`DeckAngleUsageVsSpeed`).
            let usage = t.deck_angle_usage_vs_speed.eval(i.speed);
            let axis = normalize_or(i.board_forward.cross(self.ground_normal), Vec3::ZERO);
            let deck_up = normalize_or(i.board_up - axis * i.board_up.dot(axis), Vec3::ZERO);
            self.target = normalize_or(blend(self.target, deck_up, usage), Vec3::ZERO);
        }
        let slow = normalize_or(
            self.slow_filter.filter(t.up_smoothing_slow, self.target),
            Vec3::ZERO,
        );
        let fast = normalize_or(
            self.fast_filter.filter(t.up_smoothing_fast, self.target),
            Vec3::ZERO,
        );
        let speed_fraction = i.speed * 0.1;
        let mix = t.up_smoothing_vs_speed.eval(speed_fraction);
        let filtered = normalize_or(blend(slow, fast, mix), Vec3::ZERO);
        let max_delta = t.up_max_delta_vs_speed.eval(speed_fraction) * 0.2;
        let difference = filtered - self.up;
        let bounded = difference.length().min(max_delta);
        let stepped = normalize_or(
            normalize_or(difference, Vec3::ZERO) * bounded + self.up,
            Vec3::ZERO,
        );
        let desired = stepped - self.up;
        let acceleration = clamp_length(desired - self.up_velocity, t.up_max_acceleration / 60.0);
        self.up_velocity += acceleration;
        let side = self.right * self.up_velocity.dot(self.right);
        self.up_velocity = side * t.extra_side_damping + (self.up_velocity - side);
        let moved = self.up + self.up_velocity;
        self.up = normalize_or(moved, moved);
        if self.up_velocity.dot(desired) < 0.0 {
            self.up_velocity = self.up_velocity * t.anti_wobble_damping;
        }
        self.slow_filter.current = self.up;
        self.fast_filter.current = self.up;

        // The frames (SK8 `calculate_transform`): the heading kept square to the up.
        let heading = normalize_or(i.heading, self.heading);
        let right = self.up.cross(heading);
        self.heading = normalize_or(right.cross(self.up), self.heading);
        self.right = normalize_or(right, self.right);
        // The dynamic lean (SK8 `calculate_dynamic_lean`).
        let axis = self.heading;
        let from = self.up - axis * self.up.dot(axis);
        let to = self.dynamic_up - axis * self.dynamic_up.dot(axis);
        self.lean = if from.dot(from) > 1e-5 && to.dot(to) > 1e-5 {
            wrap(signed_angle(from, to, axis))
        } else {
            0.0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;

    /// Rides a steady turn on flat ground: `side` m/s^2 of centripetal acceleration towards the
    /// board's left (positive) or right.
    fn ride(r: &mut Reckoning, d: &mut DynamicNormal, side: f32, speed: f32, ticks: u32) {
        let t = ReckoningTuning::BUILT_IN;
        let fwd = v3(0.0, 0.0, 1.0);
        let left = fwd.cross(UP);
        for _ in 0..ticks {
            d.update(&t, 4, left * side, 9.8, speed);
            r.update(
                &t,
                ReckoningInput {
                    com_to_deck: -UP,
                    ground_normal: UP,
                    dynamic_up: d.normal,
                    speed,
                    wheels: 4,
                    balance: 0.0,
                    board_up: UP,
                    board_forward: fwd,
                    heading: fwd,
                },
            );
        }
    }

    #[test]
    fn on_flat_ground_the_lean_is_the_turns_pull_against_gravity() {
        // SK8 (a full carve at 6.5 m/s, 11.4 m/s^2 into the turn): the rider's up stays the world's,
        // the dynamic normal leans into the turn and the lean settles at 0.86 to 0.88 rad.
        let mut r = Reckoning::default();
        let mut d = DynamicNormal::default();
        ride(&mut r, &mut d, 11.4, 6.5, 120);
        assert!(r.up.y > 0.9999, "{:?}", r.up);
        let steady = libm::atanf(11.4 / 9.8);
        assert!(
            (r.lean.abs() - steady).abs() < 0.02,
            "{} against {steady}",
            r.lean
        );
        let mut other = Reckoning::default();
        let mut d2 = DynamicNormal::default();
        ride(&mut other, &mut d2, -11.4, 6.5, 120);
        assert!((other.lean + r.lean).abs() < 1e-3, "symmetric");
        // Straight again it comes back within a few tenths of a second.
        ride(&mut r, &mut d, 0.0, 6.5, 30);
        assert!(r.lean.abs() < 0.05, "{}", r.lean);
    }

    #[test]
    fn the_dynamic_normal_steps_no_further_than_its_limit() {
        let t = ReckoningTuning::BUILT_IN;
        let mut d = DynamicNormal::default();
        // Settle the constructor's start.
        for _ in 0..60 {
            d.update(&t, 4, Vec3::ZERO, 9.8, 6.5);
        }
        let before = d.normal;
        d.update(&t, 4, v3(30.0, 0.0, 0.0), 9.8, 6.5);
        let max = t.dynamic_max_delta_vs_speed.eval(6.5 / 24.0) * t.dynamic_max_delta;
        assert!((d.normal - before).length() <= max + 1e-4);
        // Lifted, it holds.
        let held = d.normal;
        d.update(&t, 0, v3(0.0, 30.0, 0.0), 9.8, 6.5);
        assert_eq!(d.normal, held);
    }
}
