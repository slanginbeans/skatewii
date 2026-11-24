//! Tuning for the moves of phase 8: body flips (Skate 3's `physics_reckoning` flip fields, the start
//! curve and grab time of `physics_airstates`, the upside-down fall of `physics_wipeout`, the gesture
//! windows of `anim_motion/body_flip`), the pump channel (`anim_motion/anim_pump`), bumps
//! (`anim_motion/bumps`), the powerslide's pose (`anim_motion/power_slide`) and the wipeout's lean and
//! twist (`anim_wipeout`). The built-in values are flatground's own; the player's disc supplies
//! Skate 3's through the rules pack (`fg_import rules`).

use super::Curve;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveTuning {
    // Body flips (physics).
    /// The flight (s) a flip needs before it starts, against the landing surface's up y.
    pub flip_start_time: Curve,
    /// A flip keeps going this long (s into the air) even without the grab.
    pub flip_min_grab_time: f32,
    /// Turns over the flight: the flip's speed is this x 2 pi over the flight's time.
    pub flip_scalar: f32,
    /// Share of the way to the wanted flip speed a tick, and the fastest flip (rad/s).
    pub flip_smoothing: f32,
    pub flip_max_speed: f32,
    /// The body spin's share while flipping.
    pub flip_spin_scale: f32,
    /// Upside down without the grab: off when the body's up y is under `fall_min_up_y` and its dot with
    /// the landing surface's up under `fall_max_dot`.
    pub fall_min_up_y: f32,
    pub fall_max_dot: f32,
    /// That bail is checked (the physics mode's `Hash_2B913C786BEFF4CD`; off on Easy).
    pub upside_down_bail: bool,
    /// The physics mode's `PerfectBodyFlips` (Easy): the flip's speed is `2.4 pi` over the flight
    /// whatever the grab, slowed so it ends exactly one turn round (SK8 known/trajectory.rs
    /// `InitTrajectoryInfo`, known/body.rs `CalculateBodyFlipSpeed`, body_flip.rs `UpdateBodyFlip`).
    pub perfect_body_flips: bool,
    // Body flips (the action graph's BodyFlippingSignal).
    /// A flip gesture counts this long (s), and only this long after take-off (s).
    pub flip_gesture_window: f32,
    pub flip_takeoff_window: f32,

    // The pump channel (Pumping).
    /// The pump layer's weight against the filtered pumping input (0..1).
    pub pump_amplify: Curve,
    /// Share of the new input a tick, the pumping acceleration that is full input, the weight that
    /// starts a new pump, and the channel's fades (s).
    pub pump_blend: f32,
    pub pump_max: f32,
    pub pump_new: f32,
    pub pump_blend_in: f32,
    pub pump_blend_out: f32,

    // Bumps (IsBumped, SetBumpCoefficients).
    /// The sideways part of the board's acceleration counts this much; bumped past `bump_min`
    /// (m/s^2), the bump trees' weight from `bump_min_blend` at `bump_min` to full at `bump_max`.
    pub bump_scale_x: f32,
    pub bump_min: f32,
    pub bump_max: f32,
    pub bump_min_blend: f32,

    // The powerslide's pose (PowerSlideSpin, PowerSlideDecel).
    /// The slide tree's lean against the speed along the ground (m/s), and its smoothing.
    pub slide_speed_to_lean: Curve,
    pub slide_smooth: f32,
    /// The slide tree's spin against the travel along the board (-1 backwards .. 1 forwards).
    pub slide_spin: Curve,

    // The wipeout's body (Wipeout).
    /// Rate limits (rad a tick) and blends of the lean and twist the bail trees read, the stick's
    /// blends into the wipeout gesture and the gesture size that is a controlled wipeout.
    pub wipe_lean_vel: f32,
    pub wipe_lean_blend: f32,
    pub wipe_twist_vel: f32,
    pub wipe_twist_acc: f32,
    pub wipe_twist_blend: f32,
    pub wipe_gesture_x_blend: f32,
    pub wipe_gesture_y_blend: f32,
    pub wipe_gesture_threshold: f32,
}

impl MoveTuning {
    /// flatground's own values.
    pub const BUILT_IN: MoveTuning = MoveTuning {
        flip_start_time: Curve::new(&[(0.0, 1.3), (0.5, 1.15), (0.9, 0.9), (1.0, 0.85)]),
        flip_min_grab_time: 0.25,
        flip_scalar: 1.2,
        flip_smoothing: 0.1,
        flip_max_speed: 6.0,
        flip_spin_scale: 0.6,
        fall_min_up_y: -0.6,
        fall_max_dot: -0.5,
        upside_down_bail: super::ModeTuning::NORMAL.upside_down_bail,
        perfect_body_flips: super::ModeTuning::NORMAL.perfect_body_flips,
        flip_gesture_window: 0.15,
        flip_takeoff_window: 0.4,
        pump_amplify: Curve::new(&[(0.0, 0.0), (0.3, 0.2), (1.0, 1.0)]),
        pump_blend: 0.25,
        pump_max: 0.25,
        pump_new: 0.06,
        pump_blend_in: 0.15,
        pump_blend_out: 0.3,
        bump_scale_x: 1.0,
        bump_min: 120.0,
        bump_max: 300.0,
        bump_min_blend: 0.4,
        slide_speed_to_lean: Curve::new(&[(0.0, 0.0), (4.0, 0.5), (10.0, 1.0)]),
        slide_smooth: 0.1,
        slide_spin: Curve::new(&[(-1.0, -1.0), (1.0, 1.0)]),
        wipe_lean_vel: 0.1,
        wipe_lean_blend: 0.2,
        wipe_twist_vel: 0.1,
        wipe_twist_acc: 0.02,
        wipe_twist_blend: 0.2,
        wipe_gesture_x_blend: 0.2,
        wipe_gesture_y_blend: 0.2,
        wipe_gesture_threshold: 0.5,
    };

    /// Takes over a value from the rules pack (`class/key/field`). Returns whether it was one of these.
    pub(super) fn set(&mut self, name: &str, f: Option<f32>, c: Option<Curve>) -> bool {
        macro_rules! scalar {
            ($field:expr) => {
                match f {
                    Some(f) => {
                        $field = f;
                        true
                    }
                    None => false,
                }
            };
        }
        macro_rules! curve {
            ($field:expr) => {
                match c {
                    Some(c) => {
                        $field = c;
                        true
                    }
                    None => false,
                }
            };
        }
        match name {
            "physics_airstates/default/Hash_AA79C93673533C5B" => curve!(self.flip_start_time),
            "physics_airstates/default/BodyFlipMinGrabTimeFraction" => {
                scalar!(self.flip_min_grab_time)
            }
            "physics_reckoning/default/FlipScalar" => scalar!(self.flip_scalar),
            "physics_reckoning/default/FlipSpeedSmoothingFactor" => scalar!(self.flip_smoothing),
            "physics_reckoning/default/FlipMaxSpeed" => scalar!(self.flip_max_speed),
            "physics_reckoning/default/FlipBodySpinScalar" => scalar!(self.flip_spin_scale),
            "physics_wipeout/default/Wipeout_AirFallingMinUpY" => scalar!(self.fall_min_up_y),
            "physics_wipeout/default/Wipeout_AirFallingMaxAngle" => scalar!(self.fall_max_dot),
            "anim_motion/body_flip/extend_bodyflip_gesture" => scalar!(self.flip_gesture_window),
            "anim_motion/body_flip/extend_takeoff_point" => scalar!(self.flip_takeoff_window),
            "anim_motion/anim_pump/pump_amplify" => curve!(self.pump_amplify),
            "anim_motion/anim_pump/pump_prop_blend" => scalar!(self.pump_blend),
            "anim_motion/anim_pump/max_phys_pump" => scalar!(self.pump_max),
            "anim_motion/anim_pump/new_pump_thresh" => scalar!(self.pump_new),
            "anim_motion/anim_pump/pump_blendin" => scalar!(self.pump_blend_in),
            "anim_motion/anim_pump/pump_blendout" => scalar!(self.pump_blend_out),
            "anim_motion/bumps/scale_x_acc" => scalar!(self.bump_scale_x),
            "anim_motion/bumps/min_bump_mag" => scalar!(self.bump_min),
            "anim_motion/bumps/max_bump_mag" => scalar!(self.bump_max),
            "anim_motion/bumps/min_bump_blend_value" => scalar!(self.bump_min_blend),
            "anim_motion/power_slide/slide_speed_to_lean" => curve!(self.slide_speed_to_lean),
            "anim_motion/power_slide/slide_smooth" => scalar!(self.slide_smooth),
            "anim_motion/power_slide/slide_phys_to_anim_spin" => curve!(self.slide_spin),
            "anim_wipeout/default/clamp_lean_vel" => scalar!(self.wipe_lean_vel),
            "anim_wipeout/default/blend_lean" => scalar!(self.wipe_lean_blend),
            "anim_wipeout/default/clamp_twist_vel" => scalar!(self.wipe_twist_vel),
            "anim_wipeout/default/clamp_twist_acc" => scalar!(self.wipe_twist_acc),
            "anim_wipeout/default/blend_twist" => scalar!(self.wipe_twist_blend),
            "anim_wipeout/default/blend_gesture_x" => scalar!(self.wipe_gesture_x_blend),
            "anim_wipeout/default/blend_gesture_y" => scalar!(self.wipe_gesture_y_blend),
            "anim_wipeout/default/controlled_drives_thresh" => scalar!(self.wipe_gesture_threshold),
            _ => false,
        }
    }
}

/// Where each value lives in Skate 3's tuning database (`class/key/field`).
pub const PATHS: &[&str] = &[
    "physics_airstates/default/Hash_AA79C93673533C5B",
    "physics_airstates/default/BodyFlipMinGrabTimeFraction",
    "physics_reckoning/default/FlipScalar",
    "physics_reckoning/default/FlipSpeedSmoothingFactor",
    "physics_reckoning/default/FlipMaxSpeed",
    "physics_reckoning/default/FlipBodySpinScalar",
    "physics_wipeout/default/Wipeout_AirFallingMinUpY",
    "physics_wipeout/default/Wipeout_AirFallingMaxAngle",
    "anim_motion/body_flip/extend_bodyflip_gesture",
    "anim_motion/body_flip/extend_takeoff_point",
    "anim_motion/anim_pump/pump_amplify",
    "anim_motion/anim_pump/pump_prop_blend",
    "anim_motion/anim_pump/max_phys_pump",
    "anim_motion/anim_pump/new_pump_thresh",
    "anim_motion/anim_pump/pump_blendin",
    "anim_motion/anim_pump/pump_blendout",
    "anim_motion/bumps/scale_x_acc",
    "anim_motion/bumps/min_bump_mag",
    "anim_motion/bumps/max_bump_mag",
    "anim_motion/bumps/min_bump_blend_value",
    "anim_motion/power_slide/slide_speed_to_lean",
    "anim_motion/power_slide/slide_smooth",
    "anim_motion/power_slide/slide_phys_to_anim_spin",
    "anim_wipeout/default/clamp_lean_vel",
    "anim_wipeout/default/blend_lean",
    "anim_wipeout/default/clamp_twist_vel",
    "anim_wipeout/default/clamp_twist_acc",
    "anim_wipeout/default/blend_twist",
    "anim_wipeout/default/blend_gesture_x",
    "anim_wipeout/default/blend_gesture_y",
    "anim_wipeout/default/controlled_drives_thresh",
];
